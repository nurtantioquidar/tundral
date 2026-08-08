import type {
  ModelRoute,
  ProviderModelCapability,
  RoutingProvider,
  StoredUpstreamCredential,
} from "../database";
import { codexConversationFromRequest, codexPayload, type CodexResult } from "./adapters/codex";
import type { AppServerCallbacks } from "./adapters/codex-app-server";
import {
  OpenAiUpstreamError,
  OpenAiUpstreamPool,
  type FetchLike,
  type OpenAiProviderConfig,
} from "./adapters/openai-pool";
import { MODEL_RATES } from "../billing/pricing";
import { requestedReasoningEffort, type ReasoningEffort } from "./reasoning-effort";

export interface ModelRouterGraph {
  providers: RoutingProvider[];
  credentials: StoredUpstreamCredential[];
  capabilities: ProviderModelCapability[];
  routes: ModelRoute[];
}

export interface ModelRouterRequest {
  path: "/v1/chat/completions" | "/v1/responses";
  publicModel: string;
  body: Record<string, unknown>;
  outputLimit: number;
  callbacks?: AppServerCallbacks;
}

export interface ModelRouterResult {
  payload: Record<string, unknown>;
  status: number;
  routeRevision: number;
  routeTargetId: string;
  providerId: string;
  credentialId?: string;
}

export type CodexExecutor = (
  model: string,
  reasoningEffort: ReasoningEffort | undefined,
  maxOutputTokens: number,
  conversation: ReturnType<typeof codexConversationFromRequest>,
  tools: unknown,
  callbacks: AppServerCallbacks,
) => Promise<CodexResult>;

interface RuntimeProvider {
  config: RoutingProvider;
  inflight: number;
}

interface RuntimeTarget {
  id: string;
  priority: number;
  weight: number;
  maxConcurrency: number;
  inflight: number;
  cooldownUntil: number;
  provider: RuntimeProvider;
  capability: ProviderModelCapability;
  pool: OpenAiUpstreamPool | null;
}

interface RuntimeRoute {
  publicModelId: string;
  revision: number;
  tiers: Map<number, RuntimeTarget[]>;
  schedules: Map<number, RuntimeTarget[]>;
  cursors: Map<number, number>;
}

export class ModelRouteError extends Error {
  constructor(message: string, readonly status = 502, readonly type = "upstream_error") {
    super(message);
  }
}

function pathCapability(path: ModelRouterRequest["path"]): ProviderModelCapability["apiPaths"][number] {
  return path === "/v1/chat/completions" ? "chat_completions" : "responses";
}

function isRetryableFailure(cause: unknown): boolean {
  if (!(cause instanceof OpenAiUpstreamError)) return true;
  return cause.status >= 500 || [408, 409, 425, 429].includes(cause.status);
}

export class ModelRouter {
  private readonly runtimeRoutes = new Map<string, RuntimeRoute>();
  private readonly runtimeProviders = new Map<string, RuntimeProvider>();
  private readonly cooldownMs: number;

  constructor(
    graph: ModelRouterGraph,
    private readonly decryptCredential: (credential: StoredUpstreamCredential) => string,
    private readonly codexExecutor: CodexExecutor,
    private readonly codexAvailable: boolean,
    private readonly environment: Record<string, string | undefined> = process.env,
    private readonly fetchImpl: FetchLike = fetch,
    private readonly now: () => number = Date.now,
  ) {
    this.cooldownMs = Number(environment.UPSTREAM_FAILURE_COOLDOWN_MS || 30_000);
    if (!Number.isSafeInteger(this.cooldownMs) || this.cooldownMs <= 0) {
      throw new Error("UPSTREAM_FAILURE_COOLDOWN_MS must be a positive integer");
    }
    this.build(graph);
  }

  private build(graph: ModelRouterGraph): void {
    const capabilities = new Map(graph.capabilities.map((capability) => [capability.id, capability]));
    const credentialsByProvider = new Map<string, StoredUpstreamCredential[]>();
    for (const credential of graph.credentials) {
      const existing = credentialsByProvider.get(credential.providerId) ?? [];
      existing.push(credential);
      credentialsByProvider.set(credential.providerId, existing);
    }
    for (const provider of graph.providers) {
      this.runtimeProviders.set(provider.id, { config: provider, inflight: 0 });
    }

    for (const route of graph.routes) {
      const rate = MODEL_RATES[route.publicModelId];
      /* A stored route for a model the rate card no longer sells (for example a retired alias) goes
       * dormant instead of preventing startup. Publishing still rejects unknown models explicitly in
       * UpstreamRegistry.publishRoute. */
      if (!rate) continue;
      if (!route.enabled) continue;
      const tiers = new Map<number, RuntimeTarget[]>();

      for (const target of route.targets) {
        if (!target.enabled) continue;
        const capability = capabilities.get(target.capabilityId);
        if (!capability) throw new Error(`Model route target ${target.id} has no capability`);
        if (capability.canonicalModelId !== rate.upstreamModel) {
          throw new Error(
            `Model route ${route.publicModelId} cannot use capability ${capability.id}: canonical model must be ${rate.upstreamModel}`,
          );
        }
        const provider = this.runtimeProviders.get(capability.providerId);
        if (!provider) throw new Error(`Capability ${capability.id} has no provider`);
        if (!provider.config.enabled || !capability.enabled) continue;

        let pool: OpenAiUpstreamPool | null = null;
        if (provider.config.transport === "openai_http") {
          if (!provider.config.baseUrl) throw new Error(`Provider ${provider.config.id} requires a base URL`);
          const credentialConfigs: OpenAiProviderConfig[] = (credentialsByProvider.get(provider.config.id) ?? [])
            .filter((credential) => (
              credential.enabled &&
              (!credential.models || credential.models.includes(capability.canonicalModelId))
            ))
            .map((credential) => ({
              id: credential.id,
              baseUrl: provider.config.baseUrl!,
              apiKey: this.decryptCredential(credential),
              enabled: true,
              weight: credential.weight,
              maxConcurrency: credential.maxConcurrency,
              models: null,
            }));
          pool = OpenAiUpstreamPool.fromProviders(
            credentialConfigs,
            this.environment,
            this.fetchImpl,
            this.now,
          );
          if (!pool.configured) continue;
        } else if (!this.codexAvailable) {
          continue;
        }

        const runtimeTarget: RuntimeTarget = {
          id: target.id,
          priority: target.priority,
          weight: target.weight,
          maxConcurrency: target.maxConcurrency,
          inflight: 0,
          cooldownUntil: 0,
          provider,
          capability,
          pool,
        };
        const tier = tiers.get(target.priority) ?? [];
        tier.push(runtimeTarget);
        tiers.set(target.priority, tier);
      }

      if (tiers.size === 0) continue;
      const schedules = new Map<number, RuntimeTarget[]>();
      for (const [priority, tier] of tiers) {
        schedules.set(
          priority,
          tier.flatMap((target) => Array.from({ length: target.weight }, () => target)),
        );
      }
      this.runtimeRoutes.set(route.publicModelId, {
        publicModelId: route.publicModelId,
        revision: route.revision,
        tiers,
        schedules,
        cursors: new Map(),
      });
    }
  }

  get configured(): boolean {
    return this.runtimeRoutes.size > 0;
  }

  publicModels(path?: ModelRouterRequest["path"]): string[] {
    const requiredPath = path ? pathCapability(path) : null;
    return [...this.runtimeRoutes.values()]
      .filter((route) => !requiredPath || [...route.tiers.values()].some(
        (tier) => tier.some((target) => target.capability.apiPaths.includes(requiredPath)),
      ))
      .map((route) => route.publicModelId)
      .sort();
  }

  supports(publicModel: string, path: ModelRouterRequest["path"]): boolean {
    return this.publicModels(path).includes(publicModel);
  }

  usesCodex(publicModel: string): boolean {
    const route = this.runtimeRoutes.get(publicModel);
    return Boolean(route && [...route.tiers.values()].some(
      (tier) => tier.some((target) => target.provider.config.transport === "codex_cli"),
    ));
  }

  status(): {
    configured: boolean;
    routes: number;
    targets: number;
    availableTargets: number;
    inflight: number;
  } {
    const targets = [...this.runtimeRoutes.values()].flatMap((route) => [...route.tiers.values()].flat());
    const now = this.now();
    return {
      configured: this.configured,
      routes: this.runtimeRoutes.size,
      targets: targets.length,
      availableTargets: targets.filter((target) => (
        target.cooldownUntil <= now &&
        target.inflight < target.maxConcurrency &&
        target.provider.inflight < target.provider.config.maxConcurrency
      )).length,
      inflight: targets.reduce((total, target) => total + target.inflight, 0),
    };
  }

  private acquire(
    route: RuntimeRoute,
    priority: number,
    path: ProviderModelCapability["apiPaths"][number],
    attempted: ReadonlySet<string>,
  ): RuntimeTarget | null {
    const schedule = route.schedules.get(priority) ?? [];
    if (schedule.length === 0) return null;
    const cursor = route.cursors.get(priority) ?? 0;
    const now = this.now();
    for (let offset = 0; offset < schedule.length; offset += 1) {
      const position = (cursor + offset) % schedule.length;
      const target = schedule[position]!;
      if (
        attempted.has(target.id) ||
        !target.capability.apiPaths.includes(path) ||
        target.cooldownUntil > now ||
        target.inflight >= target.maxConcurrency ||
        target.provider.inflight >= target.provider.config.maxConcurrency
      ) {
        continue;
      }
      route.cursors.set(priority, (position + 1) % schedule.length);
      target.inflight += 1;
      target.provider.inflight += 1;
      return target;
    }
    return null;
  }

  private release(target: RuntimeTarget, failed: boolean): void {
    target.inflight = Math.max(0, target.inflight - 1);
    target.provider.inflight = Math.max(0, target.provider.inflight - 1);
    target.cooldownUntil = failed ? this.now() + this.cooldownMs : 0;
  }

  async execute(request: ModelRouterRequest): Promise<ModelRouterResult> {
    const route = this.runtimeRoutes.get(request.publicModel);
    if (!route) {
      throw new ModelRouteError("No published route is available for the requested model", 503, "server_error");
    }
    /* Validated at the public boundary; re-derived here because only Codex needs it as an argument —
     * HTTP transports carry the customer's reasoning fields inside the body untouched. */
    const reasoningEffort = requestedReasoningEffort(request.path, request.body);
    const requiredPath = pathCapability(request.path);
    const priorities = [...route.tiers.keys()].sort((left, right) => left - right);
    let attemptedAny = false;

    for (const priority of priorities) {
      const attempted = new Set<string>();
      const tierSize = route.tiers.get(priority)?.length ?? 0;
      while (attempted.size < tierSize) {
        const target = this.acquire(route, priority, requiredPath, attempted);
        if (!target) break;
        attempted.add(target.id);
        attemptedAny = true;
        let emitted = false;
        const callbacks: AppServerCallbacks = {
          onTextDelta: (delta) => {
            emitted = true;
            request.callbacks?.onTextDelta?.(delta);
          },
          onToolCall: (call) => {
            emitted = true;
            request.callbacks?.onToolCall?.(call);
          },
        };
        try {
          let payload: Record<string, unknown>;
          let status = 200;
          let credentialId: string | undefined;
          if (target.provider.config.transport === "codex_cli") {
            const result = await this.codexExecutor(
              target.capability.upstreamModelId,
              reasoningEffort,
              request.outputLimit,
              codexConversationFromRequest(request.path, request.body),
              request.body.tools,
              callbacks,
            );
            payload = codexPayload(request.path, request.publicModel, result);
          } else {
            const upstream = await target.pool!.execute({
              path: request.path,
              body: request.body,
              upstreamModel: target.capability.upstreamModelId,
              outputLimit: request.outputLimit,
            });
            payload = upstream.payload;
            status = upstream.status;
            credentialId = upstream.providerId;
          }
          this.release(target, false);
          return {
            payload,
            status,
            routeRevision: route.revision,
            routeTargetId: target.id,
            providerId: target.provider.config.id,
            credentialId,
          };
        } catch (cause) {
          this.release(target, true);
          if (emitted || !isRetryableFailure(cause)) {
            if (cause instanceof OpenAiUpstreamError) {
              throw new ModelRouteError(cause.message, cause.status, cause.type);
            }
            throw new ModelRouteError(cause instanceof Error ? cause.message : "Upstream failure");
          }
        }
      }
    }

    if (!attemptedAny) {
      throw new ModelRouteError(
        "Upstream capacity is temporarily unavailable",
        503,
        "upstream_capacity_error",
      );
    }
    throw new ModelRouteError("All configured route targets failed");
  }
}

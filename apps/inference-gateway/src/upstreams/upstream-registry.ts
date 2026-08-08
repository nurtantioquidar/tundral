import { randomUUID } from "node:crypto";
import { isIP } from "node:net";
import { CredentialCipher } from "./credential-cipher";
import {
  GatewayDatabase,
  type ModelRoute,
  type ProviderModelCapability,
  type PublicUpstreamCredential,
  type RoutingProvider,
  type StoredUpstreamCredential,
  type UpstreamTransport,
} from "../database";
import { ModelRouter, type ModelRouterRequest, type ModelRouterResult } from "./model-router";
import { OpenAiUpstreamPool, type FetchLike } from "./adapters/openai-pool";
import { MODEL_RATES } from "../billing/pricing";
import { runCodexAppServer } from "./adapters/codex-app-server";

export type UpstreamConfigSource = "environment" | "database";

export interface CreateRoutingProvider {
  name: string;
  transport: UpstreamTransport;
  baseUrl: string | null;
  enabled: boolean;
  maxConcurrency: number;
}

export interface UpdateRoutingProvider {
  name?: string;
  baseUrl?: string | null;
  enabled?: boolean;
  maxConcurrency?: number;
  expectedRevision?: number;
}

export interface CreateCredential {
  name: string;
  apiKey: string;
  enabled: boolean;
  weight: number;
  maxConcurrency: number;
  models: string[] | null;
}

export interface CreateCapability {
  canonicalModelId: string;
  upstreamModelId: string;
  apiPaths: ProviderModelCapability["apiPaths"];
  enabled: boolean;
}

function isPrivateIpv4(hostname: string): boolean {
  const parts = hostname.split(".").map(Number);
  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)) {
    return false;
  }
  const [a, b] = parts as [number, number, number, number];
  return (
    a === 10 ||
    a === 127 ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    a === 0
  );
}

function isPrivateHost(hostname: string): boolean {
  const normalized = hostname.toLowerCase().replace(/^\[|\]$/g, "");
  if (normalized === "localhost" || normalized.endsWith(".localhost")) return true;
  const ipVersion = isIP(normalized);
  if (ipVersion === 4) return isPrivateIpv4(normalized);
  if (ipVersion === 6) {
    return normalized === "::1" ||
      normalized.startsWith("fc") ||
      normalized.startsWith("fd") ||
      normalized.startsWith("fe8") ||
      normalized.startsWith("fe9") ||
      normalized.startsWith("fea") ||
      normalized.startsWith("feb");
  }
  return false;
}

export class UpstreamRegistry {
  private readonly environmentPoolValue: OpenAiUpstreamPool;
  private routerValue: ModelRouter | null = null;

  constructor(
    private readonly db: GatewayDatabase,
    private readonly cipher: CredentialCipher | null,
    readonly source: UpstreamConfigSource,
    private readonly environment: Record<string, string | undefined> = process.env,
    private readonly fetchImpl: FetchLike = fetch,
  ) {
    if (source === "database" && !cipher) {
      throw new Error("UPSTREAM_CREDENTIAL_ENCRYPTION_KEY is required when UPSTREAM_CONFIG_SOURCE=database");
    }
    this.environmentPoolValue = OpenAiUpstreamPool.fromEnvironment(environment);
    if (source === "database") this.refreshRouter();
  }

  get pool(): OpenAiUpstreamPool {
    return this.environmentPoolValue;
  }

  get router(): ModelRouter | null {
    return this.routerValue;
  }

  get appliedFromDatabase(): boolean {
    return this.source === "database";
  }

  get configured(): boolean {
    return this.source === "database"
      ? Boolean(this.routerValue?.configured)
      : this.environmentPoolValue.configured;
  }

  status() {
    return this.source === "database"
      ? this.routerValue!.status()
      : this.environmentPoolValue.status();
  }

  publicModels(path?: ModelRouterRequest["path"]): string[] {
    return this.source === "database"
      ? this.routerValue!.publicModels(path)
      : Object.keys(MODEL_RATES).sort();
  }

  supports(publicModel: string, path: ModelRouterRequest["path"]): boolean {
    return this.source === "database"
      ? this.routerValue!.supports(publicModel, path)
      : Boolean(MODEL_RATES[publicModel]);
  }

  usesCodex(publicModel: string): boolean {
    return this.source === "database" && this.routerValue!.usesCodex(publicModel);
  }

  execute(request: ModelRouterRequest): Promise<ModelRouterResult> {
    if (this.source !== "database") throw new Error("Database model routing is not active");
    return this.routerValue!.execute(request);
  }

  private requireCipher(): CredentialCipher {
    if (!this.cipher) throw new Error("Upstream credential encryption is not configured");
    return this.cipher;
  }

  private decrypt(credential: StoredUpstreamCredential): string {
    return this.requireCipher().decrypt(
      credential.encryptionContext,
      credential.credentialCiphertext,
    );
  }

  private graph() {
    return {
      providers: this.db.listRoutingProviders(),
      credentials: this.db.loadUpstreamCredentials(),
      capabilities: this.db.listProviderModelCapabilities(),
      routes: this.db.listModelRoutes(),
    };
  }

  private buildRouter(graph = this.graph()): ModelRouter {
    return new ModelRouter(
      graph,
      (credential) => this.decrypt(credential),
      runCodexAppServer,
      Boolean(Bun.which(this.environment.CODEX_EXECUTABLE || "codex")),
      this.environment,
    );
  }

  private refreshRouter(): boolean {
    if (!this.appliedFromDatabase) return false;
    const next = this.buildRouter();
    this.routerValue = next;
    return true;
  }

  private validateBaseUrl(baseUrl: string): string {
    const url = new URL(baseUrl);
    if (url.protocol !== "https:" && this.environment.ALLOW_INSECURE_UPSTREAM_HTTP !== "true") {
      throw new Error("Database-managed upstream base_url must use https");
    }
    if (url.username || url.password || url.search || url.hash) {
      throw new Error("Database-managed upstream base_url must not contain credentials, a query, or a fragment");
    }
    if (isPrivateHost(url.hostname) && this.environment.ALLOW_PRIVATE_UPSTREAM_NETWORKS !== "true") {
      throw new Error("Database-managed upstream base_url must not target a private network");
    }
    return url.toString().replace(/\/$/, "");
  }

  listProviders(): RoutingProvider[] {
    return this.db.listRoutingProviders();
  }

  getProvider(providerId: string): RoutingProvider | null {
    return this.db.getRoutingProvider(providerId);
  }

  createProvider(args: CreateRoutingProvider): { provider: RoutingProvider; applied: boolean } {
    const baseUrl = args.transport === "openai_http"
      ? this.validateBaseUrl(args.baseUrl ?? "")
      : null;
    const provider = this.db.createRoutingProvider({
      id: randomUUID(),
      ...args,
      baseUrl,
    });
    return { provider, applied: this.refreshRouter() };
  }

  updateProvider(
    providerId: string,
    args: UpdateRoutingProvider,
  ): { provider: RoutingProvider; applied: boolean } {
    const current = this.db.getRoutingProvider(providerId);
    if (!current) throw new Error("Upstream provider not found");
    const baseUrl = current.transport === "openai_http"
      ? this.validateBaseUrl(args.baseUrl === undefined ? current.baseUrl ?? "" : args.baseUrl ?? "")
      : null;
    const provider = this.db.updateRoutingProvider(providerId, { ...args, baseUrl });
    return { provider, applied: this.refreshRouter() };
  }

  deleteProvider(providerId: string, expectedRevision?: number): boolean {
    const deleted = this.db.deleteRoutingProvider(providerId, expectedRevision);
    if (deleted) this.refreshRouter();
    return deleted;
  }

  listCredentials(providerId: string): PublicUpstreamCredential[] {
    if (!this.getProvider(providerId)) throw new Error("Upstream provider not found");
    return this.db.listUpstreamCredentials(providerId);
  }

  getCredential(credentialId: string): StoredUpstreamCredential | null {
    return this.db.getUpstreamCredential(credentialId);
  }

  createCredential(
    providerId: string,
    args: CreateCredential,
  ): { credential: PublicUpstreamCredential; applied: boolean } {
    const provider = this.getProvider(providerId);
    if (!provider) throw new Error("Upstream provider not found");
    if (provider.transport !== "openai_http") {
      throw new Error("Codex subscription providers do not accept API keys");
    }
    const id = randomUUID();
    const credential = this.db.createUpstreamCredential({
      id,
      providerId,
      name: args.name,
      credentialCiphertext: this.requireCipher().encrypt(id, args.apiKey),
      encryptionContext: id,
      enabled: args.enabled,
      weight: args.weight,
      maxConcurrency: args.maxConcurrency,
      models: args.models,
    });
    return { credential, applied: this.refreshRouter() };
  }

  updateCredential(
    credentialId: string,
    args: Parameters<GatewayDatabase["updateUpstreamCredential"]>[1],
  ): { credential: PublicUpstreamCredential; applied: boolean } {
    const credential = this.db.updateUpstreamCredential(credentialId, args);
    return { credential, applied: this.refreshRouter() };
  }

  rotateCredential(
    credentialId: string,
    apiKey: string,
    expectedRevision?: number,
  ): { credential: PublicUpstreamCredential; applied: boolean } {
    const current = this.db.getUpstreamCredential(credentialId);
    if (!current) throw new Error("Upstream credential not found");
    const credential = this.db.rotateUpstreamCredential(credentialId, {
      credentialCiphertext: this.requireCipher().encrypt(credentialId, apiKey),
      encryptionContext: credentialId,
      expectedRevision,
    });
    return { credential, applied: this.refreshRouter() };
  }

  deleteCredential(credentialId: string, expectedRevision?: number): boolean {
    const deleted = this.db.deleteUpstreamCredential(credentialId, expectedRevision);
    if (deleted) this.refreshRouter();
    return deleted;
  }

  listCapabilities(providerId?: string): ProviderModelCapability[] {
    if (providerId && !this.getProvider(providerId)) throw new Error("Upstream provider not found");
    return this.db.listProviderModelCapabilities(providerId);
  }

  getCapability(capabilityId: string): ProviderModelCapability | null {
    return this.db.getProviderModelCapability(capabilityId);
  }

  createCapability(
    providerId: string,
    args: CreateCapability,
  ): { capability: ProviderModelCapability; applied: boolean } {
    const capability = this.db.createProviderModelCapability({
      id: randomUUID(),
      providerId,
      ...args,
    });
    return { capability, applied: this.refreshRouter() };
  }

  updateCapability(
    capabilityId: string,
    args: Parameters<GatewayDatabase["updateProviderModelCapability"]>[1],
  ): { capability: ProviderModelCapability; applied: boolean } {
    const capability = this.db.updateProviderModelCapability(capabilityId, args);
    return { capability, applied: this.refreshRouter() };
  }

  deleteCapability(capabilityId: string, expectedRevision?: number): boolean {
    const deleted = this.db.deleteProviderModelCapability(capabilityId, expectedRevision);
    if (deleted) this.refreshRouter();
    return deleted;
  }

  listRoutes(): ModelRoute[] {
    return this.db.listModelRoutes();
  }

  getRoute(publicModelId: string): ModelRoute | null {
    return this.db.getModelRoute(publicModelId);
  }

  publishRoute(args: Parameters<GatewayDatabase["publishModelRoute"]>[0]): {
    route: ModelRoute;
    applied: boolean;
  } {
    const rate = MODEL_RATES[args.publicModelId];
    if (!rate) throw new Error(`Unsupported model: ${args.publicModelId}`);
    if (args.targets.length === 0 && args.enabled) {
      throw new Error("An enabled model route requires at least one target");
    }
    const seen = new Set<string>();
    for (const target of args.targets) {
      if (seen.has(target.capabilityId)) throw new Error("A capability may appear only once in a model route");
      seen.add(target.capabilityId);
      const capability = this.db.getProviderModelCapability(target.capabilityId);
      if (!capability) throw new Error(`Upstream model capability not found: ${target.capabilityId}`);
      if (capability.canonicalModelId !== rate.upstreamModel) {
        throw new Error(
          `Route target ${target.capabilityId} is ${capability.canonicalModelId}; ${args.publicModelId} requires ${rate.upstreamModel}`,
        );
      }
    }

    // Validate a complete projected snapshot before publishing the new immutable revision.
    const projected = this.graph();
    projected.routes = [
      ...projected.routes.filter((route) => route.publicModelId !== args.publicModelId),
      {
        publicModelId: args.publicModelId,
        enabled: args.enabled,
        revision: (this.db.getModelRoute(args.publicModelId)?.revision ?? 0) + 1,
        targets: args.targets.map((target) => ({
          id: target.id ?? randomUUID(),
          publicModelId: args.publicModelId,
          capabilityId: target.capabilityId,
          priority: target.priority,
          weight: target.weight,
          maxConcurrency: target.maxConcurrency,
          enabled: target.enabled,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        })),
        createdAt: Date.now(),
        updatedAt: Date.now(),
      },
    ];
    this.buildRouter(projected);
    const route = this.db.publishModelRoute(args);
    return { route, applied: this.refreshRouter() };
  }

  async testCapability(
    capabilityId: string,
    publicModelId: string,
    path: ModelRouterRequest["path"],
  ): Promise<ModelRouterResult> {
    const capability = this.db.getProviderModelCapability(capabilityId);
    if (!capability) throw new Error("Upstream model capability not found");
    const provider = this.db.getRoutingProvider(capability.providerId);
    if (!provider) throw new Error("Upstream provider not found");
    const targetId = randomUUID();
    const testRouter = this.buildRouter({
      providers: [{ ...provider, enabled: true }],
      credentials: this.db.loadUpstreamCredentials(provider.id),
      capabilities: [{ ...capability, enabled: true }],
      routes: [{
        publicModelId,
        enabled: true,
        revision: 1,
        targets: [{
          id: targetId,
          publicModelId,
          capabilityId,
          priority: 1,
          weight: 1,
          maxConcurrency: 1,
          enabled: true,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        }],
        createdAt: Date.now(),
        updatedAt: Date.now(),
      }],
    });
    return testRouter.execute({
      path,
      publicModel: publicModelId,
      body: path === "/v1/responses"
        ? { input: "Reply with OK." }
        : { messages: [{ role: "user", content: "Reply with OK." }] },
      outputLimit: 8,
    });
  }

  /* The models the provider itself reports from GET /v1/models, so the operator console can offer a
   * picker instead of hand-typed IDs. Read-only: authenticates with one stored credential, never
   * returns or logs the key, and never mutates configuration. */
  async discoverProviderModels(providerId: string): Promise<string[]> {
    const provider = this.db.getRoutingProvider(providerId);
    if (!provider) throw new Error("Upstream provider not found");
    if (provider.transport !== "openai_http" || !provider.baseUrl) {
      throw new Error("Model discovery requires an OpenAI-compatible HTTP provider");
    }
    const credential = this.db.loadUpstreamCredentials(provider.id).find((stored) => stored.enabled);
    if (!credential) {
      throw new Error("Store an API key first — model discovery authenticates against the provider");
    }
    let response: Response;
    try {
      response = await this.fetchImpl(
        provider.baseUrl.endsWith("/v1") ? `${provider.baseUrl}/models` : `${provider.baseUrl}/v1/models`,
        {
          headers: { authorization: `Bearer ${this.decrypt(credential)}` },
          signal: AbortSignal.timeout(15_000),
        },
      );
    } catch {
      throw new Error("The provider could not be reached for model discovery");
    }
    if (!response.ok) throw new Error(`The provider answered /v1/models with HTTP ${response.status}`);
    const payload = await response.json().catch(() => null) as { data?: unknown } | null;
    if (!payload || !Array.isArray(payload.data)) {
      throw new Error("The provider's /v1/models response was not a model list");
    }
    const ids = payload.data
      .map((entry) => (entry && typeof entry === "object" ? (entry as Record<string, unknown>).id : undefined))
      .filter((id): id is string => typeof id === "string" && id.length > 0 && id.length <= 160);
    return [...new Set(ids)].sort().slice(0, 500);
  }

  recordTest(targetId: string, succeeded: boolean, metadata: Record<string, unknown>): void {
    this.db.recordAudit(
      succeeded ? "upstream_model.test_succeeded" : "upstream_model.test_failed",
      "upstream_model",
      targetId,
      metadata,
    );
  }
}

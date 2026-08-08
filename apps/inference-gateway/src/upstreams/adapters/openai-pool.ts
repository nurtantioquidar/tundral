/** Transport request after public-model routing has resolved an upstream model. */
export interface OpenAiUpstreamRequest {
  path: "/v1/chat/completions" | "/v1/responses";
  body: Record<string, unknown>;
  upstreamModel: string;
  outputLimit: number;
}

export interface OpenAiUpstreamResult {
  payload: Record<string, unknown>;
  status: number;
  providerId: string;
}

export interface OpenAiProviderConfig {
  id: string;
  baseUrl: string;
  apiKey: string;
  enabled?: boolean;
  weight?: number;
  maxConcurrency?: number;
  models?: string[] | null;
}

interface RawProvider {
  id?: unknown;
  base_url?: unknown;
  api_key?: unknown;
  api_key_env?: unknown;
  weight?: unknown;
  max_concurrency?: unknown;
  models?: unknown;
  enabled?: unknown;
}

interface Provider {
  id: string;
  baseUrl: string;
  apiKey: string;
  weight: number;
  maxConcurrency: number;
  models: ReadonlySet<string> | null;
  inflight: number;
  cooldownUntil: number;
}

export type FetchLike = (input: string | URL | Request, init?: RequestInit) => Promise<Response>;

export class OpenAiUpstreamError extends Error {
  constructor(message: string, readonly status = 502, readonly type = "upstream_error") {
    super(message);
  }
}

function positiveInteger(value: unknown, name: string, fallback: number, maximum = Number.MAX_SAFE_INTEGER): number {
  if (value === undefined) return fallback;
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0 || value > maximum) {
    throw new Error(`${name} must be a positive integer no greater than ${maximum}`);
  }
  return value;
}

function environmentPositiveInteger(value: string | undefined, name: string, fallback: number): number {
  if (value === undefined || value === "") return fallback;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) throw new Error(`${name} must be a positive integer`);
  return parsed;
}

function normalizeBaseUrl(value: unknown, providerId: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(`Provider ${providerId} requires base_url`);
  const url = new URL(value);
  if (!["http:", "https:"].includes(url.protocol)) throw new Error(`Provider ${providerId} base_url must use http or https`);
  if (url.username || url.password || url.search || url.hash) {
    throw new Error(`Provider ${providerId} base_url must not contain credentials, a query, or a fragment`);
  }
  return url.toString().replace(/\/$/, "");
}

function providerModels(value: unknown, providerId: string): ReadonlySet<string> | null {
  if (value === undefined) return null;
  if (!Array.isArray(value) || value.length === 0 || value.some((model) => typeof model !== "string" || !model)) {
    throw new Error(`Provider ${providerId} models must be a non-empty string array`);
  }
  return new Set(value);
}

function endpointUrl(baseUrl: string, path: OpenAiUpstreamRequest["path"]): string {
  return baseUrl.endsWith("/v1") ? `${baseUrl}${path.slice(3)}` : `${baseUrl}${path}`;
}

/* The customer's own reasoning fields (`reasoning_effort` on chat, `reasoning.effort` on
 * responses) pass through untouched — effort is a per-request harness choice, validated at the
 * public boundary, never forced by the gateway. */
function upstreamBody(request: OpenAiUpstreamRequest): Record<string, unknown> {
  return {
    ...request.body,
    model: request.upstreamModel,
    stream: false,
    ...(request.path === "/v1/responses"
      ? { max_output_tokens: request.outputLimit }
      : { max_completion_tokens: request.outputLimit }),
  };
}

function isRetryableStatus(status: number): boolean {
  return [401, 402, 403, 404, 408, 409, 425, 429].includes(status) || status >= 500;
}

function safeFailureStatus(status: number): number {
  return status >= 400 && status < 500 && !isRetryableStatus(status) ? status : 502;
}

export class OpenAiUpstreamPool {
  private readonly providers: Provider[];
  private readonly schedule: number[];
  private cursor = 0;

  constructor(
    providers: Provider[],
    private readonly fetchImpl: FetchLike = fetch,
    private readonly cooldownMs = 30_000,
    private readonly requestTimeoutMs = 120_000,
    private readonly maxAttempts = providers.length,
    private readonly now: () => number = Date.now,
  ) {
    this.providers = providers;
    this.schedule = providers.flatMap((provider, index) => Array.from({ length: provider.weight }, () => index));
  }

  static fromEnvironment(
    environment: Record<string, string | undefined> = process.env,
    fetchImpl: FetchLike = fetch,
    now: () => number = Date.now,
  ): OpenAiUpstreamPool {
    const defaultMaxConcurrency = environmentPositiveInteger(
      environment.UPSTREAM_MAX_CONCURRENCY_PER_PROVIDER,
      "UPSTREAM_MAX_CONCURRENCY_PER_PROVIDER",
      4,
    );
    const rawJson = environment.OPENAI_UPSTREAMS_JSON?.trim();
    let rawProviders: RawProvider[];
    if (rawJson) {
      let parsed: unknown;
      try {
        parsed = JSON.parse(rawJson);
      } catch {
        throw new Error("OPENAI_UPSTREAMS_JSON must be valid JSON");
      }
      if (!Array.isArray(parsed)) throw new Error("OPENAI_UPSTREAMS_JSON must be a JSON array");
      rawProviders = parsed as RawProvider[];
    } else if (environment.UPSTREAM_API_KEY) {
      rawProviders = [{
        id: "legacy",
        base_url: environment.UPSTREAM_BASE_URL || "https://api.openai.com",
        api_key: environment.UPSTREAM_API_KEY,
        max_concurrency: defaultMaxConcurrency,
      }];
    } else {
      rawProviders = [];
    }

    return OpenAiUpstreamPool.fromRawProviders(rawProviders, environment, fetchImpl, now);
  }

  static fromProviders(
    providers: OpenAiProviderConfig[],
    environment: Record<string, string | undefined> = process.env,
    fetchImpl: FetchLike = fetch,
    now: () => number = Date.now,
  ): OpenAiUpstreamPool {
    return OpenAiUpstreamPool.fromRawProviders(
      providers.map((provider) => ({
        id: provider.id,
        base_url: provider.baseUrl,
        api_key: provider.apiKey,
        enabled: provider.enabled,
        weight: provider.weight,
        max_concurrency: provider.maxConcurrency,
        models: provider.models === null ? undefined : provider.models,
      })),
      environment,
      fetchImpl,
      now,
    );
  }

  private static fromRawProviders(
    rawProviders: RawProvider[],
    environment: Record<string, string | undefined>,
    fetchImpl: FetchLike,
    now: () => number,
  ): OpenAiUpstreamPool {
    const defaultMaxConcurrency = environmentPositiveInteger(
      environment.UPSTREAM_MAX_CONCURRENCY_PER_PROVIDER,
      "UPSTREAM_MAX_CONCURRENCY_PER_PROVIDER",
      4,
    );
    const ids = new Set<string>();
    const providers: Provider[] = [];
    for (const [index, raw] of rawProviders.entries()) {
      if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
        throw new Error(`Provider at index ${index} must be an object`);
      }
      if (raw.enabled === false) continue;
      if (raw.enabled !== undefined && raw.enabled !== true) {
        throw new Error(`Provider at index ${index} enabled must be a boolean`);
      }
      if (typeof raw.id !== "string" || !/^[A-Za-z0-9._-]{1,80}$/.test(raw.id)) {
        throw new Error(`Provider at index ${index} requires a URL-safe id`);
      }
      if (ids.has(raw.id)) throw new Error(`Duplicate provider id: ${raw.id}`);
      ids.add(raw.id);

      const inlineKey = typeof raw.api_key === "string" ? raw.api_key : null;
      const keyEnvironmentName = typeof raw.api_key_env === "string" ? raw.api_key_env : null;
      if (inlineKey && keyEnvironmentName) throw new Error(`Provider ${raw.id} must use only one of api_key or api_key_env`);
      if (raw.api_key !== undefined && inlineKey === null) throw new Error(`Provider ${raw.id} api_key must be a string`);
      if (raw.api_key_env !== undefined && keyEnvironmentName === null) throw new Error(`Provider ${raw.id} api_key_env must be a string`);
      const apiKey = inlineKey || (keyEnvironmentName ? environment[keyEnvironmentName] : undefined);
      if (!apiKey) throw new Error(`Provider ${raw.id} API key is not configured`);

      providers.push({
        id: raw.id,
        baseUrl: normalizeBaseUrl(raw.base_url, raw.id),
        apiKey,
        weight: positiveInteger(raw.weight, `Provider ${raw.id} weight`, 1, 100),
        maxConcurrency: positiveInteger(
          raw.max_concurrency,
          `Provider ${raw.id} max_concurrency`,
          defaultMaxConcurrency,
          10_000,
        ),
        models: providerModels(raw.models, raw.id),
        inflight: 0,
        cooldownUntil: 0,
      });
    }

    const cooldownMs = environmentPositiveInteger(environment.UPSTREAM_FAILURE_COOLDOWN_MS, "UPSTREAM_FAILURE_COOLDOWN_MS", 30_000);
    const requestTimeoutMs = environmentPositiveInteger(environment.UPSTREAM_REQUEST_TIMEOUT_MS, "UPSTREAM_REQUEST_TIMEOUT_MS", 120_000);
    const defaultAttempts = Math.max(providers.length, 1);
    const maxAttempts = environmentPositiveInteger(environment.UPSTREAM_MAX_ATTEMPTS, "UPSTREAM_MAX_ATTEMPTS", defaultAttempts);
    return new OpenAiUpstreamPool(providers, fetchImpl, cooldownMs, requestTimeoutMs, maxAttempts, now);
  }

  get configured(): boolean {
    return this.providers.length > 0;
  }

  status(): { configured: boolean; providers: number; available: number; inflight: number; capacity: number } {
    const now = this.now();
    return {
      configured: this.configured,
      providers: this.providers.length,
      available: this.providers.filter((provider) => provider.cooldownUntil <= now && provider.inflight < provider.maxConcurrency).length,
      inflight: this.providers.reduce((total, provider) => total + provider.inflight, 0),
      capacity: this.providers.reduce((total, provider) => total + provider.maxConcurrency, 0),
    };
  }

  private acquire(upstreamModel: string, excluded: ReadonlySet<number>): { provider: Provider; index: number } | null {
    if (this.schedule.length === 0) return null;
    const now = this.now();
    for (let offset = 0; offset < this.schedule.length; offset += 1) {
      const position = (this.cursor + offset) % this.schedule.length;
      const index = this.schedule[position]!;
      const provider = this.providers[index]!;
      if (
        excluded.has(index) ||
        provider.cooldownUntil > now ||
        provider.inflight >= provider.maxConcurrency ||
        (provider.models && !provider.models.has(upstreamModel))
      ) {
        continue;
      }
      this.cursor = (position + 1) % this.schedule.length;
      provider.inflight += 1;
      return { provider, index };
    }
    return null;
  }

  private release(provider: Provider, failed: boolean): void {
    provider.inflight = Math.max(0, provider.inflight - 1);
    provider.cooldownUntil = failed ? this.now() + this.cooldownMs : 0;
  }

  async execute(request: OpenAiUpstreamRequest): Promise<OpenAiUpstreamResult> {
    if (!this.configured) throw new OpenAiUpstreamError("Gateway upstream is not configured", 503, "server_error");
    const attempted = new Set<number>();
    const attempts = Math.min(this.maxAttempts, this.providers.length);

    while (attempted.size < attempts) {
      const acquired = this.acquire(request.upstreamModel, attempted);
      if (!acquired) break;
      const { provider, index } = acquired;
      attempted.add(index);
      let released = false;
      try {
        const upstream = await this.fetchImpl(endpointUrl(provider.baseUrl, request.path), {
          method: "POST",
          headers: {
            "content-type": "application/json",
            authorization: `Bearer ${provider.apiKey}`,
          },
          body: JSON.stringify(upstreamBody(request)),
          signal: AbortSignal.timeout(this.requestTimeoutMs),
        });

        if (!upstream.ok) {
          const retryable = isRetryableStatus(upstream.status);
          this.release(provider, retryable);
          released = true;
          await upstream.body?.cancel().catch(() => undefined);
          if (retryable) continue;
          throw new OpenAiUpstreamError(
            `Upstream request failed with status ${upstream.status}`,
            safeFailureStatus(upstream.status),
          );
        }

        let payload: unknown;
        try {
          payload = await upstream.json();
        } catch {
          this.release(provider, true);
          released = true;
          throw new OpenAiUpstreamError("Upstream returned an invalid JSON response");
        }
        if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
          this.release(provider, true);
          released = true;
          throw new OpenAiUpstreamError("Upstream returned an invalid response");
        }
        this.release(provider, false);
        released = true;
        return {
          payload: payload as Record<string, unknown>,
          status: upstream.status,
          providerId: provider.id,
        };
      } catch (cause) {
        if (!released) this.release(provider, true);
        if (cause instanceof OpenAiUpstreamError) {
          if (cause.status !== 502 || cause.message.startsWith("Upstream returned")) throw cause;
          continue;
        }
      }
    }

    const supportsModel = this.providers.some(
      (provider) => !provider.models || provider.models.has(request.upstreamModel),
    );
    if (!supportsModel) {
      throw new OpenAiUpstreamError("No upstream provider supports the requested model", 503, "server_error");
    }
    if (attempted.size === 0) {
      throw new OpenAiUpstreamError("Upstream capacity is temporarily unavailable", 503, "upstream_capacity_error");
    }
    throw new OpenAiUpstreamError("All configured upstream providers failed");
  }
}

// Upstream transport: one OpenAI-compatible POST per request. The body is forwarded verbatim
// except for the model id, the stripped `nimble` extension, and forced usage reporting. Errors
// are classified so the router can decide between failover and passthrough.

import type { ModelConfig, ProviderConfig } from "./config";
import { providerApiKey, providerBaseUrl } from "./config";
import { chatToResponses } from "./responses";

export type UpstreamErrorKind = "rate_limited" | "server_error" | "client_error" | "transport" | "timeout" | "not_configured";

export class UpstreamError extends Error {
  constructor(
    public readonly kind: UpstreamErrorKind,
    public readonly status: number,
    public readonly body: string,
    public readonly provider: string,
  ) {
    super(`${provider}: ${kind} (${status}) ${body.slice(0, 200)}`);
  }

  /** Failover-worthy: the provider, not the request, is at fault. */
  get retryable(): boolean {
    return this.kind === "rate_limited" || this.kind === "server_error" || this.kind === "transport" || this.kind === "timeout";
  }
}

export interface UpstreamTarget {
  provider: ProviderConfig;
  model: ModelConfig;
  base_url: string;
  api_key: string;
}

export function resolveTarget(provider: ProviderConfig, model: ModelConfig): UpstreamTarget {
  const base_url = providerBaseUrl(provider);
  const api_key = providerApiKey(provider);
  if (!base_url) throw new UpstreamError("not_configured", 503, `provider ${provider.id} has no endpoint (set ${provider.base_url_env ?? "base_url"})`, provider.id);
  if (!api_key) throw new UpstreamError("not_configured", 503, `provider ${provider.id} has no API key (set ${provider.api_key_env})`, provider.id);
  return { provider, model, base_url, api_key };
}

export interface UpstreamResult {
  response: Response;
  ttfb_ms: number;
}

export async function sendUpstream(
  target: UpstreamTarget,
  body: Record<string, unknown>,
  fetchImpl: typeof fetch = fetch,
  timeoutMs = 600_000,
): Promise<UpstreamResult> {
  const started = performance.now();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  const responses = target.provider.dialect === "responses";
  if (responses) {
    const stream = body.stream === true;
    body = { ...chatToResponses(body), stream };
  }
  let response: Response;
  try {
    response = await fetchImpl(`${target.base_url}/${responses ? "responses" : "chat/completions"}`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${target.api_key}`,
        accept: body.stream ? "text/event-stream" : "application/json",
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
  } catch (error) {
    clearTimeout(timer);
    const kind: UpstreamErrorKind = controller.signal.aborted ? "timeout" : "transport";
    throw new UpstreamError(kind, 0, error instanceof Error ? error.message : String(error), target.provider.id);
  }
  if (!response.ok) {
    clearTimeout(timer);
    const text = await response.text().catch(() => "");
    const kind: UpstreamErrorKind = response.status === 429 ? "rate_limited" : response.status >= 500 ? "server_error" : "client_error";
    throw new UpstreamError(kind, response.status, text, target.provider.id);
  }
  // The timer is only for the connection/headers phase; the stream itself is unbounded.
  clearTimeout(timer);
  return { response, ttfb_ms: Math.round(performance.now() - started) };
}

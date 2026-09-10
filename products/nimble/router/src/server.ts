// The HTTP surface: an OpenAI-compatible `/v1/chat/completions` that either passes an explicit
// model through or routes `model: "nimble"`, plus models, feedback, explainability, stats, and
// health. `createApp` returns a fetch handler (testable without a socket); `startServer` binds it.

import {
  NIMBLE_CONTRACT_VERSION,
  NIMBLE_REQUEST_ID_HEADER,
  NIMBLE_ROUTING_HEADER,
  NIMBLE_VERSION_HEADER,
  mergeNimbleRequestExtensions,
  nimbleRequestExtensionFromHeaders,
  parseNimbleFeedback,
  parseNimbleRequestExtension,
  type NimbleRequestExtension,
  type NimbleRoutingResponse,
} from "@monet/contracts/nimble";

import type { ModelConfig, NimbleConfig } from "./config";
import { HealthTracker } from "./health";
import { priceUsage, type Usage } from "./pricing";
import { Router, type Decision } from "./router";
import { Store, type SessionState } from "./store";
import { ResponsesToChat, responsesToChatCompletion } from "./responses";
import { resolveTarget, sendUpstream, UpstreamError } from "./upstream";
import { normalizeUsage, splitTokenEstimate, stablePrefixHash } from "./usage";

export const VIRTUAL_MODEL = "nimble";

export interface AppDeps {
  config: NimbleConfig;
  store: Store;
  health?: HealthTracker;
  fetchImpl?: typeof fetch;
  now?: () => number;
  apiKey?: string | null;
}

export interface App {
  fetch(request: Request): Promise<Response>;
  router: Router;
  health: HealthTracker;
  store: Store;
}

function json(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", ...headers } });
}

function openAiError(status: number, message: string, type: string, code: string | null = null): Response {
  return json(status, { error: { message, type, code } });
}

function newId(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}${Math.random().toString(36).slice(2, 10)}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sessionIdFor(extension: NimbleRequestExtension | null, headers: Headers, messages: unknown[]): string {
  if (extension?.session_id) return extension.session_id;
  const header = headers.get("x-session-id");
  if (header) return header;
  // Anonymous stickiness: sessions sharing a first message share a key.
  return `anon:${stablePrefixHash(messages, null).slice(7, 39)}`;
}

export function createApp(deps: AppDeps): App {
  const { config, store } = deps;
  const now = deps.now ?? Date.now;
  const health = deps.health ?? new HealthTracker(config.health, now);
  const router = new Router(config, store, health);
  const fetchImpl = deps.fetchImpl ?? fetch;
  const apiKey = deps.apiKey === undefined ? (process.env.NIMBLE_API_KEY?.trim() || null) : deps.apiKey;
  const providers = new Map(config.providers.map((p) => [p.id, p]));

  function authorized(request: Request): boolean {
    if (!apiKey) return true;
    const header = request.headers.get("authorization") ?? "";
    const presented = header.startsWith("Bearer ") ? header.slice(7).trim() : "";
    if (presented.length !== apiKey.length) return false;
    let diff = 0;
    for (let i = 0; i < apiKey.length; i++) diff |= presented.charCodeAt(i) ^ apiKey.charCodeAt(i);
    return diff === 0;
  }

  function routingFacts(model: ModelConfig, decision: Decision | null, requestId: string, decisionId: string | null): NimbleRoutingResponse {
    return {
      version: NIMBLE_CONTRACT_VERSION,
      selected_model: model.id,
      provider: model.provider,
      model_changed: decision?.model_changed ?? false,
      cache_affinity: decision?.cache_affinity ?? 0,
      estimated_cached_tokens: decision?.estimated_cached_tokens ?? 0,
      context_limit: model.context_tokens,
      model_capabilities: {
        tools: model.capabilities.tools,
        parallel_tools: model.capabilities.parallel_tools,
        reasoning: model.capabilities.reasoning,
        prompt_cache: model.capabilities.prompt_cache,
        images: model.capabilities.images,
      },
      routing_reason: decision?.reason ?? "explicit model",
      decision_id: decisionId,
      request_id: requestId,
    };
  }

  async function chatCompletions(request: Request): Promise<Response> {
    let body: unknown;
    try {
      body = await request.json();
    } catch {
      return openAiError(400, "request body must be JSON", "invalid_request_error");
    }
    if (!isRecord(body)) return openAiError(400, "request body must be an object", "invalid_request_error");
    const messages = body.messages;
    if (!Array.isArray(messages) || messages.length === 0) {
      return openAiError(400, "messages must be a non-empty array", "invalid_request_error", "missing_messages");
    }
    const requestedModel = typeof body.model === "string" ? body.model : VIRTUAL_MODEL;
    const stream = body.stream === true;

    const extension = mergeNimbleRequestExtensions(
      parseNimbleRequestExtension(body.nimble),
      nimbleRequestExtensionFromHeaders(request.headers),
    );
    const forward: Record<string, unknown> = { ...body };
    delete forward.nimble;
    if (stream) forward.stream_options = { ...(isRecord(body.stream_options) ? body.stream_options : {}), include_usage: true };

    const requestId = newId("r");
    const at = now();
    const sessionId = sessionIdFor(extension, request.headers, messages);
    const session: SessionState = store.getSession(sessionId) ?? {
      session_id: sessionId,
      current_model: null,
      prefix_hash: null,
      stable_tokens: 0,
      last_request_at: at,
      requests: 0,
      switches: 0,
      recent_failures: [],
      escalated_attempt: null,
      failure_attempt: null,
    };
    const estimate = splitTokenEstimate(messages, body.tools);
    const prefixHash = extension?.stable_prefix_hash ?? stablePrefixHash(messages, body.tools);
    const stableTokens = extension?.estimated_stable_tokens ?? (session.prefix_hash === prefixHash && session.stable_tokens > 0 ? session.stable_tokens : estimate.stable);
    const dynamicTokens = extension?.estimated_dynamic_tokens ?? estimate.dynamic;
    const taskType = extension?.task_type ?? null;
    // Count a failed attempt once, not once per request of the next attempt.
    const attempt = extension?.attempt ?? null;
    if (extension?.previous_failure && session.current_model && session.failure_attempt !== attempt) {
      session.recent_failures = [...session.recent_failures, session.current_model].slice(-5);
      session.failure_attempt = attempt;
    }

    const routed = requestedModel === VIRTUAL_MODEL;
    const excluded = new Set<string>();
    let decision: Decision | null = null;
    let model: ModelConfig | undefined;
    if (!routed) {
      model = router.model(requestedModel);
      if (!model) {
        return openAiError(404, `model ${requestedModel} is not configured; use "nimble" or one of ${config.models.map((m) => m.id).join(", ")}`, "invalid_request_error", "model_not_found");
      }
    }

    let failovers = 0;
    // Attempt loop: routed requests fail over to the next-best candidate when a provider is at
    // fault before anything has streamed; explicit requests are proxied as-is.
    for (;;) {
      if (routed) {
        decision = router.decide({
          session,
          prefix_hash: prefixHash,
          stable_tokens: stableTokens,
          dynamic_tokens: dynamicTokens,
          task_type: taskType,
          task_complexity: extension?.task_complexity ?? null,
          attempt: extension?.attempt ?? null,
          previous_failure: extension?.previous_failure ?? null,
          verification_failed: extension?.verification_failed ?? false,
          exclude_providers: excluded,
          now_ms: at,
        });
        model = decision.chosen;
      }
      const chosen = model!;
      const provider = providers.get(chosen.provider);
      if (!provider) return openAiError(503, `provider ${chosen.provider} is not configured`, "server_error");
      forward.model = chosen.id;
      // Output budget: clients send OpenAI's classic `max_tokens`; some upstreams
      // (OpenAI reasoning models) only accept `max_completion_tokens`.
      if (provider.max_tokens_param === "max_completion_tokens" && forward.max_tokens !== undefined) {
        forward.max_completion_tokens = forward.max_tokens;
        delete forward.max_tokens;
      } else if (provider.max_tokens_param === "max_tokens" && forward.max_completion_tokens !== undefined && forward.max_tokens === undefined) {
        forward.max_tokens = forward.max_completion_tokens;
        delete forward.max_completion_tokens;
      }

      let target;
      let upstream;
      try {
        target = resolveTarget(provider, chosen);
        upstream = await sendUpstream(target, forward, fetchImpl);
      } catch (error) {
        if (!(error instanceof UpstreamError)) throw error;
        health.recordFailure(provider.id, error.kind === "rate_limited" ? 0.5 : 1);
        store.recordRequest({
          id: newId("r"),
          at,
          session_id: sessionId,
          requested_model: requestedModel,
          selected_model: chosen.id,
          provider: provider.id,
          routed,
          status: error.status || 502,
          latency_ms: Math.round(now() - at),
          input_tokens: null,
          cached_tokens: null,
          output_tokens: null,
          cost_microusd: null,
          price_verified: chosen.pricing.verified,
          prefix_hash: prefixHash,
          decision_id: null,
          error_kind: error.kind,
          task_type: taskType,
        });
        if (routed && error.retryable && failovers < config.routing.max_failovers) {
          excluded.add(provider.id);
          failovers += 1;
          const remaining = router.pool().some((m) => !excluded.has(m.provider));
          if (remaining) continue;
        }
        const status = error.kind === "not_configured" ? 503 : error.status >= 400 ? error.status : 502;
        let upstreamBody: unknown = null;
        try {
          upstreamBody = JSON.parse(error.body);
        } catch {
          /* not JSON */
        }
        return json(status, isRecord(upstreamBody) && upstreamBody.error ? upstreamBody : { error: { message: error.message, type: "upstream_error", code: error.kind, provider: provider.id } }, {
          [NIMBLE_VERSION_HEADER]: NIMBLE_CONTRACT_VERSION,
          [NIMBLE_REQUEST_ID_HEADER]: requestId,
        });
      }

      health.recordSuccess(provider.id, upstream.ttfb_ms);
      const decisionId = decision ? newId("d") : null;
      if (decision && decisionId) {
        store.recordDecision({
          id: decisionId,
          at,
          request_id: requestId,
          session_id: sessionId,
          chosen: chosen.id,
          reason: decision.reason,
          components: decision.components,
          model_changed: decision.model_changed,
          cache_affinity: decision.cache_affinity,
          estimated_cached_tokens: decision.estimated_cached_tokens,
        });
      }
      const modelChanged = session.current_model !== null && session.current_model !== chosen.id;
      store.putSession({
        ...session,
        current_model: chosen.id,
        prefix_hash: prefixHash,
        stable_tokens: stableTokens,
        last_request_at: at,
        requests: session.requests + 1,
        switches: session.switches + (modelChanged ? 1 : 0),
        escalated_attempt: decision?.decisive ? (attempt ?? 1) : session.escalated_attempt,
      });
      const facts = routingFacts(chosen, decision, requestId, decisionId);
      const factsHeaders = {
        [NIMBLE_ROUTING_HEADER]: JSON.stringify(facts),
        [NIMBLE_VERSION_HEADER]: NIMBLE_CONTRACT_VERSION,
        [NIMBLE_REQUEST_ID_HEADER]: requestId,
      };

      const finish = (usage: Usage | null, status: number) => {
        const latency = Math.round(now() - at);
        const cost = usage ? priceUsage(chosen, usage, at / 1000) : null;
        if (usage) health.observeTokens(provider.id, usage.input_tokens + usage.output_tokens);
        store.recordRequest({
          id: requestId,
          at,
          session_id: sessionId,
          requested_model: requestedModel,
          selected_model: chosen.id,
          provider: provider.id,
          routed,
          status,
          latency_ms: latency,
          input_tokens: usage?.input_tokens ?? null,
          cached_tokens: usage?.cached_tokens ?? null,
          output_tokens: usage?.output_tokens ?? null,
          cost_microusd: cost,
          price_verified: chosen.pricing.verified,
          prefix_hash: prefixHash,
          decision_id: decisionId,
          error_kind: null,
          task_type: taskType,
        });
        if (usage) {
          store.observeCache({
            provider: provider.id,
            model: chosen.id,
            prefix_hash: prefixHash,
            last_served_at: now(),
            prefix_tokens_seen: Math.max(usage.cached_tokens, Math.min(usage.input_tokens, stableTokens)),
          });
          store.observeOutput(chosen.id, taskType, usage.output_tokens);
          if (!extension?.estimated_stable_tokens) {
            const observedStable = usage.cached_tokens > 0 ? usage.cached_tokens : Math.min(usage.input_tokens, stableTokens);
            const current = store.getSession(sessionId);
            if (current) store.putSession({ ...current, stable_tokens: observedStable });
          }
        }
      };

      if (provider.dialect === "responses") {
        if (!stream) {
          const text = await upstream.response.text();
          let payload: unknown;
          try {
            payload = JSON.parse(text);
          } catch {
            finish(null, 502);
            return openAiError(502, `upstream ${provider.id} returned non-JSON`, "upstream_error");
          }
          const { completion, usage } = responsesToChatCompletion(isRecord(payload) ? payload : {}, requestId, now());
          finish(usage, 200);
          return json(200, { ...completion, nimble: facts }, factsHeaders);
        }
        const translator = new ResponsesToChat(chosen.id, requestId, now());
        const transformed = upstream.response.body!.pipeThrough(responsesSseTransform(translator, facts, finish));
        return new Response(transformed, {
          status: 200,
          headers: { "content-type": "text/event-stream", "cache-control": "no-cache", ...factsHeaders },
        });
      }

      if (!stream) {
        const text = await upstream.response.text();
        let payload: unknown;
        try {
          payload = JSON.parse(text);
        } catch {
          finish(null, 502);
          return openAiError(502, `upstream ${provider.id} returned non-JSON`, "upstream_error");
        }
        const usage = isRecord(payload) ? normalizeUsage(payload.usage) : null;
        finish(usage, 200);
        const withFacts = isRecord(payload) ? { ...payload, nimble: facts } : payload;
        return json(200, withFacts, factsHeaders);
      }

      const transformed = upstream.response.body!.pipeThrough(sseTransform(facts, finish));
      return new Response(transformed, {
        status: 200,
        headers: {
          "content-type": upstream.response.headers.get("content-type") ?? "text/event-stream",
          "cache-control": "no-cache",
          ...factsHeaders,
        },
      });
    }
  }

  /** Inject routing facts into the first data chunk and capture usage from the last. */
  function sseTransform(facts: NimbleRoutingResponse, finish: (usage: Usage | null, status: number) => void): TransformStream<Uint8Array, Uint8Array> {
    const decoder = new TextDecoder();
    const encoder = new TextEncoder();
    let pending = "";
    let injected = false;
    let usage: Usage | null = null;
    let finished = false;
    const handleLine = (line: string): string => {
      if (!line.startsWith("data:")) return line;
      const payload = line.slice(5).trim();
      if (payload === "[DONE]" || payload.length === 0) return line;
      let chunk: unknown;
      try {
        chunk = JSON.parse(payload);
      } catch {
        return line;
      }
      if (!isRecord(chunk)) return line;
      if (chunk.usage) usage = normalizeUsage(chunk.usage) ?? usage;
      if (!injected) {
        injected = true;
        return `data: ${JSON.stringify({ ...chunk, nimble: facts })}`;
      }
      return line;
    };
    return new TransformStream({
      transform(bytes, controller) {
        pending += decoder.decode(bytes, { stream: true });
        let index;
        while ((index = pending.indexOf("\n")) >= 0) {
          const line = pending.slice(0, index).replace(/\r$/, "");
          pending = pending.slice(index + 1);
          controller.enqueue(encoder.encode(handleLine(line) + "\n"));
        }
      },
      flush(controller) {
        if (pending.length > 0) controller.enqueue(encoder.encode(handleLine(pending) + "\n"));
        if (!finished) {
          finished = true;
          finish(usage, 200);
        }
      },
    });
  }

  /** Responses events in → Chat Completions chunks out, facts on the first chunk. */
  function responsesSseTransform(translator: ResponsesToChat, facts: NimbleRoutingResponse, finish: (usage: Usage | null, status: number) => void): TransformStream<Uint8Array, Uint8Array> {
    const decoder = new TextDecoder();
    const encoder = new TextEncoder();
    let pending = "";
    let injected = false;
    let finished = false;
    const emit = (controller: TransformStreamDefaultController<Uint8Array>, chunk: Record<string, unknown>) => {
      const withFacts = injected ? chunk : { ...chunk, nimble: facts };
      injected = true;
      controller.enqueue(encoder.encode(`data: ${JSON.stringify(withFacts)}\n\n`));
    };
    const handleLine = (line: string, controller: TransformStreamDefaultController<Uint8Array>) => {
      if (!line.startsWith("data:")) return;
      const payload = line.slice(5).trim();
      if (!payload || payload === "[DONE]") return;
      let event: unknown;
      try {
        event = JSON.parse(payload);
      } catch {
        return;
      }
      for (const chunk of translator.consume(event)) emit(controller, chunk);
    };
    return new TransformStream({
      transform(bytes, controller) {
        pending += decoder.decode(bytes, { stream: true });
        let index;
        while ((index = pending.indexOf("\n")) >= 0) {
          const line = pending.slice(0, index).replace(/\r$/, "");
          pending = pending.slice(index + 1);
          handleLine(line, controller);
        }
      },
      flush(controller) {
        if (pending.length > 0) handleLine(pending, controller);
        const { chunks, usage, error } = translator.finishChunks();
        if (error) {
          controller.enqueue(encoder.encode(`data: ${JSON.stringify({ error: { message: error, type: "upstream_error" } })}\n\n`));
        } else {
          for (const chunk of chunks) emit(controller, chunk);
        }
        controller.enqueue(encoder.encode("data: [DONE]\n\n"));
        if (!finished) {
          finished = true;
          finish(usage, error ? 502 : 200);
        }
      },
    });
  }

  async function feedback(request: Request): Promise<Response> {
    let body: unknown;
    try {
      body = await request.json();
    } catch {
      return openAiError(400, "feedback must be JSON", "invalid_request_error");
    }
    const parsed = parseNimbleFeedback(body);
    if (!parsed.ok) return openAiError(400, parsed.error, "invalid_request_error");
    const fb = parsed.feedback;
    const joined = fb.request_id ? store.getRequest(fb.request_id) : store.lastRequestForSession(fb.session_id);
    // Clients behind a router cannot price the routed model; Nimble priced every
    // request itself, so its own session total is authoritative when the client
    // sent nothing (or a zero from an unverified card).
    const own = store.sessionCostMicroUsd(fb.session_id);
    const cost = fb.cost_microusd && fb.cost_microusd > 0 ? fb.cost_microusd : own.cost > 0 ? own.cost : null;
    const outcome = store.recordFeedback({
      session_id: fb.session_id,
      request_id: fb.request_id ?? joined?.id ?? null,
      at: now(),
      task_type: fb.task_type ?? joined?.task_type ?? null,
      task_success: fb.task_success ?? null,
      verification_result: fb.verification_result ?? null,
      failure_type: fb.failure_type ?? null,
      attempt_count: fb.attempt_count ?? null,
      cost_microusd: cost,
      model: joined?.selected_model ?? null,
    });
    if (fb.failure_type && joined) {
      const session = store.getSession(fb.session_id);
      if (session) store.putSession({ ...session, recent_failures: [...session.recent_failures, joined.selected_model].slice(-5) });
    }
    const applied = outcome !== "duplicate";
    return json(applied ? 202 : 200, {
      accepted: applied,
      outcome,
      joined_model: joined?.selected_model ?? null,
      duplicate: !applied,
      cost_microusd: cost,
      cost_source: fb.cost_microusd && fb.cost_microusd > 0 ? "client" : own.cost > 0 ? "nimble" : null,
    });
  }

  return {
    router,
    health,
    store,
    async fetch(request: Request): Promise<Response> {
      const url = new URL(request.url);
      if (url.pathname === "/health") {
        return json(200, { ok: true, version: NIMBLE_CONTRACT_VERSION, pool: config.routing.pool, providers: health.snapshot() });
      }
      if (!authorized(request)) return openAiError(401, "invalid or missing bearer token", "authentication_error");
      if (url.pathname === "/v1/models" && request.method === "GET") {
        const created = Math.floor(now() / 1000);
        return json(200, {
          object: "list",
          data: [
            { id: VIRTUAL_MODEL, object: "model", created, owned_by: "nimble" },
            ...config.models.map((m) => ({ id: m.id, object: "model", created, owned_by: m.provider })),
          ],
        });
      }
      if (url.pathname === "/v1/chat/completions" && request.method === "POST") return chatCompletions(request);
      if (url.pathname === "/v1/feedback" && request.method === "POST") return feedback(request);
      if (url.pathname === "/v1/routing/decisions" && request.method === "GET") {
        const limit = Math.min(200, Number(url.searchParams.get("limit") ?? 20) || 20);
        return json(200, { decisions: store.recentDecisions(url.searchParams.get("session_id"), limit) });
      }
      if (url.pathname === "/v1/stats" && request.method === "GET") {
        const rows = store.allStats(url.searchParams.get("task_type") ?? undefined).map((s) => ({
          ...s,
          success_rate: s.n > 0 ? s.successes / s.n : null,
          cost_per_task_microusd: s.n > 0 ? Math.round(s.cost_sum / s.n) : null,
          attempts_per_task: s.n > 0 ? s.attempts_sum / s.n : null,
          mean_output_tokens: s.output_samples > 0 ? Math.round(s.output_tokens_sum / s.output_samples) : null,
        }));
        return json(200, { stats: rows });
      }
      return openAiError(404, `no route for ${request.method} ${url.pathname}`, "invalid_request_error");
    },
  };
}

export function startServer(deps: AppDeps) {
  const app = createApp(deps);
  return Bun.serve({
    hostname: deps.config.host,
    port: deps.config.port,
    idleTimeout: 255,
    fetch: (request) => app.fetch(request),
  });
}

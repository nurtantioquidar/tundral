import { createHash, randomUUID } from "node:crypto";
import { loadGatewayConfig } from "./config";
import { CredentialCipher } from "./upstreams/credential-cipher";
import { GatewayDatabase } from "./database";
import { calculateChargeMicros, formatUsd, MODEL_RATES, publicModel, RATE_CARD_VERSION } from "./billing/pricing";
import { estimateInputTokens, extractUsage } from "./generation/usage";
import { codexConversationFromRequest, codexPayload } from "./upstreams/adapters/codex";
import { runCodexAppServer, type AppServerCallbacks } from "./upstreams/adapters/codex-app-server";
import { OpenAiUpstreamError } from "./upstreams/adapters/openai-pool";
import { ModelRouteError } from "./upstreams/model-router";
import { requestedReasoningEffort, type ReasoningEffort } from "./upstreams/reasoning-effort";
import { createBufferedSseResponse, createLiveSseResponse, type ExecutionResult } from "./generation/streaming";
import { UpstreamRegistry } from "./upstreams/upstream-registry";
import { routeInternalRequest } from "./http/internal-router";
import { renderMetrics } from "./http/metrics";
import { apiError as error, bearerToken as bearer, tokenMatches } from "./http/response";

const config = loadGatewayConfig();
const db = new GatewayDatabase(config.databasePath);
const databaseRouting = config.upstreamConfigSource === "database";
const upstreamRegistry = !databaseRouting && config.upstreamMode !== "openai"
  ? null
  : new UpstreamRegistry(
      db,
      CredentialCipher.fromEnvironment(),
      config.upstreamConfigSource,
    );
const inflight = new Map<string, number>();
const requestWindows = new Map<string, number[]>();
let globalInflight = 0;
let generationEnabled = config.generationEnabled;

function upstreamIsReady(): boolean {
  return databaseRouting
    ? Boolean(upstreamRegistry?.configured)
    : config.upstreamMode === "openai"
      ? Boolean(upstreamRegistry?.configured)
      : Boolean(Bun.which(config.codexExecutable));
}

const startupRecovery = db.recoverStale();
if (startupRecovery.releasedReservations || startupRecovery.failedIdempotency) {
  console.log(`recovered ${startupRecovery.releasedReservations} reservations and ${startupRecovery.failedIdempotency} idempotency records`);
}
const recoveryTimer = setInterval(() => db.recoverStale(), config.recoveryIntervalMs);
(recoveryTimer as unknown as { unref?: () => void }).unref?.();

class ExecutionFailure extends Error {
  constructor(message: string, readonly status = 502, readonly type = "upstream_error") {
    super(message);
  }
}

function authorize(req: Request) {
  const key = bearer(req);
  return key ? db.authenticate(key) : null;
}

function consumeRateLimit(apiKeyId: string, limit: number): boolean {
  const now = Date.now();
  const active = (requestWindows.get(apiKeyId) || []).filter((timestamp) => timestamp > now - 60_000);
  if (active.length >= limit) {
    requestWindows.set(apiKeyId, active);
    return false;
  }
  active.push(now);
  requestWindows.set(apiKeyId, active);
  return true;
}

function maxOutput(body: Record<string, unknown>): number {
  const raw = body.max_output_tokens ?? body.max_completion_tokens ?? body.max_tokens;
  const value = raw ?? config.defaultMaxOutputTokens;
  const maximum = config.maxOutputTokensPerRequest;
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0) {
    throw new Error("Output token limit must be a positive integer");
  }
  if (!Number.isSafeInteger(maximum) || maximum <= 0) throw new Error("Gateway output limit is invalid");
  if (value > maximum) throw new Error(`Output token limit exceeds gateway maximum of ${maximum}`);
  return value;
}

async function proxyMetered(req: Request, path: string, auth: NonNullable<ReturnType<typeof authorize>>) {
  if (!generationEnabled) return error("Model generation is temporarily disabled", 503, "server_error");
  if ((databaseRouting || config.upstreamMode === "openai") && !upstreamRegistry?.configured) {
    return error("Gateway upstream is not configured", 503, "server_error");
  }
  const rawBody = await req.text();
  let body: Record<string, unknown>;
  try { body = JSON.parse(rawBody) as Record<string, unknown>; }
  catch { return error("Request body must be valid JSON", 400); }
  const wantsStream = body.stream === true;
  const requestedModel = typeof body.model === "string" ? body.model : "";
  const rate = MODEL_RATES[requestedModel];
  if (!rate) return error(`Unsupported model: ${requestedModel}`, 400);
  const requestPath = path as "/v1/chat/completions" | "/v1/responses";
  if (databaseRouting && !upstreamRegistry!.supports(requestedModel, requestPath)) {
    return error(`Model is not currently routable: ${requestedModel}`, 503, "server_error");
  }
  /* Effort is the harness's per-request choice — validate it before reserving money so a typo fails
   * fast instead of surfacing as an upstream error after a hold was taken. */
  let reasoningEffort: ReasoningEffort | undefined;
  try { reasoningEffort = requestedReasoningEffort(requestPath, body); }
  catch (cause) { return error(cause instanceof Error ? cause.message : "Invalid reasoning effort", 400); }
  const requestUsesCodex = databaseRouting
    ? upstreamRegistry!.usesCodex(requestedModel)
    : config.upstreamMode === "codex-cli";
  if (requestUsesCodex && body.parallel_tool_calls === true) {
    return error("parallel_tool_calls=true is not supported; use one tool call per turn", 400);
  }
  let outputLimit: number;
  try { outputLimit = maxOutput(body); }
  catch (cause) { return error(cause instanceof Error ? cause.message : "Invalid output token limit", 400); }

  const rawIdempotencyKey = req.headers.get("idempotency-key")?.trim() || null;
  if (rawIdempotencyKey && !/^[A-Za-z0-9._:-]{1,200}$/.test(rawIdempotencyKey)) {
    return error("Idempotency-Key must be 1-200 URL-safe characters", 400);
  }
  const requestHash = createHash("sha256").update(path).update("\0").update(rawBody).digest("hex");
  const idempotency = rawIdempotencyKey
    ? db.beginIdempotentRequest(auth.apiKeyId, rawIdempotencyKey, requestHash)
    : { state: "started" as const, requestId: randomUUID() };
  if (idempotency.state === "conflict") return error("Idempotency-Key was already used with a different request", 409, "idempotency_error");
  if (idempotency.state === "in_progress") return error("A request with this Idempotency-Key is still in progress", 409, "idempotency_error");
  if (idempotency.state === "completed") {
    const replay = { payload: idempotency.payload, status: idempotency.status };
    if (wantsStream) {
      return createBufferedSseResponse(path, requestedModel, Promise.resolve(replay), {
        "x-request-id": idempotency.requestId,
        "x-idempotent-replay": "true",
      });
    }
    return Response.json(replay.payload, {
      status: replay.status,
      headers: { "x-request-id": idempotency.requestId, "x-idempotent-replay": "true" },
    });
  }
  const requestId = idempotency.requestId;

  const current = inflight.get(auth.apiKeyId) ?? 0;
  if (current >= auth.maxConcurrency) {
    db.failIdempotentRequest(auth.apiKeyId, rawIdempotencyKey);
    return error("Concurrent request limit exceeded", 429, "rate_limit_error");
  }
  if (globalInflight >= config.maxGlobalConcurrentRequests) {
    db.failIdempotentRequest(auth.apiKeyId, rawIdempotencyKey);
    return new Response(JSON.stringify({ error: { message: "Gateway capacity is currently full", type: "rate_limit_error" } }), {
      status: 429,
      headers: { "content-type": "application/json", "retry-after": "5" },
    });
  }
  if (!consumeRateLimit(auth.apiKeyId, auth.rateLimitPerMinute)) {
    db.failIdempotentRequest(auth.apiKeyId, rawIdempotencyKey);
    return new Response(JSON.stringify({ error: { message: "Request rate limit exceeded", type: "rate_limit_error" } }), {
      status: 429,
      headers: { "content-type": "application/json", "retry-after": "60" },
    });
  }

  const codexOverhead = requestUsesCodex
    ? config.codexReservationOverheadTokens
    : 0;
  const estimatedUsage = {
    inputTokens: estimateInputTokens(body) + codexOverhead, cachedInputTokens: 0, outputTokens: outputLimit,
  };
  const reservationAmount = calculateChargeMicros(rate, estimatedUsage);
  let reservationId: string;
  try {
    reservationId = db.reserve({
      customerId: auth.customerId,
      apiKeyId: auth.apiKeyId,
      requestId,
      amountMicros: reservationAmount,
      keySpendLimitMicros: auth.keySpendLimitMicros,
      perRequestLimitMicros: auth.perRequestLimitMicros,
      dailySpendLimitMicros: auth.dailySpendLimitMicros,
    });
  } catch (cause) {
    db.failIdempotentRequest(auth.apiKeyId, rawIdempotencyKey);
    if (cause instanceof Error && cause.message === "insufficient_balance") return error("Insufficient balance", 402, "billing_error");
    if (cause instanceof Error && cause.message === "key_spend_limit") return error("API key spending limit reached", 402, "billing_error");
    if (cause instanceof Error && cause.message === "per_request_limit") return error("Request exceeds customer spending limit", 402, "billing_error");
    if (cause instanceof Error && cause.message === "daily_spend_limit") return error("Customer daily spending limit reached", 402, "billing_error");
    throw cause;
  }

  inflight.set(auth.apiKeyId, current + 1);
  globalInflight += 1;
  const execute = async (callbacks: AppServerCallbacks = {}): Promise<ExecutionResult> => {
    try {
    let payload: Record<string, unknown>;
    let upstreamStatus = 200;
    let routeRevision: number | undefined;
    let routeTargetId: string | undefined;
    if (databaseRouting) {
      const routed = await upstreamRegistry!.execute({
        path: requestPath,
        publicModel: requestedModel,
        body,
        outputLimit,
        callbacks,
      });
      payload = routed.payload;
      upstreamStatus = routed.status;
      routeRevision = routed.routeRevision;
      routeTargetId = routed.routeTargetId;
    } else if (config.upstreamMode === "codex-cli") {
      const result = await runCodexAppServer(
        rate.upstreamModel,
        reasoningEffort,
        outputLimit,
        codexConversationFromRequest(path, body),
        body.tools,
        callbacks,
      );
      payload = codexPayload(path, requestedModel, result);
    } else {
      const upstream = await upstreamRegistry!.pool.execute({
        path: requestPath,
        body,
        upstreamModel: rate.upstreamModel,
        outputLimit,
      });
      upstreamStatus = upstream.status;
      payload = upstream.payload;
    }
    const usage = extractUsage(payload);
    const billableUsage = { ...usage, outputTokens: Math.min(usage.outputTokens, outputLimit) };
    const calculatedCostMicros = calculateChargeMicros(rate, billableUsage);
    const upstreamCalculatedCostMicros = calculateChargeMicros(rate, usage);
    const costMicros = calculatedCostMicros > reservationAmount ? reservationAmount : calculatedCostMicros;
    payload.gateway_usage = {
      ...usage,
      rate_card_version: RATE_CARD_VERSION,
      billable_output_tokens: billableUsage.outputTokens,
      requested_output_limit: outputLimit,
      output_limit_exceeded: usage.outputTokens > outputLimit,
      cost_usd: formatUsd(costMicros),
      calculated_cost_usd: formatUsd(calculatedCostMicros),
      upstream_calculated_cost_usd: formatUsd(upstreamCalculatedCostMicros),
      reservation_capped: calculatedCostMicros > reservationAmount,
    };
    db.settle({
      reservationId, requestId, customerId: auth.customerId, apiKeyId: auth.apiKeyId,
      model: requestedModel, ...usage, cacheWriteTokens: usage.cacheWriteTokens ?? 0,
      costMicros, upstreamStatus, rateCardVersion: RATE_CARD_VERSION,
      idempotencyKey: rawIdempotencyKey,
      responseJson: rawIdempotencyKey ? JSON.stringify(payload) : undefined,
      responseStatus: upstreamStatus,
      routeRevision,
      routeTargetId,
    });
    return { payload, status: upstreamStatus };
    } catch (cause) {
      db.release(reservationId);
      db.failIdempotentRequest(auth.apiKeyId, rawIdempotencyKey);
      if (cause instanceof ExecutionFailure) throw cause;
      if (cause instanceof OpenAiUpstreamError) {
        throw new ExecutionFailure(cause.message, cause.status, cause.type);
      }
      if (cause instanceof ModelRouteError) {
        throw new ExecutionFailure(cause.message, cause.status, cause.type);
      }
      throw new ExecutionFailure(cause instanceof Error ? cause.message : "Upstream failure");
    } finally {
      const remaining = (inflight.get(auth.apiKeyId) ?? 1) - 1;
      if (remaining <= 0) inflight.delete(auth.apiKeyId); else inflight.set(auth.apiKeyId, remaining);
      globalInflight = Math.max(0, globalInflight - 1);
    }
  };

  if (wantsStream && (databaseRouting || config.upstreamMode === "codex-cli")) {
    return createLiveSseResponse(
      path,
      requestedModel,
      (callbacks) => execute(callbacks),
      { "x-request-id": requestId },
    );
  }
  const execution = execute();
  if (wantsStream) return createBufferedSseResponse(path, requestedModel, execution);
  try {
    const result = await execution;
    return Response.json(result.payload, { status: result.status, headers: { "x-request-id": requestId } });
  } catch (cause) {
    const failure = cause instanceof ExecutionFailure ? cause : new ExecutionFailure("Upstream failure");
    return error(failure.message, failure.status, failure.type);
  }
}

Bun.serve({
  hostname: config.host,
  port: config.port,
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === "/health") return Response.json({ status: "ok" });
    if (url.pathname === "/metrics") {
      if (!tokenMatches(config.metricsApiKey, bearer(req))) {
        return error("Not found", 404);
      }
      return new Response(renderMetrics({
        databaseReady: db.ready(),
        upstreamConfigured: upstreamIsReady(),
        generationEnabled,
        inflightRequests: globalInflight,
        configurationSource: config.upstreamConfigSource,
      }), {
        headers: { "content-type": "text/plain; version=0.0.4; charset=utf-8" },
      });
    }
    if (url.pathname === "/ready") {
      const upstreamReady = upstreamIsReady();
      const ready = db.ready() && upstreamReady && generationEnabled;
      return Response.json({ status: ready ? "ready" : "not_ready", database: db.ready(), upstream: upstreamReady, generation_enabled: generationEnabled }, { status: ready ? 200 : 503 });
    }

    const internalResponse = await routeInternalRequest(req, url, {
      db,
      adminApiKey: config.adminApiKey,
      webBffApiKey: config.webBffApiKey,
      upstreamRegistry,
      setGenerationEnabled(enabled) {
        generationEnabled = enabled;
      },
    });
    if (internalResponse) return internalResponse;
    const auth = authorize(req);
    if (!auth) return error("Invalid or missing API key", 401, "authentication_error");

    if (url.pathname === "/v1/models" && req.method === "GET") {
      const available = upstreamRegistry
        ? new Set(upstreamRegistry.publicModels())
        : new Set(Object.keys(MODEL_RATES));
      return Response.json({
        object: "list",
        data: Object.values(MODEL_RATES).filter((rate) => available.has(rate.id)).map(publicModel),
      });
    }
    if (url.pathname === "/v1/pricing" && req.method === "GET") {
      const available = upstreamRegistry
        ? new Set(upstreamRegistry.publicModels())
        : new Set(Object.keys(MODEL_RATES));
      return Response.json({
        rate_card_version: RATE_CARD_VERSION,
        currency: "USD",
        unit: "million_tokens",
        data: Object.values(MODEL_RATES).filter((rate) => available.has(rate.id)).map(publicModel),
      });
    }
    if (url.pathname === "/v1/usage" && req.method === "GET") {
      return Response.json({ balance_usd: formatUsd(db.balanceMicros(auth.customerId)), data: db.usage(auth.customerId, Number(url.searchParams.get("limit") || 100)) });
    }
    if (url.pathname === "/v1/chat/completions" && req.method === "POST") return proxyMetered(req, url.pathname, auth);
    if (url.pathname === "/v1/responses" && req.method === "POST") return proxyMetered(req, url.pathname, auth);
    return error("Not found", 404);
  },
});

console.log(`coding-gateway listening on http://${config.host}:${config.port}`);

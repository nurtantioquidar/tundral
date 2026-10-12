// Butterfly ↔ Nimble optimization contract, version nimble/v1. See
// docs/butterfly-nimble-contract.md. Every field is optional on the wire; parsers are lenient
// (unknown fields ignored, invalid fields dropped) so a partial extension never breaks a request.

export const NIMBLE_CONTRACT_VERSION = "nimble/v1";
export const NIMBLE_ROUTING_HEADER = "x-nimble-routing";
export const NIMBLE_VERSION_HEADER = "x-nimble-version";
export const NIMBLE_REQUEST_ID_HEADER = "x-nimble-request-id";
/** Request-extension header prefix: `x-nimble-session-id`, `x-nimble-task-type`, … */
export const NIMBLE_HEADER_PREFIX = "x-nimble-";

export const NIMBLE_TASK_TYPES = ["debugging", "feature", "refactor", "test", "explain", "ops", "unknown"] as const;
export type NimbleTaskType = (typeof NIMBLE_TASK_TYPES)[number];

export const NIMBLE_TASK_COMPLEXITIES = ["low", "medium", "high"] as const;
export type NimbleTaskComplexity = (typeof NIMBLE_TASK_COMPLEXITIES)[number];

export const NIMBLE_AGENT_PHASES = ["planning", "implementation", "verification", "recovery", "summarization"] as const;
export type NimbleAgentPhase = (typeof NIMBLE_AGENT_PHASES)[number];

export const NIMBLE_VERIFICATION_RESULTS = ["passed", "failed", "not_run"] as const;
export type NimbleVerificationResult = (typeof NIMBLE_VERIFICATION_RESULTS)[number];

/** Client → Nimble, carried as body `nimble` and/or `x-nimble-*` headers. */
export interface NimbleRequestExtension {
  version?: string;
  session_id?: string;
  task_type?: NimbleTaskType;
  task_complexity?: NimbleTaskComplexity;
  agent_phase?: NimbleAgentPhase;
  attempt?: number;
  previous_failure?: string;
  verification_failed?: boolean;
  stable_prefix_hash?: string;
  estimated_stable_tokens?: number;
  estimated_dynamic_tokens?: number;
  context_version?: number;
}

export interface NimbleModelCapabilities {
  tools: boolean;
  parallel_tools: boolean;
  reasoning: boolean;
  prompt_cache: "automatic" | "none";
  images: boolean;
}

/** Nimble → client, in the `x-nimble-routing` header and the first stream chunk's `nimble` field. */
export interface NimbleRoutingResponse {
  version: typeof NIMBLE_CONTRACT_VERSION;
  selected_model: string;
  provider: string;
  model_changed: boolean;
  cache_affinity: number;
  estimated_cached_tokens: number;
  context_limit: number;
  model_capabilities: NimbleModelCapabilities;
  routing_reason: string;
  decision_id: string | null;
  request_id: string;
}

/** Client → Nimble task outcome (`POST /v1/feedback`). */
export interface NimbleFeedback {
  version?: string;
  session_id: string;
  request_id?: string | null;
  task_type?: NimbleTaskType | null;
  task_success?: boolean | null;
  verification_result?: NimbleVerificationResult | null;
  failure_type?: string | null;
  attempt_count?: number | null;
  cost_microusd?: number | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 && value.length <= 512 ? value : undefined;
}

function optionalNonNegativeInt(value: unknown): number | undefined {
  if (typeof value === "string" && /^\d+$/.test(value)) value = Number(value);
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : undefined;
}

function optionalBoolean(value: unknown): boolean | undefined {
  if (value === "true") return true;
  if (value === "false") return false;
  return typeof value === "boolean" ? value : undefined;
}

function optionalEnum<T extends string>(value: unknown, allowed: readonly T[]): T | undefined {
  return typeof value === "string" && (allowed as readonly string[]).includes(value) ? (value as T) : undefined;
}

/**
 * Parse a request extension object. Returns null when the value is not an object or declares a
 * version this contract does not understand (the receiver must then ignore the extension).
 */
export function parseNimbleRequestExtension(value: unknown): NimbleRequestExtension | null {
  if (!isRecord(value)) return null;
  const version = optionalString(value.version);
  if (version !== undefined && version !== NIMBLE_CONTRACT_VERSION) return null;
  const extension: NimbleRequestExtension = {};
  if (version) extension.version = version;
  const sessionId = optionalString(value.session_id);
  if (sessionId) extension.session_id = sessionId;
  const taskType = optionalEnum(value.task_type, NIMBLE_TASK_TYPES);
  if (taskType) extension.task_type = taskType;
  const complexity = optionalEnum(value.task_complexity, NIMBLE_TASK_COMPLEXITIES);
  if (complexity) extension.task_complexity = complexity;
  const phase = optionalEnum(value.agent_phase, NIMBLE_AGENT_PHASES);
  if (phase) extension.agent_phase = phase;
  const attempt = optionalNonNegativeInt(value.attempt);
  if (attempt !== undefined && attempt >= 1) extension.attempt = attempt;
  const failure = optionalString(value.previous_failure);
  if (failure) extension.previous_failure = failure;
  const verificationFailed = optionalBoolean(value.verification_failed);
  if (verificationFailed !== undefined) extension.verification_failed = verificationFailed;
  const prefixHash = optionalString(value.stable_prefix_hash);
  if (prefixHash) extension.stable_prefix_hash = prefixHash;
  const stable = optionalNonNegativeInt(value.estimated_stable_tokens);
  if (stable !== undefined) extension.estimated_stable_tokens = stable;
  const dynamic = optionalNonNegativeInt(value.estimated_dynamic_tokens);
  if (dynamic !== undefined) extension.estimated_dynamic_tokens = dynamic;
  const contextVersion = optionalNonNegativeInt(value.context_version);
  if (contextVersion !== undefined) extension.context_version = contextVersion;
  return extension;
}

/** Read `x-nimble-<field>` headers (kebab-case) into an extension object. */
export function nimbleRequestExtensionFromHeaders(headers: { get(name: string): string | null }): NimbleRequestExtension | null {
  const raw: Record<string, unknown> = {};
  const fields = [
    "version",
    "session_id",
    "task_type",
    "task_complexity",
    "agent_phase",
    "attempt",
    "previous_failure",
    "verification_failed",
    "stable_prefix_hash",
    "estimated_stable_tokens",
    "estimated_dynamic_tokens",
    "context_version",
  ];
  let any = false;
  for (const field of fields) {
    const value = headers.get(NIMBLE_HEADER_PREFIX + field.replace(/_/g, "-"));
    if (value !== null) {
      raw[field] = value;
      any = true;
    }
  }
  if (!any) return null;
  return parseNimbleRequestExtension(raw);
}

/** Body wins over headers field by field. */
export function mergeNimbleRequestExtensions(
  body: NimbleRequestExtension | null,
  headers: NimbleRequestExtension | null,
): NimbleRequestExtension | null {
  if (!body && !headers) return null;
  return { ...(headers ?? {}), ...(body ?? {}) };
}

export type NimbleFeedbackParse = { ok: true; feedback: NimbleFeedback } | { ok: false; error: string };

export function parseNimbleFeedback(value: unknown): NimbleFeedbackParse {
  if (!isRecord(value)) return { ok: false, error: "feedback must be a JSON object" };
  const version = optionalString(value.version);
  if (version !== undefined && version !== NIMBLE_CONTRACT_VERSION) {
    return { ok: false, error: `unsupported version ${version}; expected ${NIMBLE_CONTRACT_VERSION}` };
  }
  const sessionId = optionalString(value.session_id);
  if (!sessionId) return { ok: false, error: "session_id is required" };
  const feedback: NimbleFeedback = { session_id: sessionId };
  if (version) feedback.version = version;
  const requestId = optionalString(value.request_id);
  feedback.request_id = requestId ?? null;
  feedback.task_type = optionalEnum(value.task_type, NIMBLE_TASK_TYPES) ?? null;
  const success = optionalBoolean(value.task_success);
  feedback.task_success = success ?? null;
  feedback.verification_result = optionalEnum(value.verification_result, NIMBLE_VERIFICATION_RESULTS) ?? null;
  feedback.failure_type = optionalString(value.failure_type) ?? null;
  const attempts = optionalNonNegativeInt(value.attempt_count);
  feedback.attempt_count = attempts ?? null;
  const cost = optionalNonNegativeInt(value.cost_microusd);
  feedback.cost_microusd = cost ?? null;
  return { ok: true, feedback };
}

export function parseNimbleRoutingResponse(value: unknown): NimbleRoutingResponse | null {
  if (!isRecord(value)) return null;
  if (value.version !== NIMBLE_CONTRACT_VERSION) return null;
  const selected = optionalString(value.selected_model);
  const provider = optionalString(value.provider);
  const requestId = optionalString(value.request_id);
  if (!selected || !provider || !requestId) return null;
  const caps = isRecord(value.model_capabilities) ? value.model_capabilities : {};
  const promptCache = caps.prompt_cache === "none" ? "none" : "automatic";
  const affinity = typeof value.cache_affinity === "number" ? Math.min(1, Math.max(0, value.cache_affinity)) : 0;
  return {
    version: NIMBLE_CONTRACT_VERSION,
    selected_model: selected,
    provider,
    model_changed: value.model_changed === true,
    cache_affinity: affinity,
    estimated_cached_tokens: optionalNonNegativeInt(value.estimated_cached_tokens) ?? 0,
    context_limit: optionalNonNegativeInt(value.context_limit) ?? 0,
    model_capabilities: {
      tools: caps.tools !== false,
      parallel_tools: caps.parallel_tools === true,
      reasoning: caps.reasoning === true,
      prompt_cache: promptCache,
      images: caps.images === true,
    },
    routing_reason: optionalString(value.routing_reason) ?? "",
    decision_id: optionalString(value.decision_id) ?? null,
    request_id: requestId,
  };
}

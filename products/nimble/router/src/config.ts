// Nimble configuration: providers, the model pool with pricing and capabilities, routing
// weights, and health thresholds. Loaded from a JSON file (`NIMBLE_CONFIG`, default
// `nimble.config.json` next to the package) with a few environment overrides. Nothing about the
// pool is hard-coded; the shipped file is the initial GLM Flash / DeepSeek Flash / Luna pool.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export interface ProviderConfig {
  id: string;
  /** Endpoint without `/chat/completions`. Empty when `base_url_env` must supply it. */
  base_url: string;
  base_url_env?: string;
  api_key_env: string;
  /** Name of the output-budget field the upstream accepts (`max_tokens` default;
   *  OpenAI reasoning models need `max_completion_tokens`). */
  max_tokens_param: "max_tokens" | "max_completion_tokens";
  /** Provider-side tokens-per-minute cap for this account, when known. The router keeps a
   *  sliding one-minute window of tokens sent and will not pick a provider whose window plus
   *  the request's estimate would exceed it (observed 2026-09-05: a 500k TPM cap on 100k-token
   *  requests became a fatal in-stream error). */
  tokens_per_minute?: number;
  /** `chat` = OpenAI-compatible Chat Completions passthrough; `responses` = translate to the
   *  OpenAI Responses API (function tools + reasoning on OpenAI's reasoning models). */
  dialect: "chat" | "responses";
}

export interface PricingConfig {
  /** Micro-USD per million tokens at the base (peak) rate. */
  cache_hit_input: number;
  cache_miss_input: number;
  output: number;
  version: string;
  verified: boolean;
  /** Optional UTC peak windows `[start, end)`; off-peak divides every rate by `off_peak_divisor`. */
  tiers?: { peak_windows_utc: [number, number][]; business_days_only: boolean; off_peak_divisor: number };
}

export interface CapabilitiesConfig {
  tools: boolean;
  parallel_tools: boolean;
  reasoning: boolean;
  prompt_cache: "automatic" | "none";
  images: boolean;
}

export interface ModelConfig {
  id: string;
  provider: string;
  display_name: string;
  /** 1 = cheapest/weakest … n = strongest; drives escalation. */
  tier: number;
  context_tokens: number;
  max_output_tokens: number;
  pricing: PricingConfig;
  capabilities: CapabilitiesConfig;
  /** How long a served prefix is assumed warm upstream. */
  cache_ttl_seconds: number;
  /** Optional prior per task_type in [0, 1]. */
  task_affinity?: Record<string, number>;
  /**
   * Evidence prior in [0, 1]: measured success on the operator's own task
   * suite, used when no task_affinity matches. Falls back to tier / max tier
   * when absent. Decoupled from `tier` so the escalation ladder (by cost) can
   * differ from who is expected to succeed first.
   */
  prior?: number;
  /** Expected output tokens when no observation exists. */
  default_output_tokens?: number;
}

export interface RoutingWeights {
  capability: number;
  history: number;
  cache: number;
  cost: number;
  latency: number;
  risk: number;
  switch: number;
}

export interface RoutingConfig {
  /** Model ids eligible for `model: "nimble"`. */
  pool: string[];
  weights: RoutingWeights;
  /** Fixed part of the switch penalty (style re-priming), in score units. */
  switch_penalty_fixed: number;
  /** Added to capability, scaled by tier / max tier, per escalation trigger. */
  escalation_bonus: number;
  /**
   * From this attempt on, a verified failure makes escalation decisive: only higher tiers than
   * the current model receive the bonus and the switch penalty is waived (a solution is worth
   * more than cache locality by then).
   */
  escalation_after_attempts: number;
  escalation_failures: string[];
  default_output_tokens: number;
  /** Extra candidates tried when the chosen provider fails before streaming. */
  max_failovers: number;
}

export interface HealthConfig {
  consecutive_failures_to_open: number;
  cooldown_seconds: number;
  max_cooldown_seconds: number;
  error_rate_to_open: number;
}

export interface NimbleConfig {
  host: string;
  port: number;
  db_path: string;
  providers: ProviderConfig[];
  models: ModelConfig[];
  routing: RoutingConfig;
  health: HealthConfig;
}

export const DEFAULT_WEIGHTS: RoutingWeights = {
  capability: 0.35,
  history: 0.2,
  cache: 0.2,
  cost: 0.15,
  latency: 0.05,
  risk: 0.3,
  switch: 0.25,
};

export class ConfigError extends Error {}

function expectString(value: unknown, path: string): string {
  if (typeof value !== "string" || value.length === 0) throw new ConfigError(`${path} must be a non-empty string`);
  return value;
}

function expectNumber(value: unknown, path: string, fallback?: number): number {
  if (value === undefined && fallback !== undefined) return fallback;
  if (typeof value !== "number" || !Number.isFinite(value)) throw new ConfigError(`${path} must be a number`);
  return value;
}

function expectBoolean(value: unknown, path: string, fallback?: boolean): boolean {
  if (value === undefined && fallback !== undefined) return fallback;
  if (typeof value !== "boolean") throw new ConfigError(`${path} must be a boolean`);
  return value;
}

function object(value: unknown, path: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new ConfigError(`${path} must be an object`);
  return value as Record<string, unknown>;
}

function parseProvider(raw: unknown, path: string): ProviderConfig {
  const p = object(raw, path);
  return {
    id: expectString(p.id, `${path}.id`),
    base_url: typeof p.base_url === "string" ? p.base_url : "",
    base_url_env: typeof p.base_url_env === "string" ? p.base_url_env : undefined,
    api_key_env: expectString(p.api_key_env, `${path}.api_key_env`),
    max_tokens_param: p.max_tokens_param === "max_completion_tokens" ? "max_completion_tokens" : "max_tokens",
    dialect: p.dialect === "responses" ? "responses" : "chat",
    tokens_per_minute: p.tokens_per_minute === undefined ? undefined : expectNumber(p.tokens_per_minute, `${path}.tokens_per_minute`),
  };
}

function parsePricing(raw: unknown, path: string): PricingConfig {
  const p = object(raw, path);
  const pricing: PricingConfig = {
    cache_hit_input: expectNumber(p.cache_hit_input, `${path}.cache_hit_input`),
    cache_miss_input: expectNumber(p.cache_miss_input, `${path}.cache_miss_input`),
    output: expectNumber(p.output, `${path}.output`),
    version: expectString(p.version, `${path}.version`),
    verified: expectBoolean(p.verified, `${path}.verified`, false),
  };
  if (p.tiers !== undefined) {
    const t = object(p.tiers, `${path}.tiers`);
    if (!Array.isArray(t.peak_windows_utc)) throw new ConfigError(`${path}.tiers.peak_windows_utc must be an array`);
    pricing.tiers = {
      peak_windows_utc: t.peak_windows_utc.map((w, i) => {
        if (!Array.isArray(w) || w.length !== 2) throw new ConfigError(`${path}.tiers.peak_windows_utc[${i}] must be [start, end]`);
        return [expectNumber(w[0], `${path}.tiers[${i}][0]`), expectNumber(w[1], `${path}.tiers[${i}][1]`)] as [number, number];
      }),
      business_days_only: expectBoolean(t.business_days_only, `${path}.tiers.business_days_only`, true),
      off_peak_divisor: expectNumber(t.off_peak_divisor, `${path}.tiers.off_peak_divisor`, 2),
    };
  }
  return pricing;
}

function parseModel(raw: unknown, path: string): ModelConfig {
  const m = object(raw, path);
  const caps = object(m.capabilities ?? {}, `${path}.capabilities`);
  const promptCache = caps.prompt_cache ?? "automatic";
  if (promptCache !== "automatic" && promptCache !== "none") throw new ConfigError(`${path}.capabilities.prompt_cache must be automatic|none`);
  let taskAffinity: Record<string, number> | undefined;
  if (m.task_affinity !== undefined) {
    const t = object(m.task_affinity, `${path}.task_affinity`);
    taskAffinity = {};
    for (const [k, v] of Object.entries(t)) taskAffinity[k] = Math.min(1, Math.max(0, expectNumber(v, `${path}.task_affinity.${k}`)));
  }
  const id = expectString(m.id, `${path}.id`);
  return {
    id,
    provider: expectString(m.provider, `${path}.provider`),
    display_name: typeof m.display_name === "string" ? m.display_name : id,
    tier: expectNumber(m.tier, `${path}.tier`, 1),
    context_tokens: expectNumber(m.context_tokens, `${path}.context_tokens`, 128_000),
    max_output_tokens: expectNumber(m.max_output_tokens, `${path}.max_output_tokens`, 16_384),
    pricing: parsePricing(m.pricing, `${path}.pricing`),
    capabilities: {
      tools: expectBoolean(caps.tools, `${path}.capabilities.tools`, true),
      parallel_tools: expectBoolean(caps.parallel_tools, `${path}.capabilities.parallel_tools`, false),
      reasoning: expectBoolean(caps.reasoning, `${path}.capabilities.reasoning`, false),
      prompt_cache: promptCache,
      images: expectBoolean(caps.images, `${path}.capabilities.images`, false),
    },
    cache_ttl_seconds: expectNumber(m.cache_ttl_seconds, `${path}.cache_ttl_seconds`, 600),
    task_affinity: taskAffinity,
    prior: m.prior === undefined ? undefined : Math.min(1, Math.max(0, expectNumber(m.prior, `${path}.prior`))),
    default_output_tokens: m.default_output_tokens === undefined ? undefined : expectNumber(m.default_output_tokens, `${path}.default_output_tokens`),
  };
}

export function parseConfig(raw: unknown): NimbleConfig {
  const c = object(raw, "config");
  const providers = Array.isArray(c.providers) ? c.providers.map((p, i) => parseProvider(p, `providers[${i}]`)) : [];
  const models = Array.isArray(c.models) ? c.models.map((m, i) => parseModel(m, `models[${i}]`)) : [];
  if (models.length === 0) throw new ConfigError("config.models must list at least one model");
  const providerIds = new Set(providers.map((p) => p.id));
  const modelIds = new Set<string>();
  for (const model of models) {
    if (!providerIds.has(model.provider)) throw new ConfigError(`model ${model.id} names unknown provider ${model.provider}`);
    if (modelIds.has(model.id)) throw new ConfigError(`duplicate model id ${model.id}`);
    modelIds.add(model.id);
  }
  const routingRaw = object(c.routing ?? {}, "routing");
  const pool = Array.isArray(routingRaw.pool) ? routingRaw.pool.map((id, i) => expectString(id, `routing.pool[${i}]`)) : models.map((m) => m.id);
  for (const id of pool) if (!modelIds.has(id)) throw new ConfigError(`routing.pool names unknown model ${id}`);
  const weightsRaw = object(routingRaw.weights ?? {}, "routing.weights");
  const weights: RoutingWeights = { ...DEFAULT_WEIGHTS };
  for (const key of Object.keys(DEFAULT_WEIGHTS) as (keyof RoutingWeights)[]) {
    weights[key] = expectNumber(weightsRaw[key], `routing.weights.${key}`, DEFAULT_WEIGHTS[key]);
  }
  const healthRaw = object(c.health ?? {}, "health");
  const portEnv = process.env.NIMBLE_PORT ? Number(process.env.NIMBLE_PORT) : undefined;
  return {
    host: process.env.NIMBLE_HOST ?? (typeof c.host === "string" ? c.host : "127.0.0.1"),
    port: portEnv && Number.isFinite(portEnv) ? portEnv : expectNumber(c.port, "port", 8092),
    db_path: process.env.NIMBLE_DB_PATH ?? (typeof c.db_path === "string" ? c.db_path : "nimble.db"),
    providers,
    models,
    routing: {
      pool,
      weights,
      switch_penalty_fixed: expectNumber(routingRaw.switch_penalty_fixed, "routing.switch_penalty_fixed", 0.1),
      escalation_bonus: expectNumber(routingRaw.escalation_bonus, "routing.escalation_bonus", 0.3),
      escalation_after_attempts: expectNumber(routingRaw.escalation_after_attempts, "routing.escalation_after_attempts", 2),
      escalation_failures: Array.isArray(routingRaw.escalation_failures)
        ? routingRaw.escalation_failures.map((f, i) => expectString(f, `routing.escalation_failures[${i}]`))
        // NOTE: this array is only the fallback for a config that omits the
        // key. `nimble.config.json` sets it explicitly and is what actually
        // runs — change both, and keep them in step.
        //
        // `workspace_unchanged`, `completion_not_claimed` and `unproven_claim`
        // were carved out of `uncertain_result` on 2026-09-19 and are listed
        // to preserve the escalation those attempts already had; each is a
        // judgement or protocol failure a stronger model plausibly fixes.
        //
        // `call_budget_exhausted` and `output_truncated` came out of the same
        // catch-all and are deliberately NOT listed (operator decision,
        // 2026-09-19). Both are budget exhaustions rather than capability
        // failures: the attempt ran out of calls or of room to answer, and
        // moving to a larger, dearer model does not buy either back — on the
        // output-truncation case (Muse, 66k reasoning tokens and no edits)
        // a heavier reasoner is the more likely way to repeat it.
        : [
            "test_failure",
            "compile_failure",
            "repeated_patch_failure",
            "incorrect_assumption",
            "workspace_unchanged",
            "completion_not_claimed",
            "unproven_claim",
            "attempt_stalled",
            "uncertain_result",
          ],
      default_output_tokens: expectNumber(routingRaw.default_output_tokens, "routing.default_output_tokens", 2_000),
      max_failovers: expectNumber(routingRaw.max_failovers, "routing.max_failovers", 2),
    },
    health: {
      consecutive_failures_to_open: expectNumber(healthRaw.consecutive_failures_to_open, "health.consecutive_failures_to_open", 3),
      cooldown_seconds: expectNumber(healthRaw.cooldown_seconds, "health.cooldown_seconds", 60),
      max_cooldown_seconds: expectNumber(healthRaw.max_cooldown_seconds, "health.max_cooldown_seconds", 600),
      error_rate_to_open: expectNumber(healthRaw.error_rate_to_open, "health.error_rate_to_open", 0.5),
    },
  };
}

export function defaultConfigPath(): string {
  return resolve(dirname(fileURLToPath(import.meta.url)), "..", "nimble.config.json");
}

export function loadConfig(path = process.env.NIMBLE_CONFIG ?? defaultConfigPath()): NimbleConfig {
  if (!existsSync(path)) throw new ConfigError(`config not found: ${path}`);
  return parseConfig(JSON.parse(readFileSync(path, "utf8")));
}

/** Resolve a provider's endpoint: env override > config default. */
export function providerBaseUrl(provider: ProviderConfig): string | null {
  const fromEnv = provider.base_url_env ? process.env[provider.base_url_env]?.trim() : undefined;
  const url = fromEnv || provider.base_url;
  return url ? url.replace(/\/+$/, "") : null;
}

export function providerApiKey(provider: ProviderConfig): string | null {
  const key = process.env[provider.api_key_env]?.trim();
  return key ? key : null;
}

export { join };

import type { TokenUsage } from "../billing/pricing";

type UnknownRecord = Record<string, unknown>;
const record = (value: unknown): UnknownRecord => value && typeof value === "object" ? value as UnknownRecord : {};
const count = (value: unknown): number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : 0;

export function extractUsage(payload: unknown): TokenUsage {
  const root = record(payload);
  const usage = record(root.usage);
  const inputDetails = record(usage.input_tokens_details ?? usage.prompt_tokens_details);
  return {
    inputTokens: count(usage.input_tokens ?? usage.prompt_tokens),
    cachedInputTokens: count(inputDetails.cached_tokens),
    cacheWriteTokens: count(inputDetails.cache_write_tokens),
    outputTokens: count(usage.output_tokens ?? usage.completion_tokens),
  };
}

export function estimateInputTokens(payload: unknown): number {
  const text = JSON.stringify(payload);
  return Math.max(1, Math.ceil(text.length / 4));
}

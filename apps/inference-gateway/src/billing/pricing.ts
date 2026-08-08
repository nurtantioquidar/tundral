/** Immutable rate-card entry used by reservation and settlement. */
export interface ModelRate {
  id: string;
  upstreamModel: string;
  inputMicrosPerMillion: bigint;
  cacheWriteMicrosPerMillion: bigint;
  cacheReadMicrosPerMillion: bigint;
  outputMicrosPerMillion: bigint;
}

const usd = (amount: number): bigint => BigInt(Math.round(amount * 1_000_000));

/* 2026-08-02.1 replaced the gpt-5.6-luna-low/-medium aliases with one public gpt-5.6-luna model.
 * Reasoning effort is a per-request parameter chosen by the customer's harness, not a model
 * identity, and every effort bills at the same rate. Usage settled under earlier versions keeps its
 * original version string and alias model ID.
 *
 * 2026-08-04.1 added gpt-5.4 and gpt-5.6 at OpenAI's published rates. This card is flat: OpenAI's
 * long-context surcharge (requests above 272K input tokens bill 2x input / 1.5x output upstream) is
 * not passed through, so such requests are served below upstream cost until a tiered card exists. */
export const RATE_CARD_VERSION = "2026-08-04.1";

export const MODEL_RATES: Readonly<Record<string, ModelRate>> = {
  "gpt-5.6-luna": {
    id: "gpt-5.6-luna",
    upstreamModel: "gpt-5.6-luna",
    inputMicrosPerMillion: usd(1),
    cacheWriteMicrosPerMillion: usd(1.25),
    cacheReadMicrosPerMillion: usd(0.1),
    outputMicrosPerMillion: usd(6),
  },
  /* OpenAI sells the flagship as gpt-5.6-sol and routes the bare gpt-5.6 alias to it. Monet sells
   * the alias ID because that is what providers report from /v1/models and what harnesses request. */
  "gpt-5.6": {
    id: "gpt-5.6",
    upstreamModel: "gpt-5.6",
    inputMicrosPerMillion: usd(5),
    cacheWriteMicrosPerMillion: usd(6.25),
    cacheReadMicrosPerMillion: usd(0.5),
    outputMicrosPerMillion: usd(30),
  },
  "gpt-5.5": {
    id: "gpt-5.5",
    upstreamModel: "gpt-5.5",
    inputMicrosPerMillion: usd(5),
    cacheWriteMicrosPerMillion: usd(5),
    cacheReadMicrosPerMillion: usd(0.5),
    outputMicrosPerMillion: usd(30),
  },
  /* OpenAI publishes no separate cache-write price for gpt-5.4, so writes bill as ordinary input. */
  "gpt-5.4": {
    id: "gpt-5.4",
    upstreamModel: "gpt-5.4",
    inputMicrosPerMillion: usd(2.5),
    cacheWriteMicrosPerMillion: usd(2.5),
    cacheReadMicrosPerMillion: usd(0.25),
    outputMicrosPerMillion: usd(15),
  },
};

export interface TokenUsage {
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteTokens?: number;
  outputTokens: number;
}

function charge(tokens: number, rate: bigint): bigint {
  if (!Number.isSafeInteger(tokens) || tokens < 0) throw new Error("Invalid token count");
  return (BigInt(tokens) * rate + 999_999n) / 1_000_000n;
}

export function calculateChargeMicros(rate: ModelRate, usage: TokenUsage): bigint {
  const cached = Math.min(usage.cachedInputTokens, usage.inputTokens);
  const cacheWrite = Math.min(usage.cacheWriteTokens ?? 0, usage.inputTokens - cached);
  const uncached = usage.inputTokens - cached - cacheWrite;
  return (
    charge(uncached, rate.inputMicrosPerMillion) +
    charge(cacheWrite, rate.cacheWriteMicrosPerMillion) +
    charge(cached, rate.cacheReadMicrosPerMillion) +
    charge(usage.outputTokens, rate.outputMicrosPerMillion)
  );
}

export function formatUsd(micros: bigint): string {
  const whole = micros / 1_000_000n;
  const fraction = (micros % 1_000_000n).toString().padStart(6, "0");
  return `${whole}.${fraction}`;
}

export function publicModel(rate: ModelRate) {
  return {
    id: rate.id,
    object: "model",
    owned_by: "coding-gateway",
    rate_card_version: RATE_CARD_VERSION,
    pricing: {
      currency: "USD",
      unit: "million_tokens",
      input: formatUsd(rate.inputMicrosPerMillion),
      cache_write: formatUsd(rate.cacheWriteMicrosPerMillion),
      cache_read: formatUsd(rate.cacheReadMicrosPerMillion),
      output: formatUsd(rate.outputMicrosPerMillion),
    },
  };
}

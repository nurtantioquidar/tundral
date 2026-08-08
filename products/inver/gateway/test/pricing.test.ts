import { describe, expect, test } from "bun:test";
import { calculateChargeMicros, MODEL_RATES } from "../src/billing/pricing";
import { extractUsage } from "../src/generation/usage";
import { requestedReasoningEffort } from "../src/upstreams/reasoning-effort";

describe("pricing", () => {
  test("charges uncached, cached, and output tokens separately", () => {
    const charge = calculateChargeMicros(MODEL_RATES["gpt-5.6-luna"]!, {
      inputTokens: 40_000,
      cachedInputTokens: 30_000,
      outputTokens: 5_000,
    });
    expect(charge).toBe(43_000n);
  });

  test("charges GPT-5.6 cache writes at 1.25x input", () => {
    const charge = calculateChargeMicros(MODEL_RATES["gpt-5.6-luna"]!, {
      inputTokens: 1_000_000,
      cachedInputTokens: 0,
      cacheWriteTokens: 1_000_000,
      outputTokens: 0,
    });
    expect(charge).toBe(1_250_000n);
  });

  test("sells models without effort aliases", () => {
    expect(MODEL_RATES["gpt-5.6-luna"]).toMatchObject({ upstreamModel: "gpt-5.6-luna" });
    expect(MODEL_RATES["gpt-5.6-luna-low"]).toBeUndefined();
    expect(MODEL_RATES["gpt-5.6-luna-medium"]).toBeUndefined();
    expect(MODEL_RATES["gpt-5.6-sol"]).toBeUndefined();
  });

  test("sells gpt-5.6 and gpt-5.4 at OpenAI's published rates", () => {
    expect(MODEL_RATES["gpt-5.6"]).toMatchObject({
      upstreamModel: "gpt-5.6",
      inputMicrosPerMillion: 5_000_000n,
      cacheWriteMicrosPerMillion: 6_250_000n,
      cacheReadMicrosPerMillion: 500_000n,
      outputMicrosPerMillion: 30_000_000n,
    });
    expect(MODEL_RATES["gpt-5.4"]).toMatchObject({
      upstreamModel: "gpt-5.4",
      inputMicrosPerMillion: 2_500_000n,
      cacheReadMicrosPerMillion: 250_000n,
      outputMicrosPerMillion: 15_000_000n,
    });
  });

  test("bills gpt-5.4 cache writes as ordinary input", () => {
    const withWrites = calculateChargeMicros(MODEL_RATES["gpt-5.4"]!, {
      inputTokens: 1_000_000,
      cachedInputTokens: 0,
      cacheWriteTokens: 1_000_000,
      outputTokens: 0,
    });
    const withoutWrites = calculateChargeMicros(MODEL_RATES["gpt-5.4"]!, {
      inputTokens: 1_000_000,
      cachedInputTokens: 0,
      outputTokens: 0,
    });
    expect(withWrites).toBe(withoutWrites);
  });
});

describe("requested reasoning effort", () => {
  test("reads chat completions reasoning_effort", () => {
    expect(requestedReasoningEffort("/v1/chat/completions", { reasoning_effort: "high" })).toBe("high");
    expect(requestedReasoningEffort("/v1/chat/completions", {})).toBeUndefined();
  });

  test("reads Responses API reasoning.effort", () => {
    expect(requestedReasoningEffort("/v1/responses", { reasoning: { effort: "minimal" } })).toBe("minimal");
    expect(requestedReasoningEffort("/v1/responses", { reasoning: { summary: "auto" } })).toBeUndefined();
    expect(requestedReasoningEffort("/v1/responses", {})).toBeUndefined();
  });

  test("rejects values outside the known effort set", () => {
    expect(() => requestedReasoningEffort("/v1/chat/completions", { reasoning_effort: "max" })).toThrow();
    expect(() => requestedReasoningEffort("/v1/responses", { reasoning: { effort: 3 } })).toThrow();
    expect(() => requestedReasoningEffort("/v1/responses", { reasoning: "low" })).toThrow();
  });
});

describe("usage extraction", () => {
  test("reads Responses API usage", () => {
    expect(extractUsage({ usage: { input_tokens: 100, input_tokens_details: { cached_tokens: 70 }, output_tokens: 20 } })).toEqual({
      inputTokens: 100, cachedInputTokens: 70, cacheWriteTokens: 0, outputTokens: 20,
    });
  });

  test("reads Chat Completions usage", () => {
    expect(extractUsage({ usage: { prompt_tokens: 100, prompt_tokens_details: { cached_tokens: 50 }, completion_tokens: 10 } })).toEqual({
      inputTokens: 100, cachedInputTokens: 50, cacheWriteTokens: 0, outputTokens: 10,
    });
  });
});

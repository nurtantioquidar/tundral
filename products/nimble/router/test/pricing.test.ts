import { describe, expect, test } from "bun:test";

import { defaultConfigPath, loadConfig, parseConfig, ConfigError } from "../src/config";
import { HealthTracker } from "../src/health";
import { estimateCost, priceUsage, tierAt, tokensCost } from "../src/pricing";
import { normalizeUsage, stablePrefixHash } from "../src/usage";

const config = loadConfig(defaultConfigPath());
const deepseek = config.models.find((m) => m.id === "deepseek-v4-flash")!;

describe("pricing", () => {
  test("integer micro-USD with distinct cache counters and UTC tiers", () => {
    // 2026-08-31 is a Monday; 02:00 UTC peak, 12:00 off-peak.
    const monday = 1_788_134_400;
    expect(tierAt(deepseek.pricing, monday + 2 * 3600)).toBe("peak");
    expect(tierAt(deepseek.pricing, monday + 12 * 3600)).toBe("off_peak");
    expect(tierAt(deepseek.pricing, monday - 86_400 + 7 * 3600)).toBe("off_peak");
    const usage = { input_tokens: 1_000_000, cached_tokens: 900_000, output_tokens: 10_000 };
    expect(priceUsage(deepseek, usage, monday + 2 * 3600)).toBe(12_600 + 44_000 + 13_200);
    expect(priceUsage(deepseek, usage, monday + 12 * 3600)).toBe((12_600 + 44_000 + 13_200) / 2);
    expect(tokensCost(1, 440_000)).toBe(1);
  });

  test("estimates split stable prefix from dynamic tokens", () => {
    const monday = 1_788_134_400 + 2 * 3600;
    const cold = estimateCost(deepseek, { stable_tokens: 40_000, estimated_cached_tokens: 0, dynamic_tokens: 5_000, expected_output_tokens: 1_000 }, monday);
    const warm = estimateCost(deepseek, { stable_tokens: 40_000, estimated_cached_tokens: 40_000, dynamic_tokens: 5_000, expected_output_tokens: 1_000 }, monday);
    expect(warm).toBeLessThan(cold);
    expect(cold - warm).toBe(tokensCost(40_000, 440_000) - tokensCost(40_000, 14_000));
  });
});

describe("usage normalization", () => {
  test("accepts DeepSeek and OpenAI cache fields", () => {
    expect(normalizeUsage({ prompt_tokens: 100, prompt_cache_hit_tokens: 30, completion_tokens: 5 })).toEqual({ input_tokens: 100, cached_tokens: 30, output_tokens: 5 });
    expect(normalizeUsage({ prompt_tokens: 100, prompt_tokens_details: { cached_tokens: 64 }, completion_tokens: 5 })).toEqual({ input_tokens: 100, cached_tokens: 64, output_tokens: 5 });
    expect(normalizeUsage({ completion_tokens: 5 })).toBeNull();
    expect(normalizeUsage(null)).toBeNull();
  });

  test("prefix hash depends only on the first message and tools", () => {
    const a = stablePrefixHash([{ role: "system", content: "s" }, { role: "user", content: "1" }], [{ name: "t" }]);
    const b = stablePrefixHash([{ role: "system", content: "s" }, { role: "user", content: "2" }], [{ name: "t" }]);
    const c = stablePrefixHash([{ role: "system", content: "S" }], [{ name: "t" }]);
    expect(a).toBe(b);
    expect(a).not.toBe(c);
    expect(a.startsWith("sha256:")).toBe(true);
  });
});

describe("config", () => {
  test("shipped config parses and rejects dangling references", () => {
    expect(config.routing.pool).toEqual(["muse-spark-1.3-contributor", "glm-5.3-flash", "deepseek-v4-flash", "gpt-5.6-luna"]);
    expect(() => parseConfig({ providers: [], models: [{ id: "m", provider: "ghost", pricing: { cache_hit_input: 1, cache_miss_input: 1, output: 1, version: "v" } }] })).toThrow(ConfigError);
    expect(() => parseConfig({ providers: [{ id: "p", api_key_env: "K" }], models: [{ id: "m", provider: "p", pricing: { cache_hit_input: 1, cache_miss_input: 1, output: 1, version: "v" } }], routing: { pool: ["nope"] } })).toThrow(ConfigError);
  });
});

describe("health", () => {
  test("opens after consecutive failures, half-opens after cooldown, doubles on repeated failure", () => {
    let now = 0;
    const health = new HealthTracker({ consecutive_failures_to_open: 3, cooldown_seconds: 60, max_cooldown_seconds: 600, error_rate_to_open: 0.9 }, () => now);
    expect(health.isEligible("p")).toBe(true);
    health.recordFailure("p");
    health.recordFailure("p");
    expect(health.isEligible("p")).toBe(true);
    health.recordFailure("p");
    expect(health.isEligible("p")).toBe(false);
    expect(health.risk("p")).toBe(1);
    now = 61_000;
    expect(health.isEligible("p")).toBe(true); // probe
    expect(health.isEligible("p")).toBe(false); // only one probe
    health.recordFailure("p");
    expect(health.get("p").cooldown_seconds).toBe(120);
    now = 61_000 + 121_000;
    expect(health.isEligible("p")).toBe(true);
    health.recordSuccess("p", 300);
    expect(health.isEligible("p")).toBe(true);
    expect(health.get("p").latency_ms).toBe(300);
    expect(health.risk("p")).toBeLessThan(0.35);
  });
});

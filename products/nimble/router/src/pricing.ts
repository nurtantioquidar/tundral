// Cost model: integer micro-USD end to end (never floating-point money). Rates are micro-USD per
// million tokens; optional UTC peak/off-peak schedules divide every rate off-peak. The estimate
// separates the stable prefix (partly cached) from dynamic tokens (never cached) and expected
// output.

import type { ModelConfig, PricingConfig } from "./config";

export type PriceTier = "peak" | "off_peak";

export function tierAt(pricing: PricingConfig, epochSeconds: number): PriceTier {
  const tiers = pricing.tiers;
  if (!tiers) return "peak";
  const days = Math.floor(epochSeconds / 86_400);
  const weekday = (days + 3) % 7; // 0 = Monday … 6 = Sunday (1970-01-01 was a Thursday)
  const hour = Math.floor((epochSeconds % 86_400) / 3_600);
  if (tiers.business_days_only && weekday >= 5) return "off_peak";
  return tiers.peak_windows_utc.some(([start, end]) => hour >= start && hour < end) ? "peak" : "off_peak";
}

function rateAt(pricing: PricingConfig, tier: PriceTier): { hit: number; miss: number; out: number } {
  const divisor = tier === "off_peak" && pricing.tiers ? Math.max(1, pricing.tiers.off_peak_divisor) : 1;
  return {
    hit: Math.floor(pricing.cache_hit_input / divisor),
    miss: Math.floor(pricing.cache_miss_input / divisor),
    out: Math.floor(pricing.output / divisor),
  };
}

/** ceil(tokens × rate / 1e6) in integer arithmetic. */
export function tokensCost(tokens: number, microusdPerMillion: number): number {
  const t = BigInt(Math.max(0, Math.round(tokens)));
  const r = BigInt(Math.max(0, Math.round(microusdPerMillion)));
  const product = t * r;
  return Number((product + 999_999n) / 1_000_000n);
}

export interface Usage {
  input_tokens: number;
  cached_tokens: number;
  output_tokens: number;
}

/** Actual cost of an observed request. */
export function priceUsage(model: ModelConfig, usage: Usage, epochSeconds = Date.now() / 1000): number {
  const rate = rateAt(model.pricing, tierAt(model.pricing, epochSeconds));
  const cached = Math.min(usage.cached_tokens, usage.input_tokens);
  const miss = usage.input_tokens - cached;
  return tokensCost(cached, rate.hit) + tokensCost(miss, rate.miss) + tokensCost(usage.output_tokens, rate.out);
}

export interface CostEstimateInput {
  stable_tokens: number;
  estimated_cached_tokens: number;
  dynamic_tokens: number;
  expected_output_tokens: number;
}

/** Expected cost before the request: cached prefix at the hit rate, the rest at the miss rate. */
export function estimateCost(model: ModelConfig, input: CostEstimateInput, epochSeconds = Date.now() / 1000): number {
  const rate = rateAt(model.pricing, tierAt(model.pricing, epochSeconds));
  const cached = Math.min(input.estimated_cached_tokens, input.stable_tokens);
  return (
    tokensCost(cached, rate.hit) +
    tokensCost(input.stable_tokens - cached, rate.miss) +
    tokensCost(input.dynamic_tokens, rate.miss) +
    tokensCost(input.expected_output_tokens, rate.out)
  );
}

export function formatMicroUsd(microusd: number): string {
  const whole = Math.floor(microusd / 1_000_000);
  const frac = String(microusd % 1_000_000).padStart(6, "0");
  return `$${whole}.${frac}`;
}

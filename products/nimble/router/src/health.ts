// Provider health: a small circuit breaker per provider. Consecutive failures or a high error
// EWMA open the circuit for a cooldown that doubles while failures continue; the first request
// after cooldown half-opens it (one probe). Latency EWMA feeds the routing score.

import type { HealthConfig } from "./config";

export interface ProviderHealth {
  provider: string;
  consecutive_failures: number;
  error_rate: number;
  latency_ms: number;
  cooldown_until: number;
  cooldown_seconds: number;
  half_open: boolean;
  total_requests: number;
  total_failures: number;
}

const ALPHA = 0.2;

const TOKEN_WINDOW_MS = 60_000;

export class HealthTracker {
  private readonly state = new Map<string, ProviderHealth>();
  /** Tokens sent per provider, timestamped, for the one-minute rate window. */
  private readonly tokens = new Map<string, { at: number; tokens: number }[]>();

  constructor(private readonly config: HealthConfig, private readonly now: () => number = Date.now) {}

  get(provider: string): ProviderHealth {
    let health = this.state.get(provider);
    if (!health) {
      health = {
        provider,
        consecutive_failures: 0,
        error_rate: 0,
        latency_ms: 0,
        cooldown_until: 0,
        cooldown_seconds: this.config.cooldown_seconds,
        half_open: false,
        total_requests: 0,
        total_failures: 0,
      };
      this.state.set(provider, health);
    }
    return health;
  }

  /** Eligible = circuit closed, or cooldown elapsed (then this call half-opens it for a probe). */
  isEligible(provider: string): boolean {
    const health = this.get(provider);
    if (health.cooldown_until === 0) return true;
    if (this.now() >= health.cooldown_until) {
      if (!health.half_open) {
        health.half_open = true;
        return true;
      }
      return false; // a probe is already in flight
    }
    return false;
  }

  /** Penalty in [0, 1] for the routing score. */
  risk(provider: string): number {
    const health = this.get(provider);
    if (health.cooldown_until !== 0 && this.now() < health.cooldown_until) return 1;
    return Math.min(1, health.error_rate + health.consecutive_failures * 0.15);
  }

  recordSuccess(provider: string, latencyMs: number): void {
    const health = this.get(provider);
    health.total_requests += 1;
    health.consecutive_failures = 0;
    // A success after trouble is strong evidence of recovery: decay faster
    // than the failure EWMA climbs.
    health.error_rate = health.error_rate * 0.5;
    health.latency_ms = health.latency_ms === 0 ? latencyMs : (1 - ALPHA) * health.latency_ms + ALPHA * latencyMs;
    health.cooldown_until = 0;
    health.cooldown_seconds = this.config.cooldown_seconds;
    health.half_open = false;
  }

  /** `weight` 1 for 5xx/transport/timeout, 0.5 for 429. */
  recordFailure(provider: string, weight = 1): void {
    const health = this.get(provider);
    health.total_requests += 1;
    health.total_failures += 1;
    health.consecutive_failures += weight;
    health.error_rate = (1 - ALPHA) * health.error_rate + ALPHA * weight;
    const open =
      health.consecutive_failures >= this.config.consecutive_failures_to_open || health.error_rate >= this.config.error_rate_to_open;
    if (open) {
      if (health.half_open) {
        health.cooldown_seconds = Math.min(this.config.max_cooldown_seconds, health.cooldown_seconds * 2);
      }
      health.cooldown_until = this.now() + health.cooldown_seconds * 1000;
      health.half_open = false;
    }
  }

  /** Record tokens billed by a completed request (input + output). */
  observeTokens(provider: string, tokens: number): void {
    if (!(tokens > 0)) return;
    const list = this.tokens.get(provider) ?? [];
    list.push({ at: this.now(), tokens });
    this.tokens.set(provider, this.prune(list));
  }

  /** Tokens sent to `provider` in the trailing minute. */
  tokensInWindow(provider: string): number {
    const list = this.prune(this.tokens.get(provider) ?? []);
    this.tokens.set(provider, list);
    return list.reduce((sum, entry) => sum + entry.tokens, 0);
  }

  /** True when `estimate` more tokens would stay under the provider's per-minute cap. */
  withinTokenBudget(provider: string, estimate: number, limit: number | undefined): boolean {
    if (limit === undefined || limit <= 0) return true;
    return this.tokensInWindow(provider) + estimate <= limit;
  }

  private prune(list: { at: number; tokens: number }[]): { at: number; tokens: number }[] {
    const cutoff = this.now() - TOKEN_WINDOW_MS;
    return list.filter((entry) => entry.at > cutoff);
  }

  snapshot(): ProviderHealth[] {
    return [...this.state.values()].map((h) => ({ ...h }));
  }
}

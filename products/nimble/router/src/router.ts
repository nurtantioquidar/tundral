// The deterministic, explainable router (design §8–§9). Every candidate's score components are
// returned with the decision and stored, so a decision can always be explained after the fact
// and weights can later be fitted from data instead of guessed.

import type { ModelConfig, NimbleConfig } from "./config";
import type { HealthTracker } from "./health";
import { estimateCost } from "./pricing";
import type { SessionState, Store } from "./store";

export interface RoutingRequest {
  session: SessionState;
  prefix_hash: string;
  stable_tokens: number;
  dynamic_tokens: number;
  task_type: string | null;
  task_complexity: "low" | "medium" | "high" | null;
  attempt: number | null;
  previous_failure: string | null;
  verification_failed: boolean;
  /** Providers already tried and failed for this request (failover). */
  exclude_providers?: Set<string>;
  now_ms?: number;
}

export interface ScoreComponents {
  model: string;
  provider: string;
  tier: number;
  eligible: boolean;
  capability: number;
  escalation_bonus: number;
  historical_success: number;
  history_samples: number;
  /** w_hist scaled by evidence: more clean feedback, more weight. */
  history_weight: number;
  cache_affinity: number;
  estimated_cached_tokens: number;
  expected_cost_microusd: number;
  normalized_cost: number;
  latency_ms: number;
  normalized_latency: number;
  failure_risk: number;
  switch_penalty: number;
  total: number;
}

export interface Decision {
  chosen: ModelConfig;
  reason: string;
  model_changed: boolean;
  cache_affinity: number;
  estimated_cached_tokens: number;
  escalated: boolean;
  /** A decisive escalation switched models; recorded on the session so it fires once per attempt. */
  decisive: boolean;
  components: ScoreComponents[];
}

export class Router {
  private readonly models: Map<string, ModelConfig>;
  private readonly maxTier: number;

  private readonly tokenLimits: Map<string, number | undefined>;
  constructor(
    private readonly config: NimbleConfig,
    private readonly store: Store,
    private readonly health: HealthTracker,
  ) {
    this.models = new Map(config.models.map((m) => [m.id, m]));
    this.maxTier = Math.max(1, ...config.models.map((m) => m.tier));
    this.tokenLimits = new Map(config.providers.map((p) => [p.id, p.tokens_per_minute]));
  }

  model(id: string): ModelConfig | undefined {
    return this.models.get(id);
  }

  pool(): ModelConfig[] {
    return this.config.routing.pool.map((id) => this.models.get(id)!).filter(Boolean);
  }

  /** Cache affinity of `model` for the request's prefix (design §6). */
  affinity(model: ModelConfig, prefixHash: string, stableTokens: number, nowMs: number): { affinity: number; cached: number } {
    if (model.capabilities.prompt_cache === "none" || stableTokens <= 0) return { affinity: 0, cached: 0 };
    const observed = this.store.getCacheObservation(model.provider, model.id, prefixHash);
    if (!observed) return { affinity: 0, cached: 0 };
    const warm = nowMs - observed.last_served_at <= model.cache_ttl_seconds * 1000;
    if (!warm) return { affinity: 0, cached: 0 };
    const affinity = Math.min(1, observed.prefix_tokens_seen / stableTokens);
    return { affinity, cached: Math.round(affinity * stableTokens) };
  }

  private escalationTriggered(request: RoutingRequest): string[] {
    const triggers: string[] = [];
    if ((request.attempt ?? 1) >= 2) triggers.push(`attempt ${request.attempt}`);
    if (request.verification_failed) triggers.push("verification failed");
    if (request.previous_failure && this.config.routing.escalation_failures.includes(request.previous_failure)) {
      triggers.push(request.previous_failure);
    }
    if (request.task_complexity === "high" && (request.attempt ?? 1) === 1) triggers.push("high complexity");
    return triggers;
  }

  decide(request: RoutingRequest): Decision {
    const nowMs = request.now_ms ?? Date.now();
    const weights = this.config.routing.weights;
    const current = request.session.current_model;
    const triggers = this.escalationTriggered(request);
    const escalationScale = triggers.length > 0 ? this.config.routing.escalation_bonus * Math.min(2, triggers.length) : 0;
    // Decisive escalation: the current model has verifiably failed at least
    // `escalation_after_attempts` times — locality is worth less than a fix.
    const verifiedFailure =
      request.verification_failed ||
      (request.previous_failure !== null && this.config.routing.escalation_failures.includes(request.previous_failure));
    // Once per attempt: the evidence Butterfly sends ("attempt 2, verification
    // failed") is constant for the whole attempt, so without this guard the
    // rule re-fires against whichever model is current and the session
    // ping-pongs between two models on every request (observed 2026-09-04:
    // 20 switches in one Terminal-Bench session, every one a cache miss).
    const decisive =
      current !== null &&
      verifiedFailure &&
      (request.attempt ?? 1) >= this.config.routing.escalation_after_attempts &&
      request.session.escalated_attempt !== (request.attempt ?? 1);
    const currentTier = current ? (this.models.get(current)?.tier ?? 0) : 0;
    const candidates = this.pool().filter((m) => !request.exclude_providers?.has(m.provider));

    // Raw per-candidate facts first (cost/latency need pool-wide normalization).
    const facts = candidates.map((model) => {
      const { affinity, cached } = this.affinity(model, request.prefix_hash, request.stable_tokens, nowMs);
      const stats = this.store.stats(model.id, request.task_type);
      const expectedOutput =
        stats && stats.output_samples > 0
          ? stats.output_tokens_sum / stats.output_samples
          : (model.default_output_tokens ?? this.config.routing.default_output_tokens);
      const cost = estimateCost(model, {
        stable_tokens: request.stable_tokens,
        estimated_cached_tokens: cached,
        dynamic_tokens: request.dynamic_tokens,
        expected_output_tokens: expectedOutput,
      });
      const latency = this.health.get(model.provider).latency_ms;
      // Decisive means decisive: the model that verifiably failed this task
      // is out of the running whenever a higher rung exists (its warm cache
      // and lower price no longer argue for a third identical attempt).
      // …but only when a higher rung is a credible alternative: forcing a
      // switch to a model whose evidence prior is far below the current one
      // trades a likely pass for a near-certain miss.
      const priorOf = (m: ModelConfig) => m.prior ?? m.tier / this.maxTier;
      const credibleHigherRung = candidates.some((m) => m.tier > model.tier && priorOf(m) >= priorOf(model) * 0.5);
      const benched = decisive && model.id === current && credibleHigherRung;
      // A provider whose per-minute token cap this request would blow is not
      // a candidate: the upstream would refuse it mid-stream.
      const estimate = request.stable_tokens + request.dynamic_tokens + expectedOutput;
      const withinBudget = this.health.withinTokenBudget(model.provider, estimate, this.tokenLimits.get(model.provider));
      const eligible = this.health.isEligible(model.provider) && !benched && withinBudget;
      return { model, affinity, cached, stats, cost, latency, eligible };
    });
    const maxCost = Math.max(1, ...facts.map((f) => f.cost));
    const maxLatency = Math.max(1, ...facts.map((f) => f.latency));
    const currentAffinity = current
      ? (facts.find((f) => f.model.id === current)?.affinity ?? 0)
      : 0;

    const components: ScoreComponents[] = facts.map((f) => {
      const tierRatio = f.model.tier / this.maxTier;
      // Prior: per-task affinity > evidence prior > tier ratio. The tier is
      // the escalation ladder; the prior is who is expected to succeed first.
      const prior = request.task_type && f.model.task_affinity?.[request.task_type] !== undefined
        ? f.model.task_affinity[request.task_type]!
        : (f.model.prior ?? tierRatio);
      const escalation = decisive && f.model.tier <= currentTier ? 0 : escalationScale * tierRatio;
      const capability = Math.min(1.5, prior + escalation);
      const samples = f.stats?.n ?? 0;
      const history = samples > 0 ? (f.stats!.successes + 1) / (samples + 2) : 0.5;
      // Evidence scales the history term: with no feedback it is a neutral
      // 0.5 at the base weight; with many clean samples it can outweigh the
      // tier prior. confidence = n / (n + 5).
      const confidence = samples / (samples + 5);
      const historyWeight = weights.history * (1 + 2 * confidence);
      const sessionFailures = request.session.recent_failures.filter((x) => x === f.model.id).length;
      const isCurrent = current === f.model.id;
      // A model that verifiably failed this task twice is itself a risk now.
      const failedHere = decisive && isCurrent ? 0.5 : 0;
      const risk = Math.min(1, this.health.risk(f.model.provider) + sessionFailures * 0.1 + failedHere);
      const switchPenalty =
        current === null || isCurrent || decisive ? 0 : currentAffinity * weights.cache + this.config.routing.switch_penalty_fixed;
      const normalizedCost = f.cost / maxCost;
      const normalizedLatency = f.latency / maxLatency;
      const total =
        weights.capability * capability +
        historyWeight * history +
        weights.cache * f.affinity -
        weights.cost * normalizedCost -
        weights.latency * normalizedLatency -
        weights.risk * risk -
        weights.switch * switchPenalty;
      return {
        model: f.model.id,
        provider: f.model.provider,
        tier: f.model.tier,
        eligible: f.eligible,
        capability,
        escalation_bonus: escalation,
        historical_success: history,
        history_samples: samples,
        history_weight: historyWeight,
        cache_affinity: f.affinity,
        estimated_cached_tokens: f.cached,
        expected_cost_microusd: f.cost,
        normalized_cost: normalizedCost,
        latency_ms: f.latency,
        normalized_latency: normalizedLatency,
        failure_risk: risk,
        switch_penalty: switchPenalty,
        total,
      };
    });

    const eligible = components.filter((c) => c.eligible);
    const ranked = (eligible.length > 0 ? eligible : components).slice().sort((a, b) => {
      if (b.total !== a.total) return b.total - a.total;
      if (a.model === current) return -1;
      if (b.model === current) return 1;
      return a.expected_cost_microusd - b.expected_cost_microusd;
    });
    const winner = ranked[0];
    if (!winner) throw new Error("routing pool is empty");
    const chosen = this.models.get(winner.model)!;
    const modelChanged = current !== null && current !== chosen.id;
    let reason: string;
    if (eligible.length === 0) {
      reason = `all providers unhealthy; least-bad ${chosen.id}`;
    } else if (request.exclude_providers && request.exclude_providers.size > 0) {
      reason = `failover: ${[...request.exclude_providers].join(",")} unhealthy`;
    } else if (current === null) {
      reason = triggers.length > 0 ? `first request, escalated (${triggers.join(", ")})` : `first request, best score`;
    } else if (!modelChanged) {
      reason = `sticky: affinity ${winner.cache_affinity.toFixed(2)}`;
    } else if (triggers.length > 0) {
      reason = `${decisive ? "escalated (decisive)" : "escalated"}: ${triggers.join(", ")}`;
    } else {
      const previous = components.find((c) => c.model === current);
      reason = `switched: score ${winner.total.toFixed(3)} beats ${current} ${previous ? previous.total.toFixed(3) : "n/a"} despite penalty ${winner.switch_penalty.toFixed(2)}`;
    }
    return {
      chosen,
      reason,
      model_changed: modelChanged,
      cache_affinity: winner.cache_affinity,
      estimated_cached_tokens: winner.estimated_cached_tokens,
      escalated: triggers.length > 0 && modelChanged,
      decisive: decisive && modelChanged,
      components,
    };
  }
}

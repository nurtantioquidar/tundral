import { describe, expect, test } from "bun:test";

import { loadConfig, defaultConfigPath } from "../src/config";
import { HealthTracker } from "../src/health";
import { Router, type RoutingRequest } from "../src/router";
import { Store, type SessionState } from "../src/store";

const config = loadConfig(defaultConfigPath());

function fresh(sessionId = "s1"): SessionState {
  return {
    session_id: sessionId,
    current_model: null,
    prefix_hash: null,
    stable_tokens: 0,
    last_request_at: 0,
    requests: 0,
    switches: 0,
    recent_failures: [],
    escalated_attempt: null,
    failure_attempt: null,
  };
}

function request(overrides: Partial<RoutingRequest> = {}): RoutingRequest {
  return {
    session: fresh(),
    prefix_hash: "sha256:p",
    stable_tokens: 40_000,
    dynamic_tokens: 4_000,
    task_type: null,
    task_complexity: null,
    attempt: null,
    previous_failure: null,
    verification_failed: false,
    now_ms: 1_000_000,
    ...overrides,
  };
}

function setup() {
  const store = new Store(":memory:");
  const now = { value: 1_000_000 };
  const health = new HealthTracker(config.health, () => now.value);
  const router = new Router(config, store, health);
  return { store, health, router, now };
}

describe("router", () => {
  test("first request balances capability against cost and stores every component", () => {
    const { router } = setup();
    const decision = router.decide(request());
    // The evidence prior (Terminal-Bench dev subset, 2026-09-04: GLM passed
    // where DeepSeek and Luna did not) outranks the tier ladder on a cold
    // first request; the tier still decides where escalation goes.
    expect(decision.chosen.id).toBe("glm-5.3-flash");
    const luna = decision.components.find((c) => c.model === "gpt-5.6-luna")!;
    expect(luna.tier).toBe(4);
    expect(luna.capability).toBeCloseTo(0.4, 5);
    expect(decision.model_changed).toBe(false);
    expect(decision.components).toHaveLength(4);
    for (const c of decision.components) {
      expect(c.expected_cost_microusd).toBeGreaterThan(0);
      expect(c.historical_success).toBe(0.5);
      expect(Number.isFinite(c.total)).toBe(true);
    }
    expect(decision.reason).toContain("first request");
  });

  test("a warm cache keeps the session sticky even when a peer is slightly cheaper", () => {
    const { router, store } = setup();
    // DeepSeek served this exact prefix moments ago with 38k cached tokens.
    store.observeCache({ provider: "deepseek", model: "deepseek-v4-flash", prefix_hash: "sha256:p", last_served_at: 999_000, prefix_tokens_seen: 38_000 });
    const session = { ...fresh(), current_model: "deepseek-v4-flash", prefix_hash: "sha256:p", stable_tokens: 40_000 };
    const decision = router.decide(request({ session }));
    expect(decision.chosen.id).toBe("deepseek-v4-flash");
    expect(decision.cache_affinity).toBeCloseTo(0.95, 2);
    expect(decision.estimated_cached_tokens).toBe(38_000);
    expect(decision.reason).toContain("sticky");
    const glm = decision.components.find((c) => c.model === "glm-5.3-flash")!;
    expect(glm.switch_penalty).toBeGreaterThan(0);
  });

  test("an expired cache removes the locality argument", () => {
    const { router, store } = setup();
    store.observeCache({ provider: "deepseek", model: "deepseek-v4-flash", prefix_hash: "sha256:p", last_served_at: 1_000_000 - 601_000, prefix_tokens_seen: 38_000 });
    const session = { ...fresh(), current_model: "deepseek-v4-flash", prefix_hash: "sha256:p", stable_tokens: 40_000 };
    const decision = router.decide(request({ session }));
    expect(decision.cache_affinity).toBe(0);
  });

  test("a single failure does not throw away a warm cache, repeated verified failure escalates decisively", () => {
    const { router, store } = setup();
    store.observeCache({ provider: "deepseek", model: "deepseek-v4-flash", prefix_hash: "sha256:p", last_served_at: 999_000, prefix_tokens_seen: 40_000 });
    const session = { ...fresh(), current_model: "deepseek-v4-flash", prefix_hash: "sha256:p", stable_tokens: 40_000 };
    const once = router.decide(request({ session, attempt: 1, previous_failure: "test_failure" }));
    expect(once.chosen.id).toBe("deepseek-v4-flash");
    const twice = router.decide(request({ session, attempt: 2, previous_failure: "test_failure", verification_failed: true }));
    // Escalation climbs the tier ladder (DeepSeek 1 → GLM 2 → Luna 3); among
    // the higher rungs the evidence prior picks GLM before Luna.
    expect(twice.chosen.id).toBe("glm-5.3-flash");
    expect(twice.escalated).toBe(true);
    expect(twice.reason).toContain("decisive");
    const glm = twice.components.find((c) => c.model === "glm-5.3-flash")!;
    expect(glm.switch_penalty).toBe(0);
    const luna = twice.components.find((c) => c.model === "gpt-5.6-luna")!;
    expect(luna.escalation_bonus).toBeGreaterThan(glm.escalation_bonus);
    const deepseek = twice.components.find((c) => c.model === "deepseek-v4-flash")!;
    expect(deepseek.escalation_bonus).toBe(0);
    expect(twice.decisive).toBe(true);
  });

  test("a decisive escalation fires once per attempt: the escalated model stays sticky, no ping-pong", () => {
    const { router, store } = setup();
    // The session already escalated to GLM on attempt 2; every later request
    // of attempt 2 still carries "verification failed".
    store.observeCache({ provider: "zai", model: "glm-5.3-flash", prefix_hash: "sha256:p", last_served_at: 999_500, prefix_tokens_seen: 30_000 });
    const session = { ...fresh(), current_model: "glm-5.3-flash", prefix_hash: "sha256:p", stable_tokens: 40_000, escalated_attempt: 2 };
    const again = router.decide(request({ session, attempt: 2, previous_failure: "test_failure", verification_failed: true }));
    expect(again.chosen.id).toBe("glm-5.3-flash");
    expect(again.decisive).toBe(false);
    expect(again.model_changed).toBe(false);
    expect(again.reason).toContain("sticky");
    // A new attempt with fresh failure evidence is decisive again, but Luna's
    // evidence prior (0.4) is not a credible alternative to GLM (1.0), so GLM
    // is not benched and stays.
    const next = router.decide(request({ session, attempt: 3, previous_failure: "test_failure", verification_failed: true }));
    expect(next.chosen.id).toBe("glm-5.3-flash");
    expect(next.decisive).toBe(false);
  });

  test("high complexity on a first request escalates without a switch penalty", () => {
    const { router } = setup();
    const decision = router.decide(request({ task_complexity: "high", attempt: 1 }));
    expect(decision.chosen.id).toBe("glm-5.3-flash");
    expect(decision.reason).toContain("escalated");
  });

  test("unhealthy providers are skipped and failover excludes them", () => {
    const { router, health } = setup();
    for (let i = 0; i < 3; i++) health.recordFailure("deepseek");
    const decision = router.decide(request());
    expect(decision.chosen.provider).not.toBe("deepseek");
    const excluded = router.decide(request({ exclude_providers: new Set(["deepseek", "openai"]) }));
    expect(excluded.chosen.provider).toBe("zai");
  });

  test("a provider near its tokens-per-minute cap is not a candidate for a request that would exceed it", () => {
    const { router, health } = setup();
    // 100k-token requests against OpenAI's 500k TPM: after four of them the
    // fifth would be refused mid-stream, so Luna must not be chosen even when
    // escalation would otherwise favour it.
    health.observeTokens("openai", 420_000);
    const session = { ...fresh(), current_model: "glm-5.3-flash", prefix_hash: "sha256:p", stable_tokens: 90_000 };
    const decision = router.decide(request({ session, stable_tokens: 90_000, dynamic_tokens: 10_000, task_complexity: "high", attempt: 1 }));
    const luna = decision.components.find((c) => c.model === "gpt-5.6-luna")!;
    expect(luna.eligible).toBe(false);
    expect(decision.chosen.id).not.toBe("gpt-5.6-luna");
    // A small request still fits.
    const small = router.decide(request({ session: fresh(), stable_tokens: 20_000, dynamic_tokens: 2_000 }));
    expect(small.components.find((c) => c.model === "gpt-5.6-luna")!.eligible).toBe(true);
  });

  test("historical success from feedback moves the ranking", () => {
    const { router, store } = setup();
    for (let i = 0; i < 20; i++) {
      store.recordFeedback({ session_id: `s${i}`, request_id: `r${i}`, at: 1, task_type: "debugging", task_success: i < 19, verification_result: "passed", failure_type: null, attempt_count: 1, cost_microusd: 1000, model: "glm-5.3-flash" });
      store.recordFeedback({ session_id: `t${i}`, request_id: `q${i}`, at: 1, task_type: "debugging", task_success: i < 4, verification_result: "failed", failure_type: "test_failure", attempt_count: 3, cost_microusd: 5000, model: "deepseek-v4-flash" });
    }
    const decision = router.decide(request({ task_type: "debugging" }));
    expect(decision.chosen.id).toBe("glm-5.3-flash");
    const glm = decision.components.find((c) => c.model === "glm-5.3-flash")!;
    expect(glm.historical_success).toBeCloseTo(20 / 22, 3);
    expect(glm.history_samples).toBe(20);
  });
});

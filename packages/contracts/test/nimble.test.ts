import { describe, expect, test } from "bun:test";

import {
  mergeNimbleRequestExtensions,
  nimbleRequestExtensionFromHeaders,
  parseNimbleFeedback,
  parseNimbleRequestExtension,
  parseNimbleRoutingResponse,
} from "../src/nimble";

describe("nimble request extension", () => {
  test("keeps valid fields and drops invalid ones", () => {
    const parsed = parseNimbleRequestExtension({
      version: "nimble/v1",
      session_id: "abc",
      task_type: "debugging",
      task_complexity: "extreme",
      agent_phase: "recovery",
      attempt: 2,
      previous_failure: "test_failure",
      verification_failed: true,
      estimated_stable_tokens: 48000,
      estimated_dynamic_tokens: -5,
      bogus: 1,
    });
    expect(parsed).toEqual({
      version: "nimble/v1",
      session_id: "abc",
      task_type: "debugging",
      agent_phase: "recovery",
      attempt: 2,
      previous_failure: "test_failure",
      verification_failed: true,
      estimated_stable_tokens: 48000,
    });
  });

  test("unknown versions and non-objects are ignored entirely", () => {
    expect(parseNimbleRequestExtension({ version: "nimble/v9", session_id: "x" })).toBeNull();
    expect(parseNimbleRequestExtension("nope")).toBeNull();
    expect(parseNimbleRequestExtension(null)).toBeNull();
  });

  test("headers parse with kebab-case names and body wins on merge", () => {
    const headers = new Headers({
      "x-nimble-session-id": "hdr",
      "x-nimble-attempt": "3",
      "x-nimble-verification-failed": "true",
      "x-nimble-task-type": "ops",
    });
    const fromHeaders = nimbleRequestExtensionFromHeaders(headers);
    expect(fromHeaders).toEqual({ session_id: "hdr", attempt: 3, verification_failed: true, task_type: "ops" });
    const merged = mergeNimbleRequestExtensions({ session_id: "body" }, fromHeaders);
    expect(merged?.session_id).toBe("body");
    expect(merged?.attempt).toBe(3);
    expect(nimbleRequestExtensionFromHeaders(new Headers())).toBeNull();
  });
});

describe("nimble feedback", () => {
  test("requires a session id and normalizes optional fields", () => {
    expect(parseNimbleFeedback({})).toEqual({ ok: false, error: "session_id is required" });
    const parsed = parseNimbleFeedback({
      session_id: "s",
      request_id: "r",
      task_success: true,
      verification_result: "passed",
      attempt_count: 2,
      cost_microusd: 1234,
      task_type: "feature",
    });
    expect(parsed.ok).toBe(true);
    if (parsed.ok) {
      expect(parsed.feedback.task_success).toBe(true);
      expect(parsed.feedback.failure_type).toBeNull();
      expect(parsed.feedback.cost_microusd).toBe(1234);
    }
    expect(parseNimbleFeedback({ session_id: "s", version: "nimble/v2" }).ok).toBe(false);
  });
});

describe("nimble routing response", () => {
  test("round-trips and clamps affinity", () => {
    const parsed = parseNimbleRoutingResponse({
      version: "nimble/v1",
      selected_model: "deepseek-v4-flash",
      provider: "deepseek",
      model_changed: false,
      cache_affinity: 1.7,
      estimated_cached_tokens: 48000,
      context_limit: 1000000,
      model_capabilities: { tools: true, parallel_tools: true, reasoning: true, prompt_cache: "automatic", images: false },
      routing_reason: "sticky",
      decision_id: "d1",
      request_id: "r1",
    });
    expect(parsed?.cache_affinity).toBe(1);
    expect(parsed?.model_capabilities.reasoning).toBe(true);
    expect(parseNimbleRoutingResponse({ version: "nimble/v1" })).toBeNull();
  });
});

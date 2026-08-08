import { describe, expect, test } from "bun:test";
import { createBufferedSseResponse, createLiveSseResponse, type ExecutionResult } from "../src/generation/streaming";

function chatResult(): ExecutionResult {
  return {
    status: 200,
    payload: {
      id: "chatcmpl-test",
      created: 1,
      choices: [{ message: { role: "assistant", content: "hello" }, finish_reason: "stop" }],
      usage: { prompt_tokens: 10, completion_tokens: 2 },
      gateway_usage: { cost_usd: "0.000110" },
    },
  };
}

describe("buffered SSE", () => {
  test("emits compatible chat chunks and final usage", async () => {
    const response = createBufferedSseResponse(
      "/v1/chat/completions",
      "gpt-5.6-luna",
      Promise.resolve(chatResult()),
      { "x-idempotent-replay": "true" },
    );
    const text = await response.text();
    expect(response.headers.get("x-gateway-stream-mode")).toBe("buffered");
    expect(response.headers.get("x-idempotent-replay")).toBe("true");
    expect(text).toContain('"content":"hello"');
    expect(text).toContain('"cost_usd":"0.000110"');
    expect(text).toContain("data: [DONE]");
  });

  test("client cancellation does not cancel execution", async () => {
    let resolve!: (value: ExecutionResult) => void;
    let settled = false;
    const execution = new Promise<ExecutionResult>((done) => { resolve = done; }).finally(() => { settled = true; });
    const response = createBufferedSseResponse("/v1/chat/completions", "gpt-5.6-luna", execution);
    const reader = response.body!.getReader();
    await reader.read();
    await reader.cancel();
    resolve(chatResult());
    await execution;
    expect(settled).toBe(true);
  });
});

describe("live SSE", () => {
  test("forwards incremental deltas before final usage", async () => {
    const response = createLiveSseResponse("/v1/chat/completions", "gpt-5.6-luna", async (callbacks) => {
      callbacks.onTextDelta("hel");
      await Promise.resolve();
      callbacks.onTextDelta("lo");
      return chatResult();
    });
    const text = await response.text();
    expect(response.headers.get("x-gateway-stream-mode")).toBe("live");
    expect(text.indexOf('"content":"hel"')).toBeLessThan(text.indexOf('"content":"lo"'));
    expect(text).toContain('"cost_usd":"0.000110"');
    expect(text).toContain("data: [DONE]");
  });

  test("streams a structured tool call", async () => {
    const response = createLiveSseResponse("/v1/chat/completions", "gpt-5.6-luna", async (callbacks) => {
      callbacks.onToolCall({ id: "call_1", name: "read_file", arguments: { path: "package.json" } });
      return {
        status: 200,
        payload: {
          choices: [{ message: { role: "assistant", tool_calls: [] }, finish_reason: "tool_calls" }],
          usage: { prompt_tokens: 10, completion_tokens: 2 },
          gateway_usage: { cost_usd: "0.000110" },
        },
      };
    });
    const text = await response.text();
    expect(text).toContain('"name":"read_file"');
    expect(text).toContain('"finish_reason":"tool_calls"');
  });

  test("emits a buffered HTTP payload when no live callbacks fire", async () => {
    const response = createLiveSseResponse(
      "/v1/chat/completions",
      "gpt-5.5",
      async () => chatResult(),
      { "x-request-id": "request-routed" },
    );
    const text = await response.text();
    expect(response.headers.get("x-request-id")).toBe("request-routed");
    expect(text).toContain('"content":"hello"');
    expect(text.match(/"content":"hello"/g)).toHaveLength(1);
    expect(text).toContain("data: [DONE]");
  });
});

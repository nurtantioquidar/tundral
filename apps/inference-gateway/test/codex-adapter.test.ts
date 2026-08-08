import { describe, expect, test } from "bun:test";
import { codexConversationFromRequest, codexPayload, parseCodexJsonl, promptFromRequest } from "../src/upstreams/adapters/codex";

describe("Codex adapter", () => {
  test("parses answer and usage events", () => {
    const result = parseCodexJsonl([
      JSON.stringify({ type: "item.completed", item: { type: "agent_message", text: "PILOT_OK" } }),
      JSON.stringify({ type: "turn.completed", usage: { input_tokens: 13933, cached_input_tokens: 9984, output_tokens: 8, reasoning_output_tokens: 0 } }),
    ].join("\n"));
    expect(result.text).toBe("PILOT_OK");
    expect(result.toolCalls).toEqual([]);
    expect(result.usage).toEqual({ inputTokens: 13933, cachedInputTokens: 9984, cacheWriteTokens: 0, outputTokens: 8 });
  });

  test("serializes chat messages and emits compatible usage", () => {
    const prompt = promptFromRequest("/v1/chat/completions", { messages: [{ role: "user", content: "hello" }] });
    expect(prompt).toContain('"role":"user","content":"hello"');
    const result = parseCodexJsonl([
      JSON.stringify({ type: "item.completed", item: { type: "agent_message", text: "hi" } }),
      JSON.stringify({ type: "turn.completed", usage: { input_tokens: 10, cached_input_tokens: 5, output_tokens: 2, reasoning_output_tokens: 1 } }),
    ].join("\n"));
    const payload = codexPayload("/v1/chat/completions", "gpt-5.6-luna", result);
    expect((payload.usage as Record<string, unknown>).prompt_tokens).toBe(10);
  });

  test("preserves assistant tool calls and matching tool results", () => {
    const prompt = promptFromRequest("/v1/chat/completions", {
      messages: [
        { role: "assistant", content: null, tool_calls: [{ id: "call_1", type: "function", function: { name: "read_file", arguments: '{"path":"package.json"}' } }] },
        { role: "tool", tool_call_id: "call_1", content: '{"name":"gateway"}' },
      ],
    }, false);
    expect(prompt).toContain('"tool_calls":[{"id":"call_1"');
    expect(prompt).toContain('"tool_call_id":"call_1"');
    expect(prompt).toContain('"content":"{\\"name\\":\\"gateway\\"}"');

    const conversation = codexConversationFromRequest("/v1/chat/completions", {
      messages: [
        { role: "user", content: "Read package.json" },
        { role: "assistant", content: null, tool_calls: [{ id: "call_1", type: "function", function: { name: "read_file", arguments: '{"path":"package.json"}' } }] },
        { role: "tool", tool_call_id: "call_1", content: '{"name":"gateway"}' },
      ],
    });
    expect(conversation.history).toEqual([
      { type: "message", role: "user", content: [{ type: "input_text", text: "Read package.json" }] },
      { type: "function_call", call_id: "call_1", name: "read_file", arguments: '{"path":"package.json"}' },
      { type: "function_call_output", call_id: "call_1", output: '{"name":"gateway"}' },
    ]);
    expect(conversation.inputText).toContain("latest structured tool result");
  });

  test("translates structured Codex output into OpenAI tool calls", () => {
    const prompt = promptFromRequest("/v1/chat/completions", {
      messages: [{ role: "user", content: "Read package.json" }],
      tools: [{ type: "function", function: { name: "read_file", description: "Read a file", parameters: { type: "object", properties: { path: { type: "string" } } } } }],
    });
    expect(prompt).toContain('"name":"read_file"');
    const result = parseCodexJsonl([
      JSON.stringify({ type: "item.completed", item: { type: "agent_message", text: JSON.stringify({ type: "tool_calls", content: "", tool_calls: [{ id: "call_1", name: "read_file", arguments: '{"path":"package.json"}' }] }) } }),
      JSON.stringify({ type: "turn.completed", usage: { input_tokens: 100, cached_input_tokens: 50, output_tokens: 20, reasoning_output_tokens: 0 } }),
    ].join("\n"));
    const payload = codexPayload("/v1/chat/completions", "gpt-5.6-luna", result);
    const choice = (payload.choices as Array<Record<string, unknown>>)[0]!;
    expect(choice.finish_reason).toBe("tool_calls");
    const message = choice.message as Record<string, unknown>;
    expect(message.tool_calls).toEqual([{ id: "call_1", type: "function", function: { name: "read_file", arguments: '{"path":"package.json"}' } }]);
  });
});

// OpenAI Responses API translation for upstreams that only offer function tools together with
// reasoning on `/responses` (OpenAI's own reasoning models — verified 2026-09-04 on
// gpt-5.6-luna). Clients keep speaking Chat Completions to Nimble; this module converts the
// request into Responses input items and the event stream back into `chat.completion.chunk`
// lines, so Butterfly's Nimble dialect (and any OpenAI client) works unchanged.
//
// Encrypted reasoning: requests use `store: false` + `include: ["reasoning.encrypted_content"]`.
// The encrypted items ride back to the client as an opaque `nimble_replay` array on the final
// assistant chunk; a client that echoes it on the assistant message (Butterfly does) gets it
// replayed before that turn's function calls. Clients that drop it still work — the model just
// re-reasons.

import type { Usage } from "./pricing";

export const NIMBLE_REPLAY_FIELD = "nimble_replay";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((part) => (isRecord(part) && typeof part.text === "string" ? part.text : ""))
      .join("");
  }
  return "";
}

/** Chat Completions body → Responses body. Returns null when the body is not translatable. */
export function chatToResponses(body: Record<string, unknown>): Record<string, unknown> {
  const messages = Array.isArray(body.messages) ? body.messages : [];
  const input: unknown[] = [];
  let instructions: string | undefined;
  for (const raw of messages) {
    if (!isRecord(raw)) continue;
    const role = raw.role;
    if (role === "system" || role === "developer") {
      const text = textOf(raw.content);
      if (instructions === undefined) instructions = text;
      else input.push({ role: "system", content: text });
    } else if (role === "user") {
      input.push({ role: "user", content: textOf(raw.content) });
    } else if (role === "assistant") {
      const replay = raw[NIMBLE_REPLAY_FIELD];
      if (Array.isArray(replay)) input.push(...replay);
      const text = textOf(raw.content);
      if (text.length > 0) input.push({ type: "message", role: "assistant", content: [{ type: "output_text", text }] });
      if (Array.isArray(raw.tool_calls)) {
        for (const call of raw.tool_calls) {
          if (!isRecord(call)) continue;
          const fn = isRecord(call.function) ? call.function : {};
          input.push({ type: "function_call", call_id: call.id, name: fn.name, arguments: typeof fn.arguments === "string" ? fn.arguments : JSON.stringify(fn.arguments ?? {}) });
        }
      }
    } else if (role === "tool") {
      input.push({ type: "function_call_output", call_id: raw.tool_call_id, output: textOf(raw.content) });
    }
  }
  const out: Record<string, unknown> = { model: body.model, input, stream: true, store: false };
  if (instructions !== undefined) out.instructions = instructions;
  if (Array.isArray(body.tools) && body.tools.length > 0) {
    out.tools = body.tools.map((tool) => {
      const fn = isRecord(tool) && isRecord(tool.function) ? tool.function : {};
      return { type: "function", name: fn.name, description: fn.description ?? "", parameters: fn.parameters ?? { type: "object", properties: {} }, strict: false };
    });
    out.tool_choice = typeof body.tool_choice === "string" ? body.tool_choice : "auto";
  }
  const budget = body.max_completion_tokens ?? body.max_tokens;
  if (typeof budget === "number") out.max_output_tokens = budget;
  const reasoning: Record<string, unknown> = { summary: "auto" };
  if (typeof body.reasoning_effort === "string" && body.reasoning_effort !== "none") reasoning.effort = body.reasoning_effort;
  out.reasoning = reasoning;
  out.include = ["reasoning.encrypted_content"];
  if (typeof body.temperature === "number") out.temperature = body.temperature;
  return out;
}

export function responsesUsage(raw: unknown): Usage | null {
  if (!isRecord(raw)) return null;
  const num = (v: unknown) => (typeof v === "number" && Number.isFinite(v) ? v : null);
  const input = num(raw.input_tokens);
  if (input === null) return null;
  const details = isRecord(raw.input_tokens_details) ? raw.input_tokens_details : {};
  const cached = num(details.cached_tokens) ?? 0;
  return { input_tokens: input, cached_tokens: Math.min(cached, input), output_tokens: num(raw.output_tokens) ?? 0 };
}

/** Chat-shaped usage object for the final chunk. */
export function chatUsage(usage: Usage): Record<string, unknown> {
  return {
    prompt_tokens: usage.input_tokens,
    completion_tokens: usage.output_tokens,
    total_tokens: usage.input_tokens + usage.output_tokens,
    prompt_tokens_details: { cached_tokens: usage.cached_tokens },
  };
}

interface CallState {
  index: number;
  item_id: string;
  call_id: string;
  name: string;
  done: boolean;
}

/**
 * Stateful translator: feed Responses events, get Chat Completions chunk objects. The caller
 * serializes them as `data:` lines. `finish()` returns the terminal chunk (usage + finish_reason)
 * and the usage it observed.
 */
export class ResponsesToChat {
  private readonly id: string;
  private readonly model: string;
  private readonly created: number;
  private calls: CallState[] = [];
  private replay: unknown[] = [];
  private usage: Usage | null = null;
  private finish: "stop" | "tool_calls" | "length" | null = null;
  private error: string | null = null;
  private sentRole = false;

  constructor(model: string, requestId: string, nowMs = Date.now()) {
    this.model = model;
    this.id = `chatcmpl-${requestId}`;
    this.created = Math.floor(nowMs / 1000);
  }

  private chunk(delta: Record<string, unknown>, finish_reason: string | null = null): Record<string, unknown> {
    if (!this.sentRole) {
      delta = { role: "assistant", ...delta };
      this.sentRole = true;
    }
    return { id: this.id, object: "chat.completion.chunk", created: this.created, model: this.model, choices: [{ index: 0, delta, finish_reason }] };
  }

  /** Translate one event; returns zero or more chunk objects to emit. */
  consume(event: unknown): Record<string, unknown>[] {
    if (!isRecord(event)) return [];
    const type = typeof event.type === "string" ? event.type : "";
    switch (type) {
      case "response.output_text.delta":
        return typeof event.delta === "string" ? [this.chunk({ content: event.delta })] : [];
      case "response.reasoning_summary_text.delta":
      case "response.reasoning_text.delta":
        return typeof event.delta === "string" ? [this.chunk({ reasoning_content: event.delta })] : [];
      case "response.output_item.added": {
        const item = isRecord(event.item) ? event.item : null;
        if (item && item.type === "function_call") {
          const state: CallState = { index: this.calls.length, item_id: String(item.id ?? ""), call_id: String(item.call_id ?? ""), name: String(item.name ?? ""), done: false };
          this.calls.push(state);
          return [this.chunk({ tool_calls: [{ index: state.index, id: state.call_id, type: "function", function: { name: state.name, arguments: "" } }] })];
        }
        return [];
      }
      case "response.function_call_arguments.delta": {
        const state = this.calls.find((c) => c.item_id === String(event.item_id ?? ""));
        if (!state || state.done || typeof event.delta !== "string") return [];
        return [this.chunk({ tool_calls: [{ index: state.index, function: { arguments: event.delta } }] })];
      }
      case "response.function_call_arguments.done": {
        const state = this.calls.find((c) => c.item_id === String(event.item_id ?? ""));
        if (state) state.done = true;
        return [];
      }
      case "response.output_item.done": {
        const item = isRecord(event.item) ? event.item : null;
        if (!item) return [];
        if (item.type === "reasoning" && item.encrypted_content) {
          this.replay.push({ type: "reasoning", id: item.id, encrypted_content: item.encrypted_content, summary: [] });
        } else if (item.type === "function_call") {
          const state = this.calls.find((c) => c.item_id === String(item.id ?? ""));
          if (!state) {
            // Server skipped `output_item.added`: emit the whole call at once.
            const created: CallState = { index: this.calls.length, item_id: String(item.id ?? ""), call_id: String(item.call_id ?? ""), name: String(item.name ?? ""), done: true };
            this.calls.push(created);
            return [this.chunk({ tool_calls: [{ index: created.index, id: created.call_id, type: "function", function: { name: created.name, arguments: String(item.arguments ?? "") } }] })];
          }
          state.done = true;
        }
        return [];
      }
      case "response.completed":
      case "response.incomplete":
      case "response.failed": {
        const response = isRecord(event.response) ? event.response : {};
        this.usage = responsesUsage(response.usage) ?? this.usage;
        const reason = isRecord(response.incomplete_details) ? response.incomplete_details.reason : undefined;
        if (type === "response.failed") {
          const err = isRecord(response.error) ? response.error : {};
          this.error = typeof err.message === "string" ? err.message : "response failed";
        } else if (reason === "max_output_tokens") this.finish = "length";
        else this.finish = this.calls.length > 0 ? "tool_calls" : "stop";
        return [];
      }
      case "error": {
        const err = isRecord(event.error) ? event.error : event;
        this.error = typeof err.message === "string" ? err.message : "stream error";
        return [];
      }
      default:
        return [];
    }
  }

  /** Terminal chunks: replay items + finish, then a usage-only chunk (OpenAI shape). */
  finishChunks(): { chunks: Record<string, unknown>[]; usage: Usage | null; error: string | null } {
    const chunks: Record<string, unknown>[] = [];
    const delta: Record<string, unknown> = {};
    if (this.replay.length > 0) delta[NIMBLE_REPLAY_FIELD] = this.replay;
    chunks.push(this.chunk(delta, this.finish ?? "stop"));
    if (this.usage) {
      chunks.push({ id: this.id, object: "chat.completion.chunk", created: this.created, model: this.model, choices: [], usage: chatUsage(this.usage) });
    }
    return { chunks, usage: this.usage, error: this.error };
  }
}

/** Non-streaming: a complete Responses object → a `chat.completion` object. */
export function responsesToChatCompletion(response: Record<string, unknown>, requestId: string, nowMs = Date.now()): { completion: Record<string, unknown>; usage: Usage | null } {
  const output = Array.isArray(response.output) ? response.output : [];
  let content = "";
  let reasoning = "";
  const toolCalls: unknown[] = [];
  const replay: unknown[] = [];
  for (const item of output) {
    if (!isRecord(item)) continue;
    if (item.type === "message") {
      content += textOf(item.content);
    } else if (item.type === "function_call") {
      toolCalls.push({ id: item.call_id, type: "function", function: { name: item.name, arguments: item.arguments ?? "{}" } });
    } else if (item.type === "reasoning") {
      if (Array.isArray(item.summary)) reasoning += item.summary.map((s) => (isRecord(s) && typeof s.text === "string" ? s.text : "")).join("");
      if (item.encrypted_content) replay.push({ type: "reasoning", id: item.id, encrypted_content: item.encrypted_content, summary: [] });
    }
  }
  const usage = responsesUsage(response.usage);
  const message: Record<string, unknown> = { role: "assistant", content: content.length > 0 ? content : null };
  if (reasoning) message.reasoning_content = reasoning;
  if (toolCalls.length > 0) message.tool_calls = toolCalls;
  if (replay.length > 0) message[NIMBLE_REPLAY_FIELD] = replay;
  const incomplete = isRecord(response.incomplete_details) ? response.incomplete_details.reason : undefined;
  const finish = incomplete === "max_output_tokens" ? "length" : toolCalls.length > 0 ? "tool_calls" : "stop";
  return {
    completion: {
      id: `chatcmpl-${requestId}`,
      object: "chat.completion",
      created: Math.floor(nowMs / 1000),
      model: String(response.model ?? ""),
      choices: [{ index: 0, message, finish_reason: finish }],
      ...(usage ? { usage: chatUsage(usage) } : {}),
    },
    usage,
  };
}

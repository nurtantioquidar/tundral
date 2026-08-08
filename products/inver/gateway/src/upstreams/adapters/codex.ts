import { randomUUID } from "node:crypto";
import { join } from "node:path";
import type { TokenUsage } from "../../billing/pricing";

export interface ToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

export interface CodexResult {
  text: string;
  toolCalls: ToolCall[];
  usage: TokenUsage;
  reasoningOutputTokens: number;
}

export interface CodexConversation {
  history: Array<Record<string, unknown>>;
  inputText: string;
}

function count(value: unknown): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : 0;
}

export function promptFromRequest(path: string, body: Record<string, unknown>, describeTools = true): string {
  const tools = normalizeTools(body.tools);
  const toolInstruction = describeTools && tools.length > 0
    ? `\n\nAvailable external tools:\n${JSON.stringify(tools)}\n\nChoose external tools only when needed. Do not execute them yourself. Return type=tool_calls with exact function names and arguments encoded as a JSON string, or type=message with the final answer.`
    : "";
  if (path === "/v1/responses") {
    const input = body.input;
    const serialized = typeof input === "string" ? input : JSON.stringify(input ?? "");
    return serialized + toolInstruction;
  }

  const messages = Array.isArray(body.messages) ? body.messages : [];
  const transcript = messages.flatMap((message) => {
    if (!message || typeof message !== "object") return [];
    const item = message as Record<string, unknown>;
    const serialized: Record<string, unknown> = {
      role: typeof item.role === "string" ? item.role : "user",
      content: item.content ?? null,
    };
    if (typeof item.name === "string") serialized.name = item.name;
    if (typeof item.tool_call_id === "string") serialized.tool_call_id = item.tool_call_id;
    if (Array.isArray(item.tool_calls)) serialized.tool_calls = item.tool_calls;
    return [serialized];
  });
  return `Continue the following stateless OpenAI Chat Completions transcript. Preserve the causal relationship between assistant tool_calls and tool messages with matching tool_call_id.\n<conversation_json>\n${JSON.stringify(transcript)}\n</conversation_json>` + toolInstruction;
}

function messageContent(value: unknown): string {
  if (value === null || value === undefined) return "";
  return typeof value === "string" ? value : JSON.stringify(value);
}

export function codexConversationFromRequest(path: string, body: Record<string, unknown>): CodexConversation {
  if (path !== "/v1/chat/completions") {
    return { history: [], inputText: promptFromRequest(path, body, false) };
  }
  const messages = Array.isArray(body.messages)
    ? body.messages.filter((value): value is Record<string, unknown> => Boolean(value) && typeof value === "object")
    : [];
  const last = messages.at(-1);
  const lastIsUser = last?.role === "user";
  const historyMessages = lastIsUser ? messages.slice(0, -1) : messages;
  const history = historyMessages.flatMap((item) => {
    const role = typeof item.role === "string" ? item.role : "user";
    if (role === "tool" && typeof item.tool_call_id === "string") {
      return [{ type: "function_call_output", call_id: item.tool_call_id, output: messageContent(item.content) }];
    }
    const result: Array<Record<string, unknown>> = [];
    const content = messageContent(item.content);
    if (content) {
      result.push({
        type: "message",
        role,
        content: [{ type: role === "assistant" ? "output_text" : "input_text", text: content }],
      });
    }
    if (role === "assistant" && Array.isArray(item.tool_calls)) {
      for (const rawCall of item.tool_calls) {
        if (!rawCall || typeof rawCall !== "object") continue;
        const call = rawCall as Record<string, unknown>;
        const fn = call.function && typeof call.function === "object"
          ? call.function as Record<string, unknown>
          : {};
        if (typeof call.id !== "string" || typeof fn.name !== "string") continue;
        result.push({
          type: "function_call",
          call_id: call.id,
          name: fn.name,
          arguments: typeof fn.arguments === "string" ? fn.arguments : JSON.stringify(fn.arguments ?? {}),
        });
      }
    }
    return result;
  });
  return {
    history,
    inputText: lastIsUser
      ? messageContent(last?.content)
      : "Continue the conversation using the latest structured tool result. Do not repeat a completed tool call.",
  };
}

function normalizeTools(value: unknown): Array<Record<string, unknown>> {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item) => {
    if (!item || typeof item !== "object") return [];
    const tool = item as Record<string, unknown>;
    if (tool.type !== "function") return [];
    const nested = tool.function && typeof tool.function === "object"
      ? tool.function as Record<string, unknown>
      : tool;
    if (typeof nested.name !== "string" || !nested.name) return [];
    return [{
      name: nested.name,
      description: typeof nested.description === "string" ? nested.description : "",
      parameters: nested.parameters && typeof nested.parameters === "object" ? nested.parameters : { type: "object" },
    }];
  });
}

export function parseCodexJsonl(output: string): CodexResult {
  let text = "";
  let usage: TokenUsage | null = null;
  let reasoningOutputTokens = 0;

  for (const line of output.split("\n")) {
    if (!line.trim().startsWith("{")) continue;
    let event: Record<string, unknown>;
    try { event = JSON.parse(line) as Record<string, unknown>; } catch { continue; }
    if (event.type === "item.completed") {
      const item = event.item as Record<string, unknown> | undefined;
      if (item?.type === "agent_message" && typeof item.text === "string") text = item.text;
    }
    if (event.type === "turn.completed") {
      const raw = event.usage as Record<string, unknown> | undefined;
      if (raw) {
        usage = {
          inputTokens: count(raw.input_tokens),
          cachedInputTokens: count(raw.cached_input_tokens),
          cacheWriteTokens: 0,
          outputTokens: count(raw.output_tokens),
        };
        reasoningOutputTokens = count(raw.reasoning_output_tokens);
      }
    }
  }

  if (!usage) throw new Error("Codex response did not include usage metadata");
  const structured = parseStructuredResult(text);
  return { text: structured.text, toolCalls: structured.toolCalls, usage, reasoningOutputTokens };
}

function parseStructuredResult(text: string): { text: string; toolCalls: ToolCall[] } {
  try {
    const parsed = JSON.parse(text) as Record<string, unknown>;
    if (parsed.type !== "tool_calls" && parsed.type !== "message") return { text, toolCalls: [] };
    const calls = Array.isArray(parsed.tool_calls) ? parsed.tool_calls : [];
    const toolCalls = calls.flatMap((value, index) => {
      if (!value || typeof value !== "object") return [];
      const call = value as Record<string, unknown>;
      if (typeof call.name !== "string") return [];
      let args: Record<string, unknown> = {};
      if (call.arguments && typeof call.arguments === "object" && !Array.isArray(call.arguments)) {
        args = call.arguments as Record<string, unknown>;
      } else if (typeof call.arguments === "string") {
        try {
          const decoded = JSON.parse(call.arguments) as unknown;
          if (decoded && typeof decoded === "object" && !Array.isArray(decoded)) args = decoded as Record<string, unknown>;
        } catch { /* Invalid model arguments become an empty object. */ }
      }
      return [{ id: typeof call.id === "string" && call.id ? call.id : `call_${randomUUID().replaceAll("-", "")}_${index}`, name: call.name, arguments: args }];
    });
    return {
      text: parsed.type === "message" && typeof parsed.content === "string" ? parsed.content : "",
      toolCalls,
    };
  } catch {
    return { text, toolCalls: [] };
  }
}

export async function runCodex(model: string, prompt: string, useToolSchema = false): Promise<CodexResult> {
  const executable = process.env.CODEX_EXECUTABLE || "codex";
  const args = [
    executable, "exec", "-", "--ephemeral", "--skip-git-repo-check", "--ignore-user-config",
    "--ignore-rules", "-s", "read-only", "-m", model, "--json",
  ];
  if (useToolSchema) {
    const schemaPath = process.env.CODEX_TOOL_SCHEMA_PATH || join(import.meta.dir, "../../tool-output-schema.json");
    args.push("--output-schema", schemaPath);
  }
  const spawnCodex = () => Bun.spawn(args, {
    cwd: process.env.CODEX_WORKDIR || process.cwd(),
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
    env: process.env,
  } as const);
  let proc: ReturnType<typeof spawnCodex>;
  try {
    proc = spawnCodex();
  } catch {
    throw new Error("Codex execution failed to start");
  }
  proc.stdin.write(prompt);
  proc.stdin.end();
  const [stdout, , exitCode] = await Promise.all([
    new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited,
  ]);
  if (exitCode !== 0) throw new Error(`Codex execution failed (${exitCode})`);
  return parseCodexJsonl(stdout);
}

export function codexPayload(path: string, model: string, result: CodexResult): Record<string, unknown> {
  const id = randomUUID().replaceAll("-", "");
  if (path === "/v1/chat/completions") {
    const toolCalls = result.toolCalls.map((call) => ({
      id: call.id, type: "function", function: { name: call.name, arguments: JSON.stringify(call.arguments) },
    }));
    return {
      id: `chatcmpl-${id}`, object: "chat.completion", created: Math.floor(Date.now() / 1000), model,
      choices: [{
        index: 0,
        message: { role: "assistant", content: result.text || null, ...(toolCalls.length ? { tool_calls: toolCalls } : {}) },
        finish_reason: toolCalls.length ? "tool_calls" : "stop",
      }],
      usage: {
        prompt_tokens: result.usage.inputTokens,
        completion_tokens: result.usage.outputTokens,
        total_tokens: result.usage.inputTokens + result.usage.outputTokens,
        prompt_tokens_details: { cached_tokens: result.usage.cachedInputTokens },
        completion_tokens_details: { reasoning_tokens: result.reasoningOutputTokens },
      },
    };
  }
  const output = result.toolCalls.length
    ? result.toolCalls.map((call) => ({
        id: call.id, type: "function_call", call_id: call.id, name: call.name,
        arguments: JSON.stringify(call.arguments), status: "completed",
      }))
    : [{
        id: `msg_${id}`, type: "message", status: "completed", role: "assistant",
        content: [{ type: "output_text", text: result.text, annotations: [] }],
      }];
  return {
    id: `resp_${id}`, object: "response", created_at: Math.floor(Date.now() / 1000), status: "completed", model,
    output,
    output_text: result.text,
    usage: {
      input_tokens: result.usage.inputTokens,
      input_tokens_details: { cached_tokens: result.usage.cachedInputTokens },
      output_tokens: result.usage.outputTokens,
      output_tokens_details: { reasoning_tokens: result.reasoningOutputTokens },
      total_tokens: result.usage.inputTokens + result.usage.outputTokens,
    },
  };
}

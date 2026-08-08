import type { CodexConversation, CodexResult, ToolCall } from "./codex";
import type { ReasoningEffort } from "../reasoning-effort";

type JsonRecord = Record<string, any>;

export interface AppServerCallbacks {
  onTextDelta?: (delta: string) => void;
  onToolCall?: (call: ToolCall) => void;
}

function normalizeDynamicTools(value: unknown): JsonRecord[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item) => {
    if (!item || typeof item !== "object") return [];
    const tool = item as JsonRecord;
    if (tool.type !== "function") return [];
    const fn = tool.function && typeof tool.function === "object" ? tool.function as JsonRecord : tool;
    if (typeof fn.name !== "string" || !fn.name) return [];
    return [{
      type: "function",
      name: fn.name,
      description: typeof fn.description === "string" ? fn.description : "",
      inputSchema: fn.parameters && typeof fn.parameters === "object"
        ? fn.parameters
        : { type: "object", properties: {}, additionalProperties: false },
    }];
  });
}

export async function runCodexAppServer(
  model: string,
  reasoningEffort: ReasoningEffort | undefined,
  maxOutputTokens: number,
  conversation: CodexConversation,
  tools: unknown,
  callbacks: AppServerCallbacks = {},
): Promise<CodexResult> {
  const executable = process.env.CODEX_EXECUTABLE || "codex";
  const cwd = process.env.CODEX_WORKDIR || process.cwd();
  const spawn = () => Bun.spawn([executable, "app-server", "--stdio"], {
    cwd,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
    env: process.env,
  } as const);
  let proc: ReturnType<typeof spawn>;
  try { proc = spawn(); } catch { throw new Error("Codex app-server failed to start"); }

  const dynamicTools = normalizeDynamicTools(tools);
  let requestId = 1;
  let injectRequestId = 0;
  let turnStartRequestId = 0;
  let threadId = "";
  let turnId = "";
  let text = "";
  let completedText = "";
  let toolCall: ToolCall | null = null;
  let usage: CodexResult["usage"] | null = null;
  let reasoningOutputTokens = 0;
  let turnCompleted = false;
  let turnStatus = "";
  let settled = false;
  let interrupted = false;
  const timeoutMs = Number(process.env.CODEX_APP_SERVER_TIMEOUT_MS || 300_000);

  const send = (message: unknown) => proc.stdin.write(`${JSON.stringify(message)}\n`);
  const startTurn = () => {
    turnStartRequestId = ++requestId;
    send({
      id: turnStartRequestId,
      method: "turn/start",
      params: {
        threadId,
        input: [{ type: "text", text: conversation.inputText, text_elements: [] }],
        effort: reasoningEffort,
      },
    });
  };
  const cleanup = () => {
    try { proc.stdin.end(); } catch { /* already closed */ }
    try { proc.kill(); } catch { /* already exited */ }
  };

  return await new Promise<CodexResult>((resolve, reject) => {
    const timer = setTimeout(() => finishError(new Error("Codex app-server timed out")), timeoutMs);
    const finishError = (cause: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      cleanup();
      reject(cause);
    };
    const maybeFinish = () => {
      if (settled || !turnCompleted || !usage) return;
      if (turnStatus !== "completed" && !(turnStatus === "interrupted" && toolCall)) {
        finishError(new Error(`Codex turn failed (${turnStatus || "unknown"})`));
        return;
      }
      settled = true;
      clearTimeout(timer);
      cleanup();
      resolve({
        text: completedText || text,
        toolCalls: toolCall ? [toolCall] : [],
        usage,
        reasoningOutputTokens,
      });
    };

    void new Response(proc.stderr).text();
    void (async () => {
      let buffer = "";
      try {
        for await (const chunk of proc.stdout) {
          buffer += new TextDecoder().decode(chunk, { stream: true });
          const lines = buffer.split("\n");
          buffer = lines.pop() || "";
          for (const line of lines) {
            if (!line.trim()) continue;
            let message: JsonRecord;
            try { message = JSON.parse(line) as JsonRecord; } catch { continue; }

            if (message.id === 1 && message.result) {
              send({ method: "initialized" });
              send({
                id: ++requestId,
                method: "thread/start",
                params: {
                  model,
                  cwd,
                  approvalPolicy: "never",
                  sandbox: "read-only",
                  ephemeral: true,
                  historyMode: "legacy",
                  dynamicTools,
                  developerInstructions: `Act as a stateless model backend. Do not access the local filesystem or execute commands. When a supplied dynamic tool is needed, invoke it through the native dynamic-tool mechanism; never print, serialize, or describe a tool call as assistant text or JSON. Request at most one dynamic tool per turn. After a matching tool result is present in the conversation transcript, use it to continue the task instead of repeating the completed call. Keep the final answer within ${maxOutputTokens} output tokens.`,
                },
              });
              continue;
            }
            if (message.id === 2 && message.result?.thread?.id) {
              threadId = message.result.thread.id;
              if (conversation.history.length > 0) {
                injectRequestId = ++requestId;
                send({
                  id: injectRequestId,
                  method: "thread/inject_items",
                  params: { threadId, items: conversation.history },
                });
              } else {
                startTurn();
              }
              continue;
            }
            if (injectRequestId && message.id === injectRequestId && message.result) {
              startTurn();
              continue;
            }
            if (turnStartRequestId && message.id === turnStartRequestId && message.result?.turn?.id) {
              turnId = message.result.turn.id;
              continue;
            }
            if (message.method === "item/agentMessage/delta" && typeof message.params?.delta === "string") {
              const delta = message.params.delta;
              text += delta;
              callbacks.onTextDelta?.(delta);
              continue;
            }
            if (message.method === "item/completed" && message.params?.item?.type === "agentMessage") {
              completedText = typeof message.params.item.text === "string" ? message.params.item.text : completedText;
              continue;
            }
            if (message.method === "item/tool/call" && message.params && !toolCall) {
              toolCall = {
                id: String(message.params.callId),
                name: String(message.params.tool),
                arguments: message.params.arguments && typeof message.params.arguments === "object"
                  ? message.params.arguments as Record<string, unknown>
                  : {},
              };
              callbacks.onToolCall?.(toolCall);
              if (!interrupted) {
                interrupted = true;
                send({
                  id: ++requestId,
                  method: "turn/interrupt",
                  params: { threadId: String(message.params.threadId || threadId), turnId: String(message.params.turnId || turnId) },
                });
              }
              continue;
            }
            if (message.method === "thread/tokenUsage/updated" && message.params?.tokenUsage?.last) {
              const last = message.params.tokenUsage.last;
              usage = {
                inputTokens: Number(last.inputTokens || 0),
                cachedInputTokens: Number(last.cachedInputTokens || 0),
                cacheWriteTokens: 0,
                outputTokens: Number(last.outputTokens || 0),
              };
              reasoningOutputTokens = Number(last.reasoningOutputTokens || 0);
              maybeFinish();
              continue;
            }
            if (message.method === "turn/completed") {
              turnCompleted = true;
              turnStatus = String(message.params?.turn?.status || "unknown");
              maybeFinish();
            }
          }
        }
        if (!settled) finishError(new Error("Codex app-server exited before completion"));
      } catch {
        finishError(new Error("Codex app-server protocol failed"));
      }
    })();

    send({
      id: requestId,
      method: "initialize",
      params: {
        clientInfo: { name: "coding-gateway", title: "Coding Gateway", version: "0.1.0" },
        capabilities: { experimentalApi: true, requestAttestation: false },
      },
    });
  });
}

import { randomUUID } from "node:crypto";

/** Settled upstream result serialized by the public HTTP layer. */
export interface ExecutionResult {
  payload: Record<string, unknown>;
  status: number;
}

export interface LiveCallbacks {
  onTextDelta: (delta: string) => void;
  onToolCall: (call: { id: string; name: string; arguments: Record<string, unknown> }) => void;
}

function chatEvents(model: string, payload: Record<string, unknown>): string[] {
  const choices = Array.isArray(payload.choices) ? payload.choices : [];
  const first = choices[0] && typeof choices[0] === "object" ? choices[0] as Record<string, unknown> : {};
  const message = first.message && typeof first.message === "object" ? first.message as Record<string, unknown> : {};
  const id = typeof payload.id === "string" ? payload.id : `chatcmpl-${randomUUID().replaceAll("-", "")}`;
  const created = typeof payload.created === "number" ? payload.created : Math.floor(Date.now() / 1000);
  const events: string[] = [];
  events.push(JSON.stringify({
    id, object: "chat.completion.chunk", created, model,
    choices: [{ index: 0, delta: { role: "assistant", content: "" }, finish_reason: null }],
  }));
  if (Array.isArray(message.tool_calls) && message.tool_calls.length > 0) {
    events.push(JSON.stringify({
      id, object: "chat.completion.chunk", created, model,
      choices: [{ index: 0, delta: { tool_calls: message.tool_calls }, finish_reason: null }],
    }));
  } else if (typeof message.content === "string" && message.content) {
    events.push(JSON.stringify({
      id, object: "chat.completion.chunk", created, model,
      choices: [{ index: 0, delta: { content: message.content }, finish_reason: null }],
    }));
  }
  events.push(JSON.stringify({
    id, object: "chat.completion.chunk", created, model,
    choices: [{ index: 0, delta: {}, finish_reason: first.finish_reason === "tool_calls" ? "tool_calls" : "stop" }],
  }));
  events.push(JSON.stringify({
    id, object: "chat.completion.chunk", created, model, choices: [],
    usage: payload.usage, gateway_usage: payload.gateway_usage,
  }));
  return events.map((event) => `data: ${event}\n\n`).concat("data: [DONE]\n\n");
}

function responseEvents(model: string, payload: Record<string, unknown>): string[] {
  const response = { ...payload, model };
  const output = Array.isArray(payload.output) ? payload.output : [];
  const events: string[] = [
    `event: response.created\ndata: ${JSON.stringify({ type: "response.created", response: { ...response, status: "in_progress", output: [] } })}\n\n`,
  ];
  for (let index = 0; index < output.length; index++) {
    const item = output[index];
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    events.push(`event: response.output_item.added\ndata: ${JSON.stringify({ type: "response.output_item.added", output_index: index, item: record })}\n\n`);
    if (record.type === "message") {
      const content = Array.isArray(record.content) ? record.content : [];
      const first = content[0] && typeof content[0] === "object" ? content[0] as Record<string, unknown> : {};
      if (typeof first.text === "string" && first.text) {
        events.push(`event: response.output_text.delta\ndata: ${JSON.stringify({ type: "response.output_text.delta", output_index: index, content_index: 0, delta: first.text })}\n\n`);
      }
    }
    events.push(`event: response.output_item.done\ndata: ${JSON.stringify({ type: "response.output_item.done", output_index: index, item: record })}\n\n`);
  }
  events.push(`event: response.completed\ndata: ${JSON.stringify({ type: "response.completed", response })}\n\n`);
  events.push("data: [DONE]\n\n");
  return events;
}

export function createBufferedSseResponse(
  path: string,
  model: string,
  execution: Promise<ExecutionResult>,
  extraHeaders: Record<string, string> = {},
): Response {
  const encoder = new TextEncoder();
  let connected = true;
  let heartbeat: ReturnType<typeof setInterval> | undefined;
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode(": coding-gateway stream started\n\n"));
      heartbeat = setInterval(() => {
        if (!connected) return;
        try { controller.enqueue(encoder.encode(": keep-alive\n\n")); } catch { connected = false; }
      }, Number(process.env.STREAM_HEARTBEAT_MS || 5000));

      execution.then(({ payload }) => {
        const events = path === "/v1/chat/completions"
          ? chatEvents(model, payload)
          : responseEvents(model, payload);
        if (connected) {
          for (const event of events) controller.enqueue(encoder.encode(event));
          controller.close();
        }
      }).catch((cause) => {
        if (connected) {
          const message = cause instanceof Error ? cause.message : "Upstream failure";
          controller.enqueue(encoder.encode(`event: error\ndata: ${JSON.stringify({ error: { type: "upstream_error", message } })}\n\n`));
          controller.enqueue(encoder.encode("data: [DONE]\n\n"));
          controller.close();
        }
      }).finally(() => {
        if (heartbeat) clearInterval(heartbeat);
      });
    },
    cancel() {
      connected = false;
      if (heartbeat) clearInterval(heartbeat);
      // Deliberately do not cancel `execution`: it owns upstream metering and settlement.
    },
  });
  return new Response(stream, {
    headers: {
      "content-type": "text/event-stream; charset=utf-8",
      "cache-control": "no-cache, no-transform",
      connection: "keep-alive",
      "x-accel-buffering": "no",
      "x-gateway-stream-mode": "buffered",
      ...extraHeaders,
    },
  });
}

export function createLiveSseResponse(
  path: string,
  model: string,
  startExecution: (callbacks: LiveCallbacks) => Promise<ExecutionResult>,
  extraHeaders: Record<string, string> = {},
): Response {
  const encoder = new TextEncoder();
  const streamId = randomUUID().replaceAll("-", "");
  const responseId = path === "/v1/chat/completions" ? `chatcmpl-${streamId}` : `resp_${streamId}`;
  const messageId = `msg_${streamId}`;
  const created = Math.floor(Date.now() / 1000);
  let connected = true;
  let heartbeat: ReturnType<typeof setInterval> | undefined;
  let responsesMessageStarted = false;
  let emittedOutput = false;

  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      const emit = (event: string) => {
        if (!connected) return;
        try { controller.enqueue(encoder.encode(event)); } catch { connected = false; }
      };
      if (path === "/v1/chat/completions") {
        emit(`data: ${JSON.stringify({
          id: responseId, object: "chat.completion.chunk", created, model,
          choices: [{ index: 0, delta: { role: "assistant", content: "" }, finish_reason: null }],
        })}\n\n`);
      } else {
        emit(`event: response.created\ndata: ${JSON.stringify({
          type: "response.created",
          response: { id: responseId, object: "response", created_at: created, status: "in_progress", model, output: [] },
        })}\n\n`);
      }
      heartbeat = setInterval(() => emit(": keep-alive\n\n"), Number(process.env.STREAM_HEARTBEAT_MS || 5000));

      const execution = startExecution({
        onTextDelta(delta) {
          emittedOutput = true;
          if (path === "/v1/chat/completions") {
            emit(`data: ${JSON.stringify({
              id: responseId, object: "chat.completion.chunk", created, model,
              choices: [{ index: 0, delta: { content: delta }, finish_reason: null }],
            })}\n\n`);
          } else {
            if (!responsesMessageStarted) {
              responsesMessageStarted = true;
              emit(`event: response.output_item.added\ndata: ${JSON.stringify({
                type: "response.output_item.added", output_index: 0,
                item: { id: messageId, type: "message", status: "in_progress", role: "assistant", content: [] },
              })}\n\n`);
            }
            emit(`event: response.output_text.delta\ndata: ${JSON.stringify({
              type: "response.output_text.delta", output_index: 0, content_index: 0, delta,
            })}\n\n`);
          }
        },
        onToolCall(call) {
          emittedOutput = true;
          if (path === "/v1/chat/completions") {
            emit(`data: ${JSON.stringify({
              id: responseId, object: "chat.completion.chunk", created, model,
              choices: [{ index: 0, delta: { tool_calls: [{
                index: 0, id: call.id, type: "function",
                function: { name: call.name, arguments: JSON.stringify(call.arguments) },
              }] }, finish_reason: null }],
            })}\n\n`);
          } else {
            const item = {
              id: call.id, type: "function_call", call_id: call.id, name: call.name,
              arguments: JSON.stringify(call.arguments), status: "completed",
            };
            emit(`event: response.output_item.added\ndata: ${JSON.stringify({ type: "response.output_item.added", output_index: 0, item })}\n\n`);
            emit(`event: response.output_item.done\ndata: ${JSON.stringify({ type: "response.output_item.done", output_index: 0, item })}\n\n`);
          }
        },
      });

      execution.then(({ payload }) => {
        payload.id = responseId;
        if (path === "/v1/chat/completions") {
          const choices = Array.isArray(payload.choices) ? payload.choices : [];
          const first = choices[0] && typeof choices[0] === "object" ? choices[0] as Record<string, unknown> : {};
          const message = first.message && typeof first.message === "object"
            ? first.message as Record<string, unknown>
            : {};
          if (!emittedOutput && Array.isArray(message.tool_calls) && message.tool_calls.length > 0) {
            emit(`data: ${JSON.stringify({
              id: responseId, object: "chat.completion.chunk", created, model,
              choices: [{ index: 0, delta: { tool_calls: message.tool_calls }, finish_reason: null }],
            })}\n\n`);
          } else if (!emittedOutput && typeof message.content === "string" && message.content) {
            emit(`data: ${JSON.stringify({
              id: responseId, object: "chat.completion.chunk", created, model,
              choices: [{ index: 0, delta: { content: message.content }, finish_reason: null }],
            })}\n\n`);
          }
          emit(`data: ${JSON.stringify({
            id: responseId, object: "chat.completion.chunk", created, model,
            choices: [{ index: 0, delta: {}, finish_reason: first.finish_reason === "tool_calls" ? "tool_calls" : "stop" }],
          })}\n\n`);
          emit(`data: ${JSON.stringify({
            id: responseId, object: "chat.completion.chunk", created, model, choices: [],
            usage: payload.usage, gateway_usage: payload.gateway_usage,
          })}\n\n`);
        } else {
          if (!emittedOutput) {
            const output = Array.isArray(payload.output) ? payload.output : [];
            for (let index = 0; index < output.length; index += 1) {
              const item = output[index];
              if (!item || typeof item !== "object") continue;
              const record = item as Record<string, unknown>;
              emit(`event: response.output_item.added\ndata: ${JSON.stringify({
                type: "response.output_item.added", output_index: index, item: record,
              })}\n\n`);
              if (record.type === "message") {
                const content = Array.isArray(record.content) ? record.content : [];
                const first = content[0] && typeof content[0] === "object"
                  ? content[0] as Record<string, unknown>
                  : {};
                if (typeof first.text === "string" && first.text) {
                  emit(`event: response.output_text.delta\ndata: ${JSON.stringify({
                    type: "response.output_text.delta",
                    output_index: index,
                    content_index: 0,
                    delta: first.text,
                  })}\n\n`);
                }
              }
              emit(`event: response.output_item.done\ndata: ${JSON.stringify({
                type: "response.output_item.done", output_index: index, item: record,
              })}\n\n`);
            }
          }
          emit(`event: response.completed\ndata: ${JSON.stringify({ type: "response.completed", response: payload })}\n\n`);
        }
        emit("data: [DONE]\n\n");
        if (connected) controller.close();
      }).catch((cause) => {
        const message = cause instanceof Error ? cause.message : "Upstream failure";
        emit(`event: error\ndata: ${JSON.stringify({ error: { type: "upstream_error", message } })}\n\n`);
        emit("data: [DONE]\n\n");
        if (connected) controller.close();
      }).finally(() => {
        if (heartbeat) clearInterval(heartbeat);
      });
    },
    cancel() {
      connected = false;
      if (heartbeat) clearInterval(heartbeat);
      // The upstream promise is intentionally not cancelled; it must finish metering and settlement.
    },
  });

  return new Response(stream, {
    headers: {
      "content-type": "text/event-stream; charset=utf-8",
      "cache-control": "no-cache, no-transform",
      connection: "keep-alive",
      "x-accel-buffering": "no",
      "x-gateway-stream-mode": "live",
      ...extraHeaders,
    },
  });
}

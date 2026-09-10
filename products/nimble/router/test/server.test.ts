import { describe, expect, test } from "bun:test";

import { defaultConfigPath, loadConfig, type NimbleConfig } from "../src/config";
import { createApp } from "../src/server";
import { Store } from "../src/store";

const baseConfig = loadConfig(defaultConfigPath());

/** A fake upstream: records the bodies it receives and answers per provider base URL. */
function fakeUpstream(options: { fail?: Record<string, number>; stream?: boolean } = {}) {
  const calls: { url: string; body: any; auth: string | null }[] = [];
  const fetchImpl = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const body = JSON.parse(String(init?.body));
    const headers = new Headers(init?.headers);
    calls.push({ url, body, auth: headers.get("authorization") });
    const provider = url.includes("deepseek") ? "deepseek" : url.includes("z.ai") ? "zai" : url.includes("meta.ai") ? "meta" : "openai";
    const failStatus = options.fail?.[provider];
    if (failStatus) return new Response(JSON.stringify({ error: { message: "boom" } }), { status: failStatus });
    if (url.endsWith("/responses")) {
      // OpenAI Responses API shape: a reasoning item, a function call, usage.
      const usageR = { input_tokens: 1000, input_tokens_details: { cached_tokens: 100 }, output_tokens: 50 };
      if (!body.stream) {
        return Response.json({ id: "resp_1", object: "response", model: body.model, status: "completed", output: [
          { type: "reasoning", id: "rs_1", encrypted_content: "ENC", summary: [{ type: "summary_text", text: "hmm" }] },
          { type: "message", role: "assistant", content: [{ type: "output_text", text: "hello" }] },
        ], usage: usageR });
      }
      const ev = (e: unknown) => `data: ${JSON.stringify(e)}\n\n`;
      const events = [
        ev({ type: "response.created", response: { id: "resp_1" } }),
        ev({ type: "response.output_item.added", output_index: 0, item: { type: "reasoning", id: "rs_1", summary: [] } }),
        ev({ type: "response.reasoning_summary_text.delta", item_id: "rs_1", delta: "hmm" }),
        ev({ type: "response.output_item.done", output_index: 0, item: { type: "reasoning", id: "rs_1", encrypted_content: "ENC", summary: [] } }),
        ev({ type: "response.output_item.added", output_index: 1, item: { type: "function_call", id: "fc_1", call_id: "call_1", name: "read_file", arguments: "" } }),
        ev({ type: "response.function_call_arguments.delta", item_id: "fc_1", output_index: 1, delta: "{\"path\":" }),
        ev({ type: "response.function_call_arguments.delta", item_id: "fc_1", output_index: 1, delta: "\"a.rs\"}" }),
        ev({ type: "response.function_call_arguments.done", item_id: "fc_1", output_index: 1, arguments: "{\"path\":\"a.rs\"}" }),
        ev({ type: "response.output_item.done", output_index: 1, item: { type: "function_call", id: "fc_1", call_id: "call_1", name: "read_file", arguments: "{\"path\":\"a.rs\"}" } }),
        ev({ type: "response.completed", response: { id: "resp_1", status: "completed", usage: usageR } }),
      ];
      const stream = new ReadableStream<Uint8Array>({
        start(controller) {
          for (const chunk of events) controller.enqueue(new TextEncoder().encode(chunk));
          controller.close();
        },
      });
      return new Response(stream, { status: 200, headers: { "content-type": "text/event-stream" } });
    }
    const usage =
      provider === "deepseek"
        ? { prompt_tokens: 1000, prompt_cache_hit_tokens: 800, prompt_cache_miss_tokens: 200, completion_tokens: 50 }
        : { prompt_tokens: 1000, prompt_tokens_details: { cached_tokens: 100 }, completion_tokens: 50 };
    if (body.stream) {
      const chunks = [
        `data: ${JSON.stringify({ id: "c", object: "chat.completion.chunk", model: body.model, choices: [{ index: 0, delta: { content: "hel" } }] })}\n\n`,
        `data: ${JSON.stringify({ id: "c", object: "chat.completion.chunk", model: body.model, choices: [{ index: 0, delta: { content: "lo" }, finish_reason: "stop" }] })}\n\n`,
        `data: ${JSON.stringify({ id: "c", object: "chat.completion.chunk", model: body.model, choices: [], usage })}\n\n`,
        "data: [DONE]\n\n",
      ];
      const stream = new ReadableStream<Uint8Array>({
        start(controller) {
          for (const chunk of chunks) controller.enqueue(new TextEncoder().encode(chunk));
          controller.close();
        },
      });
      return new Response(stream, { status: 200, headers: { "content-type": "text/event-stream" } });
    }
    return new Response(JSON.stringify({ id: "c", object: "chat.completion", model: body.model, choices: [{ index: 0, message: { role: "assistant", content: "hello" }, finish_reason: "stop" }], usage }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  }) as unknown as typeof fetch;
  return { calls, fetchImpl };
}

function appWith(options: { fail?: Record<string, number>; config?: NimbleConfig; apiKey?: string | null } = {}) {
  process.env.DEEPSEEK_API_KEY = "sk-ds";
  process.env.ZAI_API_KEY = "sk-zai";
  process.env.META_API_KEY = "sk-meta";
  process.env.OPENAI_API_KEY = "sk-oa";
  const upstream = fakeUpstream({ fail: options.fail });
  const store = new Store(":memory:");
  const app = createApp({ config: options.config ?? baseConfig, store, fetchImpl: upstream.fetchImpl, apiKey: options.apiKey ?? null });
  return { app, store, upstream };
}

const post = (app: { fetch(r: Request): Promise<Response> }, path: string, body: unknown, headers: Record<string, string> = {}) =>
  app.fetch(new Request(`http://nimble.local${path}`, { method: "POST", headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body) }));

describe("chat completions", () => {
  test("explicit model passes through with routing facts and telemetry", async () => {
    const { app, store, upstream } = appWith();
    const response = await post(app, "/v1/chat/completions", { model: "deepseek-v4-flash", messages: [{ role: "system", content: "sys" }, { role: "user", content: "hi" }], nimble: { session_id: "abc" } });
    expect(response.status).toBe(200);
    const payload = (await response.json()) as any;
    expect(payload.choices[0].message.content).toBe("hello");
    expect(payload.nimble.selected_model).toBe("deepseek-v4-flash");
    expect(payload.nimble.routing_reason).toBe("explicit model");
    const facts = JSON.parse(response.headers.get("x-nimble-routing")!);
    expect(facts.version).toBe("nimble/v1");
    expect(upstream.calls[0]!.auth).toBe("Bearer sk-ds");
    expect(upstream.calls[0]!.body.nimble).toBeUndefined();
    expect(upstream.calls[0]!.url).toBe("https://api.deepseek.com/chat/completions");
    const record = store.lastRequestForSession("abc")!;
    expect(record.routed).toBe(false);
    expect(record.cached_tokens).toBe(800);
    expect(record.cost_microusd).toBeGreaterThan(0);
    expect(record.price_verified).toBe(true);
  });

  test("routed requests pick a model, stick to it, and report affinity from observed usage", async () => {
    const { app, store, upstream } = appWith();
    const body = { model: "nimble", max_tokens: 64, messages: [{ role: "system", content: "x".repeat(4000) }, { role: "user", content: "hi" }], nimble: { session_id: "s", task_type: "feature" } };
    const first = await post(app, "/v1/chat/completions", body);
    const firstFacts = JSON.parse(first.headers.get("x-nimble-routing")!);
    expect(firstFacts.selected_model).toBe("glm-5.3-flash");
    expect(upstream.calls[0]!.url.endsWith("/chat/completions")).toBe(true);
    expect(upstream.calls[0]!.body.max_tokens).toBe(64);
    expect(firstFacts.model_changed).toBe(false);
    expect(firstFacts.decision_id).toBeTruthy();
    const second = await post(app, "/v1/chat/completions", body);
    const secondFacts = JSON.parse(second.headers.get("x-nimble-routing")!);
    expect(secondFacts.selected_model).toBe("glm-5.3-flash");
    expect(secondFacts.routing_reason).toContain("sticky");
    expect(secondFacts.cache_affinity).toBeGreaterThan(0);
    const decisions = store.recentDecisions("s");
    expect(decisions).toHaveLength(2);
    expect((decisions[0]!.components as unknown[]).length).toBe(4);
    const session = store.getSession("s")!;
    expect(session.requests).toBe(2);
    expect(session.switches).toBe(0);
  });

  test("streaming injects facts into the first chunk and records usage at the end", async () => {
    const { app, store } = appWith();
    // Explicit chat-dialect model: exercises the passthrough stream transform.
    const response = await post(app, "/v1/chat/completions", { model: "deepseek-v4-flash", stream: true, messages: [{ role: "user", content: "hi" }] }, { "x-nimble-session-id": "stream1" });
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toContain("text/event-stream");
    const text = await response.text();
    const lines = text.split("\n").filter((l) => l.startsWith("data:"));
    const firstChunk = JSON.parse(lines[0]!.slice(5));
    expect(firstChunk.nimble.selected_model).toBe("deepseek-v4-flash");
    expect(firstChunk.choices[0].delta.content).toBe("hel");
    expect(JSON.parse(lines[1]!.slice(5)).nimble).toBeUndefined();
    expect(lines.at(-1)).toBe("data: [DONE]");
    const record = store.lastRequestForSession("stream1")!;
    expect(record.input_tokens).toBe(1000);
    expect(record.cached_tokens).toBe(800);
    expect(record.output_tokens).toBe(50);
  });

  test("provider failure fails over for routed requests and passes through for explicit ones", async () => {
    // Z.ai (GLM, the first choice under the evidence prior) is down.
    const { app, upstream } = appWith({ fail: { zai: 503 } });
    const routed = await post(app, "/v1/chat/completions", { model: "nimble", messages: [{ role: "user", content: "hi" }] });
    expect(routed.status).toBe(200);
    const facts = JSON.parse(routed.headers.get("x-nimble-routing")!);
    expect(facts.provider).not.toBe("zai");
    expect(facts.routing_reason).toContain("failover");
    expect(upstream.calls.length).toBe(2);
    const explicit = await post(app, "/v1/chat/completions", { model: "glm-5.3-flash", messages: [{ role: "user", content: "hi" }] });
    expect(explicit.status).toBe(503);
    expect((await explicit.json()).error.message).toBe("boom");
  });

  test("unknown models, bad bodies, and missing keys are OpenAI-shaped errors", async () => {
    const { app } = appWith();
    const unknown = await post(app, "/v1/chat/completions", { model: "gpt-9", messages: [{ role: "user", content: "hi" }] });
    expect(unknown.status).toBe(404);
    expect((await unknown.json()).error.code).toBe("model_not_found");
    const empty = await post(app, "/v1/chat/completions", { model: "nimble", messages: [] });
    expect(empty.status).toBe(400);
    delete process.env.ZAI_API_KEY;
    const notConfigured = await post(app, "/v1/chat/completions", { model: "glm-5.3-flash", messages: [{ role: "user", content: "hi" }] });
    expect(notConfigured.status).toBe(503);
    process.env.ZAI_API_KEY = "sk-zai";
  });

  test("bearer auth gates everything but /health", async () => {
    const { app } = appWith({ apiKey: "secret" });
    expect((await app.fetch(new Request("http://n/health"))).status).toBe(200);
    expect((await app.fetch(new Request("http://n/v1/models"))).status).toBe(401);
    expect((await app.fetch(new Request("http://n/v1/models", { headers: { authorization: "Bearer secret" } }))).status).toBe(200);
  });
});

describe("feedback and stats", () => {
  test("feedback joins to the session's last decision and updates stats idempotently", async () => {
    const { app, store } = appWith();
    await post(app, "/v1/chat/completions", { model: "nimble", messages: [{ role: "user", content: "hi" }], nimble: { session_id: "fb", task_type: "debugging" } });
    const first = await post(app, "/v1/feedback", { version: "nimble/v1", session_id: "fb", task_success: true, verification_result: "passed", attempt_count: 2, cost_microusd: 4200 });
    expect(first.status).toBe(202);
    const firstBody = (await first.json()) as any;
    expect(firstBody.joined_model).toBe("glm-5.3-flash");
    expect(firstBody.cost_source).toBe("client");
    const dup = await post(app, "/v1/feedback", { session_id: "fb", task_success: true });
    expect(dup.status).toBe(200);
    const dupBody = (await dup.json()) as any;
    expect(dupBody.duplicate).toBe(true);
    expect(dupBody.outcome).toBe("duplicate");
    const stats = store.stats("glm-5.3-flash", "debugging")!;
    expect(stats.n).toBe(1);
    expect(stats.successes).toBe(1);
    expect(stats.cost_sum).toBe(4200);
    expect(stats.output_samples).toBe(1);
    const view = await (await app.fetch(new Request("http://n/v1/stats?task_type=debugging"))).json();
    expect(view.stats[0].success_rate).toBe(1);
    const bad = await post(app, "/v1/feedback", { task_success: true });
    expect(bad.status).toBe(400);
  });

  test("a verifier's verdict upgrades a client's null outcome once, and only once", async () => {
    const { app, store } = appWith();
    await post(app, "/v1/chat/completions", { model: "nimble", messages: [{ role: "user", content: "hi" }], nimble: { session_id: "vf", task_type: "feature" } });
    // The client had no oracle: null outcome, nothing learned.
    const client = await post(app, "/v1/feedback", { version: "nimble/v1", session_id: "vf", task_success: null, verification_result: "not_run", attempt_count: 2 });
    expect(client.status).toBe(202);
    expect(((await client.json()) as any).outcome).toBe("recorded");
    expect(store.stats("glm-5.3-flash", "feature")?.n ?? 0).toBe(0);
    // The benchmark verifier reports the real verdict for the same session.
    const verifier = await post(app, "/v1/feedback", { version: "nimble/v1", session_id: "vf", task_success: false, verification_result: "failed", attempt_count: 2 });
    expect(verifier.status).toBe(202);
    expect(((await verifier.json()) as any).outcome).toBe("upgraded");
    const stats = store.stats("glm-5.3-flash", "feature")!;
    expect(stats.n).toBe(1);
    expect(stats.successes).toBe(0);
    // A second verdict does not double count.
    const again = await post(app, "/v1/feedback", { version: "nimble/v1", session_id: "vf", task_success: true, verification_result: "passed" });
    expect(again.status).toBe(200);
    expect(store.stats("glm-5.3-flash", "feature")!.n).toBe(1);
  });

  test("OpenAI upstream speaks the Responses API and comes back as chat chunks with replayable reasoning", async () => {
    const { app, store, upstream } = appWith();
    const tools = [{ type: "function", function: { name: "read_file", description: "Read", parameters: { type: "object", properties: { path: { type: "string" } } } } }];
    const priorReplay = [{ type: "reasoning", id: "rs_0", encrypted_content: "OLD", summary: [] }];
    const response = await post(app, "/v1/chat/completions", {
      model: "gpt-5.6-luna",
      stream: true,
      max_tokens: 512,
      reasoning_effort: "high",
      tools,
      messages: [
        { role: "system", content: "sys" },
        { role: "user", content: "read it" },
        { role: "assistant", content: null, nimble_replay: priorReplay, tool_calls: [{ id: "call_0", type: "function", function: { name: "read_file", arguments: "{}" } }] },
        { role: "tool", tool_call_id: "call_0", content: "old output" },
      ],
      nimble: { session_id: "resp1" },
    });
    expect(response.status).toBe(200);
    // Upstream request was translated.
    const sent = upstream.calls.at(-1)!;
    expect(sent.url).toBe("https://api.openai.com/v1/responses");
    expect(sent.body.instructions).toBe("sys");
    expect(sent.body.reasoning).toEqual({ summary: "auto", effort: "high" });
    expect(sent.body.max_output_tokens).toBe(512);
    expect(sent.body.store).toBe(false);
    expect(sent.body.include).toEqual(["reasoning.encrypted_content"]);
    expect(sent.body.tools[0]).toMatchObject({ type: "function", name: "read_file", strict: false });
    const kinds = sent.body.input.map((i: any) => i.type ?? i.role);
    expect(kinds).toEqual(["user", "reasoning", "function_call", "function_call_output"]);
    expect(sent.body.input[1].encrypted_content).toBe("OLD");
    // Response came back as Chat Completions chunks.
    const text = await response.text();
    const chunks = text.split("\n").filter((l) => l.startsWith("data:") && !l.includes("[DONE]")).map((l) => JSON.parse(l.slice(5)));
    expect(chunks[0]!.nimble.selected_model).toBe("gpt-5.6-luna");
    expect(chunks[0]!.choices[0].delta.reasoning_content).toBe("hmm");
    const toolStart = chunks.find((c) => c.choices[0]?.delta?.tool_calls?.[0]?.id === "call_1")!;
    expect(toolStart.choices[0].delta.tool_calls[0].function.name).toBe("read_file");
    const args = chunks.flatMap((c) => c.choices[0]?.delta?.tool_calls ?? []).map((t: any) => t.function?.arguments ?? "").join("");
    expect(args).toBe('{"path":"a.rs"}');
    const final = chunks.find((c) => c.choices[0]?.finish_reason)!;
    expect(final.choices[0].finish_reason).toBe("tool_calls");
    expect(final.choices[0].delta.nimble_replay[0].encrypted_content).toBe("ENC");
    const usageChunk = chunks.find((c) => c.usage)!;
    expect(usageChunk.usage.prompt_tokens_details.cached_tokens).toBe(100);
    expect(text.trim().endsWith("data: [DONE]")).toBe(true);
    const record = store.lastRequestForSession("resp1")!;
    expect(record.cached_tokens).toBe(100);
    expect(record.output_tokens).toBe(50);
    // Non-streaming translation too.
    const plain = await post(app, "/v1/chat/completions", { model: "gpt-5.6-luna", messages: [{ role: "user", content: "hi" }] });
    const completion = (await plain.json()) as any;
    expect(completion.object).toBe("chat.completion");
    expect(completion.choices[0].message.content).toBe("hello");
    expect(completion.choices[0].message.reasoning_content).toBe("hmm");
    expect(completion.choices[0].message.nimble_replay[0].encrypted_content).toBe("ENC");
  });

  test("feedback without a client cost uses Nimble's own priced requests", async () => {
    const { app } = appWith();
    await post(app, "/v1/chat/completions", { model: "nimble", messages: [{ role: "user", content: "hi" }], nimble: { session_id: "fb2", task_type: "ops" } });
    const reply = (await (await post(app, "/v1/feedback", { session_id: "fb2", task_success: false, failure_type: "test_failure", attempt_count: 3 })).json()) as any;
    expect(reply.cost_source).toBe("nimble");
    expect(reply.cost_microusd).toBeGreaterThan(0);
  });

  test("models endpoint lists the virtual model first", async () => {
    const { app } = appWith();
    const list = await (await app.fetch(new Request("http://n/v1/models"))).json();
    expect(list.data[0].id).toBe("nimble");
    expect(list.data.map((m: any) => m.id)).toContain("glm-5.3-flash");
  });
});

// Fake OpenAI-compatible upstream for the Butterfly → Nimble loopback smoke test.
// Answers every chat completion with a streamed "ok" and DeepSeek-style usage.
const seen: any[] = [];
Bun.serve({
  hostname: "127.0.0.1",
  port: 8093,
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === "/seen") return Response.json(seen);
    const body: any = await req.json();
    if (url.pathname.endsWith("/responses")) {
      seen.push({ model: body.model, has_nimble: "nimble" in body, api: "responses", items: body.input.length, stream: body.stream, auth: req.headers.get("authorization") });
      const ev = (e: unknown) => `data: ${JSON.stringify(e)}\n\n`;
      const usage = { input_tokens: 900, input_tokens_details: { cached_tokens: 600 }, output_tokens: 3 };
      const events = [
        ev({ type: "response.output_item.added", output_index: 0, item: { type: "reasoning", id: "rs_1", summary: [] } }),
        ev({ type: "response.output_item.done", output_index: 0, item: { type: "reasoning", id: "rs_1", encrypted_content: "ENC", summary: [] } }),
        ev({ type: "response.output_text.delta", delta: "ok" }),
        ev({ type: "response.completed", response: { id: "resp_1", status: "completed", usage } }),
      ];
      return new Response(events.join(""), { headers: { "content-type": "text/event-stream" } });
    }
    if (!url.pathname.endsWith("/chat/completions")) return new Response("nope", { status: 404 });
    seen.push({ model: body.model, has_nimble: "nimble" in body, messages: body.messages.length, stream: body.stream, auth: req.headers.get("authorization") });
    const usage = { prompt_tokens: 900, prompt_cache_hit_tokens: 600, prompt_cache_miss_tokens: 300, completion_tokens: 3 };
    const mk = (delta: any, extra: any = {}) => `data: ${JSON.stringify({ id: "c1", object: "chat.completion.chunk", model: body.model, choices: [{ index: 0, delta, ...extra }] })}\n\n`;
    const chunks = [mk({ role: "assistant", content: "o" }), mk({ content: "k" }, { finish_reason: "stop" }), `data: ${JSON.stringify({ id: "c1", object: "chat.completion.chunk", model: body.model, choices: [], usage })}\n\n`, "data: [DONE]\n\n"];
    return new Response(chunks.join(""), { headers: { "content-type": "text/event-stream" } });
  },
});
console.log("fake upstream on 8093");

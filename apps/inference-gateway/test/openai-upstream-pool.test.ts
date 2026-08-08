import { describe, expect, test } from "bun:test";
import { OpenAiUpstreamPool, type OpenAiUpstreamRequest } from "../src/upstreams/adapters/openai-pool";

const request: OpenAiUpstreamRequest = {
  path: "/v1/chat/completions",
  body: {
    model: "public-model",
    messages: [{ role: "user", content: "hello" }],
    stream: true,
    reasoning_effort: "medium",
  },
  upstreamModel: "gpt-upstream",
  outputLimit: 321,
};

function successPayload() {
  return {
    id: "chatcmpl-test",
    choices: [{ message: { role: "assistant", content: "hello" } }],
    usage: { prompt_tokens: 10, completion_tokens: 2 },
  };
}

function environment(providers: unknown, extra: Record<string, string> = {}): Record<string, string> {
  return {
    OPENAI_UPSTREAMS_JSON: JSON.stringify(providers),
    UPSTREAM_FAILURE_COOLDOWN_MS: "1000",
    UPSTREAM_REQUEST_TIMEOUT_MS: "10000",
    ...extra,
  };
}

function authorization(init?: RequestInit): string | null {
  return new Headers(init?.headers).get("authorization");
}

describe("OpenAI-compatible upstream pool", () => {
  test("round-robins keys and accepts base URLs with or without /v1", async () => {
    const calls: Array<{ url: string; authorization: string | null; body: Record<string, unknown> }> = [];
    const pool = OpenAiUpstreamPool.fromEnvironment(
      environment([
        { id: "first", base_url: "https://one.example/v1", api_key: "key-one" },
        { id: "second", base_url: "https://two.example", api_key: "key-two" },
      ]),
      async (input, init) => {
        calls.push({
          url: String(input),
          authorization: authorization(init),
          body: JSON.parse(String(init?.body)) as Record<string, unknown>,
        });
        return Response.json(successPayload());
      },
    );

    await pool.execute(request);
    await pool.execute(request);

    expect(calls.map((call) => call.url)).toEqual([
      "https://one.example/v1/chat/completions",
      "https://two.example/v1/chat/completions",
    ]);
    expect(calls.map((call) => call.authorization)).toEqual(["Bearer key-one", "Bearer key-two"]);
    expect(calls[0]!.body).toMatchObject({
      model: "gpt-upstream",
      stream: false,
      max_completion_tokens: 321,
      reasoning_effort: "medium",
    });
  });

  test("passes the customer's Responses API reasoning object through untouched", async () => {
    let forwarded: Record<string, unknown> | undefined;
    const pool = OpenAiUpstreamPool.fromEnvironment(
      environment([{ id: "provider", base_url: "https://pool.example/v1", api_key: "key" }]),
      async (_input, init) => {
        forwarded = JSON.parse(String(init?.body)) as Record<string, unknown>;
        return Response.json(successPayload());
      },
    );

    await pool.execute({
      ...request,
      path: "/v1/responses",
      body: { model: "public-model", input: "hello", reasoning: { effort: "high", summary: "auto" } },
    });

    expect(forwarded).toMatchObject({
      model: "gpt-upstream",
      max_output_tokens: 321,
      reasoning: { effort: "high", summary: "auto" },
    });
  });

  test("supports weighted selection", async () => {
    const keys: Array<string | null> = [];
    const pool = OpenAiUpstreamPool.fromEnvironment(
      environment([
        { id: "primary", base_url: "https://pool.example", api_key: "primary-key", weight: 2 },
        { id: "secondary", base_url: "https://pool.example", api_key: "secondary-key", weight: 1 },
      ]),
      async (_input, init) => {
        keys.push(authorization(init));
        return Response.json(successPayload());
      },
    );

    await pool.execute(request);
    await pool.execute(request);
    await pool.execute(request);

    expect(keys).toEqual(["Bearer primary-key", "Bearer primary-key", "Bearer secondary-key"]);
  });

  test("fails over retryable responses and cools down the failed key", async () => {
    let now = 10_000;
    const keys: Array<string | null> = [];
    const pool = OpenAiUpstreamPool.fromEnvironment(
      environment([
        { id: "depleted", base_url: "https://pool.example", api_key: "bad-key" },
        { id: "healthy", base_url: "https://pool.example", api_key: "good-key" },
      ]),
      async (_input, init) => {
        const key = authorization(init);
        keys.push(key);
        return key === "Bearer bad-key"
          ? Response.json({ error: { message: "rate limited" } }, { status: 429 })
          : Response.json(successPayload());
      },
      () => now,
    );

    await pool.execute(request);
    await pool.execute(request);
    now += 1001;
    await pool.execute(request);

    expect(keys).toEqual([
      "Bearer bad-key",
      "Bearer good-key",
      "Bearer good-key",
      "Bearer bad-key",
      "Bearer good-key",
    ]);
  });

  test("uses another key when one key is at its concurrency limit", async () => {
    let releaseFirst!: () => void;
    const firstCanFinish = new Promise<void>((resolve) => { releaseFirst = resolve; });
    const keys: Array<string | null> = [];
    const pool = OpenAiUpstreamPool.fromEnvironment(
      environment([
        { id: "first", base_url: "https://pool.example", api_key: "key-one", max_concurrency: 1 },
        { id: "second", base_url: "https://pool.example", api_key: "key-two", max_concurrency: 1 },
      ]),
      async (_input, init) => {
        const key = authorization(init);
        keys.push(key);
        if (key === "Bearer key-one") await firstCanFinish;
        return Response.json(successPayload());
      },
    );

    const first = pool.execute(request);
    await Promise.resolve();
    const second = pool.execute(request);
    await second;
    releaseFirst();
    await first;

    expect(keys).toEqual(["Bearer key-one", "Bearer key-two"]);
  });

  test("filters providers by upstream model", async () => {
    const keys: Array<string | null> = [];
    const pool = OpenAiUpstreamPool.fromEnvironment(
      environment([
        { id: "other-model", base_url: "https://pool.example", api_key: "wrong-key", models: ["other"] },
        { id: "right-model", base_url: "https://pool.example", api_key: "right-key", models: ["gpt-upstream"] },
      ]),
      async (_input, init) => {
        keys.push(authorization(init));
        return Response.json(successPayload());
      },
    );

    await pool.execute(request);
    expect(keys).toEqual(["Bearer right-key"]);
  });

  test("does not retry a request validation response", async () => {
    let calls = 0;
    const pool = OpenAiUpstreamPool.fromEnvironment(
      environment([
        { id: "first", base_url: "https://one.example", api_key: "key-one" },
        { id: "second", base_url: "https://two.example", api_key: "key-two" },
      ]),
      async () => {
        calls += 1;
        return Response.json({ error: { message: "invalid request" } }, { status: 400 });
      },
    );

    await expect(pool.execute(request)).rejects.toMatchObject({ status: 400 });
    expect(calls).toBe(1);
  });

  test("resolves API keys through named environment variables and preserves legacy configuration", async () => {
    const explicit = OpenAiUpstreamPool.fromEnvironment(environment([
      { id: "vendor", base_url: "https://vendor.example/v1", api_key_env: "VENDOR_API_KEY" },
    ], { VENDOR_API_KEY: "secret" }));
    const legacy = OpenAiUpstreamPool.fromEnvironment({
      UPSTREAM_BASE_URL: "https://legacy.example",
      UPSTREAM_API_KEY: "legacy-secret",
    });

    expect(explicit.status()).toMatchObject({ configured: true, providers: 1, capacity: 4 });
    expect(legacy.status()).toMatchObject({ configured: true, providers: 1, capacity: 4 });
  });

  test("rejects ambiguous or duplicate provider configuration", () => {
    expect(() => OpenAiUpstreamPool.fromEnvironment(environment([
      { id: "same", base_url: "https://one.example", api_key: "one" },
      { id: "same", base_url: "https://two.example", api_key: "two" },
    ]))).toThrow("Duplicate provider id");

    expect(() => OpenAiUpstreamPool.fromEnvironment(environment([
      { id: "provider", base_url: "https://one.example", api_key: "one", api_key_env: "OTHER_KEY" },
    ], { OTHER_KEY: "two" }))).toThrow("only one of api_key or api_key_env");
  });
});

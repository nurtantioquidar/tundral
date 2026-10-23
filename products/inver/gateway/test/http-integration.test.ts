import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const ADMIN_KEY = "integration-admin-secret";
const WEB_KEY = "integration-web-bff-secret";
const integrationEnabled = process.env.RUN_HTTP_INTEGRATION === "true";
const workspace = resolve(import.meta.dir, "..");
const temporaryDirectory = mkdtempSync(resolve(tmpdir(), "inver-http-integration-"));

let upstream: ReturnType<typeof Bun.serve>;
let gateway: ReturnType<typeof Bun.spawn>;
let gatewayUrl: string;

async function waitUntilReady(url: string): Promise<void> {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(`${url}/ready`);
      if (response.ok) return;
    } catch {
      // The child process has not bound its port yet.
    }
    await Bun.sleep(25);
  }
  throw new Error("Disposable gateway did not become ready");
}

function internal(
  path: string,
  key: string,
  init: RequestInit = {},
): Promise<Response> {
  return fetch(`${gatewayUrl}${path}`, {
    ...init,
    headers: {
      authorization: `Bearer ${key}`,
      ...(init.body ? { "content-type": "application/json" } : {}),
      ...init.headers,
    },
  });
}

beforeAll(async () => {
  if (!integrationEnabled) return;
  upstream = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request) {
      const body = await request.json() as {
        model?: string;
        messages?: Array<{ content?: string }>;
      };
      if (body.messages?.some((message) => message.content === "force failure")) {
        return Response.json({ error: { message: "synthetic upstream failure" } }, { status: 500 });
      }
      return Response.json({
        id: "chatcmpl-integration",
        object: "chat.completion",
        model: body.model,
        choices: [{
          index: 0,
          message: { role: "assistant", content: "integration response" },
          finish_reason: "stop",
        }],
        usage: {
          prompt_tokens: 100,
          prompt_tokens_details: { cached_tokens: 20 },
          completion_tokens: 10,
        },
      });
    },
  });

  const portProbe = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch: () => new Response("reserved"),
  });
  const gatewayPort = portProbe.port;
  portProbe.stop(true);
  gatewayUrl = `http://127.0.0.1:${gatewayPort}`;

  gateway = Bun.spawn([process.execPath, "src/server.ts"], {
    cwd: workspace,
    env: {
      ...process.env,
      HOST: "127.0.0.1",
      PORT: String(gatewayPort),
      DATABASE_PATH: resolve(temporaryDirectory, "gateway.db"),
      ADMIN_API_KEY: ADMIN_KEY,
      WEB_BFF_API_KEY: WEB_KEY,
      REQUIRE_SCOPED_INTERNAL_KEYS: "true",
      UPSTREAM_MODE: "openai",
      UPSTREAM_CONFIG_SOURCE: "environment",
      OPENAI_UPSTREAMS_JSON: JSON.stringify([{
        id: "integration-stub",
        base_url: `http://127.0.0.1:${upstream.port}`,
        api_key: "integration-upstream-key",
        max_concurrency: 2,
      }]),
      UPSTREAM_MAX_ATTEMPTS: "1",
      UPSTREAM_REQUEST_TIMEOUT_MS: "5000",
      DEFAULT_MAX_OUTPUT_TOKENS: "32",
      MAX_OUTPUT_TOKENS_PER_REQUEST: "128",
      RESERVATION_TTL_MS: "10000",
      IDEMPOTENCY_IN_PROGRESS_TTL_MS: "15000",
      RECOVERY_INTERVAL_MS: "10000",
    },
    stdout: "ignore",
    stderr: "ignore",
  });
  await waitUntilReady(gatewayUrl);
});

afterAll(async () => {
  if (!integrationEnabled) return;
  gateway?.kill();
  await gateway?.exited;
  upstream?.stop(true);
  rmSync(temporaryDirectory, { recursive: true, force: true });
});

describe("gateway HTTP boundary", () => {
  const integrationTest = integrationEnabled ? test : test.skip;
  integrationTest("registers, credits, settles, replays, scopes, and releases failures", async () => {
    const registration = await internal("/internal/accounts/register", WEB_KEY, {
      method: "POST",
      body: JSON.stringify({
        name: "Integration Customer",
        email: "integration@example.test",
        password: "integration-password-2026",
      }),
    });
    expect(registration.status).toBe(201);
    const account = await registration.json() as {
      customerId: string;
      apiKey: string;
    };

    const forbidden = await internal("/internal/customers", WEB_KEY);
    expect(forbidden.status).toBe(403);
    const visibleCustomer = await internal(
      `/internal/customers/${account.customerId}`,
      WEB_KEY,
    );
    expect(visibleCustomer.status).toBe(200);

    const credit = await internal(
      `/internal/customers/${account.customerId}/credits`,
      ADMIN_KEY,
      {
        method: "POST",
        body: JSON.stringify({
          amount_usd: "1.00",
          kind: "promotional_credit",
          reference: "http-integration-credit",
        }),
      },
    );
    expect(credit.status).toBe(201);

    const generationBody = JSON.stringify({
      model: "gpt-5.6-luna",
      messages: [{ role: "user", content: "hello" }],
      max_tokens: 32,
      reasoning_effort: "low",
    });
    const generate = () => fetch(`${gatewayUrl}/v1/chat/completions`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${account.apiKey}`,
        "content-type": "application/json",
        "idempotency-key": "http-integration-generation",
      },
      body: generationBody,
    });
    const first = await generate();
    expect(first.status).toBe(200);
    const firstPayload = await first.json() as {
      id: string;
      gateway_usage: { cost_usd: string };
    };
    expect(firstPayload.gateway_usage.cost_usd).toMatch(/^\d+\.\d{6}$/);

    const replay = await generate();
    expect(replay.status).toBe(200);
    expect(replay.headers.get("x-idempotent-replay")).toBe("true");
    expect((await replay.json() as { id: string }).id).toBe(firstPayload.id);

    const usage = await fetch(`${gatewayUrl}/v1/usage`, {
      headers: { authorization: `Bearer ${account.apiKey}` },
    });
    const usagePayload = await usage.json() as { data: unknown[] };
    expect(usagePayload.data).toHaveLength(1);

    const failed = await fetch(`${gatewayUrl}/v1/chat/completions`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${account.apiKey}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({
        model: "gpt-5.6-luna",
        messages: [{ role: "user", content: "force failure" }],
        max_tokens: 32,
      }),
    });
    expect(failed.status).toBeGreaterThanOrEqual(500);

    const reconciliation = await internal(
      `/internal/customers/${account.customerId}/reconciliation`,
      WEB_KEY,
    );
    const reconciliationPayload = await reconciliation.json() as {
      activeHoldsMicros: number;
      usageCount: number;
      reconciled: boolean;
    };
    expect(reconciliationPayload).toMatchObject({
      activeHoldsMicros: 0,
      usageCount: 1,
      reconciled: true,
    });
  }, 15_000);

  integrationTest("exposes the capability collection to admin only, through the real router", async () => {
    // Reaches routeUpstreamAdmin only if the internal router's dispatch guard matches the bare
    // collection path — a unit test that calls the handler directly cannot catch that.
    const asAdmin = await internal("/internal/upstream-models", ADMIN_KEY);
    expect(asAdmin.status).toBe(200);
    expect(await asAdmin.json()).toMatchObject({ object: "list" });

    // Provider topology must never be reachable with the scoped web credential.
    const asWebBff = await internal("/internal/upstream-models", WEB_KEY);
    expect(asWebBff.status).toBe(403);

    const unauthenticated = await fetch(`${gatewayUrl}/internal/upstream-models`);
    expect(unauthenticated.status).toBe(401);
  }, 15_000);

  integrationTest("serves the whole rate card to operators, not just the routable subset", async () => {
    const routable = await internal("/internal/pricing", ADMIN_KEY);
    const full = await internal("/internal/rate-card", ADMIN_KEY);
    expect(full.status).toBe(200);
    const routablePayload = await routable.json() as { data: Array<{ id: string }> };
    const fullPayload = await full.json() as { rate_card_version: string; data: Array<{ id: string }> };

    // The operator view must never be narrower than the customer view, and must carry every model the
    // gateway knows how to price — otherwise a model with no route yet is invisible on the screen used
    // to give it one.
    expect(fullPayload.data.length).toBeGreaterThanOrEqual(routablePayload.data.length);
    expect(fullPayload.data.map((model) => model.id)).toEqual(
      expect.arrayContaining(["gpt-5.6-luna", "gpt-5.5"]),
    );
    expect(fullPayload.rate_card_version).toBeTruthy();

    // Customers must not learn about models the gateway cannot currently serve.
    const asWebBff = await internal("/internal/rate-card", WEB_KEY);
    expect(asWebBff.status).toBe(403);
  }, 15_000);
});

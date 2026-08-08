import { afterEach, describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { GatewayDatabase } from "../src/database";

const databases: GatewayDatabase[] = [];
afterEach(() => databases.splice(0).forEach((database) => database.db.close()));

describe("ledger", () => {
  test("credits USDC idempotently and settles a reservation", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("pilot");
    const auth = db.authenticate(customer.apiKey)!;
    db.creditUsdc({ customerId: customer.customerId, chain: "base", txHash: "0xabc", logIndex: 0, amountAtomic: 10_000_000n, confirmations: 10 });
    expect(db.balanceMicros(customer.customerId)).toBe(10_000_000n);
    expect(() => db.creditUsdc({ customerId: customer.customerId, chain: "base", txHash: "0xabc", logIndex: 0, amountAtomic: 10_000_000n, confirmations: 10 })).toThrow();

    const reservationId = db.reserve({ customerId: customer.customerId, apiKeyId: auth.apiKeyId, requestId: "req-1", amountMicros: 1_000_000n });
    expect(db.balanceMicros(customer.customerId)).toBe(9_000_000n);
    db.settle({ reservationId, requestId: "req-1", customerId: customer.customerId, apiKeyId: auth.apiKeyId, model: "gpt-5.6-luna", inputTokens: 1000, cachedInputTokens: 0, cacheWriteTokens: 0, outputTokens: 100, costMicros: 8_000n, upstreamStatus: 200 });
    expect(db.balanceMicros(customer.customerId)).toBe(9_992_000n);
  });

  test("rejects reservations larger than available balance", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("empty");
    const auth = db.authenticate(customer.apiKey)!;
    expect(() => db.reserve({ customerId: customer.customerId, apiKeyId: auth.apiKeyId, requestId: "empty-1", amountMicros: 1n })).toThrow("insufficient_balance");
  });

  test("replays an idempotent request only after atomic settlement", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("idempotent");
    const auth = db.authenticate(customer.apiKey)!;
    db.creditUsdc({ customerId: customer.customerId, chain: "base", txHash: "0xidem", logIndex: 0, amountAtomic: 1_000_000n, confirmations: 10 });

    const first = db.beginIdempotentRequest(auth.apiKeyId, "retry-1", "hash-1");
    expect(first.state).toBe("started");
    if (first.state !== "started") throw new Error("request did not start");
    expect(db.beginIdempotentRequest(auth.apiKeyId, "retry-1", "hash-1").state).toBe("in_progress");
    expect(db.beginIdempotentRequest(auth.apiKeyId, "retry-1", "different").state).toBe("conflict");

    const reservationId = db.reserve({ customerId: customer.customerId, apiKeyId: auth.apiKeyId, requestId: first.requestId, amountMicros: 100_000n });
    const payload = { id: "chatcmpl-replay", choices: [{ message: { content: "done" } }] };
    db.settle({
      reservationId,
      requestId: first.requestId,
      customerId: customer.customerId,
      apiKeyId: auth.apiKeyId,
      model: "gpt-5.6-luna",
      inputTokens: 100,
      cachedInputTokens: 0,
      cacheWriteTokens: 0,
      outputTokens: 10,
      costMicros: 1_000n,
      upstreamStatus: 200,
      idempotencyKey: "retry-1",
      responseJson: JSON.stringify(payload),
      responseStatus: 200,
    });

    const replay = db.beginIdempotentRequest(auth.apiKeyId, "retry-1", "hash-1");
    expect(replay).toEqual({ state: "completed", requestId: first.requestId, payload, status: 200 });
    expect(db.balanceMicros(customer.customerId)).toBe(999_000n);
  });

  test("allows a failed idempotent request to be retried", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("retry");
    const auth = db.authenticate(customer.apiKey)!;
    const first = db.beginIdempotentRequest(auth.apiKeyId, "retry-2", "hash-2");
    expect(first.state).toBe("started");
    db.failIdempotentRequest(auth.apiKeyId, "retry-2");
    const second = db.beginIdempotentRequest(auth.apiKeyId, "retry-2", "hash-2");
    expect(second.state).toBe("started");
    if (first.state === "started" && second.state === "started") expect(second.requestId).not.toBe(first.requestId);
  });

  test("manages customer keys, freezing, and idempotent offline credits", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("friend");
    const credit = db.creditManual({ customerId: customer.customerId, amountMicros: 10_000_000n, kind: "offline_payment", reference: "friend-payment-1", note: "cash" });
    expect(credit.replayed).toBe(false);
    expect(db.creditManual({ customerId: customer.customerId, amountMicros: 10_000_000n, kind: "offline_payment", reference: "friend-payment-1", note: "cash" }).replayed).toBe(true);
    expect(db.balanceMicros(customer.customerId)).toBe(10_000_000n);

    const second = db.createApiKey(customer.customerId, { name: "laptop", maxConcurrency: 1, rateLimitPerMinute: 10 });
    expect(db.authenticate(second.apiKey)?.maxConcurrency).toBe(1);
    expect(db.listApiKeys(customer.customerId)).toHaveLength(2);
    expect(db.revokeApiKey(second.apiKeyId)).toBe(true);
    expect(db.authenticate(second.apiKey)).toBeNull();

    db.updateCustomer(customer.customerId, { active: false });
    expect(db.authenticate(customer.apiKey)).toBeNull();
    db.updateCustomer(customer.customerId, { active: true, dailySpendLimitMicros: 2_000_000n, perRequestLimitMicros: 1_000_000n });
    expect(db.authenticate(customer.apiKey)?.dailySpendLimitMicros).toBe(2_000_000);
  });

  test("registers customer accounts without storing a plaintext password", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const registered = db.registerCustomerAccount(
      "Acme",
      "OWNER@EXAMPLE.COM",
      "$argon2id$test-hash",
    );
    expect(registered.email).toBe("owner@example.com");
    expect(registered.apiKey).toStartWith("sk-cg-");
    expect(db.accountForAuthentication("owner@example.com")).toMatchObject({
      customerId: registered.customerId,
      email: "owner@example.com",
      passwordHash: "$argon2id$test-hash",
      active: 1,
    });
    expect(db.getCustomer(registered.customerId)).toMatchObject({
      email: "owner@example.com",
    });
    expect(() => db.registerCustomerAccount(
      "Duplicate",
      "owner@example.com",
      "$argon2id$another-hash",
    )).toThrow();
  });

  test("enforces and reports a lifetime spending cap per API key", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("key-budget");
    db.creditManual({
      customerId: customer.customerId,
      amountMicros: 10_000_000n,
      kind: "promotional_credit",
      reference: "key-budget-credit",
    });
    const limited = db.createApiKey(customer.customerId, {
      name: "ci",
      spendLimitMicros: 1_000_000n,
    });
    const auth = db.authenticate(limited.apiKey)!;
    expect(auth.keySpendLimitMicros).toBe(1_000_000);
    const reservationId = db.reserve({
      customerId: customer.customerId,
      apiKeyId: auth.apiKeyId,
      requestId: "key-budget-1",
      amountMicros: 800_000n,
      keySpendLimitMicros: auth.keySpendLimitMicros,
    });
    db.settle({
      reservationId,
      requestId: "key-budget-1",
      customerId: customer.customerId,
      apiKeyId: auth.apiKeyId,
      model: "gpt-5.6-luna",
      inputTokens: 1,
      cachedInputTokens: 0,
      cacheWriteTokens: 0,
      outputTokens: 1,
      costMicros: 700_000n,
      upstreamStatus: 200,
    });
    expect(() => db.reserve({
      customerId: customer.customerId,
      apiKeyId: auth.apiKeyId,
      requestId: "key-budget-2",
      amountMicros: 300_001n,
      keySpendLimitMicros: auth.keySpendLimitMicros,
    })).toThrow("key_spend_limit");
    expect(db.listApiKeys(customer.customerId).find(
      (key) => (key as { id: string }).id === limited.apiKeyId,
    )).toMatchObject({
      name: "ci",
      spend_limit_micros: 1_000_000,
      spent_micros: 700_000,
      active_holds_micros: 0,
    });
    db.updateApiKey(customer.customerId, limited.apiKeyId, {
      name: "ci-updated",
      spendLimitMicros: 2_000_000n,
    });
    expect(db.authenticate(limited.apiKey)?.keySpendLimitMicros).toBe(2_000_000);
  });

  test("enforces request and daily spending caps across reservations", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("limited");
    const auth = db.authenticate(customer.apiKey)!;
    db.creditManual({ customerId: customer.customerId, amountMicros: 10_000_000n, kind: "promotional_credit", reference: "limited-credit" });
    expect(() => db.reserve({ customerId: customer.customerId, apiKeyId: auth.apiKeyId, requestId: "too-big", amountMicros: 1_000_001n, perRequestLimitMicros: 1_000_000 })).toThrow("per_request_limit");
    db.reserve({ customerId: customer.customerId, apiKeyId: auth.apiKeyId, requestId: "daily-1", amountMicros: 1_000_000n, dailySpendLimitMicros: 1_500_000 });
    expect(() => db.reserve({ customerId: customer.customerId, apiKeyId: auth.apiKeyId, requestId: "daily-2", amountMicros: 500_001n, dailySpendLimitMicros: 1_500_000 })).toThrow("daily_spend_limit");
  });

  test("recovers expired reservations and unlocks idempotent retries", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("recovery");
    const auth = db.authenticate(customer.apiKey)!;
    db.creditManual({ customerId: customer.customerId, amountMicros: 1_000_000n, kind: "promotional_credit", reference: "recovery-credit" });
    const request = db.beginIdempotentRequest(auth.apiKeyId, "crashed", "hash");
    if (request.state !== "started") throw new Error("request did not start");
    db.reserve({ customerId: customer.customerId, apiKeyId: auth.apiKeyId, requestId: request.requestId, amountMicros: 100_000n, now: 1 });
    expect(db.balanceMicros(customer.customerId)).toBe(900_000n);
    expect(db.recoverStale(1_000_000).releasedReservations).toBe(1);
    expect(db.balanceMicros(customer.customerId)).toBe(1_000_000n);
    expect(db.beginIdempotentRequest(auth.apiKeyId, "crashed", "hash").state).toBe("started");
  });

  test("stores rate-card and balance snapshots and reconciles the ledger", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("reconcile");
    const auth = db.authenticate(customer.apiKey)!;
    db.creditManual({ customerId: customer.customerId, amountMicros: 1_000_000n, kind: "promotional_credit", reference: "reconcile-credit" });
    const reservationId = db.reserve({ customerId: customer.customerId, apiKeyId: auth.apiKeyId, requestId: "rated-1", amountMicros: 100_000n });
    db.settle({
      reservationId,
      requestId: "rated-1",
      customerId: customer.customerId,
      apiKeyId: auth.apiKeyId,
      model: "gpt-5.6-luna",
      rateCardVersion: "test-v1",
      inputTokens: 100,
      cachedInputTokens: 0,
      cacheWriteTokens: 0,
      outputTokens: 10,
      costMicros: 2_000n,
      upstreamStatus: 200,
      routeRevision: 4,
      routeTargetId: "target-private",
    });
    const usage = db.usage(customer.customerId, 1)[0] as Record<string, unknown>;
    expect(usage.rate_card_version).toBe("test-v1");
    expect(usage.balance_before_micros).toBe(1_000_000);
    expect(usage.balance_after_micros).toBe(998_000);
    expect(usage).not.toHaveProperty("route_target_id");
    expect(db.db.query(`
      SELECT route_revision AS routeRevision, route_target_id AS routeTargetId
      FROM usage_events WHERE request_id='rated-1'
    `).get()).toEqual({ routeRevision: 4, routeTargetId: "target-private" });
    expect(db.reconcile(customer.customerId)).toMatchObject({ reconciled: true, usageChargesMicros: 2_000, availableBalanceMicros: 998_000 });
  });

  test("stores normalized providers, credentials, capabilities, and atomic route revisions", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const provider = db.createRoutingProvider({
      id: "provider-normalized",
      name: "Normalized",
      transport: "openai_http",
      baseUrl: "https://normalized.example/v1",
      enabled: true,
      maxConcurrency: 8,
    });
    const credential = db.createUpstreamCredential({
      id: "credential-normalized",
      providerId: provider.id,
      name: "primary",
      credentialCiphertext: "ciphertext",
      encryptionContext: "credential-normalized",
      enabled: true,
      weight: 2,
      maxConcurrency: 3,
      models: ["gpt-5.5"],
    });
    const capability = db.createProviderModelCapability({
      id: "capability-normalized",
      providerId: provider.id,
      canonicalModelId: "gpt-5.5",
      upstreamModelId: "vendor-gpt-5.5",
      apiPaths: ["chat_completions"],
      enabled: true,
    });
    const first = db.publishModelRoute({
      publicModelId: "gpt-5.5",
      enabled: true,
      expectedRevision: 0,
      targets: [{
        capabilityId: capability.id,
        priority: 2,
        weight: 4,
        maxConcurrency: 5,
        enabled: true,
      }],
    });
    const second = db.publishModelRoute({
      publicModelId: "gpt-5.5",
      enabled: true,
      expectedRevision: 1,
      targets: [{
        capabilityId: capability.id,
        priority: 1,
        weight: 1,
        maxConcurrency: 5,
        enabled: true,
      }],
    });

    expect(credential).not.toHaveProperty("credentialCiphertext");
    expect(db.loadUpstreamCredentials()[0]).toMatchObject({
      credentialCiphertext: "ciphertext",
      encryptionContext: "credential-normalized",
    });
    expect(first).toMatchObject({ revision: 1, targets: [{ priority: 2, weight: 4 }] });
    expect(second).toMatchObject({ revision: 2, targets: [{ priority: 1, weight: 1 }] });
    expect(() => db.publishModelRoute({
      publicModelId: "gpt-5.5",
      enabled: false,
      expectedRevision: 1,
      targets: [],
    })).toThrow("model_route_revision_conflict");

    expect(db.deleteRoutingProvider(provider.id, 1)).toBe(true);
    expect(db.loadUpstreamCredentials()).toEqual([]);
    expect(db.listProviderModelCapabilities()).toEqual([]);
    expect(db.getModelRoute("gpt-5.5")?.targets).toEqual([]);
  });

  test("manages encrypted upstream provider metadata without exposing credentials", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const created = db.createUpstreamProvider({
      id: "provider-1",
      name: "Vendor One",
      baseUrl: "https://vendor.example/v1",
      credentialCiphertext: "v1.iv.tag.ciphertext",
      enabled: false,
      weight: 2,
      maxConcurrency: 3,
      models: ["gpt-5.5"],
    });
    expect(created).toMatchObject({
      id: "provider-1",
      enabled: false,
      revision: 1,
      credentialVersion: 1,
      hasApiKey: true,
    });
    expect(created).not.toHaveProperty("credentialCiphertext");
    expect(db.loadUpstreamProviders()[0]).toMatchObject({
      credentialCiphertext: "v1.iv.tag.ciphertext",
      models: ["gpt-5.5"],
    });

    const updated = db.updateUpstreamProvider("provider-1", {
      enabled: true,
      weight: 4,
      expectedRevision: 1,
    });
    expect(updated).toMatchObject({ enabled: true, weight: 4, revision: 2 });
    expect(() => db.updateUpstreamProvider("provider-1", {
      enabled: false,
      expectedRevision: 1,
    })).toThrow("upstream_provider_revision_conflict");

    const rotated = db.rotateUpstreamProviderKey("provider-1", "v1.new.tag.ciphertext", 2);
    expect(rotated).toMatchObject({ revision: 3, credentialVersion: 2 });
    expect(db.loadUpstreamProviders()[0]?.credentialCiphertext).toBe("v1.new.tag.ciphertext");
    expect(db.listUpstreamProviders()[0]).not.toHaveProperty("credentialCiphertext");

    expect(db.deleteUpstreamProvider("provider-1", 3)).toBe(true);
    expect(db.listUpstreamProviders()).toEqual([]);
    const actions = db.auditEvents(10).map((event) => (event as Record<string, unknown>).action);
    expect(actions).toContain("upstream_provider.create");
    expect(actions).toContain("upstream_provider.update");
    expect(actions).toContain("upstream_provider.rotate_key");
    expect(actions).toContain("upstream_provider.delete");
  });
});

describe("usage summary", () => {
  test("aggregates usage events by day and model inside the window", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const customer = db.createCustomer("summary");
    const auth = db.authenticate(customer.apiKey)!;
    db.creditManual({ customerId: customer.customerId, amountMicros: 10_000_000n, kind: "promotional_credit", reference: "summary-credit-1" });

    for (const [requestId, model, cost] of [["sum-1", "gpt-5.6-luna", 5_000n], ["sum-2", "gpt-5.5", 20_000n]] as const) {
      const reservationId = db.reserve({ customerId: customer.customerId, apiKeyId: auth.apiKeyId, requestId, amountMicros: cost });
      db.settle({
        reservationId, requestId, customerId: customer.customerId, apiKeyId: auth.apiKeyId,
        model, inputTokens: 1_000, cachedInputTokens: 200, cacheWriteTokens: 0, outputTokens: 300,
        costMicros: cost, upstreamStatus: 200,
      });
    }

    const summary = db.usageSummary(30) as {
      days: number;
      daily: Array<{ requests: number; input_tokens: number; output_tokens: number; cost_micros: number }>;
      models: Array<{ model: string; cost_micros: number }>;
      totals: { requests: number; input_tokens: number; output_tokens: number; cost_micros: number };
    };
    expect(summary.days).toBe(30);
    expect(summary.totals).toMatchObject({ requests: 2, input_tokens: 2_000, output_tokens: 600, cost_micros: 25_000 });
    expect(summary.daily).toHaveLength(1);
    expect(summary.daily[0]).toMatchObject({ requests: 2, cost_micros: 25_000 });
    expect(summary.models.map((row) => row.model)).toEqual(["gpt-5.5", "gpt-5.6-luna"]);
    expect(db.usageSummary(0).days).toBe(1);
    expect(db.usageSummary(500).days).toBe(90);
  });
});

describe("provider schema migration", () => {
  test("normalizes legacy embedded credentials and initializes matching routes", () => {
    const directory = mkdtempSync(join(tmpdir(), "monet-provider-migration-"));
    const path = join(directory, "gateway.db");
    try {
      const legacy = new Database(path, { create: true });
      legacy.run(`
        CREATE TABLE upstream_providers (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL COLLATE NOCASE UNIQUE,
          base_url TEXT NOT NULL,
          credential_ciphertext TEXT NOT NULL,
          enabled INTEGER NOT NULL DEFAULT 0,
          weight INTEGER NOT NULL DEFAULT 1,
          max_concurrency INTEGER NOT NULL DEFAULT 4,
          models_json TEXT,
          revision INTEGER NOT NULL DEFAULT 1,
          credential_version INTEGER NOT NULL DEFAULT 1,
          credential_updated_at INTEGER NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        )
      `);
      legacy.run(`
        INSERT INTO upstream_providers VALUES
          ('legacy-provider', 'Legacy', 'https://legacy.example/v1', 'encrypted-key',
           1, 3, 2, '["gpt-5.5"]', 1, 4, 10, 10, 10)
      `);
      legacy.close();

      const migrated = new GatewayDatabase(path);
      expect(migrated.loadUpstreamCredentials("legacy-provider")[0]).toMatchObject({
        name: "primary",
        credentialCiphertext: "encrypted-key",
        encryptionContext: "legacy-provider",
        weight: 3,
        credentialVersion: 4,
      });
      expect(migrated.listProviderModelCapabilities("legacy-provider")).toMatchObject([{
        canonicalModelId: "gpt-5.5",
        upstreamModelId: "gpt-5.5",
      }]);
      expect(migrated.getModelRoute("gpt-5.5")).toMatchObject({
        revision: 1,
        targets: [{ priority: 1, weight: 3 }],
      });
      migrated.db.close();
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

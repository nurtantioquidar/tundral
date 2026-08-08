import { afterEach, describe, expect, test } from "bun:test";
import { CredentialCipher } from "../src/upstreams/credential-cipher";
import { GatewayDatabase } from "../src/database";
import { routeUpstreamAdmin } from "../src/http/upstream-admin";
import { UpstreamRegistry } from "../src/upstreams/upstream-registry";

const databases: GatewayDatabase[] = [];
const key = Buffer.alloc(32, 3).toString("base64");
const environment = {
  UPSTREAM_MAX_CONCURRENCY_PER_PROVIDER: "4",
  UPSTREAM_MAX_ATTEMPTS: "3",
  UPSTREAM_FAILURE_COOLDOWN_MS: "1000",
  UPSTREAM_REQUEST_TIMEOUT_MS: "10000",
};

afterEach(() => databases.splice(0).forEach((database) => database.db.close()));

function databaseRegistry(extra: Record<string, string> = {}) {
  const db = new GatewayDatabase(":memory:");
  databases.push(db);
  const cipher = CredentialCipher.fromBase64(key);
  return {
    db,
    registry: new UpstreamRegistry(db, cipher, "database", { ...environment, ...extra }),
  };
}

describe("database-backed upstream registry", () => {
  test("publishes an immutable model route across provider lifecycle changes", () => {
    const { db, registry } = databaseRegistry();
    expect(registry.status()).toMatchObject({ configured: false, routes: 0 });

    const provider = registry.createProvider({
      name: "Vendor One",
      transport: "openai_http",
      baseUrl: "https://vendor.example/v1",
      enabled: false,
      maxConcurrency: 2,
    }).provider;
    const credential = registry.createCredential(provider.id, {
      name: "primary",
      apiKey: "provider-secret",
      enabled: true,
      weight: 1,
      maxConcurrency: 2,
      models: ["gpt-5.5"],
    }).credential;
    const capability = registry.createCapability(provider.id, {
      canonicalModelId: "gpt-5.5",
      upstreamModelId: "vendor-gpt-5.5",
      apiPaths: ["chat_completions", "responses"],
      enabled: true,
    }).capability;
    const published = registry.publishRoute({
      publicModelId: "gpt-5.5",
      enabled: true,
      expectedRevision: 0,
      targets: [{
        capabilityId: capability.id,
        priority: 1,
        weight: 1,
        maxConcurrency: 2,
        enabled: true,
      }],
    });

    expect(published.route.revision).toBe(1);
    expect(registry.configured).toBe(false);
    expect(db.getUpstreamCredential(credential.id)?.credentialCiphertext).not.toContain("provider-secret");

    const enabled = registry.updateProvider(provider.id, { enabled: true, expectedRevision: 1 });
    expect(enabled.provider.revision).toBe(2);
    expect(registry.status()).toMatchObject({ configured: true, routes: 1, targets: 1 });

    const rotated = registry.rotateCredential(credential.id, "replacement-secret", 1);
    expect(rotated.credential).toMatchObject({ revision: 2, credentialVersion: 2 });
    expect(registry.configured).toBe(true);

    expect(registry.deleteProvider(provider.id, 2)).toBe(true);
    expect(registry.status()).toMatchObject({ configured: false, routes: 0 });
    expect(registry.getRoute("gpt-5.5")?.targets).toEqual([]);
  });

  test("discovers provider models with a stored credential", async () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const fetched: Array<{ url: string; authorization: string | null }> = [];
    const registry = new UpstreamRegistry(
      db,
      CredentialCipher.fromBase64(key),
      "database",
      environment,
      async (input, init) => {
        fetched.push({ url: String(input), authorization: new Headers(init?.headers).get("authorization") });
        return Response.json({ data: [{ id: "vendor-b" }, { id: "vendor-a" }, { id: "vendor-a" }, { bogus: true }] });
      },
    );
    const provider = registry.createProvider({
      name: "Vendor",
      transport: "openai_http",
      baseUrl: "https://vendor.example/v1",
      enabled: true,
      maxConcurrency: 2,
    }).provider;

    // No stored key yet — discovery has nothing to authenticate with.
    await expect(registry.discoverProviderModels(provider.id)).rejects.toThrow("Store an API key first");

    registry.createCredential(provider.id, {
      name: "primary",
      apiKey: "provider-secret",
      enabled: true,
      weight: 1,
      maxConcurrency: 2,
      models: null,
    });
    await expect(registry.discoverProviderModels(provider.id)).resolves.toEqual(["vendor-a", "vendor-b"]);
    expect(fetched).toEqual([{ url: "https://vendor.example/v1/models", authorization: "Bearer provider-secret" }]);

    const codex = registry.createProvider({
      name: "Codex",
      transport: "codex_cli",
      baseUrl: null,
      enabled: true,
      maxConcurrency: 1,
    }).provider;
    await expect(registry.discoverProviderModels(codex.id)).rejects.toThrow("OpenAI-compatible HTTP provider");
    await expect(registry.discoverProviderModels("missing")).rejects.toThrow("Upstream provider not found");
  });

  test("keeps the environment pool authoritative while staging database configuration", () => {
    const db = new GatewayDatabase(":memory:");
    databases.push(db);
    const registry = new UpstreamRegistry(db, CredentialCipher.fromBase64(key), "environment", {
      ...environment,
      UPSTREAM_API_KEY: "bootstrap-key",
      UPSTREAM_BASE_URL: "https://bootstrap.example",
    });
    expect(registry.status()).toMatchObject({ configured: true, providers: 1 });

    const created = registry.createProvider({
      name: "Staged",
      transport: "openai_http",
      baseUrl: "https://staged.example",
      enabled: true,
      maxConcurrency: 5,
    });
    expect(created.applied).toBe(false);
    expect(registry.status()).toMatchObject({ configured: true, providers: 1, capacity: 4 });
    expect(db.listRoutingProviders()).toHaveLength(1);
  });

  test("validates provider destinations before persistence", () => {
    const { db, registry } = databaseRegistry();
    expect(() => registry.createProvider({
      name: "Broken",
      transport: "openai_http",
      baseUrl: "file:///tmp/provider",
      enabled: true,
      maxConcurrency: 1,
    })).toThrow("must use https");
    expect(db.listRoutingProviders()).toEqual([]);

    expect(() => registry.createProvider({
      name: "Local",
      transport: "openai_http",
      baseUrl: "http://127.0.0.1:9000/v1",
      enabled: false,
      maxConcurrency: 1,
    })).toThrow("must use https");

    const local = databaseRegistry({
      ALLOW_INSECURE_UPSTREAM_HTTP: "true",
      ALLOW_PRIVATE_UPSTREAM_NETWORKS: "true",
    });
    expect(local.registry.createProvider({
      name: "Local",
      transport: "openai_http",
      baseUrl: "http://127.0.0.1:9000/v1",
      enabled: false,
      maxConcurrency: 1,
    }).provider.name).toBe("Local");
  });

  test("rejects semantic model fallback before publishing", () => {
    const { registry } = databaseRegistry();
    const provider = registry.createProvider({
      name: "Different Model",
      transport: "openai_http",
      baseUrl: "https://vendor.example",
      enabled: false,
      maxConcurrency: 1,
    }).provider;
    const capability = registry.createCapability(provider.id, {
      canonicalModelId: "deepseek-v4-pro",
      upstreamModelId: "deepseek-v4-pro",
      apiPaths: ["chat_completions"],
      enabled: true,
    }).capability;
    expect(() => registry.publishRoute({
      publicModelId: "gpt-5.5",
      enabled: true,
      expectedRevision: 0,
      targets: [{
        capabilityId: capability.id,
        priority: 1,
        weight: 1,
        maxConcurrency: 1,
        enabled: true,
      }],
    })).toThrow("requires gpt-5.5");
    expect(registry.getRoute("gpt-5.5")).toBeNull();
  });

  test("serves the split admin flow without returning credentials", async () => {
    const { registry } = databaseRegistry();
    const providerResponse = await routeUpstreamAdmin(
      new Request("http://gateway/internal/upstreams", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          name: "Vendor API",
          transport: "openai_http",
          base_url: "https://vendor.example/v1",
          enabled: false,
        }),
      }),
      new URL("http://gateway/internal/upstreams"),
      registry,
    );
    expect(providerResponse?.status).toBe(201);
    const provider = await providerResponse!.json() as { id: string; revision: number };

    const credentialUrl = `http://gateway/internal/upstreams/${provider.id}/credentials`;
    const credentialResponse = await routeUpstreamAdmin(
      new Request(credentialUrl, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ name: "primary", api_key: "never-return-this" }),
      }),
      new URL(credentialUrl),
      registry,
    );
    expect(credentialResponse?.status).toBe(201);
    const credentialText = await credentialResponse!.text();
    expect(credentialText).not.toContain("never-return-this");
    expect(credentialText).not.toContain("credentialCiphertext");

    const modelUrl = `http://gateway/internal/upstreams/${provider.id}/models`;
    const modelResponse = await routeUpstreamAdmin(
      new Request(modelUrl, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          canonical_model_id: "gpt-5.5",
          upstream_model_id: "vendor-gpt-5.5",
          api_paths: ["chat_completions"],
        }),
      }),
      new URL(modelUrl),
      registry,
    );
    const capability = await modelResponse!.json() as { id: string };

    const routeUrl = "http://gateway/internal/model-routes/gpt-5.5";
    const routeResponse = await routeUpstreamAdmin(
      new Request(routeUrl, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          revision: 0,
          targets: [{ capability_id: capability.id, priority: 1 }],
        }),
      }),
      new URL(routeUrl),
      registry,
    );
    expect(routeResponse?.status).toBe(200);
    expect(await routeResponse!.json()).toMatchObject({
      public_model_id: "gpt-5.5",
      revision: 1,
      targets: [{ capability_id: capability.id, priority: 1 }],
    });

    const staleResponse = await routeUpstreamAdmin(
      new Request(routeUrl, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          revision: 0,
          targets: [{ capability_id: capability.id, priority: 1 }],
        }),
      }),
      new URL(routeUrl),
      registry,
    );
    expect(staleResponse?.status).toBe(409);
  });

  test("lists every capability across providers in one read", async () => {
    const { registry } = databaseRegistry();
    const made: string[] = [];
    for (const [index, canonical] of ["gpt-5.5", "gpt-5.6-luna"].entries()) {
      const provider = registry.createProvider({
        name: `Vendor ${index}`,
        transport: "openai_http",
        baseUrl: `https://vendor-${index}.example/v1`,
        enabled: true,
        maxConcurrency: 2,
      }).provider;
      made.push(registry.createCapability(provider.id, {
        canonicalModelId: canonical,
        upstreamModelId: `vendor-${canonical}`,
        apiPaths: ["chat_completions"],
        enabled: true,
      }).capability.id);
    }

    const response = await routeUpstreamAdmin(
      new Request("http://gateway/internal/upstream-models"),
      new URL("http://gateway/internal/upstream-models"),
      registry,
    );
    expect(response?.status).toBe(200);
    const body = await response!.json() as { object: string; data: Array<{ id: string; provider_id: string }> };
    expect(body.object).toBe("list");
    expect(body.data.map((capability) => capability.id).sort()).toEqual([...made].sort());
    // Capabilities span providers, so the collection is not scoped to one of them.
    expect(new Set(body.data.map((capability) => capability.provider_id)).size).toBe(2);
  });

  test("does not confuse the capability collection with a single capability", async () => {
    const { registry } = databaseRegistry();
    const missing = await routeUpstreamAdmin(
      new Request("http://gateway/internal/upstream-models/does-not-exist"),
      new URL("http://gateway/internal/upstream-models/does-not-exist"),
      registry,
    );
    // The single-capability path still resolves independently of the new collection route.
    expect(missing?.status).not.toBe(200);
  });
});

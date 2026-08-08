import { describe, expect, test } from "bun:test";
import type {
  ModelRoute,
  ProviderModelCapability,
  RoutingProvider,
  StoredUpstreamCredential,
} from "../src/database";
import { ModelRouter, type CodexExecutor } from "../src/upstreams/model-router";

const environment = {
  UPSTREAM_FAILURE_COOLDOWN_MS: "1000",
  UPSTREAM_REQUEST_TIMEOUT_MS: "10000",
  UPSTREAM_MAX_ATTEMPTS: "3",
};

function provider(id: string, transport: RoutingProvider["transport"] = "openai_http"): RoutingProvider {
  return {
    id,
    name: id,
    transport,
    baseUrl: transport === "openai_http" ? `https://${id}.example/v1` : null,
    enabled: true,
    maxConcurrency: 10,
    revision: 1,
    createdAt: 1,
    updatedAt: 1,
  };
}

function credential(providerId: string): StoredUpstreamCredential {
  return {
    id: `key-${providerId}`,
    providerId,
    name: "primary",
    credentialCiphertext: `secret-${providerId}`,
    encryptionContext: `key-${providerId}`,
    enabled: true,
    weight: 1,
    maxConcurrency: 10,
    models: null,
    revision: 1,
    credentialVersion: 1,
    credentialUpdatedAt: 1,
    createdAt: 1,
    updatedAt: 1,
  };
}

function capability(providerId: string, canonicalModelId = "gpt-5.5"): ProviderModelCapability {
  return {
    id: `cap-${providerId}`,
    providerId,
    canonicalModelId,
    upstreamModelId: `${providerId}-gpt-5.5`,
    apiPaths: ["chat_completions", "responses"],
    enabled: true,
    verifiedAt: null,
    revision: 1,
    createdAt: 1,
    updatedAt: 1,
  };
}

function route(targets: Array<{ providerId: string; priority: number; weight?: number }>): ModelRoute {
  return {
    publicModelId: "gpt-5.5",
    enabled: true,
    revision: 7,
    targets: targets.map(({ providerId, priority, weight = 1 }) => ({
      id: `target-${providerId}`,
      publicModelId: "gpt-5.5",
      capabilityId: `cap-${providerId}`,
      priority,
      weight,
      maxConcurrency: 10,
      enabled: true,
      createdAt: 1,
      updatedAt: 1,
    })),
    createdAt: 1,
    updatedAt: 1,
  };
}

const unusedCodex: CodexExecutor = async () => {
  throw new Error("Codex should not be called");
};

function success(providerId: string) {
  return {
    id: `chatcmpl-${providerId}`,
    choices: [{ message: { role: "assistant", content: providerId } }],
    usage: { prompt_tokens: 1, completion_tokens: 1 },
  };
}

const request = {
  path: "/v1/chat/completions" as const,
  publicModel: "gpt-5.5",
  body: { model: "gpt-5.5", messages: [{ role: "user", content: "hello" }] },
  outputLimit: 32,
};

describe("model priority router", () => {
  test("shares an equal-priority tier by configured weight", async () => {
    const selected: string[] = [];
    const providers = [provider("one"), provider("two")];
    const router = new ModelRouter(
      {
        providers,
        credentials: providers.map(({ id }) => credential(id)),
        capabilities: providers.map(({ id }) => capability(id)),
        routes: [route([
          { providerId: "one", priority: 1, weight: 2 },
          { providerId: "two", priority: 1, weight: 1 },
        ])],
      },
      (stored) => stored.credentialCiphertext,
      unusedCodex,
      false,
      environment,
      async (input) => {
        const providerId = new URL(String(input)).hostname.split(".")[0]!;
        selected.push(providerId);
        return Response.json(success(providerId));
      },
    );

    await router.execute(request);
    await router.execute(request);
    await router.execute(request);

    expect(selected).toEqual(["one", "one", "two"]);
    expect(router.publicModels()).toEqual(["gpt-5.5"]);
  });

  test("uses the next priority only after a retryable failure", async () => {
    const selected: string[] = [];
    const providers = [provider("primary"), provider("fallback")];
    const router = new ModelRouter(
      {
        providers,
        credentials: providers.map(({ id }) => credential(id)),
        capabilities: providers.map(({ id }) => capability(id)),
        routes: [route([
          { providerId: "primary", priority: 1 },
          { providerId: "fallback", priority: 2 },
        ])],
      },
      (stored) => stored.credentialCiphertext,
      unusedCodex,
      false,
      environment,
      async (input) => {
        const providerId = new URL(String(input)).hostname.split(".")[0]!;
        selected.push(providerId);
        return providerId === "primary"
          ? Response.json({ error: { message: "limited" } }, { status: 429 })
          : Response.json(success(providerId));
      },
    );

    const result = await router.execute(request);
    expect(selected).toEqual(["primary", "fallback"]);
    expect(result).toMatchObject({
      routeRevision: 7,
      routeTargetId: "target-fallback",
      providerId: "fallback",
      credentialId: "key-fallback",
    });
  });

  test("does not fall through on a request validation failure", async () => {
    let calls = 0;
    const providers = [provider("primary"), provider("fallback")];
    const router = new ModelRouter(
      {
        providers,
        credentials: providers.map(({ id }) => credential(id)),
        capabilities: providers.map(({ id }) => capability(id)),
        routes: [route([
          { providerId: "primary", priority: 1 },
          { providerId: "fallback", priority: 2 },
        ])],
      },
      (stored) => stored.credentialCiphertext,
      unusedCodex,
      false,
      environment,
      async () => {
        calls += 1;
        return Response.json({ error: { message: "invalid" } }, { status: 400 });
      },
    );

    await expect(router.execute(request)).rejects.toMatchObject({ status: 400 });
    expect(calls).toBe(1);
  });

  test("rejects a semantically different capability", () => {
    const first = provider("different");
    expect(() => new ModelRouter(
      {
        providers: [first],
        credentials: [credential(first.id)],
        capabilities: [capability(first.id, "deepseek-v4-pro")],
        routes: [route([{ providerId: first.id, priority: 1 }])],
      },
      (stored) => stored.credentialCiphertext,
      unusedCodex,
      false,
      environment,
    )).toThrow("canonical model must be gpt-5.5");
  });

  test("hands the customer's per-request reasoning effort to the Codex executor", async () => {
    const codex = provider("codex", "codex_cli");
    const efforts: Array<string | undefined> = [];
    const codexExecutor: CodexExecutor = async (_model, effort) => {
      efforts.push(effort);
      return {
        text: "ok",
        toolCalls: [],
        usage: { inputTokens: 1, cachedInputTokens: 0, outputTokens: 1 },
        reasoningOutputTokens: 0,
      };
    };
    const router = new ModelRouter(
      {
        providers: [codex],
        credentials: [],
        capabilities: [capability(codex.id)],
        routes: [route([{ providerId: codex.id, priority: 1 }])],
      },
      (stored) => stored.credentialCiphertext,
      codexExecutor,
      true,
      environment,
    );

    await router.execute({ ...request, body: { ...request.body, reasoning_effort: "high" } });
    await router.execute(request);
    expect(efforts).toEqual(["high", undefined]);
  });

  test("can fall back from Codex before output, but never after output starts", async () => {
    const codex = provider("codex", "codex_cli");
    const http = provider("http");
    let httpCalls = 0;
    let emitBeforeFailure = false;
    const codexExecutor: CodexExecutor = async (
      _model,
      _effort,
      _limit,
      _conversation,
      _tools,
      callbacks,
    ) => {
      if (emitBeforeFailure) callbacks.onTextDelta?.("partial");
      throw new Error("Codex unavailable");
    };
    const build = () => new ModelRouter(
      {
        providers: [codex, http],
        credentials: [credential(http.id)],
        capabilities: [capability(codex.id), capability(http.id)],
        routes: [route([
          { providerId: codex.id, priority: 1 },
          { providerId: http.id, priority: 2 },
        ])],
      },
      (stored) => stored.credentialCiphertext,
      codexExecutor,
      true,
      environment,
      async () => {
        httpCalls += 1;
        return Response.json(success("http"));
      },
    );

    expect((await build().execute(request)).providerId).toBe("http");
    expect(httpCalls).toBe(1);

    emitBeforeFailure = true;
    await expect(build().execute({ ...request, callbacks: { onTextDelta: () => undefined } }))
      .rejects.toThrow("Codex unavailable");
    expect(httpCalls).toBe(1);
  });
});

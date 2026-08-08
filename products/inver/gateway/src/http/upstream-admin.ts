import type {
  ModelRoute,
  ProviderModelCapability,
  PublicUpstreamCredential,
  RoutingProvider,
} from "../database";
import { ModelRouteError } from "../upstreams/model-router";
import { UpstreamRegistry } from "../upstreams/upstream-registry";
import { extractUsage } from "../generation/usage";

function error(message: string, status: number, type = "invalid_request_error") {
  return Response.json({ error: { message, type } }, { status });
}

function requiredString(value: unknown, field: string, maximum: number): string {
  if (typeof value !== "string" || !value.trim() || value.length > maximum) {
    throw new Error(`${field} is required and must be at most ${maximum} characters`);
  }
  return value.trim();
}

function optionalBoolean(value: unknown, field: string, fallback: boolean): boolean {
  if (value === undefined) return fallback;
  if (typeof value !== "boolean") throw new Error(`${field} must be a boolean`);
  return value;
}

function optionalPositiveInteger(
  value: unknown,
  field: string,
  fallback: number,
  maximum: number,
): number {
  const parsed = value === undefined ? fallback : value;
  if (typeof parsed !== "number" || !Number.isSafeInteger(parsed) || parsed < 1 || parsed > maximum) {
    throw new Error(`${field} must be an integer between 1 and ${maximum}`);
  }
  return parsed;
}

function optionalModels(value: unknown, fallback: string[] | null): string[] | null {
  if (value === undefined) return fallback;
  if (value === null) return null;
  if (
    !Array.isArray(value) ||
    value.length === 0 ||
    value.some((model) => typeof model !== "string" || !model.trim() || model.length > 160)
  ) {
    throw new Error("models must be null or a non-empty array of canonical model IDs");
  }
  return [...new Set(value.map((model) => (model as string).trim()))];
}

function optionalRevision(value: unknown): number | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new Error("revision must be a non-negative integer");
  }
  return value;
}

function apiPaths(value: unknown, fallback?: ProviderModelCapability["apiPaths"]) {
  if (value === undefined && fallback) return fallback;
  if (
    !Array.isArray(value) ||
    value.length === 0 ||
    value.some((path) => path !== "chat_completions" && path !== "responses")
  ) {
    throw new Error("api_paths must contain chat_completions and/or responses");
  }
  return [...new Set(value)] as ProviderModelCapability["apiPaths"];
}

function adminError(cause: unknown): Response {
  const message = cause instanceof Error ? cause.message : "Invalid upstream configuration";
  if (
    message === "upstream_provider_revision_conflict" ||
    message === "upstream_credential_revision_conflict" ||
    message === "upstream_model_revision_conflict" ||
    message === "model_route_revision_conflict"
  ) {
    return error("Configuration was changed by another administrator", 409, "conflict_error");
  }
  if (
    message === "Upstream provider not found" ||
    message === "Upstream credential not found" ||
    message === "Upstream model capability not found"
  ) {
    return error(message, 404);
  }
  if (message.includes("UNIQUE constraint failed")) {
    return error("A configuration with that name or mapping already exists", 409, "conflict_error");
  }
  if (message.includes("credential could not be decrypted") || message.includes("unsupported format")) {
    return error("Stored upstream credentials are unavailable", 503, "server_error");
  }
  if (message === "Upstream credential encryption is not configured") {
    return error(message, 503, "server_error");
  }
  return error(message, 400);
}

function serializeProvider(provider: RoutingProvider) {
  return {
    id: provider.id,
    name: provider.name,
    transport: provider.transport,
    base_url: provider.baseUrl,
    enabled: provider.enabled,
    max_concurrency: provider.maxConcurrency,
    revision: provider.revision,
    created_at: provider.createdAt,
    updated_at: provider.updatedAt,
  };
}

function serializeCredential(credential: PublicUpstreamCredential) {
  return {
    id: credential.id,
    provider_id: credential.providerId,
    name: credential.name,
    enabled: credential.enabled,
    weight: credential.weight,
    max_concurrency: credential.maxConcurrency,
    models: credential.models,
    revision: credential.revision,
    credential_version: credential.credentialVersion,
    credential_updated_at: credential.credentialUpdatedAt,
    has_api_key: true,
    created_at: credential.createdAt,
    updated_at: credential.updatedAt,
  };
}

function serializeCapability(capability: ProviderModelCapability) {
  return {
    id: capability.id,
    provider_id: capability.providerId,
    canonical_model_id: capability.canonicalModelId,
    upstream_model_id: capability.upstreamModelId,
    api_paths: capability.apiPaths,
    enabled: capability.enabled,
    verified_at: capability.verifiedAt,
    revision: capability.revision,
    created_at: capability.createdAt,
    updated_at: capability.updatedAt,
  };
}

function serializeRoute(route: ModelRoute) {
  return {
    public_model_id: route.publicModelId,
    enabled: route.enabled,
    revision: route.revision,
    targets: route.targets.map((target) => ({
      id: target.id,
      capability_id: target.capabilityId,
      priority: target.priority,
      weight: target.weight,
      max_concurrency: target.maxConcurrency,
      enabled: target.enabled,
    })),
    created_at: route.createdAt,
    updated_at: route.updatedAt,
  };
}

function revisionQuery(url: URL): number | undefined {
  const raw = url.searchParams.get("revision");
  return raw === null ? undefined : optionalRevision(Number(raw));
}

export async function routeUpstreamAdmin(
  req: Request,
  url: URL,
  registry: UpstreamRegistry,
): Promise<Response | null> {
  if (url.pathname === "/internal/upstreams" && req.method === "GET") {
    return Response.json({
      object: "list",
      data: registry.listProviders().map(serializeProvider),
      runtime: {
        configuration_source: registry.source,
        applied_from_database: registry.appliedFromDatabase,
        ...registry.status(),
      },
    });
  }

  if (url.pathname === "/internal/upstreams" && req.method === "POST") {
    try {
      const body = await req.json() as Record<string, unknown>;
      const transport = body.transport ?? "openai_http";
      if (transport !== "openai_http" && transport !== "codex_cli") {
        throw new Error("transport must be openai_http or codex_cli");
      }
      if (transport === "codex_cli" && body.base_url !== undefined && body.base_url !== null) {
        throw new Error("base_url is not used by codex_cli providers");
      }
      const result = registry.createProvider({
        name: requiredString(body.name, "name", 80),
        transport,
        baseUrl: transport === "openai_http"
          ? requiredString(body.base_url, "base_url", 2_000)
          : null,
        enabled: optionalBoolean(body.enabled, "enabled", false),
        maxConcurrency: optionalPositiveInteger(
          body.max_concurrency,
          "max_concurrency",
          Number(process.env.UPSTREAM_MAX_CONCURRENCY_PER_PROVIDER || 4),
          10_000,
        ),
      });
      return Response.json({
        ...serializeProvider(result.provider),
        applied_to_runtime: result.applied,
      }, { status: 201 });
    } catch (cause) {
      return adminError(cause);
    }
  }

  const credentialCollection = url.pathname.match(/^\/internal\/upstreams\/([^/]+)\/credentials$/);
  if (credentialCollection && req.method === "GET") {
    try {
      return Response.json({
        object: "list",
        data: registry.listCredentials(credentialCollection[1]!).map(serializeCredential),
      });
    } catch (cause) {
      return adminError(cause);
    }
  }
  if (credentialCollection && req.method === "POST") {
    try {
      const body = await req.json() as Record<string, unknown>;
      const result = registry.createCredential(credentialCollection[1]!, {
        name: requiredString(body.name, "name", 80),
        apiKey: requiredString(body.api_key, "api_key", 20_000),
        enabled: optionalBoolean(body.enabled, "enabled", true),
        weight: optionalPositiveInteger(body.weight, "weight", 1, 100),
        maxConcurrency: optionalPositiveInteger(
          body.max_concurrency,
          "max_concurrency",
          Number(process.env.UPSTREAM_MAX_CONCURRENCY_PER_PROVIDER || 4),
          10_000,
        ),
        models: optionalModels(body.models, null),
      });
      return Response.json({
        ...serializeCredential(result.credential),
        applied_to_runtime: result.applied,
      }, { status: 201 });
    } catch (cause) {
      return adminError(cause);
    }
  }

  /* Read-only discovery for the operator console's capability picker: the models the provider
   * itself reports. Failures here are provider problems, not configuration writes, so they map to
   * upstream errors rather than validation errors. */
  const availableModels = url.pathname.match(/^\/internal\/upstreams\/([^/]+)\/available-models$/);
  if (availableModels && req.method === "GET") {
    try {
      return Response.json({
        object: "list",
        data: await registry.discoverProviderModels(availableModels[1]!),
      });
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : "Model discovery failed";
      if (
        message.startsWith("The provider could not be reached") ||
        message.startsWith("The provider answered") ||
        message.startsWith("The provider's /v1/models")
      ) {
        return error(message, 502, "upstream_error");
      }
      return adminError(cause);
    }
  }

  const capabilityCollection = url.pathname.match(/^\/internal\/upstreams\/([^/]+)\/models$/);
  if (capabilityCollection && req.method === "GET") {
    try {
      return Response.json({
        object: "list",
        data: registry.listCapabilities(capabilityCollection[1]!).map(serializeCapability),
      });
    } catch (cause) {
      return adminError(cause);
    }
  }
  if (capabilityCollection && req.method === "POST") {
    try {
      const body = await req.json() as Record<string, unknown>;
      const result = registry.createCapability(capabilityCollection[1]!, {
        canonicalModelId: requiredString(body.canonical_model_id, "canonical_model_id", 160),
        upstreamModelId: requiredString(body.upstream_model_id, "upstream_model_id", 160),
        apiPaths: apiPaths(body.api_paths),
        enabled: optionalBoolean(body.enabled, "enabled", true),
      });
      return Response.json({
        ...serializeCapability(result.capability),
        applied_to_runtime: result.applied,
      }, { status: 201 });
    } catch (cause) {
      return adminError(cause);
    }
  }

  /* Every capability across every provider in one read. An operator console needs the whole set to
   * offer route candidates for a public model; without this it has to fan out one request per
   * provider, which grows linearly with the pool. */
  if (url.pathname === "/internal/upstream-models" && req.method === "GET") {
    try {
      return Response.json({
        object: "list",
        data: registry.listCapabilities().map(serializeCapability),
      });
    } catch (cause) {
      return adminError(cause);
    }
  }

  const rotateCredential = url.pathname.match(/^\/internal\/upstream-credentials\/([^/]+)\/rotate-key$/);
  if (rotateCredential && req.method === "POST") {
    try {
      const body = await req.json() as Record<string, unknown>;
      const result = registry.rotateCredential(
        rotateCredential[1]!,
        requiredString(body.api_key, "api_key", 20_000),
        optionalRevision(body.revision),
      );
      return Response.json({
        ...serializeCredential(result.credential),
        applied_to_runtime: result.applied,
      });
    } catch (cause) {
      return adminError(cause);
    }
  }

  const credentialMatch = url.pathname.match(/^\/internal\/upstream-credentials\/([^/]+)$/);
  if (credentialMatch && req.method === "PATCH") {
    try {
      const body = await req.json() as Record<string, unknown>;
      const stored = registry.getCredential(credentialMatch[1]!);
      if (!stored) return error("Upstream credential not found", 404);
      const result = registry.updateCredential(credentialMatch[1]!, {
        name: body.name === undefined ? undefined : requiredString(body.name, "name", 80),
        enabled: body.enabled === undefined
          ? undefined
          : optionalBoolean(body.enabled, "enabled", stored.enabled),
        weight: optionalPositiveInteger(body.weight, "weight", stored.weight, 100),
        maxConcurrency: optionalPositiveInteger(
          body.max_concurrency,
          "max_concurrency",
          stored.maxConcurrency,
          10_000,
        ),
        models: optionalModels(body.models, stored.models),
        expectedRevision: optionalRevision(body.revision),
      });
      return Response.json({
        ...serializeCredential(result.credential),
        applied_to_runtime: result.applied,
      });
    } catch (cause) {
      return adminError(cause);
    }
  }
  if (credentialMatch && req.method === "DELETE") {
    try {
      return registry.deleteCredential(credentialMatch[1]!, revisionQuery(url))
        ? new Response(null, { status: 204 })
        : error("Upstream credential not found", 404);
    } catch (cause) {
      return adminError(cause);
    }
  }

  const capabilityTest = url.pathname.match(/^\/internal\/upstream-models\/([^/]+)\/test$/);
  if (capabilityTest && req.method === "POST") {
    const capabilityId = capabilityTest[1]!;
    try {
      const body = await req.json() as Record<string, unknown>;
      const publicModel = requiredString(body.public_model_id, "public_model_id", 160);
      const path = body.path ?? "/v1/chat/completions";
      if (path !== "/v1/chat/completions" && path !== "/v1/responses") {
        throw new Error("path must be /v1/chat/completions or /v1/responses");
      }
      const result = await registry.testCapability(capabilityId, publicModel, path);
      const usage = extractUsage(result.payload);
      registry.updateCapability(capabilityId, { verifiedAt: Date.now() });
      registry.recordTest(capabilityId, true, {
        publicModel,
        path,
        upstreamStatus: result.status,
      });
      return Response.json({
        ok: true,
        public_model_id: publicModel,
        path,
        upstream_status: result.status,
        usage: {
          input_tokens: usage.inputTokens,
          cached_input_tokens: usage.cachedInputTokens,
          cache_write_tokens: usage.cacheWriteTokens ?? 0,
          output_tokens: usage.outputTokens,
        },
      });
    } catch (cause) {
      registry.recordTest(capabilityId, false, {
        type: cause instanceof ModelRouteError ? cause.type : "invalid_test_request",
      });
      if (cause instanceof ModelRouteError) {
        return error(cause.message, cause.status >= 500 ? cause.status : 502, cause.type);
      }
      return adminError(cause);
    }
  }

  const capabilityMatch = url.pathname.match(/^\/internal\/upstream-models\/([^/]+)$/);
  if (capabilityMatch && req.method === "PATCH") {
    try {
      const body = await req.json() as Record<string, unknown>;
      const stored = registry.getCapability(capabilityMatch[1]!);
      if (!stored) return error("Upstream model capability not found", 404);
      if (body.canonical_model_id !== undefined) {
        throw new Error("canonical_model_id is immutable; create a new capability instead");
      }
      const result = registry.updateCapability(capabilityMatch[1]!, {
        upstreamModelId: body.upstream_model_id === undefined
          ? undefined
          : requiredString(body.upstream_model_id, "upstream_model_id", 160),
        apiPaths: apiPaths(body.api_paths, stored.apiPaths),
        enabled: body.enabled === undefined
          ? undefined
          : optionalBoolean(body.enabled, "enabled", stored.enabled),
        expectedRevision: optionalRevision(body.revision),
      });
      return Response.json({
        ...serializeCapability(result.capability),
        applied_to_runtime: result.applied,
      });
    } catch (cause) {
      return adminError(cause);
    }
  }
  if (capabilityMatch && req.method === "DELETE") {
    try {
      return registry.deleteCapability(capabilityMatch[1]!, revisionQuery(url))
        ? new Response(null, { status: 204 })
        : error("Upstream model capability not found", 404);
    } catch (cause) {
      return adminError(cause);
    }
  }

  if (url.pathname === "/internal/model-routes" && req.method === "GET") {
    return Response.json({
      object: "list",
      data: registry.listRoutes().map(serializeRoute),
    });
  }

  const routeMatch = url.pathname.match(/^\/internal\/model-routes\/([^/]+)$/);
  if (routeMatch && req.method === "GET") {
    const route = registry.getRoute(decodeURIComponent(routeMatch[1]!));
    return route ? Response.json(serializeRoute(route)) : error("Model route not found", 404);
  }
  if (routeMatch && req.method === "PUT") {
    try {
      const body = await req.json() as Record<string, unknown>;
      if (!Array.isArray(body.targets)) throw new Error("targets must be an array");
      const targets = body.targets.map((value, index) => {
        if (!value || typeof value !== "object" || Array.isArray(value)) {
          throw new Error(`targets[${index}] must be an object`);
        }
        const target = value as Record<string, unknown>;
        return {
          capabilityId: requiredString(target.capability_id, `targets[${index}].capability_id`, 160),
          priority: optionalPositiveInteger(target.priority, `targets[${index}].priority`, 1, 10_000),
          weight: optionalPositiveInteger(target.weight, `targets[${index}].weight`, 1, 100),
          maxConcurrency: optionalPositiveInteger(
            target.max_concurrency,
            `targets[${index}].max_concurrency`,
            10_000,
            10_000,
          ),
          enabled: optionalBoolean(target.enabled, `targets[${index}].enabled`, true),
        };
      });
      const result = registry.publishRoute({
        publicModelId: decodeURIComponent(routeMatch[1]!),
        enabled: optionalBoolean(body.enabled, "enabled", true),
        expectedRevision: optionalRevision(body.revision),
        targets,
      });
      return Response.json({
        ...serializeRoute(result.route),
        applied_to_runtime: result.applied,
      });
    } catch (cause) {
      return adminError(cause);
    }
  }

  const providerMatch = url.pathname.match(/^\/internal\/upstreams\/([^/]+)$/);
  if (providerMatch && req.method === "GET") {
    const provider = registry.getProvider(providerMatch[1]!);
    return provider ? Response.json(serializeProvider(provider)) : error("Upstream provider not found", 404);
  }
  if (providerMatch && req.method === "PATCH") {
    try {
      const body = await req.json() as Record<string, unknown>;
      const stored = registry.getProvider(providerMatch[1]!);
      if (!stored) return error("Upstream provider not found", 404);
      if (body.transport !== undefined) {
        throw new Error("transport is immutable; create a new provider instead");
      }
      const result = registry.updateProvider(providerMatch[1]!, {
        name: body.name === undefined ? undefined : requiredString(body.name, "name", 80),
        baseUrl: body.base_url === undefined
          ? undefined
          : body.base_url === null
            ? null
            : requiredString(body.base_url, "base_url", 2_000),
        enabled: body.enabled === undefined
          ? undefined
          : optionalBoolean(body.enabled, "enabled", stored.enabled),
        maxConcurrency: optionalPositiveInteger(
          body.max_concurrency,
          "max_concurrency",
          stored.maxConcurrency,
          10_000,
        ),
        expectedRevision: optionalRevision(body.revision),
      });
      return Response.json({
        ...serializeProvider(result.provider),
        applied_to_runtime: result.applied,
      });
    } catch (cause) {
      return adminError(cause);
    }
  }
  if (providerMatch && req.method === "DELETE") {
    try {
      return registry.deleteProvider(providerMatch[1]!, revisionQuery(url))
        ? new Response(null, { status: 204 })
        : error("Upstream provider not found", 404);
    } catch (cause) {
      return adminError(cause);
    }
  }

  return null;
}

export {};

interface Provider {
  id: string;
  name: string;
  transport: "openai_http" | "codex_cli";
  enabled: boolean;
}

interface Capability {
  id: string;
  canonical_model_id: string;
}

interface Route {
  public_model_id: string;
}

const gatewayUrl = (process.env.GATEWAY_URL || "http://127.0.0.1:8080")
  .replace(/\/v1\/?$/, "")
  .replace(/\/$/, "");
const adminKey = process.env.ADMIN_API_KEY;
const providerName = process.env.CODEX_PROVIDER_NAME || "Codex subscription";

if (!adminKey) throw new Error("ADMIN_API_KEY is required");

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`${gatewayUrl}${path}`, {
    ...init,
    headers: {
      authorization: `Bearer ${adminKey}`,
      ...(init.body ? { "content-type": "application/json" } : {}),
      ...init.headers,
    },
  });
  const payload: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    const message = payload && typeof payload === "object" &&
      "error" in payload &&
      payload.error && typeof payload.error === "object" &&
      "message" in payload.error &&
      typeof payload.error.message === "string"
      ? payload.error.message
      : `Gateway returned ${response.status}`;
    throw new Error(message);
  }
  return payload as T;
}

const providerResponse = await request<{ data: Provider[]; runtime: {
  configuration_source: string;
} }>("/internal/upstreams");
if (providerResponse.runtime.configuration_source !== "database") {
  throw new Error("Start the gateway with UPSTREAM_CONFIG_SOURCE=database before bootstrapping");
}

let provider = providerResponse.data.find((candidate) => candidate.name === providerName);
if (provider && provider.transport !== "codex_cli") {
  throw new Error(`Provider “${providerName}” already exists with a different transport`);
}
if (provider && !provider.enabled) {
  throw new Error(`Provider “${providerName}” exists but is disabled; enable it explicitly first`);
}
if (!provider) {
  provider = await request<Provider>("/internal/upstreams", {
    method: "POST",
    body: JSON.stringify({
      name: providerName,
      transport: "codex_cli",
      enabled: true,
      max_concurrency: 1,
    }),
  });
  console.log(`Created Codex provider: ${provider.name}`);
} else {
  console.log(`Using existing Codex provider: ${provider.name}`);
}

const capabilityResponse = await request<{ data: Capability[] }>(
  `/internal/upstreams/${provider.id}/models`,
);
const capabilities = new Map(
  capabilityResponse.data.map((capability) => [capability.canonical_model_id, capability]),
);
for (const canonicalModel of ["gpt-5.6-luna", "gpt-5.5"]) {
  if (capabilities.has(canonicalModel)) continue;
  const capability = await request<Capability>(`/internal/upstreams/${provider.id}/models`, {
    method: "POST",
    body: JSON.stringify({
      canonical_model_id: canonicalModel,
      upstream_model_id: canonicalModel,
      api_paths: ["chat_completions", "responses"],
      enabled: true,
    }),
  });
  capabilities.set(canonicalModel, capability);
  console.log(`Created capability: ${canonicalModel}`);
}

const routeResponse = await request<{ data: Route[] }>("/internal/model-routes");
const existingRoutes = new Set(routeResponse.data.map((route) => route.public_model_id));
const desiredRoutes = [
  ["gpt-5.6-luna-low", "gpt-5.6-luna"],
  ["gpt-5.6-luna-medium", "gpt-5.6-luna"],
  ["gpt-5.5", "gpt-5.5"],
] as const;

for (const [publicModel, canonicalModel] of desiredRoutes) {
  if (existingRoutes.has(publicModel)) {
    console.log(`Kept existing route: ${publicModel}`);
    continue;
  }
  const capability = capabilities.get(canonicalModel);
  if (!capability) throw new Error(`Missing capability for ${canonicalModel}`);
  await request(`/internal/model-routes/${encodeURIComponent(publicModel)}`, {
    method: "PUT",
    body: JSON.stringify({
      enabled: true,
      revision: 0,
      targets: [{
        capability_id: capability.id,
        priority: 1,
        weight: 1,
        max_concurrency: 1,
        enabled: true,
      }],
    }),
  });
  console.log(`Published route: ${publicModel}`);
}

console.log("Codex upstream bootstrap complete. No paid generation probe was run.");

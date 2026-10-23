import "server-only";
import type {
  ApiKeyRow,
  CreateApiKeyRequest,
  CreditCustomerRequest,
  CreatedApiKey,
  CreatedCustomer,
  CreditResult,
  CustomerAccount,
  CustomerRow,
  RegisterCustomerAccountRequest,
  ModelRoute,
  PricingResponse,
  Reconciliation,
  RegisteredCustomerAccount,
  UpstreamCredential,
  CreateUpstreamCredentialRequest,
  CreateUpstreamModelRequest,
  CreateUpstreamProviderRequest,
  UpstreamModelCapability,
  UpstreamProvider,
  UpstreamRuntime,
  UpstreamTestResult,
  UpdateApiKeyRequest,
  UpdateCustomerRequest,
  UpdateUpstreamCredentialRequest,
  UpdateUpstreamProviderRequest,
  PublishModelRouteRequest,
  UsageEvent,
  UsageSummary,
} from "@tundral/contracts";
import { parseGatewayError } from "@tundral/contracts/validation";

const gatewayUrl = () => (process.env.GATEWAY_URL || "http://127.0.0.1:8080").replace(/\/$/, "");

export class GatewayError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
  }
}

async function request<T>(path: string, token: string, init: RequestInit = {}): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${gatewayUrl()}${path}`, {
      ...init,
      headers: {
        authorization: `Bearer ${token}`,
        ...(init.body ? { "content-type": "application/json" } : {}),
        ...init.headers,
      },
      cache: "no-store",
    });
  } catch {
    /* A refused or dropped connection is an expected operational state, not a programming error.
     * Normalising it to a GatewayError 503 lets every page render its gateway-unavailable UI
     * instead of falling through to the framework error page. */
    throw new GatewayError("The inference gateway could not be reached at its configured GATEWAY_URL", 503);
  }
  if (response.status === 204) return undefined as T;
  const payload: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    const envelope = parseGatewayError(payload);
    throw new GatewayError(envelope?.error.message || `Gateway returned ${response.status}`, response.status);
  }
  return payload as T;
}

function adminKey(): string {
  const key = process.env.GATEWAY_ADMIN_KEY;
  if (!key) throw new GatewayError("GATEWAY_ADMIN_KEY is not configured", 500);
  return key;
}

function webBffKey(): string {
  const key = process.env.GATEWAY_WEB_KEY || process.env.GATEWAY_ADMIN_KEY;
  if (!key) throw new GatewayError("GATEWAY_WEB_KEY is not configured", 500);
  return key;
}

const admin = <T>(path: string, init: RequestInit = {}) => request<T>(path, adminKey(), init);
const customer = <T>(path: string, init: RequestInit = {}) => request<T>(path, webBffKey(), init);

export const gatewayCustomer = {
  getCustomer: (id: string) =>
    customer<Omit<CustomerRow, "posted_balance_micros" | "active_key_count">>(
      `/internal/customers/${id}`,
    ),
  registerAccount: (args: RegisterCustomerAccountRequest) =>
    customer<RegisteredCustomerAccount>("/internal/accounts/register", {
      method: "POST",
      body: JSON.stringify(args),
    }),
  authenticateAccount: (email: string, password: string) =>
    customer<CustomerAccount>("/internal/accounts/authenticate", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),
  usage: (id: string, limit = 100) =>
    customer<{ data: UsageEvent[] }>(
      `/internal/customers/${id}/usage?limit=${limit}`,
    ).then((result) => result.data),
  reconciliation: (id: string) =>
    customer<Reconciliation>(`/internal/customers/${id}/reconciliation`),
  listKeys: (id: string) =>
    customer<{ data: ApiKeyRow[] }>(`/internal/customers/${id}/keys`).then(
      (result) => result.data,
    ),
  createKey: (id: string, args: CreateApiKeyRequest) =>
    customer<CreatedApiKey>(`/internal/customers/${id}/keys`, {
      method: "POST",
      body: JSON.stringify(args),
    }),
  updateKey: (customerId: string, keyId: string, patch: UpdateApiKeyRequest) =>
    customer<ApiKeyRow>(`/internal/customers/${customerId}/keys/${keyId}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  revokeKey: (customerId: string, keyId: string) =>
    customer<void>(`/internal/customers/${customerId}/keys/${keyId}`, {
      method: "DELETE",
    }),
  pricing: () => customer<PricingResponse>("/internal/pricing"),
};

export const gatewayAdmin = {
  listCustomers: () => admin<{ data: CustomerRow[] }>("/internal/customers").then((r) => r.data),
  getCustomer: (id: string) =>
    admin<Omit<CustomerRow, "posted_balance_micros" | "active_key_count">>(`/internal/customers/${id}`),
  createCustomer: (name: string) =>
    admin<CreatedCustomer>("/internal/customers", {
      method: "POST",
      body: JSON.stringify({ name }),
    }),
  updateCustomer: (id: string, patch: UpdateCustomerRequest) =>
    admin(`/internal/customers/${id}`, { method: "PATCH", body: JSON.stringify(patch) }),
  usage: (id: string, limit = 100) => admin<{ data: UsageEvent[] }>(`/internal/customers/${id}/usage?limit=${limit}`).then((r) => r.data),
  usageSummary: (days = 30) => admin<UsageSummary>(`/internal/usage/summary?days=${days}`),
  reconciliation: (id: string) => admin<Reconciliation>(`/internal/customers/${id}/reconciliation`),
  listKeys: (id: string) => admin<{ data: ApiKeyRow[] }>(`/internal/customers/${id}/keys`).then((r) => r.data),
  createKey: (id: string, args: CreateApiKeyRequest) =>
    admin<CreatedApiKey>(`/internal/customers/${id}/keys`, {
      method: "POST",
      body: JSON.stringify(args),
    }),
  updateKey: (customerId: string, keyId: string, patch: UpdateApiKeyRequest) =>
    admin<ApiKeyRow>(`/internal/customers/${customerId}/keys/${keyId}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  revokeKey: (customerId: string, keyId: string) =>
    admin<void>(`/internal/customers/${customerId}/keys/${keyId}`, { method: "DELETE" }),
  credit: (id: string, args: CreditCustomerRequest) =>
    admin<CreditResult>(`/internal/customers/${id}/credits`, {
      method: "POST",
      body: JSON.stringify(args),
    }),
  listUpstreams: () => admin<{ data: UpstreamProvider[]; runtime: UpstreamRuntime }>("/internal/upstreams"),
  createUpstream: (args: CreateUpstreamProviderRequest) =>
    admin<UpstreamProvider & { applied_to_runtime: boolean }>("/internal/upstreams", {
      method: "POST",
      body: JSON.stringify(args),
    }),
  updateUpstream: (
    id: string,
    patch: UpdateUpstreamProviderRequest,
  ) =>
    admin<UpstreamProvider & { applied_to_runtime: boolean }>(`/internal/upstreams/${id}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  listUpstreamCredentials: (providerId: string) =>
    admin<{ data: UpstreamCredential[] }>(`/internal/upstreams/${providerId}/credentials`).then((r) => r.data),
  createUpstreamCredential: (providerId: string, args: CreateUpstreamCredentialRequest) =>
    admin<UpstreamCredential & { applied_to_runtime: boolean }>(`/internal/upstreams/${providerId}/credentials`, {
      method: "POST",
      body: JSON.stringify(args),
    }),
  updateUpstreamCredential: (id: string, patch: UpdateUpstreamCredentialRequest) =>
    admin<UpstreamCredential & { applied_to_runtime: boolean }>(`/internal/upstream-credentials/${id}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  rotateUpstreamCredential: (id: string, apiKey: string, revision: number) =>
    admin<UpstreamCredential & { applied_to_runtime: boolean }>(`/internal/upstream-credentials/${id}/rotate-key`, {
      method: "POST",
      body: JSON.stringify({ api_key: apiKey, revision }),
    }),
  deleteUpstreamCredential: (id: string, revision: number) =>
    admin<void>(`/internal/upstream-credentials/${id}?revision=${revision}`, { method: "DELETE" }),
  deleteUpstream: (id: string, revision: number) =>
    admin<void>(`/internal/upstreams/${id}?revision=${revision}`, { method: "DELETE" }),
  /* Every capability in one read. The routing overview needs the whole set to offer route candidates;
   * fanning out listUpstreamModels per provider grows linearly with the pool. */
  listAllUpstreamModels: () =>
    admin<{ data: UpstreamModelCapability[] }>("/internal/upstream-models").then((r) => r.data),
  listUpstreamModels: (providerId: string) =>
    admin<{ data: UpstreamModelCapability[] }>(`/internal/upstreams/${providerId}/models`).then((r) => r.data),
  /* The models the provider itself reports from its /v1/models, authenticated with a stored
   * credential — powers the capability picker so operators select rather than hand-type IDs. */
  discoverUpstreamModels: (providerId: string) =>
    admin<{ data: string[] }>(`/internal/upstreams/${providerId}/available-models`).then((r) => r.data),
  createUpstreamModel: (providerId: string, args: CreateUpstreamModelRequest) =>
    admin<UpstreamModelCapability & { applied_to_runtime: boolean }>(`/internal/upstreams/${providerId}/models`, {
      method: "POST",
      body: JSON.stringify(args),
    }),
  deleteUpstreamModel: (id: string, revision: number) =>
    admin<void>(`/internal/upstream-models/${id}?revision=${revision}`, { method: "DELETE" }),
  testUpstreamModel: (id: string, publicModelId: string) =>
    admin<UpstreamTestResult>(`/internal/upstream-models/${id}/test`, {
      method: "POST",
      body: JSON.stringify({ public_model_id: publicModelId }),
    }),
  listModelRoutes: () =>
    admin<{ data: ModelRoute[] }>("/internal/model-routes").then((r) => r.data),
  publishModelRoute: (
    publicModelId: string,
    args: PublishModelRouteRequest,
  ) =>
    admin<ModelRoute & { applied_to_runtime: boolean }>(
      `/internal/model-routes/${encodeURIComponent(publicModelId)}`,
      { method: "PUT", body: JSON.stringify(args) },
    ),
  pricing: () =>
    admin<PricingResponse>("/internal/pricing"),
  /* The complete rate card, including models with no route yet. /internal/pricing narrows to what the
   * gateway can currently serve, which hides exactly the models an operator needs to route. */
  rateCard: () =>
    admin<PricingResponse>("/internal/rate-card"),
};

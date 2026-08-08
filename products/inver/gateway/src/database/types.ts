export type ManualCreditKind = "promotional_credit" | "offline_payment" | "manual_adjustment";

export type IdempotencyResult =
  | { state: "started"; requestId: string }
  | { state: "in_progress" }
  | { state: "conflict" }
  | { state: "completed"; requestId: string; payload: Record<string, unknown>; status: number };

export interface AuthContext {
  apiKeyId: string;
  customerId: string;
  maxConcurrency: number;
  rateLimitPerMinute: number;
  keySpendLimitMicros: number | null;
  dailySpendLimitMicros: number | null;
  perRequestLimitMicros: number | null;
}

export interface StoredUpstreamProvider {
  id: string;
  name: string;
  baseUrl: string;
  credentialCiphertext: string;
  enabled: boolean;
  weight: number;
  maxConcurrency: number;
  models: string[] | null;
  revision: number;
  credentialVersion: number;
  credentialUpdatedAt: number;
  createdAt: number;
  updatedAt: number;
}

export interface PublicUpstreamProvider extends Omit<StoredUpstreamProvider, "credentialCiphertext"> {
  hasApiKey: true;
}

export type UpstreamTransport = "openai_http" | "codex_cli";

export interface RoutingProvider {
  id: string;
  name: string;
  transport: UpstreamTransport;
  baseUrl: string | null;
  enabled: boolean;
  maxConcurrency: number;
  revision: number;
  createdAt: number;
  updatedAt: number;
}

export interface StoredUpstreamCredential {
  id: string;
  providerId: string;
  name: string;
  credentialCiphertext: string;
  encryptionContext: string;
  enabled: boolean;
  weight: number;
  maxConcurrency: number;
  models: string[] | null;
  revision: number;
  credentialVersion: number;
  credentialUpdatedAt: number;
  createdAt: number;
  updatedAt: number;
}

export interface PublicUpstreamCredential extends Omit<
  StoredUpstreamCredential,
  "credentialCiphertext" | "encryptionContext"
> {
  hasApiKey: true;
}

export interface ProviderModelCapability {
  id: string;
  providerId: string;
  canonicalModelId: string;
  upstreamModelId: string;
  apiPaths: Array<"chat_completions" | "responses">;
  enabled: boolean;
  verifiedAt: number | null;
  revision: number;
  createdAt: number;
  updatedAt: number;
}

export interface ModelRouteTarget {
  id: string;
  publicModelId: string;
  capabilityId: string;
  priority: number;
  weight: number;
  maxConcurrency: number;
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface ModelRoute {
  publicModelId: string;
  enabled: boolean;
  revision: number;
  targets: ModelRouteTarget[];
  createdAt: number;
  updatedAt: number;
}

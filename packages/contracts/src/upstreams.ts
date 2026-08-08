export type UpstreamTransport = "openai_http" | "codex_cli";
export type UpstreamApiPath = "chat_completions" | "responses";

export interface UpstreamProvider {
  id: string;
  name: string;
  transport: UpstreamTransport;
  base_url: string | null;
  enabled: boolean;
  max_concurrency: number;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface CreateUpstreamProviderRequest {
  name: string;
  transport: UpstreamTransport;
  base_url?: string;
  enabled: boolean;
  max_concurrency?: number;
}

export interface UpdateUpstreamProviderRequest {
  name?: string;
  base_url?: string | null;
  enabled?: boolean;
  max_concurrency?: number;
  revision?: number;
}

export interface UpstreamCredential {
  id: string;
  provider_id: string;
  name: string;
  enabled: boolean;
  weight: number;
  max_concurrency: number;
  models: string[] | null;
  revision: number;
  credential_version: number;
  credential_updated_at: number;
  has_api_key: true;
  created_at: number;
  updated_at: number;
}

export interface CreateUpstreamCredentialRequest {
  name: string;
  api_key: string;
  enabled: boolean;
  weight?: number;
  max_concurrency?: number;
  models?: string[] | null;
}

export interface UpdateUpstreamCredentialRequest {
  name?: string;
  enabled?: boolean;
  weight?: number;
  max_concurrency?: number;
  models?: string[] | null;
  revision?: number;
}

export interface UpstreamModelCapability {
  id: string;
  provider_id: string;
  canonical_model_id: string;
  upstream_model_id: string;
  api_paths: UpstreamApiPath[];
  enabled: boolean;
  verified_at: number | null;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface CreateUpstreamModelRequest {
  canonical_model_id: string;
  upstream_model_id: string;
  api_paths: UpstreamApiPath[];
  enabled: boolean;
}

export interface ModelRouteTarget {
  id: string;
  capability_id: string;
  priority: number;
  weight: number;
  max_concurrency: number;
  enabled: boolean;
}

export interface ModelRoute {
  public_model_id: string;
  enabled: boolean;
  revision: number;
  targets: ModelRouteTarget[];
  created_at: number;
  updated_at: number;
}

export interface PublishModelRouteRequest {
  enabled: boolean;
  revision: number;
  targets: Array<{
    capability_id: string;
    priority: number;
    weight: number;
    max_concurrency: number;
    enabled: boolean;
  }>;
}

export interface UpstreamRuntime {
  configuration_source: "environment" | "database";
  applied_from_database: boolean;
  configured: boolean;
  providers?: number;
  available?: number;
  capacity?: number;
  routes?: number;
  targets?: number;
  availableTargets?: number;
  inflight: number;
}

/** GET /internal/upstreams/:id/available-models — model IDs the provider itself reports. */
export interface UpstreamDiscoveredModels {
  object: "list";
  data: string[];
}

export interface UpstreamTestResult {
  ok: boolean;
  public_model_id: string;
  path: string;
  upstream_status: number;
  usage: {
    input_tokens: number;
    cached_input_tokens: number;
    cache_write_tokens: number;
    output_tokens: number;
  };
}

export interface UpstreamMutationResult<T> {
  data: T;
  applied_to_runtime: boolean;
}

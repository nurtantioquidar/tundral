export type UpstreamMode = "openai" | "codex-cli";
export type UpstreamConfigSource = "environment" | "database";

export interface GatewayConfig {
  host: string;
  port: number;
  databasePath: string;
  adminApiKey: string;
  webBffApiKey: string;
  metricsApiKey: string;
  requireScopedInternalKeys: boolean;
  upstreamMode: UpstreamMode;
  upstreamConfigSource: UpstreamConfigSource;
  generationEnabled: boolean;
  recoveryIntervalMs: number;
  defaultMaxOutputTokens: number;
  maxOutputTokensPerRequest: number;
  maxGlobalConcurrentRequests: number;
  codexReservationOverheadTokens: number;
  codexExecutable: string;
}

function positiveInteger(
  environment: Record<string, string | undefined>,
  name: string,
  fallback: number,
): number {
  const value = Number(environment[name] ?? fallback);
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${name} must be a positive integer`);
  }
  return value;
}

function nonNegativeInteger(
  environment: Record<string, string | undefined>,
  name: string,
  fallback: number,
): number {
  const value = Number(environment[name] ?? fallback);
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${name} must be a non-negative integer`);
  }
  return value;
}

export function loadGatewayConfig(
  environment: Record<string, string | undefined> = process.env,
): GatewayConfig {
  const upstreamMode = environment.UPSTREAM_MODE || "openai";
  if (upstreamMode !== "openai" && upstreamMode !== "codex-cli") {
    throw new Error("UPSTREAM_MODE must be openai or codex-cli");
  }
  const upstreamConfigSource = environment.UPSTREAM_CONFIG_SOURCE || "environment";
  if (upstreamConfigSource !== "environment" && upstreamConfigSource !== "database") {
    throw new Error("UPSTREAM_CONFIG_SOURCE must be environment or database");
  }

  const defaultMaxOutputTokens = positiveInteger(environment, "DEFAULT_MAX_OUTPUT_TOKENS", 4096);
  const maxOutputTokensPerRequest = positiveInteger(
    environment,
    "MAX_OUTPUT_TOKENS_PER_REQUEST",
    16_384,
  );
  if (defaultMaxOutputTokens > maxOutputTokensPerRequest) {
    throw new Error("DEFAULT_MAX_OUTPUT_TOKENS must not exceed MAX_OUTPUT_TOKENS_PER_REQUEST");
  }
  const adminApiKey = environment.ADMIN_API_KEY || "";
  const webBffApiKey = environment.WEB_BFF_API_KEY || adminApiKey;
  const requireScopedInternalKeys = environment.REQUIRE_SCOPED_INTERNAL_KEYS === "true";
  if (
    requireScopedInternalKeys &&
    (!adminApiKey || !webBffApiKey || adminApiKey === webBffApiKey)
  ) {
    throw new Error(
      "REQUIRE_SCOPED_INTERNAL_KEYS requires distinct ADMIN_API_KEY and WEB_BFF_API_KEY values",
    );
  }

  return {
    host: environment.HOST || "127.0.0.1",
    port: positiveInteger(environment, "PORT", 8080),
    databasePath: environment.DATABASE_PATH || "./gateway.db",
    adminApiKey,
    webBffApiKey,
    metricsApiKey: environment.METRICS_API_KEY || "",
    requireScopedInternalKeys,
    upstreamMode,
    upstreamConfigSource,
    generationEnabled: environment.GENERATION_ENABLED !== "false",
    recoveryIntervalMs: positiveInteger(environment, "RECOVERY_INTERVAL_MS", 60_000),
    defaultMaxOutputTokens,
    maxOutputTokensPerRequest,
    maxGlobalConcurrentRequests: positiveInteger(
      environment,
      "MAX_GLOBAL_CONCURRENT_REQUESTS",
      4,
    ),
    codexReservationOverheadTokens: nonNegativeInteger(
      environment,
      "CODEX_RESERVATION_OVERHEAD_TOKENS",
      50_000,
    ),
    codexExecutable: environment.CODEX_EXECUTABLE || "codex",
  };
}

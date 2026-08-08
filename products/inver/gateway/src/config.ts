import { Config, ConfigError, ConfigProvider, Effect, Either, Redacted } from "effect";

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

const invalid = (message: string) => ConfigError.InvalidData([], message);

/* `withDefault` recovers only from missing data, never from invalid data — so a fallback applies
 * when the variable is absent, while a present-but-invalid value still fails loudly. That ordering
 * is what keeps a typo in PORT from silently booting on the default. */
function boundedInteger(name: string, fallback: number, minimum: 0 | 1): Config.Config<number> {
  const requirement = minimum === 1 ? "a positive integer" : "a non-negative integer";
  return Config.integer(name).pipe(
    Config.validate({
      message: `${name} must be ${requirement}`,
      validation: (value) => Number.isSafeInteger(value) && value >= minimum,
    }),
    Config.withDefault(fallback),
  );
}

function literalUnion<const T extends readonly string[]>(
  name: string,
  allowed: T,
  fallback: T[number],
): Config.Config<T[number]> {
  return Config.string(name).pipe(
    Config.withDefault(fallback),
    Config.mapOrFail((value) =>
      allowed.includes(value)
        ? Either.right(value as T[number])
        : Either.left(invalid(`${name} must be ${allowed.join(" or ")}`)),
    ),
  );
}

/* These two flags are deliberately not `Config.boolean`. The gateway's contract is that only the
 * exact literal flips them: strict key scoping is off unless "true", and generation stays on unless
 * "false". `Config.boolean` would reject an unrecognised value instead, turning a stray flag into a
 * refused boot for an emergency switch that must stay predictable. */
const exactlyTrue = (name: string): Config.Config<boolean> =>
  Config.string(name).pipe(
    Config.withDefault(""),
    Config.map((value) => value === "true"),
  );

const unlessExactlyFalse = (name: string): Config.Config<boolean> =>
  Config.string(name).pipe(
    Config.withDefault(""),
    Config.map((value) => value !== "false"),
  );

/* Secrets are read as `Redacted` so an accidental log or interpolation of the config object prints
 * `<redacted>` rather than the key. They are unwrapped only at the boundary below, where the
 * existing string-typed callers still need them. */
const secret = (name: string): Config.Config<Redacted.Redacted<string>> =>
  Config.redacted(name).pipe(Config.withDefault(Redacted.make("")));

const string = (name: string, fallback: string): Config.Config<string> =>
  Config.string(name).pipe(Config.withDefault(fallback));

/* The single source of truth for gateway configuration. Later migration stages provide this
 * directly through a Layer; `loadGatewayConfig` is the synchronous adapter for callers that have
 * not moved yet. */
export const gatewayConfig: Config.Config<GatewayConfig> = Config.all({
  host: string("HOST", "127.0.0.1"),
  port: boundedInteger("PORT", 8080, 1),
  databasePath: string("DATABASE_PATH", "./gateway.db"),
  adminApiKey: secret("ADMIN_API_KEY"),
  webBffApiKey: secret("WEB_BFF_API_KEY"),
  metricsApiKey: secret("METRICS_API_KEY"),
  requireScopedInternalKeys: exactlyTrue("REQUIRE_SCOPED_INTERNAL_KEYS"),
  upstreamMode: literalUnion("UPSTREAM_MODE", ["openai", "codex-cli"] as const, "openai"),
  upstreamConfigSource: literalUnion(
    "UPSTREAM_CONFIG_SOURCE",
    ["environment", "database"] as const,
    "environment",
  ),
  generationEnabled: unlessExactlyFalse("GENERATION_ENABLED"),
  recoveryIntervalMs: boundedInteger("RECOVERY_INTERVAL_MS", 60_000, 1),
  defaultMaxOutputTokens: boundedInteger("DEFAULT_MAX_OUTPUT_TOKENS", 4096, 1),
  maxOutputTokensPerRequest: boundedInteger("MAX_OUTPUT_TOKENS_PER_REQUEST", 16_384, 1),
  maxGlobalConcurrentRequests: boundedInteger("MAX_GLOBAL_CONCURRENT_REQUESTS", 4, 1),
  codexReservationOverheadTokens: boundedInteger("CODEX_RESERVATION_OVERHEAD_TOKENS", 50_000, 0),
  codexExecutable: string("CODEX_EXECUTABLE", "codex"),
}).pipe(
  Config.mapOrFail((raw) => {
    if (raw.defaultMaxOutputTokens > raw.maxOutputTokensPerRequest) {
      return Either.left(
        invalid("DEFAULT_MAX_OUTPUT_TOKENS must not exceed MAX_OUTPUT_TOKENS_PER_REQUEST"),
      );
    }

    const adminApiKey = Redacted.value(raw.adminApiKey);
    /* An unset or empty web key falls back to the admin key. Strict scoping below is what forbids
     * that fallback in production. */
    const webBffApiKey = Redacted.value(raw.webBffApiKey) || adminApiKey;

    if (raw.requireScopedInternalKeys && (!adminApiKey || !webBffApiKey || adminApiKey === webBffApiKey)) {
      return Either.left(
        invalid(
          "REQUIRE_SCOPED_INTERNAL_KEYS requires distinct ADMIN_API_KEY and WEB_BFF_API_KEY values",
        ),
      );
    }

    return Either.right({
      ...raw,
      adminApiKey,
      webBffApiKey,
      metricsApiKey: Redacted.value(raw.metricsApiKey),
    });
  }),
);

export function loadGatewayConfig(
  environment: Record<string, string | undefined> = process.env,
): GatewayConfig {
  const entries: Array<[string, string]> = [];
  for (const [name, value] of Object.entries(environment)) {
    if (value !== undefined) {
      entries.push([name, value]);
    }
  }

  return Effect.runSync(
    gatewayConfig.pipe(Effect.withConfigProvider(ConfigProvider.fromMap(new Map(entries)))),
  );
}

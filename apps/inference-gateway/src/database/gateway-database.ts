import { Database } from "bun:sqlite";
import { createHash, randomUUID } from "node:crypto";
import { AccountRepository } from "../accounts/account-repository";
import { runMigrations } from "./migrate";
import type {
  AuthContext,
  IdempotencyResult,
  ManualCreditKind,
  ModelRoute,
  ModelRouteTarget,
  ProviderModelCapability,
  PublicUpstreamCredential,
  PublicUpstreamProvider,
  RoutingProvider,
  StoredUpstreamCredential,
  StoredUpstreamProvider,
  UpstreamTransport,
} from "./types";
import { MODEL_RATES } from "../billing/pricing";

export type {
  AuthContext,
  IdempotencyResult,
  ManualCreditKind,
  ModelRoute,
  ModelRouteTarget,
  ProviderModelCapability,
  PublicUpstreamCredential,
  PublicUpstreamProvider,
  RoutingProvider,
  StoredUpstreamCredential,
  StoredUpstreamProvider,
  UpstreamTransport,
} from "./types";

function safeInteger(value: bigint, label: string): number {
  if (value < BigInt(Number.MIN_SAFE_INTEGER) || value > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error(`${label} is out of range`);
  }
  return Number(value);
}

function utcDayStart(now = Date.now()): number {
  const date = new Date(now);
  return Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate());
}

export class GatewayDatabase {
  readonly db: Database;
  readonly accounts: AccountRepository;

  constructor(path = process.env.DATABASE_PATH || "./gateway.db") {
    this.db = new Database(path, { create: true, strict: true });
    this.accounts = new AccountRepository(this.db);
    this.db.run("PRAGMA journal_mode = WAL");
    this.db.run("PRAGMA foreign_keys = ON");
    this.db.run("PRAGMA busy_timeout = 5000");
    this.migrate();
  }

  private migrate() {
    runMigrations(this.db);
    const reservationTtl = Number(process.env.RESERVATION_TTL_MS || 600_000);
    this.db.run("UPDATE reservations SET expires_at=COALESCE(expires_at, created_at + ?)", [reservationTtl]);
    this.migrateLegacyProviderCredentials();
  }
  private migrateLegacyProviderCredentials() {
    const providers = this.db.query(`
      SELECT id, name, credential_ciphertext AS credentialCiphertext, enabled, weight,
             max_concurrency AS maxConcurrency, models_json AS modelsJson,
             credential_version AS credentialVersion,
             credential_updated_at AS credentialUpdatedAt, created_at AS createdAt,
             updated_at AS updatedAt
      FROM upstream_providers
      WHERE credential_ciphertext<>''
        AND NOT EXISTS (SELECT 1 FROM upstream_credentials c WHERE c.provider_id=upstream_providers.id)
    `).all() as Array<{
      id: string;
      name: string;
      credentialCiphertext: string;
      enabled: number;
      weight: number;
      maxConcurrency: number;
      modelsJson: string | null;
      credentialVersion: number;
      credentialUpdatedAt: number;
      createdAt: number;
      updatedAt: number;
    }>;
    const tx = this.db.transaction(() => {
      for (const provider of providers) {
        this.db.run(`
          INSERT INTO upstream_credentials
            (id, provider_id, name, credential_ciphertext, encryption_context, enabled, weight,
             max_concurrency, models_json, revision, credential_version,
             credential_updated_at, created_at, updated_at)
          VALUES (?, ?, 'primary', ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?)
        `, [
          randomUUID(), provider.id, provider.credentialCiphertext, provider.id, provider.enabled,
          provider.weight, provider.maxConcurrency, provider.modelsJson, provider.credentialVersion,
          provider.credentialUpdatedAt, provider.createdAt, provider.updatedAt,
        ]);
        const models = provider.modelsJson === null
          ? [...new Set(Object.values(MODEL_RATES).map((rate) => rate.upstreamModel))]
          : JSON.parse(provider.modelsJson) as string[];
        for (const model of models) {
          this.db.run(`
            INSERT OR IGNORE INTO provider_model_capabilities
              (id, provider_id, canonical_model_id, upstream_model_id, api_paths_json,
               enabled, verified_at, revision, created_at, updated_at)
            VALUES (?, ?, ?, ?, '["chat_completions","responses"]', 1, NULL, 1, ?, ?)
          `, [randomUUID(), provider.id, model, model, provider.createdAt, provider.updatedAt]);
        }
      }
      const routeCount = this.db.query("SELECT COUNT(*) AS count FROM model_routes").get() as { count: number };
      if (providers.length > 0 && routeCount.count === 0) {
        const capabilities = this.db.query(`
          SELECT c.id, c.canonical_model_id AS canonicalModelId,
                 p.weight, p.enabled, c.created_at AS createdAt
          FROM provider_model_capabilities c
          JOIN upstream_providers p ON p.id=c.provider_id
        `).all() as Array<{
          id: string; canonicalModelId: string; weight: number; enabled: number; createdAt: number;
        }>;
        for (const rate of Object.values(MODEL_RATES)) {
          const targets = capabilities.filter(
            (capability) => capability.canonicalModelId === rate.upstreamModel,
          );
          if (targets.length === 0) continue;
          const now = Date.now();
          this.db.run("INSERT INTO model_routes VALUES (?, 1, 1, ?, ?)", [rate.id, now, now]);
          for (const target of targets) {
            this.db.run(`
              INSERT INTO model_route_targets
                (id, public_model_id, capability_id, priority, weight,
                 max_concurrency, enabled, created_at, updated_at)
              VALUES (?, ?, ?, 1, ?, 10000, ?, ?, ?)
            `, [
              randomUUID(), rate.id, target.id, target.weight, target.enabled,
              target.createdAt, now,
            ]);
          }
        }
      }
    });
    tx.immediate();
  }

  static hashKey(key: string): string {
    return createHash("sha256").update(key).digest("hex");
  }

  private audit(action: string, targetType: string, targetId: string, metadata: Record<string, unknown> = {}) {
    this.db.run("INSERT INTO admin_audit_events VALUES (?, ?, ?, ?, ?, ?)", [
      randomUUID(), action, targetType, targetId, JSON.stringify(metadata), Date.now(),
    ]);
  }

  recordAudit(action: string, targetType: string, targetId: string, metadata: Record<string, unknown> = {}) {
    this.audit(action, targetType, targetId, metadata);
  }

  createCustomer(name: string) {
    return this.accounts.createCustomer(name);
  }

  registerCustomerAccount(name: string, email: string, passwordHash: string) {
    return this.accounts.registerCustomerAccount(name, email, passwordHash);
  }

  accountForAuthentication(email: string) {
    return this.accounts.accountForAuthentication(email);
  }

  createApiKey(
    customerId: string,
    args: Parameters<AccountRepository["createApiKey"]>[1],
  ) {
    return this.accounts.createApiKey(customerId, args);
  }

  listApiKeys(customerId: string) {
    return this.accounts.listApiKeys(customerId);
  }

  updateApiKey(
    customerId: string,
    apiKeyId: string,
    args: Parameters<AccountRepository["updateApiKey"]>[2],
  ) {
    return this.accounts.updateApiKey(customerId, apiKeyId, args);
  }

  revokeCustomerApiKey(customerId: string, apiKeyId: string): boolean {
    return this.accounts.revokeCustomerApiKey(customerId, apiKeyId);
  }

  revokeApiKey(apiKeyId: string): boolean {
    return this.accounts.revokeApiKey(apiKeyId);
  }

  private routingProvider(row: {
    id: string; name: string; transport: UpstreamTransport; baseUrl: string;
    enabled: number; maxConcurrency: number; revision: number; createdAt: number; updatedAt: number;
  }): RoutingProvider {
    return {
      ...row,
      baseUrl: row.transport === "codex_cli" ? null : row.baseUrl,
      enabled: row.enabled === 1,
    };
  }

  listRoutingProviders(): RoutingProvider[] {
    const rows = this.db.query(`
      SELECT id, name, transport, base_url AS baseUrl, enabled,
             max_concurrency AS maxConcurrency, revision,
             created_at AS createdAt, updated_at AS updatedAt
      FROM upstream_providers ORDER BY created_at, id
    `).all() as Parameters<GatewayDatabase["routingProvider"]>[0][];
    return rows.map((row) => this.routingProvider(row));
  }

  getRoutingProvider(providerId: string): RoutingProvider | null {
    const row = this.db.query(`
      SELECT id, name, transport, base_url AS baseUrl, enabled,
             max_concurrency AS maxConcurrency, revision,
             created_at AS createdAt, updated_at AS updatedAt
      FROM upstream_providers WHERE id=?
    `).get(providerId) as Parameters<GatewayDatabase["routingProvider"]>[0] | null;
    return row ? this.routingProvider(row) : null;
  }

  createRoutingProvider(args: {
    id: string; name: string; transport: UpstreamTransport; baseUrl: string | null;
    enabled: boolean; maxConcurrency: number;
  }): RoutingProvider {
    const now = Date.now();
    this.db.transaction(() => {
      this.db.run(`
        INSERT INTO upstream_providers
          (id, name, base_url, credential_ciphertext, enabled, weight, max_concurrency,
           models_json, revision, credential_version, credential_updated_at,
           created_at, updated_at, transport)
        VALUES (?, ?, ?, '', ?, 1, ?, NULL, 1, 1, ?, ?, ?, ?)
      `, [
        args.id, args.name, args.baseUrl ?? "", args.enabled ? 1 : 0,
        args.maxConcurrency, now, now, now, args.transport,
      ]);
      this.audit("upstream_provider.create", "upstream_provider", args.id, {
        name: args.name, transport: args.transport, enabled: args.enabled,
        maxConcurrency: args.maxConcurrency,
      });
    }).immediate();
    return this.getRoutingProvider(args.id)!;
  }

  updateRoutingProvider(providerId: string, args: {
    name?: string; baseUrl?: string | null; enabled?: boolean; maxConcurrency?: number;
    expectedRevision?: number;
  }): RoutingProvider {
    const current = this.getRoutingProvider(providerId);
    if (!current) throw new Error("Upstream provider not found");
    if (args.expectedRevision !== undefined && args.expectedRevision !== current.revision) {
      throw new Error("upstream_provider_revision_conflict");
    }
    const next = {
      name: args.name ?? current.name,
      baseUrl: args.baseUrl === undefined ? current.baseUrl : args.baseUrl,
      enabled: args.enabled ?? current.enabled,
      maxConcurrency: args.maxConcurrency ?? current.maxConcurrency,
    };
    const now = Date.now();
    this.db.transaction(() => {
      const updated = this.db.run(`
        UPDATE upstream_providers
        SET name=?, base_url=?, enabled=?, max_concurrency=?, revision=revision+1, updated_at=?
        WHERE id=? AND revision=?
      `, [
        next.name, next.baseUrl ?? "", next.enabled ? 1 : 0, next.maxConcurrency,
        now, providerId, current.revision,
      ]);
      if (updated.changes !== 1) throw new Error("upstream_provider_revision_conflict");
      this.audit("upstream_provider.update", "upstream_provider", providerId, {
        previousRevision: current.revision, enabled: next.enabled,
        maxConcurrency: next.maxConcurrency,
      });
    }).immediate();
    return this.getRoutingProvider(providerId)!;
  }

  deleteRoutingProvider(providerId: string, expectedRevision?: number): boolean {
    const current = this.getRoutingProvider(providerId);
    if (!current) return false;
    if (expectedRevision !== undefined && expectedRevision !== current.revision) {
      throw new Error("upstream_provider_revision_conflict");
    }
    this.db.transaction(() => {
      const deleted = this.db.run("DELETE FROM upstream_providers WHERE id=? AND revision=?", [
        providerId, current.revision,
      ]);
      if (deleted.changes < 1) throw new Error("upstream_provider_revision_conflict");
      this.audit("upstream_provider.delete", "upstream_provider", providerId, {
        name: current.name, revision: current.revision, transport: current.transport,
      });
    }).immediate();
    return true;
  }

  private upstreamCredential(row: {
    id: string; providerId: string; name: string; credentialCiphertext: string;
    encryptionContext: string; enabled: number; weight: number; maxConcurrency: number;
    modelsJson: string | null; revision: number; credentialVersion: number;
    credentialUpdatedAt: number; createdAt: number; updatedAt: number;
  }): StoredUpstreamCredential {
    return {
      ...row,
      enabled: row.enabled === 1,
      models: row.modelsJson === null ? null : JSON.parse(row.modelsJson) as string[],
    };
  }

  loadUpstreamCredentials(providerId?: string): StoredUpstreamCredential[] {
    const sql = `
      SELECT id, provider_id AS providerId, name,
             credential_ciphertext AS credentialCiphertext,
             encryption_context AS encryptionContext, enabled, weight,
             max_concurrency AS maxConcurrency, models_json AS modelsJson,
             revision, credential_version AS credentialVersion,
             credential_updated_at AS credentialUpdatedAt,
             created_at AS createdAt, updated_at AS updatedAt
      FROM upstream_credentials
      ${providerId ? "WHERE provider_id=?" : ""}
      ORDER BY created_at, id
    `;
    const rows = (providerId ? this.db.query(sql).all(providerId) : this.db.query(sql).all()) as Array<
      Parameters<GatewayDatabase["upstreamCredential"]>[0]
    >;
    return rows.map((row) => this.upstreamCredential(row));
  }

  getUpstreamCredential(credentialId: string): StoredUpstreamCredential | null {
    const row = this.db.query(`
      SELECT id, provider_id AS providerId, name,
             credential_ciphertext AS credentialCiphertext,
             encryption_context AS encryptionContext, enabled, weight,
             max_concurrency AS maxConcurrency, models_json AS modelsJson,
             revision, credential_version AS credentialVersion,
             credential_updated_at AS credentialUpdatedAt,
             created_at AS createdAt, updated_at AS updatedAt
      FROM upstream_credentials WHERE id=?
    `).get(credentialId) as Parameters<GatewayDatabase["upstreamCredential"]>[0] | null;
    return row ? this.upstreamCredential(row) : null;
  }

  private publicCredential(credential: StoredUpstreamCredential): PublicUpstreamCredential {
    const {
      credentialCiphertext: _ciphertext,
      encryptionContext: _context,
      ...safe
    } = credential;
    return { ...safe, hasApiKey: true };
  }

  listUpstreamCredentials(providerId: string): PublicUpstreamCredential[] {
    return this.loadUpstreamCredentials(providerId).map((credential) => this.publicCredential(credential));
  }

  createUpstreamCredential(args: {
    id: string; providerId: string; name: string; credentialCiphertext: string;
    encryptionContext: string; enabled: boolean; weight: number; maxConcurrency: number;
    models: string[] | null;
  }): PublicUpstreamCredential {
    if (!this.getRoutingProvider(args.providerId)) throw new Error("Upstream provider not found");
    const now = Date.now();
    this.db.transaction(() => {
      this.db.run(`
        INSERT INTO upstream_credentials
          (id, provider_id, name, credential_ciphertext, encryption_context, enabled,
           weight, max_concurrency, models_json, revision, credential_version,
           credential_updated_at, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 1, ?, ?, ?)
      `, [
        args.id, args.providerId, args.name, args.credentialCiphertext,
        args.encryptionContext, args.enabled ? 1 : 0, args.weight, args.maxConcurrency,
        args.models === null ? null : JSON.stringify(args.models), now, now, now,
      ]);
      this.audit("upstream_credential.create", "upstream_credential", args.id, {
        providerId: args.providerId, name: args.name, enabled: args.enabled,
        weight: args.weight, maxConcurrency: args.maxConcurrency, models: args.models,
      });
    }).immediate();
    return this.publicCredential(this.getUpstreamCredential(args.id)!);
  }

  updateUpstreamCredential(credentialId: string, args: {
    name?: string; enabled?: boolean; weight?: number; maxConcurrency?: number;
    models?: string[] | null; expectedRevision?: number;
  }): PublicUpstreamCredential {
    const current = this.getUpstreamCredential(credentialId);
    if (!current) throw new Error("Upstream credential not found");
    if (args.expectedRevision !== undefined && args.expectedRevision !== current.revision) {
      throw new Error("upstream_credential_revision_conflict");
    }
    const next = {
      name: args.name ?? current.name,
      enabled: args.enabled ?? current.enabled,
      weight: args.weight ?? current.weight,
      maxConcurrency: args.maxConcurrency ?? current.maxConcurrency,
      models: args.models === undefined ? current.models : args.models,
    };
    const now = Date.now();
    this.db.transaction(() => {
      const updated = this.db.run(`
        UPDATE upstream_credentials
        SET name=?, enabled=?, weight=?, max_concurrency=?, models_json=?,
            revision=revision+1, updated_at=?
        WHERE id=? AND revision=?
      `, [
        next.name, next.enabled ? 1 : 0, next.weight, next.maxConcurrency,
        next.models === null ? null : JSON.stringify(next.models), now,
        credentialId, current.revision,
      ]);
      if (updated.changes !== 1) throw new Error("upstream_credential_revision_conflict");
      this.audit("upstream_credential.update", "upstream_credential", credentialId, {
        providerId: current.providerId, previousRevision: current.revision,
        enabled: next.enabled, weight: next.weight,
        maxConcurrency: next.maxConcurrency, models: next.models,
      });
    }).immediate();
    return this.publicCredential(this.getUpstreamCredential(credentialId)!);
  }

  rotateUpstreamCredential(credentialId: string, args: {
    credentialCiphertext: string; encryptionContext: string; expectedRevision?: number;
  }): PublicUpstreamCredential {
    const current = this.getUpstreamCredential(credentialId);
    if (!current) throw new Error("Upstream credential not found");
    if (args.expectedRevision !== undefined && args.expectedRevision !== current.revision) {
      throw new Error("upstream_credential_revision_conflict");
    }
    const now = Date.now();
    this.db.transaction(() => {
      const updated = this.db.run(`
        UPDATE upstream_credentials
        SET credential_ciphertext=?, encryption_context=?,
            credential_version=credential_version+1, credential_updated_at=?,
            revision=revision+1, updated_at=?
        WHERE id=? AND revision=?
      `, [
        args.credentialCiphertext, args.encryptionContext, now, now,
        credentialId, current.revision,
      ]);
      if (updated.changes !== 1) throw new Error("upstream_credential_revision_conflict");
      this.audit("upstream_credential.rotate_key", "upstream_credential", credentialId, {
        providerId: current.providerId, previousRevision: current.revision,
        previousCredentialVersion: current.credentialVersion,
      });
    }).immediate();
    return this.publicCredential(this.getUpstreamCredential(credentialId)!);
  }

  deleteUpstreamCredential(credentialId: string, expectedRevision?: number): boolean {
    const current = this.getUpstreamCredential(credentialId);
    if (!current) return false;
    if (expectedRevision !== undefined && expectedRevision !== current.revision) {
      throw new Error("upstream_credential_revision_conflict");
    }
    this.db.transaction(() => {
      const deleted = this.db.run("DELETE FROM upstream_credentials WHERE id=? AND revision=?", [
        credentialId, current.revision,
      ]);
      if (deleted.changes < 1) throw new Error("upstream_credential_revision_conflict");
      this.audit("upstream_credential.delete", "upstream_credential", credentialId, {
        providerId: current.providerId, name: current.name,
        revision: current.revision, credentialVersion: current.credentialVersion,
      });
    }).immediate();
    return true;
  }

  private modelCapability(row: {
    id: string; providerId: string; canonicalModelId: string; upstreamModelId: string;
    apiPathsJson: string; enabled: number; verifiedAt: number | null; revision: number;
    createdAt: number; updatedAt: number;
  }): ProviderModelCapability {
    return {
      ...row,
      apiPaths: JSON.parse(row.apiPathsJson) as ProviderModelCapability["apiPaths"],
      enabled: row.enabled === 1,
    };
  }

  listProviderModelCapabilities(providerId?: string): ProviderModelCapability[] {
    const sql = `
      SELECT id, provider_id AS providerId, canonical_model_id AS canonicalModelId,
             upstream_model_id AS upstreamModelId, api_paths_json AS apiPathsJson,
             enabled, verified_at AS verifiedAt, revision,
             created_at AS createdAt, updated_at AS updatedAt
      FROM provider_model_capabilities
      ${providerId ? "WHERE provider_id=?" : ""}
      ORDER BY created_at, id
    `;
    const rows = (providerId ? this.db.query(sql).all(providerId) : this.db.query(sql).all()) as Array<
      Parameters<GatewayDatabase["modelCapability"]>[0]
    >;
    return rows.map((row) => this.modelCapability(row));
  }

  getProviderModelCapability(capabilityId: string): ProviderModelCapability | null {
    return this.listProviderModelCapabilities().find((capability) => capability.id === capabilityId) ?? null;
  }

  createProviderModelCapability(args: {
    id: string; providerId: string; canonicalModelId: string; upstreamModelId: string;
    apiPaths: ProviderModelCapability["apiPaths"]; enabled: boolean;
  }): ProviderModelCapability {
    if (!this.getRoutingProvider(args.providerId)) throw new Error("Upstream provider not found");
    const now = Date.now();
    this.db.transaction(() => {
      this.db.run(`
        INSERT INTO provider_model_capabilities
          (id, provider_id, canonical_model_id, upstream_model_id, api_paths_json,
           enabled, verified_at, revision, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, NULL, 1, ?, ?)
      `, [
        args.id, args.providerId, args.canonicalModelId, args.upstreamModelId,
        JSON.stringify(args.apiPaths), args.enabled ? 1 : 0, now, now,
      ]);
      this.audit("upstream_model.create", "upstream_model", args.id, {
        providerId: args.providerId, canonicalModelId: args.canonicalModelId,
        upstreamModelId: args.upstreamModelId, apiPaths: args.apiPaths, enabled: args.enabled,
      });
    }).immediate();
    return this.getProviderModelCapability(args.id)!;
  }

  updateProviderModelCapability(capabilityId: string, args: {
    upstreamModelId?: string; apiPaths?: ProviderModelCapability["apiPaths"];
    enabled?: boolean; verifiedAt?: number | null; expectedRevision?: number;
  }): ProviderModelCapability {
    const current = this.getProviderModelCapability(capabilityId);
    if (!current) throw new Error("Upstream model capability not found");
    if (args.expectedRevision !== undefined && args.expectedRevision !== current.revision) {
      throw new Error("upstream_model_revision_conflict");
    }
    const now = Date.now();
    const next = {
      upstreamModelId: args.upstreamModelId ?? current.upstreamModelId,
      apiPaths: args.apiPaths ?? current.apiPaths,
      enabled: args.enabled ?? current.enabled,
      verifiedAt: args.verifiedAt === undefined ? current.verifiedAt : args.verifiedAt,
    };
    this.db.transaction(() => {
      const updated = this.db.run(`
        UPDATE provider_model_capabilities
        SET upstream_model_id=?, api_paths_json=?, enabled=?, verified_at=?,
            revision=revision+1, updated_at=?
        WHERE id=? AND revision=?
      `, [
        next.upstreamModelId, JSON.stringify(next.apiPaths), next.enabled ? 1 : 0,
        next.verifiedAt, now, capabilityId, current.revision,
      ]);
      if (updated.changes !== 1) throw new Error("upstream_model_revision_conflict");
      this.audit("upstream_model.update", "upstream_model", capabilityId, {
        providerId: current.providerId, previousRevision: current.revision,
        upstreamModelId: next.upstreamModelId, apiPaths: next.apiPaths,
        enabled: next.enabled, verifiedAt: next.verifiedAt,
      });
    }).immediate();
    return this.getProviderModelCapability(capabilityId)!;
  }

  deleteProviderModelCapability(capabilityId: string, expectedRevision?: number): boolean {
    const current = this.getProviderModelCapability(capabilityId);
    if (!current) return false;
    if (expectedRevision !== undefined && expectedRevision !== current.revision) {
      throw new Error("upstream_model_revision_conflict");
    }
    this.db.transaction(() => {
      const deleted = this.db.run("DELETE FROM provider_model_capabilities WHERE id=? AND revision=?", [
        capabilityId, current.revision,
      ]);
      if (deleted.changes < 1) throw new Error("upstream_model_revision_conflict");
      this.audit("upstream_model.delete", "upstream_model", capabilityId, {
        providerId: current.providerId, canonicalModelId: current.canonicalModelId,
        revision: current.revision,
      });
    }).immediate();
    return true;
  }

  getModelRoute(publicModelId: string): ModelRoute | null {
    const row = this.db.query(`
      SELECT public_model_id AS publicModelId, enabled, revision,
             created_at AS createdAt, updated_at AS updatedAt
      FROM model_routes WHERE public_model_id=?
    `).get(publicModelId) as {
      publicModelId: string; enabled: number; revision: number; createdAt: number; updatedAt: number;
    } | null;
    if (!row) return null;
    const targets = this.db.query(`
      SELECT id, public_model_id AS publicModelId, capability_id AS capabilityId,
             priority, weight, max_concurrency AS maxConcurrency, enabled,
             created_at AS createdAt, updated_at AS updatedAt
      FROM model_route_targets WHERE public_model_id=? ORDER BY priority, created_at, id
    `).all(publicModelId) as Array<Omit<ModelRouteTarget, "enabled"> & { enabled: number }>;
    return {
      ...row,
      enabled: row.enabled === 1,
      targets: targets.map((target) => ({ ...target, enabled: target.enabled === 1 })),
    };
  }

  listModelRoutes(): ModelRoute[] {
    const ids = this.db.query("SELECT public_model_id AS publicModelId FROM model_routes ORDER BY public_model_id")
      .all() as Array<{ publicModelId: string }>;
    return ids.map(({ publicModelId }) => this.getModelRoute(publicModelId)!);
  }

  publishModelRoute(args: {
    publicModelId: string; enabled: boolean; expectedRevision?: number;
    targets: Array<{
      id?: string; capabilityId: string; priority: number; weight: number;
      maxConcurrency: number; enabled: boolean;
    }>;
  }): ModelRoute {
    const current = this.getModelRoute(args.publicModelId);
    if (args.expectedRevision !== undefined && args.expectedRevision !== (current?.revision ?? 0)) {
      throw new Error("model_route_revision_conflict");
    }
    const now = Date.now();
    const nextRevision = (current?.revision ?? 0) + 1;
    this.db.transaction(() => {
      if (current) {
        const updated = this.db.run(`
          UPDATE model_routes SET enabled=?, revision=?, updated_at=?
          WHERE public_model_id=? AND revision=?
        `, [args.enabled ? 1 : 0, nextRevision, now, args.publicModelId, current.revision]);
        if (updated.changes !== 1) throw new Error("model_route_revision_conflict");
        this.db.run("DELETE FROM model_route_targets WHERE public_model_id=?", [args.publicModelId]);
      } else {
        this.db.run("INSERT INTO model_routes VALUES (?, ?, 1, ?, ?)", [
          args.publicModelId, args.enabled ? 1 : 0, now, now,
        ]);
      }
      for (const target of args.targets) {
        this.db.run(`
          INSERT INTO model_route_targets
            (id, public_model_id, capability_id, priority, weight,
             max_concurrency, enabled, created_at, updated_at)
          VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        `, [
          target.id ?? randomUUID(), args.publicModelId, target.capabilityId,
          target.priority, target.weight, target.maxConcurrency,
          target.enabled ? 1 : 0, now, now,
        ]);
      }
      this.audit("model_route.publish", "model_route", args.publicModelId, {
        previousRevision: current?.revision ?? 0, revision: nextRevision,
        enabled: args.enabled, targets: args.targets.map((target) => ({
          capabilityId: target.capabilityId, priority: target.priority,
          weight: target.weight, maxConcurrency: target.maxConcurrency,
          enabled: target.enabled,
        })),
      });
    }).immediate();
    return this.getModelRoute(args.publicModelId)!;
  }

  private upstreamProvider(row: {
    id: string;
    name: string;
    baseUrl: string;
    credentialCiphertext: string;
    enabled: number;
    weight: number;
    maxConcurrency: number;
    modelsJson: string | null;
    revision: number;
    credentialVersion: number;
    credentialUpdatedAt: number;
    createdAt: number;
    updatedAt: number;
  }): StoredUpstreamProvider {
    const models = row.modelsJson === null ? null : JSON.parse(row.modelsJson) as unknown;
    if (models !== null && (!Array.isArray(models) || models.some((model) => typeof model !== "string"))) {
      throw new Error(`Upstream provider ${row.id} has invalid stored models`);
    }
    return {
      id: row.id,
      name: row.name,
      baseUrl: row.baseUrl,
      credentialCiphertext: row.credentialCiphertext,
      enabled: row.enabled === 1,
      weight: row.weight,
      maxConcurrency: row.maxConcurrency,
      models: models as string[] | null,
      revision: row.revision,
      credentialVersion: row.credentialVersion,
      credentialUpdatedAt: row.credentialUpdatedAt,
      createdAt: row.createdAt,
      updatedAt: row.updatedAt,
    };
  }

  private publicUpstreamProvider(provider: StoredUpstreamProvider): PublicUpstreamProvider {
    const { credentialCiphertext: _credentialCiphertext, ...safe } = provider;
    return { ...safe, hasApiKey: true };
  }

  loadUpstreamProviders(): StoredUpstreamProvider[] {
    const rows = this.db.query(`
      SELECT id, name, base_url AS baseUrl, credential_ciphertext AS credentialCiphertext,
             enabled, weight, max_concurrency AS maxConcurrency, models_json AS modelsJson,
             revision, credential_version AS credentialVersion,
             credential_updated_at AS credentialUpdatedAt, created_at AS createdAt,
             updated_at AS updatedAt
      FROM upstream_providers ORDER BY created_at, id
    `).all() as Parameters<GatewayDatabase["upstreamProvider"]>[0][];
    return rows.map((row) => this.upstreamProvider(row));
  }

  listUpstreamProviders(): PublicUpstreamProvider[] {
    return this.loadUpstreamProviders().map((provider) => this.publicUpstreamProvider(provider));
  }

  getUpstreamProvider(providerId: string): StoredUpstreamProvider | null {
    const row = this.db.query(`
      SELECT id, name, base_url AS baseUrl, credential_ciphertext AS credentialCiphertext,
             enabled, weight, max_concurrency AS maxConcurrency, models_json AS modelsJson,
             revision, credential_version AS credentialVersion,
             credential_updated_at AS credentialUpdatedAt, created_at AS createdAt,
             updated_at AS updatedAt
      FROM upstream_providers WHERE id=?
    `).get(providerId) as Parameters<GatewayDatabase["upstreamProvider"]>[0] | null;
    return row ? this.upstreamProvider(row) : null;
  }

  createUpstreamProvider(args: {
    id: string;
    name: string;
    baseUrl: string;
    credentialCiphertext: string;
    enabled: boolean;
    weight: number;
    maxConcurrency: number;
    models: string[] | null;
  }): PublicUpstreamProvider {
    const name = args.name.trim();
    if (!/^[^\u0000-\u001f]{1,80}$/.test(name)) throw new Error("Invalid upstream provider name");
    const now = Date.now();
    const tx = this.db.transaction(() => {
      this.db.run(`
        INSERT INTO upstream_providers
          (id, name, base_url, credential_ciphertext, enabled, weight, max_concurrency,
           models_json, revision, credential_version, credential_updated_at, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, 1, ?, ?, ?)
      `, [
        args.id, name, args.baseUrl, args.credentialCiphertext, args.enabled ? 1 : 0,
        args.weight, args.maxConcurrency, args.models === null ? null : JSON.stringify(args.models),
        now, now, now,
      ]);
      this.audit("upstream_provider.create", "upstream_provider", args.id, {
        name,
        enabled: args.enabled,
        weight: args.weight,
        maxConcurrency: args.maxConcurrency,
        models: args.models,
      });
    });
    tx.immediate();
    return this.publicUpstreamProvider(this.getUpstreamProvider(args.id)!);
  }

  updateUpstreamProvider(providerId: string, args: {
    name?: string;
    baseUrl?: string;
    enabled?: boolean;
    weight?: number;
    maxConcurrency?: number;
    models?: string[] | null;
    expectedRevision?: number;
  }): PublicUpstreamProvider {
    const current = this.getUpstreamProvider(providerId);
    if (!current) throw new Error("Upstream provider not found");
    if (args.expectedRevision !== undefined && args.expectedRevision !== current.revision) {
      throw new Error("upstream_provider_revision_conflict");
    }
    const name = args.name === undefined ? current.name : args.name.trim();
    if (!/^[^\u0000-\u001f]{1,80}$/.test(name)) throw new Error("Invalid upstream provider name");
    const next = {
      name,
      baseUrl: args.baseUrl ?? current.baseUrl,
      enabled: args.enabled ?? current.enabled,
      weight: args.weight ?? current.weight,
      maxConcurrency: args.maxConcurrency ?? current.maxConcurrency,
      models: args.models === undefined ? current.models : args.models,
    };
    const now = Date.now();
    const tx = this.db.transaction(() => {
      const updated = this.db.run(`
        UPDATE upstream_providers
        SET name=?, base_url=?, enabled=?, weight=?, max_concurrency=?, models_json=?,
            revision=revision+1, updated_at=?
        WHERE id=? AND revision=?
      `, [
        next.name, next.baseUrl, next.enabled ? 1 : 0, next.weight, next.maxConcurrency,
        next.models === null ? null : JSON.stringify(next.models), now, providerId, current.revision,
      ]);
      if (updated.changes !== 1) throw new Error("upstream_provider_revision_conflict");
      this.audit("upstream_provider.update", "upstream_provider", providerId, {
        previousRevision: current.revision,
        enabled: next.enabled,
        weight: next.weight,
        maxConcurrency: next.maxConcurrency,
        models: next.models,
      });
    });
    tx.immediate();
    return this.publicUpstreamProvider(this.getUpstreamProvider(providerId)!);
  }

  rotateUpstreamProviderKey(
    providerId: string,
    credentialCiphertext: string,
    expectedRevision?: number,
  ): PublicUpstreamProvider {
    const current = this.getUpstreamProvider(providerId);
    if (!current) throw new Error("Upstream provider not found");
    if (expectedRevision !== undefined && expectedRevision !== current.revision) {
      throw new Error("upstream_provider_revision_conflict");
    }
    const now = Date.now();
    const tx = this.db.transaction(() => {
      const updated = this.db.run(`
        UPDATE upstream_providers
        SET credential_ciphertext=?, credential_version=credential_version+1,
            credential_updated_at=?, revision=revision+1, updated_at=?
        WHERE id=? AND revision=?
      `, [credentialCiphertext, now, now, providerId, current.revision]);
      if (updated.changes !== 1) throw new Error("upstream_provider_revision_conflict");
      this.audit("upstream_provider.rotate_key", "upstream_provider", providerId, {
        previousRevision: current.revision,
        previousCredentialVersion: current.credentialVersion,
      });
    });
    tx.immediate();
    return this.publicUpstreamProvider(this.getUpstreamProvider(providerId)!);
  }

  deleteUpstreamProvider(providerId: string, expectedRevision?: number): boolean {
    const current = this.getUpstreamProvider(providerId);
    if (!current) return false;
    if (expectedRevision !== undefined && expectedRevision !== current.revision) {
      throw new Error("upstream_provider_revision_conflict");
    }
    const tx = this.db.transaction(() => {
      const deleted = this.db.run("DELETE FROM upstream_providers WHERE id=? AND revision=?", [
        providerId, current.revision,
      ]);
      if (deleted.changes !== 1) throw new Error("upstream_provider_revision_conflict");
      this.audit("upstream_provider.delete", "upstream_provider", providerId, {
        name: current.name,
        revision: current.revision,
        credentialVersion: current.credentialVersion,
      });
    });
    tx.immediate();
    return true;
  }

  updateCustomer(customerId: string, args: {
    active?: boolean; dailySpendLimitMicros?: bigint | null; perRequestLimitMicros?: bigint | null;
  }) {
    const current = this.db.query("SELECT id, active, daily_spend_limit_micros AS dailyLimit, per_request_limit_micros AS requestLimit FROM customers WHERE id=?").get(customerId) as {
      id: string; active: number; dailyLimit: number | null; requestLimit: number | null;
    } | null;
    if (!current) throw new Error("Customer not found");
    const daily = args.dailySpendLimitMicros === undefined ? current.dailyLimit
      : args.dailySpendLimitMicros === null ? null : safeInteger(args.dailySpendLimitMicros, "Daily limit");
    const perRequest = args.perRequestLimitMicros === undefined ? current.requestLimit
      : args.perRequestLimitMicros === null ? null : safeInteger(args.perRequestLimitMicros, "Request limit");
    if (daily !== null && daily <= 0) throw new Error("Daily limit must be positive");
    if (perRequest !== null && perRequest <= 0) throw new Error("Request limit must be positive");
    const active = args.active === undefined ? current.active : args.active ? 1 : 0;
    const tx = this.db.transaction(() => {
      this.db.run("UPDATE customers SET active=?, daily_spend_limit_micros=?, per_request_limit_micros=?, updated_at=? WHERE id=?", [
        active, daily, perRequest, Date.now(), customerId,
      ]);
      this.audit(active ? "customer.update" : "customer.freeze", "customer", customerId, { active: Boolean(active), daily, perRequest });
    });
    tx.immediate();
    return this.getCustomer(customerId);
  }

  getCustomer(customerId: string) {
    return this.db.query(`
      SELECT c.id, c.name, c.active, c.daily_spend_limit_micros, c.per_request_limit_micros,
             c.created_at, c.updated_at, a.email
      FROM customers c
      LEFT JOIN customer_accounts a ON a.customer_id=c.id
      WHERE c.id=?
    `).get(customerId);
  }

  listCustomers() {
    return this.db.query(`
      SELECT c.id, c.name, c.active, c.daily_spend_limit_micros, c.per_request_limit_micros,
             c.created_at, c.updated_at, a.email,
             COALESCE((
               SELECT SUM(l.amount_micros) FROM ledger_entries l WHERE l.customer_id=c.id
             ), 0) AS posted_balance_micros,
             (SELECT COUNT(*) FROM api_keys k WHERE k.customer_id=c.id AND k.active=1) AS active_key_count
      FROM customers c
      LEFT JOIN customer_accounts a ON a.customer_id=c.id
      ORDER BY c.created_at DESC
    `).all();
  }

  authenticate(secret: string): AuthContext | null {
    return this.db.query(`
      SELECT k.id AS apiKeyId, k.customer_id AS customerId, k.max_concurrency AS maxConcurrency,
             k.rate_limit_per_minute AS rateLimitPerMinute,
             k.spend_limit_micros AS keySpendLimitMicros,
             c.daily_spend_limit_micros AS dailySpendLimitMicros,
             c.per_request_limit_micros AS perRequestLimitMicros
      FROM api_keys k JOIN customers c ON c.id=k.customer_id
      WHERE k.key_hash=? AND k.active=1 AND c.active=1
        AND (k.expires_at IS NULL OR k.expires_at>?)
    `).get(GatewayDatabase.hashKey(secret), Date.now()) as AuthContext | null;
  }

  postedBalanceMicros(customerId: string): bigint {
    const row = this.db.query("SELECT COALESCE(SUM(amount_micros), 0) AS amount FROM ledger_entries WHERE customer_id=?").get(customerId) as { amount: number };
    return BigInt(row.amount);
  }

  activeHoldsMicros(customerId: string): bigint {
    const row = this.db.query("SELECT COALESCE(SUM(amount_micros), 0) AS amount FROM reservations WHERE customer_id=? AND status='active'").get(customerId) as { amount: number };
    return BigInt(row.amount);
  }

  balanceMicros(customerId: string): bigint {
    return this.postedBalanceMicros(customerId) - this.activeHoldsMicros(customerId);
  }

  reserve(args: {
    customerId: string; apiKeyId: string; requestId: string; amountMicros: bigint;
    keySpendLimitMicros?: number | null; perRequestLimitMicros?: number | null;
    dailySpendLimitMicros?: number | null; now?: number;
  }): string {
    const amount = safeInteger(args.amountMicros, "Reservation");
    if (amount <= 0) throw new Error("Invalid reservation");
    if (args.perRequestLimitMicros !== null && args.perRequestLimitMicros !== undefined && amount > args.perRequestLimitMicros) {
      throw new Error("per_request_limit");
    }
    const id = randomUUID();
    const now = args.now ?? Date.now();
    const expiresAt = now + Number(process.env.RESERVATION_TTL_MS || 600_000);
    const tx = this.db.transaction(() => {
      if (this.balanceMicros(args.customerId) < args.amountMicros) throw new Error("insufficient_balance");
      if (args.keySpendLimitMicros !== null && args.keySpendLimitMicros !== undefined) {
        const spent = this.db.query(`
          SELECT COALESCE(SUM(cost_micros), 0) AS amount
          FROM usage_events WHERE api_key_id=?
        `).get(args.apiKeyId) as { amount: number };
        const holds = this.db.query(`
          SELECT COALESCE(SUM(amount_micros), 0) AS amount
          FROM reservations WHERE api_key_id=? AND status='active'
        `).get(args.apiKeyId) as { amount: number };
        if (BigInt(spent.amount) + BigInt(holds.amount) + args.amountMicros > BigInt(args.keySpendLimitMicros)) {
          throw new Error("key_spend_limit");
        }
      }
      if (args.dailySpendLimitMicros !== null && args.dailySpendLimitMicros !== undefined) {
        const spent = this.db.query("SELECT COALESCE(SUM(cost_micros), 0) AS amount FROM usage_events WHERE customer_id=? AND created_at>=?").get(args.customerId, utcDayStart(now)) as { amount: number };
        const holds = this.activeHoldsMicros(args.customerId);
        if (BigInt(spent.amount) + holds + args.amountMicros > BigInt(args.dailySpendLimitMicros)) throw new Error("daily_spend_limit");
      }
      this.db.run(`
        INSERT INTO reservations
          (id, customer_id, api_key_id, request_id, amount_micros, status, created_at, expires_at, settled_at)
        VALUES (?, ?, ?, ?, ?, 'active', ?, ?, NULL)
      `, [id, args.customerId, args.apiKeyId, args.requestId, amount, now, expiresAt]);
    });
    tx.immediate();
    return id;
  }

  release(reservationId: string) {
    this.db.run("UPDATE reservations SET status='released', settled_at=? WHERE id=? AND status='active'", [Date.now(), reservationId]);
  }

  beginIdempotentRequest(apiKeyId: string, idempotencyKey: string, requestHash: string): IdempotencyResult {
    const tx = this.db.transaction((): IdempotencyResult => {
      const existing = this.db.query(`
        SELECT request_hash AS requestHash, request_id AS requestId, status,
               response_json AS responseJson, response_status AS responseStatus
        FROM idempotency_requests WHERE api_key_id=? AND idempotency_key=?
      `).get(apiKeyId, idempotencyKey) as {
        requestHash: string; requestId: string; status: string;
        responseJson: string | null; responseStatus: number | null;
      } | null;
      if (!existing) {
        const requestId = randomUUID();
        const now = Date.now();
        this.db.run("INSERT INTO idempotency_requests VALUES (?, ?, ?, ?, ?, 'in_progress', NULL, NULL, ?, ?)", [
          randomUUID(), apiKeyId, idempotencyKey, requestHash, requestId, now, now,
        ]);
        return { state: "started", requestId };
      }
      if (existing.requestHash !== requestHash) return { state: "conflict" };
      if (existing.status === "in_progress") return { state: "in_progress" };
      if (existing.status === "completed" && existing.responseJson && existing.responseStatus !== null) {
        return { state: "completed", requestId: existing.requestId, payload: JSON.parse(existing.responseJson) as Record<string, unknown>, status: existing.responseStatus };
      }
      const requestId = randomUUID();
      this.db.run(`
        UPDATE idempotency_requests SET request_id=?, status='in_progress', response_json=NULL,
          response_status=NULL, updated_at=? WHERE api_key_id=? AND idempotency_key=?
      `, [requestId, Date.now(), apiKeyId, idempotencyKey]);
      return { state: "started", requestId };
    });
    return tx.immediate();
  }

  failIdempotentRequest(apiKeyId: string, idempotencyKey: string | null) {
    if (!idempotencyKey) return;
    this.db.run("UPDATE idempotency_requests SET status='failed', updated_at=? WHERE api_key_id=? AND idempotency_key=? AND status='in_progress'", [
      Date.now(), apiKeyId, idempotencyKey,
    ]);
  }

  settle(args: {
    reservationId: string; requestId: string; customerId: string; apiKeyId: string; model: string;
    rateCardVersion?: string; inputTokens: number; cachedInputTokens: number; cacheWriteTokens: number;
    outputTokens: number; costMicros: bigint; upstreamStatus: number; idempotencyKey?: string | null;
    responseJson?: string; responseStatus?: number; routeRevision?: number; routeTargetId?: string;
  }) {
    const cost = safeInteger(args.costMicros, "Charge");
    if (cost < 0) throw new Error("Charge too large");
    const tx = this.db.transaction(() => {
      const reservation = this.db.query("SELECT status, amount_micros AS amountMicros FROM reservations WHERE id=?").get(args.reservationId) as { status: string; amountMicros: number } | null;
      if (!reservation || reservation.status !== "active") throw new Error("Reservation is not active");
      if (cost > reservation.amountMicros) throw new Error("Charge exceeds reservation");
      const balanceBefore = this.postedBalanceMicros(args.customerId);
      const balanceAfter = balanceBefore - args.costMicros;
      const usageId = randomUUID();
      const createdAt = Date.now();
      this.db.run(`
        INSERT INTO usage_events
          (id, customer_id, api_key_id, request_id, model, rate_card_version, input_tokens,
           cached_input_tokens, cache_write_tokens, output_tokens, cost_micros,
           balance_before_micros, balance_after_micros, upstream_status,
           route_revision, route_target_id, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      `, [
        usageId, args.customerId, args.apiKeyId, args.requestId, args.model,
        args.rateCardVersion || "legacy", args.inputTokens, args.cachedInputTokens,
        args.cacheWriteTokens, args.outputTokens, cost, safeInteger(balanceBefore, "Balance"),
        safeInteger(balanceAfter, "Balance"), args.upstreamStatus,
        args.routeRevision ?? null, args.routeTargetId ?? null, createdAt,
      ]);
      this.db.run("INSERT INTO ledger_entries VALUES (?, ?, 'usage_charge', ?, ?, ?, ?)", [
        randomUUID(), args.customerId, -cost, `usage:${args.requestId}`,
        JSON.stringify({ model: args.model, usageId, rateCardVersion: args.rateCardVersion || "legacy" }), createdAt,
      ]);
      this.db.run("UPDATE reservations SET status='settled', settled_at=? WHERE id=?", [createdAt, args.reservationId]);
      if (args.idempotencyKey) {
        if (!args.responseJson || args.responseStatus === undefined) throw new Error("Idempotent response is missing");
        const updated = this.db.run(`
          UPDATE idempotency_requests SET status='completed', response_json=?, response_status=?, updated_at=?
          WHERE api_key_id=? AND idempotency_key=? AND request_id=? AND status='in_progress'
        `, [args.responseJson, args.responseStatus, createdAt, args.apiKeyId, args.idempotencyKey, args.requestId]);
        if (updated.changes !== 1) throw new Error("Idempotent request is not active");
      }
    });
    tx.immediate();
  }

  creditManual(args: {
    customerId: string; amountMicros: bigint; kind: ManualCreditKind; reference: string; note?: string;
  }) {
    const amount = safeInteger(args.amountMicros, "Credit");
    if (amount === 0) throw new Error("Credit amount cannot be zero");
    if (args.kind !== "manual_adjustment" && amount < 0) throw new Error("Credit amount must be positive");
    const cleanReference = args.reference.trim();
    if (!/^[A-Za-z0-9._:-]{1,160}$/.test(cleanReference)) throw new Error("Invalid credit reference");
    const reference = `credit:${args.kind}:${cleanReference}`;
    const metadata = { sourceReference: cleanReference, note: args.note?.slice(0, 500) || "" };
    const tx = this.db.transaction(() => {
      const existing = this.db.query("SELECT id, customer_id AS customerId, amount_micros AS amount, metadata_json AS metadata FROM ledger_entries WHERE reference=?").get(reference) as {
        id: string; customerId: string; amount: number; metadata: string;
      } | null;
      if (existing) {
        if (existing.customerId !== args.customerId || existing.amount !== amount || existing.metadata !== JSON.stringify(metadata)) throw new Error("credit_reference_conflict");
        return { ledgerEntryId: existing.id, creditedMicros: BigInt(existing.amount), replayed: true };
      }
      const customer = this.db.query("SELECT id FROM customers WHERE id=?").get(args.customerId);
      if (!customer) throw new Error("Customer not found");
      const id = randomUUID();
      this.db.run("INSERT INTO ledger_entries VALUES (?, ?, ?, ?, ?, ?, ?)", [
        id, args.customerId, args.kind, amount, reference, JSON.stringify(metadata), Date.now(),
      ]);
      this.audit("credit.create", "customer", args.customerId, { kind: args.kind, amountMicros: amount, reference: cleanReference });
      return { ledgerEntryId: id, creditedMicros: BigInt(amount), replayed: false };
    });
    return tx.immediate();
  }

  creditUsdc(args: { customerId: string; chain: string; txHash: string; logIndex: number; amountAtomic: bigint; confirmations: number }) {
    const amount = safeInteger(args.amountAtomic, "USDC amount");
    if (amount <= 0) throw new Error("Invalid USDC amount");
    const id = randomUUID();
    const reference = `usdc:${args.chain}:${args.txHash.toLowerCase()}:${args.logIndex}`;
    const tx = this.db.transaction(() => {
      this.db.run("INSERT INTO usdc_deposits VALUES (?, ?, ?, ?, ?, ?, ?, ?)", [
        id, args.customerId, args.chain, args.txHash.toLowerCase(), args.logIndex,
        amount, args.confirmations, Date.now(),
      ]);
      this.db.run("INSERT INTO ledger_entries VALUES (?, ?, 'usdc_deposit', ?, ?, ?, ?)", [
        randomUUID(), args.customerId, amount, reference, JSON.stringify({ depositId: id }), Date.now(),
      ]);
    });
    tx.immediate();
    return { depositId: id, creditedMicros: args.amountAtomic };
  }

  recoverStale(now = Date.now()) {
    const idempotencyCutoff = now - Number(process.env.IDEMPOTENCY_IN_PROGRESS_TTL_MS || 900_000);
    const tx = this.db.transaction(() => {
      const stale = this.db.query("SELECT id, request_id AS requestId FROM reservations WHERE status='active' AND expires_at<=?").all(now) as Array<{ id: string; requestId: string | null }>;
      for (const reservation of stale) {
        this.db.run("UPDATE reservations SET status='released', settled_at=? WHERE id=? AND status='active'", [now, reservation.id]);
        if (reservation.requestId) {
          this.db.run("UPDATE idempotency_requests SET status='failed', updated_at=? WHERE request_id=? AND status='in_progress'", [now, reservation.requestId]);
        }
      }
      const orphaned = this.db.run(`
        UPDATE idempotency_requests SET status='failed', updated_at=?
        WHERE status='in_progress' AND updated_at<=?
          AND NOT EXISTS (
            SELECT 1 FROM reservations r
            WHERE r.request_id=idempotency_requests.request_id AND r.status='active'
          )
      `, [now, idempotencyCutoff]);
      if (stale.length || orphaned.changes) this.audit("recovery.run", "system", "gateway", { releasedReservations: stale.length, failedIdempotency: orphaned.changes });
      return { releasedReservations: stale.length, failedIdempotency: orphaned.changes };
    });
    return tx.immediate();
  }

  reconcile(customerId: string) {
    const customer = this.getCustomer(customerId);
    if (!customer) throw new Error("Customer not found");
    const ledger = this.db.query(`
      SELECT COALESCE(SUM(amount_micros), 0) AS posted,
             COALESCE(SUM(CASE WHEN amount_micros>0 THEN amount_micros ELSE 0 END), 0) AS credits,
             COALESCE(SUM(CASE WHEN amount_micros<0 THEN -amount_micros ELSE 0 END), 0) AS debits,
             COALESCE(SUM(CASE WHEN kind='usage_charge' THEN -amount_micros ELSE 0 END), 0) AS usageDebits
      FROM ledger_entries WHERE customer_id=?
    `).get(customerId) as { posted: number; credits: number; debits: number; usageDebits: number };
    const usage = this.db.query("SELECT COALESCE(SUM(cost_micros), 0) AS charges, COUNT(*) AS count FROM usage_events WHERE customer_id=?").get(customerId) as { charges: number; count: number };
    const missing = this.db.query(`
      SELECT COUNT(*) AS count FROM usage_events u
      WHERE u.customer_id=? AND NOT EXISTS (
        SELECT 1 FROM ledger_entries l WHERE l.reference='usage:' || u.request_id
      )
    `).get(customerId) as { count: number };
    const holds = this.activeHoldsMicros(customerId);
    const available = BigInt(ledger.posted) - holds;
    return {
      customerId,
      postedBalanceMicros: ledger.posted,
      activeHoldsMicros: safeInteger(holds, "Holds"),
      availableBalanceMicros: safeInteger(available, "Balance"),
      totalCreditsMicros: ledger.credits,
      totalDebitsMicros: ledger.debits,
      usageLedgerDebitsMicros: ledger.usageDebits,
      usageChargesMicros: usage.charges,
      usageCount: usage.count,
      missingUsageLedgerEntries: missing.count,
      reconciled: missing.count === 0 && ledger.usageDebits === usage.charges,
    };
  }

  usage(customerId: string, limit = 100) {
    return this.db.query(`
      SELECT u.request_id, u.api_key_id, k.name AS api_key_name, u.model, u.rate_card_version,
             u.input_tokens, u.cached_input_tokens, u.cache_write_tokens, u.output_tokens,
             u.cost_micros, u.balance_before_micros, u.balance_after_micros,
             u.upstream_status, u.created_at
      FROM usage_events u
      JOIN api_keys k ON k.id=u.api_key_id
      WHERE u.customer_id=? ORDER BY u.created_at DESC LIMIT ?
    `).all(customerId, Math.min(Math.max(limit, 1), 500));
  }

  usageSummary(days = 30) {
    const window = Math.min(Math.max(Math.trunc(days) || 1, 1), 90);
    const since = utcDayStart() - (window - 1) * 86_400_000;
    const daily = this.db.query(`
      SELECT date(created_at / 1000, 'unixepoch') AS day, COUNT(*) AS requests,
             COALESCE(SUM(input_tokens), 0) AS input_tokens,
             COALESCE(SUM(cached_input_tokens), 0) AS cached_input_tokens,
             COALESCE(SUM(output_tokens), 0) AS output_tokens,
             COALESCE(SUM(cost_micros), 0) AS cost_micros
      FROM usage_events WHERE created_at >= ? GROUP BY day ORDER BY day
    `).all(since);
    const models = this.db.query(`
      SELECT model, COUNT(*) AS requests,
             COALESCE(SUM(input_tokens), 0) AS input_tokens,
             COALESCE(SUM(cached_input_tokens), 0) AS cached_input_tokens,
             COALESCE(SUM(output_tokens), 0) AS output_tokens,
             COALESCE(SUM(cost_micros), 0) AS cost_micros
      FROM usage_events WHERE created_at >= ? GROUP BY model ORDER BY cost_micros DESC
    `).all(since);
    const totals = this.db.query(`
      SELECT COUNT(*) AS requests,
             COALESCE(SUM(input_tokens), 0) AS input_tokens,
             COALESCE(SUM(cached_input_tokens), 0) AS cached_input_tokens,
             COALESCE(SUM(output_tokens), 0) AS output_tokens,
             COALESCE(SUM(cost_micros), 0) AS cost_micros
      FROM usage_events WHERE created_at >= ?
    `).get(since);
    return { days: window, since, daily, models, totals };
  }

  auditEvents(limit = 100) {
    return this.db.query("SELECT id, action, target_type, target_id, metadata_json, created_at FROM admin_audit_events ORDER BY created_at DESC LIMIT ?").all(Math.min(Math.max(limit, 1), 500));
  }

  ready(): boolean {
    const row = this.db.query("PRAGMA quick_check").get() as Record<string, string> | null;
    return Boolean(row && Object.values(row)[0] === "ok");
  }
}

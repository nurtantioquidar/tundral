import type { DatabaseMigration } from "../migration";
import { ensureColumn } from "../migration";

export const upstreamRoutingMigration: DatabaseMigration = {
  version: 3,
  name: "upstream-routing",
  up(database) {
    database.run(`
      CREATE TABLE IF NOT EXISTS upstream_providers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL COLLATE NOCASE UNIQUE,
        base_url TEXT NOT NULL,
        credential_ciphertext TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
        weight INTEGER NOT NULL DEFAULT 1 CHECK(weight BETWEEN 1 AND 100),
        max_concurrency INTEGER NOT NULL DEFAULT 4 CHECK(max_concurrency BETWEEN 1 AND 10000),
        models_json TEXT,
        revision INTEGER NOT NULL DEFAULT 1,
        credential_version INTEGER NOT NULL DEFAULT 1,
        credential_updated_at INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS upstream_credentials (
        id TEXT PRIMARY KEY,
        provider_id TEXT NOT NULL REFERENCES upstream_providers(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        credential_ciphertext TEXT NOT NULL,
        encryption_context TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
        weight INTEGER NOT NULL DEFAULT 1 CHECK(weight BETWEEN 1 AND 100),
        max_concurrency INTEGER NOT NULL DEFAULT 4 CHECK(max_concurrency BETWEEN 1 AND 10000),
        models_json TEXT,
        revision INTEGER NOT NULL DEFAULT 1,
        credential_version INTEGER NOT NULL DEFAULT 1,
        credential_updated_at INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        UNIQUE(provider_id, name)
      );
      CREATE TABLE IF NOT EXISTS provider_model_capabilities (
        id TEXT PRIMARY KEY,
        provider_id TEXT NOT NULL REFERENCES upstream_providers(id) ON DELETE CASCADE,
        canonical_model_id TEXT NOT NULL,
        upstream_model_id TEXT NOT NULL,
        api_paths_json TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
        verified_at INTEGER,
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        UNIQUE(provider_id, canonical_model_id)
      );
      CREATE TABLE IF NOT EXISTS model_routes (
        public_model_id TEXT PRIMARY KEY,
        enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
        revision INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS model_route_targets (
        id TEXT PRIMARY KEY,
        public_model_id TEXT NOT NULL REFERENCES model_routes(public_model_id) ON DELETE CASCADE,
        capability_id TEXT NOT NULL REFERENCES provider_model_capabilities(id) ON DELETE CASCADE,
        priority INTEGER NOT NULL CHECK(priority BETWEEN 1 AND 10000),
        weight INTEGER NOT NULL DEFAULT 1 CHECK(weight BETWEEN 1 AND 100),
        max_concurrency INTEGER NOT NULL DEFAULT 10000 CHECK(max_concurrency BETWEEN 1 AND 10000),
        enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        UNIQUE(public_model_id, capability_id)
      );
    `);
    ensureColumn(database, "upstream_providers", "transport", "TEXT NOT NULL DEFAULT 'openai_http'");
  },
};

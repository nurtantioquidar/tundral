import type { DatabaseMigration } from "../migration";

export const coreLedgerMigration: DatabaseMigration = {
  version: 1,
  name: "core-ledger",
  up(database) {
    database.run(`
      CREATE TABLE IF NOT EXISTS customers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        active INTEGER NOT NULL DEFAULT 1,
        daily_spend_limit_micros INTEGER,
        per_request_limit_micros INTEGER,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS api_keys (
        id TEXT PRIMARY KEY,
        customer_id TEXT NOT NULL REFERENCES customers(id),
        key_hash TEXT NOT NULL UNIQUE,
        key_prefix TEXT NOT NULL,
        name TEXT NOT NULL DEFAULT 'default',
        active INTEGER NOT NULL DEFAULT 1,
        max_concurrency INTEGER NOT NULL DEFAULT 2,
        rate_limit_per_minute INTEGER NOT NULL DEFAULT 60,
        spend_limit_micros INTEGER,
        expires_at INTEGER,
        created_at INTEGER NOT NULL,
        revoked_at INTEGER
      );
      CREATE TABLE IF NOT EXISTS ledger_entries (
        id TEXT PRIMARY KEY,
        customer_id TEXT NOT NULL REFERENCES customers(id),
        kind TEXT NOT NULL,
        amount_micros INTEGER NOT NULL,
        reference TEXT NOT NULL UNIQUE,
        metadata_json TEXT NOT NULL DEFAULT '{}',
        created_at INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS reservations (
        id TEXT PRIMARY KEY,
        customer_id TEXT NOT NULL REFERENCES customers(id),
        api_key_id TEXT NOT NULL REFERENCES api_keys(id),
        request_id TEXT,
        amount_micros INTEGER NOT NULL,
        status TEXT NOT NULL CHECK(status IN ('active','settled','released')),
        created_at INTEGER NOT NULL,
        expires_at INTEGER,
        settled_at INTEGER
      );
      CREATE TABLE IF NOT EXISTS usage_events (
        id TEXT PRIMARY KEY,
        customer_id TEXT NOT NULL REFERENCES customers(id),
        api_key_id TEXT NOT NULL REFERENCES api_keys(id),
        request_id TEXT NOT NULL UNIQUE,
        model TEXT NOT NULL,
        rate_card_version TEXT NOT NULL DEFAULT 'legacy',
        input_tokens INTEGER NOT NULL,
        cached_input_tokens INTEGER NOT NULL,
        cache_write_tokens INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        cost_micros INTEGER NOT NULL,
        balance_before_micros INTEGER,
        balance_after_micros INTEGER,
        upstream_status INTEGER NOT NULL,
        route_revision INTEGER,
        route_target_id TEXT,
        created_at INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS usdc_deposits (
        id TEXT PRIMARY KEY,
        customer_id TEXT NOT NULL REFERENCES customers(id),
        chain TEXT NOT NULL,
        tx_hash TEXT NOT NULL,
        log_index INTEGER NOT NULL,
        amount_atomic INTEGER NOT NULL,
        confirmations INTEGER NOT NULL,
        credited_at INTEGER NOT NULL,
        UNIQUE(chain, tx_hash, log_index)
      );
      CREATE TABLE IF NOT EXISTS idempotency_requests (
        id TEXT PRIMARY KEY,
        api_key_id TEXT NOT NULL REFERENCES api_keys(id),
        idempotency_key TEXT NOT NULL,
        request_hash TEXT NOT NULL,
        request_id TEXT NOT NULL UNIQUE,
        status TEXT NOT NULL CHECK(status IN ('in_progress','completed','failed')),
        response_json TEXT,
        response_status INTEGER,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        UNIQUE(api_key_id, idempotency_key)
      );
      CREATE TABLE IF NOT EXISTS admin_audit_events (
        id TEXT PRIMARY KEY,
        action TEXT NOT NULL,
        target_type TEXT NOT NULL,
        target_id TEXT NOT NULL,
        metadata_json TEXT NOT NULL DEFAULT '{}',
        created_at INTEGER NOT NULL
      );
    `);
  },
};

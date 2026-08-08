import type { DatabaseMigration } from "../migration";
import { ensureColumn } from "../migration";

export const customerAccountsMigration: DatabaseMigration = {
  version: 2,
  name: "customer-accounts-and-key-limits",
  up(database) {
    database.run(`
      CREATE TABLE IF NOT EXISTS customer_accounts (
        id TEXT PRIMARY KEY,
        customer_id TEXT NOT NULL UNIQUE REFERENCES customers(id),
        email TEXT NOT NULL COLLATE NOCASE UNIQUE,
        password_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
      );
    `);
    ensureColumn(database, "customers", "active", "INTEGER NOT NULL DEFAULT 1");
    ensureColumn(database, "customers", "daily_spend_limit_micros", "INTEGER");
    ensureColumn(database, "customers", "per_request_limit_micros", "INTEGER");
    ensureColumn(database, "customers", "updated_at", "INTEGER");
    ensureColumn(database, "api_keys", "name", "TEXT NOT NULL DEFAULT 'default'");
    ensureColumn(database, "api_keys", "rate_limit_per_minute", "INTEGER NOT NULL DEFAULT 60");
    ensureColumn(database, "api_keys", "spend_limit_micros", "INTEGER");
    ensureColumn(database, "api_keys", "expires_at", "INTEGER");
    ensureColumn(database, "api_keys", "revoked_at", "INTEGER");
    ensureColumn(database, "reservations", "request_id", "TEXT");
    ensureColumn(database, "reservations", "expires_at", "INTEGER");
    ensureColumn(database, "usage_events", "rate_card_version", "TEXT NOT NULL DEFAULT 'legacy'");
    ensureColumn(database, "usage_events", "balance_before_micros", "INTEGER");
    ensureColumn(database, "usage_events", "balance_after_micros", "INTEGER");
    ensureColumn(database, "usage_events", "route_revision", "INTEGER");
    ensureColumn(database, "usage_events", "route_target_id", "TEXT");

    database.run("UPDATE customers SET updated_at=COALESCE(updated_at, created_at)");
  },
};

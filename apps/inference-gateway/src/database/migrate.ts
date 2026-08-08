import type { Database } from "bun:sqlite";
import type { DatabaseMigration } from "./migration";
import { coreLedgerMigration } from "./migrations/001-core-ledger";
import { customerAccountsMigration } from "./migrations/002-customer-accounts-and-key-limits";
import { upstreamRoutingMigration } from "./migrations/003-upstream-routing";
import { indexesAndRecoveryMigration } from "./migrations/004-indexes-and-recovery";

const migrations: DatabaseMigration[] = [
  coreLedgerMigration,
  customerAccountsMigration,
  upstreamRoutingMigration,
  indexesAndRecoveryMigration,
];

export function runMigrations(database: Database): void {
  database.run(`
    CREATE TABLE IF NOT EXISTS schema_migrations (
      version INTEGER PRIMARY KEY,
      name TEXT NOT NULL UNIQUE,
      applied_at INTEGER NOT NULL
    )
  `);
  const rows = database.query("SELECT version FROM schema_migrations").all() as Array<{ version: number }>;
  const applied = new Set(rows.map((row) => row.version));

  for (const migration of migrations) {
    if (applied.has(migration.version)) continue;
    const apply = database.transaction(() => {
      migration.up(database);
      database.run(
        "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
        [migration.version, migration.name, Date.now()],
      );
    });
    apply.immediate();
  }
}

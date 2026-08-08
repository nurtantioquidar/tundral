import type { DatabaseMigration } from "../migration";

export const indexesAndRecoveryMigration: DatabaseMigration = {
  version: 4,
  name: "indexes-and-recovery",
  up(database) {
    database.run(`
      CREATE INDEX IF NOT EXISTS idx_ledger_customer ON ledger_entries(customer_id, created_at);
      CREATE INDEX IF NOT EXISTS idx_reservation_customer ON reservations(customer_id, status);
      CREATE INDEX IF NOT EXISTS idx_reservation_expiry ON reservations(status, expires_at);
      CREATE INDEX IF NOT EXISTS idx_usage_customer ON usage_events(customer_id, created_at);
      CREATE INDEX IF NOT EXISTS idx_usage_api_key ON usage_events(api_key_id, created_at);
      CREATE INDEX IF NOT EXISTS idx_customer_accounts_customer ON customer_accounts(customer_id);
      CREATE INDEX IF NOT EXISTS idx_audit_created ON admin_audit_events(created_at);
      CREATE INDEX IF NOT EXISTS idx_upstream_provider_enabled ON upstream_providers(enabled, created_at);
      CREATE INDEX IF NOT EXISTS idx_upstream_credentials_provider ON upstream_credentials(provider_id, enabled);
      CREATE INDEX IF NOT EXISTS idx_provider_models_provider ON provider_model_capabilities(provider_id, enabled);
      CREATE INDEX IF NOT EXISTS idx_route_targets_model ON model_route_targets(public_model_id, priority);
    `);
  },
};

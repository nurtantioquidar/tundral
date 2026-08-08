import type { Database } from "bun:sqlite";
import { createHash, randomBytes, randomUUID } from "node:crypto";

function safeInteger(value: bigint, label: string): number {
  if (value < BigInt(Number.MIN_SAFE_INTEGER) || value > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error(`${label} is out of range`);
  }
  return Number(value);
}

function hashKey(key: string): string {
  return createHash("sha256").update(key).digest("hex");
}

interface CreatedKey {
  apiKeyId: string;
  apiKey: string;
  keyPrefix: string;
}

export class AccountRepository {
  constructor(private readonly database: Database) {}

  private audit(
    action: string,
    targetType: string,
    targetId: string,
    metadata: Record<string, unknown> = {},
  ): void {
    this.database.run("INSERT INTO admin_audit_events VALUES (?, ?, ?, ?, ?, ?)", [
      randomUUID(), action, targetType, targetId, JSON.stringify(metadata), Date.now(),
    ]);
  }

  private insertApiKey(customerId: string, args: {
    name?: string; maxConcurrency?: number; rateLimitPerMinute?: number;
    spendLimitMicros?: bigint | null; expiresAt?: number | null;
  } = {}) {
    const id = randomUUID();
    const secret = `sk-cg-${randomBytes(24).toString("base64url")}`;
    const maxConcurrency = args.maxConcurrency ?? Number(process.env.MAX_CONCURRENT_REQUESTS_PER_KEY || 2);
    const rateLimit = args.rateLimitPerMinute ?? Number(process.env.REQUESTS_PER_MINUTE_PER_KEY || 60);
    const spendLimit = args.spendLimitMicros === undefined || args.spendLimitMicros === null
      ? null
      : safeInteger(args.spendLimitMicros, "Key spend limit");
    if (!Number.isSafeInteger(maxConcurrency) || maxConcurrency < 1 || maxConcurrency > 100) throw new Error("Invalid max concurrency");
    if (!Number.isSafeInteger(rateLimit) || rateLimit < 1 || rateLimit > 10_000) throw new Error("Invalid rate limit");
    if (spendLimit !== null && spendLimit <= 0) throw new Error("Key spend limit must be positive");
    if (args.expiresAt !== null && args.expiresAt !== undefined && (!Number.isSafeInteger(args.expiresAt) || args.expiresAt <= Date.now())) {
      throw new Error("Invalid key expiration");
    }
    this.database.run(`
      INSERT INTO api_keys
        (id, customer_id, key_hash, key_prefix, name, active, max_concurrency,
         rate_limit_per_minute, spend_limit_micros, expires_at, created_at, revoked_at)
      VALUES (?, ?, ?, ?, ?, 1, ?, ?, ?, ?, ?, NULL)
    `, [
      id, customerId, hashKey(secret), secret.slice(0, 12),
      args.name?.trim() || "default", maxConcurrency, rateLimit, spendLimit,
      args.expiresAt ?? null, Date.now(),
    ]);
    return { apiKeyId: id, apiKey: secret, keyPrefix: secret.slice(0, 12) };
  }

  createCustomer(name: string) {
    const cleanName = name.trim();
    if (!cleanName || cleanName.length > 120) throw new Error("Invalid customer name");
    const customerId = randomUUID();
    let key!: CreatedKey;
    const now = Date.now();
    const tx = this.database.transaction(() => {
      this.database.run(`
        INSERT INTO customers
          (id, name, active, daily_spend_limit_micros, per_request_limit_micros, created_at, updated_at)
        VALUES (?, ?, 1, NULL, NULL, ?, ?)
      `, [customerId, cleanName, now, now]);
      key = this.insertApiKey(customerId, { name: "initial" });
      this.audit("customer.create", "customer", customerId, { name: cleanName, initialApiKeyId: key.apiKeyId });
    });
    tx.immediate();
    return { customerId, apiKeyId: key.apiKeyId, apiKey: key.apiKey };
  }

  registerCustomerAccount(name: string, email: string, passwordHash: string) {
    const cleanName = name.trim();
    const cleanEmail = email.trim().toLowerCase();
    if (!cleanName || cleanName.length > 120) throw new Error("Invalid customer name");
    if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(cleanEmail) || cleanEmail.length > 254) {
      throw new Error("Invalid email address");
    }
    if (!passwordHash || passwordHash.length > 512) throw new Error("Invalid password hash");
    const customerId = randomUUID();
    const accountId = randomUUID();
    const now = Date.now();
    let key!: CreatedKey;
    const tx = this.database.transaction(() => {
      this.database.run(`
        INSERT INTO customers
          (id, name, active, daily_spend_limit_micros, per_request_limit_micros, created_at, updated_at)
        VALUES (?, ?, 1, NULL, NULL, ?, ?)
      `, [customerId, cleanName, now, now]);
      this.database.run(`
        INSERT INTO customer_accounts
          (id, customer_id, email, password_hash, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?)
      `, [accountId, customerId, cleanEmail, passwordHash, now, now]);
      key = this.insertApiKey(customerId, { name: "default" });
      this.audit("customer.register", "customer", customerId, {
        email: cleanEmail,
        initialApiKeyId: key.apiKeyId,
      });
    });
    tx.immediate();
    return {
      customerId,
      name: cleanName,
      email: cleanEmail,
      apiKeyId: key.apiKeyId,
      apiKey: key.apiKey,
    };
  }

  accountForAuthentication(email: string) {
    return this.database.query(`
      SELECT a.customer_id AS customerId, a.email, a.password_hash AS passwordHash,
             c.name, c.active
      FROM customer_accounts a
      JOIN customers c ON c.id=a.customer_id
      WHERE a.email=?
    `).get(email.trim().toLowerCase()) as {
      customerId: string;
      email: string;
      passwordHash: string;
      name: string;
      active: number;
    } | null;
  }

  createApiKey(customerId: string, args: {
    name?: string; maxConcurrency?: number; rateLimitPerMinute?: number;
    spendLimitMicros?: bigint | null; expiresAt?: number | null;
  } = {}) {
    let result!: CreatedKey;
    const tx = this.database.transaction(() => {
      const customer = this.database.query("SELECT id FROM customers WHERE id=?").get(customerId);
      if (!customer) throw new Error("Customer not found");
      result = this.insertApiKey(customerId, args);
      this.audit("api_key.create", "api_key", result.apiKeyId, { customerId, name: args.name || "default" });
    });
    tx.immediate();
    return result;
  }

  listApiKeys(customerId: string) {
    return this.database.query(`
      SELECT id, key_prefix, name, active, max_concurrency, rate_limit_per_minute,
             spend_limit_micros, expires_at, created_at, revoked_at,
             COALESCE((
               SELECT SUM(u.cost_micros) FROM usage_events u WHERE u.api_key_id=api_keys.id
             ), 0) AS spent_micros,
             COALESCE((
               SELECT SUM(r.amount_micros) FROM reservations r
               WHERE r.api_key_id=api_keys.id AND r.status='active'
             ), 0) AS active_holds_micros
      FROM api_keys WHERE customer_id=? ORDER BY created_at DESC
    `).all(customerId);
  }

  updateApiKey(customerId: string, apiKeyId: string, args: {
    name?: string;
    spendLimitMicros?: bigint | null;
  }) {
    const current = this.database.query(`
      SELECT id, name, spend_limit_micros AS spendLimitMicros
      FROM api_keys WHERE id=? AND customer_id=?
    `).get(apiKeyId, customerId) as {
      id: string;
      name: string;
      spendLimitMicros: number | null;
    } | null;
    if (!current) throw new Error("API key not found");
    const name = args.name === undefined ? current.name : args.name.trim();
    if (!name || name.length > 80) throw new Error("Invalid API key name");
    const spendLimit = args.spendLimitMicros === undefined
      ? current.spendLimitMicros
      : args.spendLimitMicros === null
        ? null
        : safeInteger(args.spendLimitMicros, "Key spend limit");
    if (spendLimit !== null && spendLimit <= 0) throw new Error("Key spend limit must be positive");
    const tx = this.database.transaction(() => {
      this.database.run(
        "UPDATE api_keys SET name=?, spend_limit_micros=? WHERE id=? AND customer_id=?",
        [name, spendLimit, apiKeyId, customerId],
      );
      this.audit("api_key.update", "api_key", apiKeyId, {
        customerId,
        name,
        spendLimitMicros: spendLimit,
      });
    });
    tx.immediate();
    return this.listApiKeys(customerId).find((key) => (key as { id: string }).id === apiKeyId);
  }

  revokeCustomerApiKey(customerId: string, apiKeyId: string): boolean {
    const tx = this.database.transaction(() => {
      const updated = this.database.run(
        "UPDATE api_keys SET active=0, revoked_at=? WHERE id=? AND customer_id=? AND active=1",
        [Date.now(), apiKeyId, customerId],
      );
      if (updated.changes === 1) {
        this.audit("api_key.revoke", "api_key", apiKeyId, { customerId });
      }
      return updated.changes === 1;
    });
    return tx.immediate();
  }

  revokeApiKey(apiKeyId: string): boolean {
    const tx = this.database.transaction(() => {
      const updated = this.database.run("UPDATE api_keys SET active=0, revoked_at=? WHERE id=? AND active=1", [Date.now(), apiKeyId]);
      if (updated.changes === 1) this.audit("api_key.revoke", "api_key", apiKeyId);
      return updated.changes === 1;
    });
    return tx.immediate();
  }

}

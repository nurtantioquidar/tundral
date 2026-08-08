import { describe, expect, test } from "bun:test";
import { GatewayDatabase } from "../src/database";

describe("database migrations", () => {
  test("records every ordered migration", () => {
    const gateway = new GatewayDatabase(":memory:");
    const rows = gateway.db.query(
      "SELECT version, name FROM schema_migrations ORDER BY version",
    ).all();
    expect(rows).toEqual([
      { version: 1, name: "core-ledger" },
      { version: 2, name: "customer-accounts-and-key-limits" },
      { version: 3, name: "upstream-routing" },
      { version: 4, name: "indexes-and-recovery" },
    ]);
  });
});

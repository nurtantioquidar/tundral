import type { Database } from "bun:sqlite";

export interface DatabaseMigration {
  version: number;
  name: string;
  up(database: Database): void;
}

export function ensureColumn(
  database: Database,
  table: string,
  column: string,
  definition: string,
): void {
  const rows = database.query(`PRAGMA table_info(${table})`).all() as Array<{ name: string }>;
  if (!rows.some((row) => row.name === column)) {
    database.run(`ALTER TABLE ${table} ADD COLUMN ${column} ${definition}`);
  }
}

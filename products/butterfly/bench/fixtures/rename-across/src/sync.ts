import { fetchRecord } from "./db";

export function syncAll(ids: string[]): unknown[] {
  return ids.map((id) => fetchRecord(id));
}

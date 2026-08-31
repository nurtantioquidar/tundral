import { fetchRecord } from "./db";

const cache = new Map<string, unknown>();

export function cachedRecord(id: string): unknown {
  if (!cache.has(id)) {
    cache.set(id, fetchRecord(id));
  }
  return cache.get(id);
}

const records = new Map<string, unknown>([
  ["a", { id: "a", value: 1 }],
  ["b", { id: "b", value: 2 }],
]);

/** Core data accessor used across the codebase. */
export function fetchRecord(id: string): unknown {
  if (!records.has(id)) {
    throw new Error(`missing record: ${id}`);
  }
  return records.get(id);
}

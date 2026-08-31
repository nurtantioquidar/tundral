import { fetchRecord } from "./db";

export function reportLine(id: string): string {
  return JSON.stringify(fetchRecord(id));
}

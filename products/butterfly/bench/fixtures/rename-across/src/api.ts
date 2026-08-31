import { fetchRecord } from "./db";

export function getEndpoint(id: string): { status: number; body: unknown } {
  return { status: 200, body: fetchRecord(id) };
}

import type { GatewayErrorEnvelope } from "@monet/contracts/validation";
import { timingSafeEqual } from "node:crypto";

export function apiError(message: string, status: number, type = "invalid_request_error") {
  const payload: GatewayErrorEnvelope = { error: { message, type } };
  return Response.json(payload, { status });
}

export function bearerToken(request: Request): string | null {
  const value = request.headers.get("authorization");
  return value?.startsWith("Bearer ") ? value.slice(7).trim() : null;
}

export function tokenMatches(expected: string, candidate: string | null): boolean {
  if (!expected || !candidate) return false;
  const expectedBytes = Buffer.from(expected);
  const candidateBytes = Buffer.from(candidate);
  return expectedBytes.length === candidateBytes.length &&
    timingSafeEqual(expectedBytes, candidateBytes);
}

export function parseUsdMicros(value: unknown, allowNegative = false): bigint {
  if (typeof value !== "string" || !/^-?\d+(?:\.\d{1,6})?$/.test(value)) {
    throw new Error("USD amount must be a decimal string with at most 6 places");
  }
  const negative = value.startsWith("-");
  if (negative && !allowNegative) throw new Error("USD amount must be positive");
  const unsigned = negative ? value.slice(1) : value;
  const [whole, fraction = ""] = unsigned.split(".");
  const micros = BigInt(whole!) * 1_000_000n + BigInt(fraction.padEnd(6, "0"));
  return negative ? -micros : micros;
}

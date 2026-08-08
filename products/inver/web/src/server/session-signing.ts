import "server-only";
import { createHash, createHmac, timingSafeEqual } from "node:crypto";

function secret(): Buffer {
  const raw = process.env.SESSION_SECRET;
  if (!raw || raw.length < 16) throw new Error("SESSION_SECRET is not configured");
  return createHash("sha256").update(raw).digest();
}

export const sessionCookieOptions = {
  httpOnly: true,
  sameSite: "lax" as const,
  secure: process.env.NODE_ENV === "production",
  path: "/",
};

export function signSession(payload: string): string {
  return createHmac("sha256", secret()).update(payload).digest("base64url");
}

export function constantTimeEqual(a: string, b: string): boolean {
  const bufferA = Buffer.from(a);
  const bufferB = Buffer.from(b);
  return bufferA.length === bufferB.length && timingSafeEqual(bufferA, bufferB);
}

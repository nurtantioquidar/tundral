import "server-only";
import { cookies } from "next/headers";
import { constantTimeEqual, sessionCookieOptions, signSession } from "./session-signing";

const ADMIN_COOKIE = "inver_admin";
/* The cookie of the pre-Tundral name, cleared at sign-out. */
const LEGACY_ADMIN_COOKIE = "monet_admin";
const ADMIN_SESSION_MS = 12 * 60 * 60 * 1000;

export function verifyAdminPassword(candidate: string): boolean {
  const expected = process.env.ADMIN_CONSOLE_PASSWORD;
  return Boolean(expected && constantTimeEqual(candidate, expected));
}

export async function createAdminSession(): Promise<void> {
  const expiresAt = Date.now() + ADMIN_SESSION_MS;
  const payload = `admin.${expiresAt}`;
  (await cookies()).set(ADMIN_COOKIE, `${payload}.${signSession(payload)}`, {
    ...sessionCookieOptions,
    maxAge: Math.floor(ADMIN_SESSION_MS / 1000),
  });
}

export async function hasAdminSession(): Promise<boolean> {
  const value = (await cookies()).get(ADMIN_COOKIE)?.value;
  if (!value) return false;
  const parts = value.split(".");
  if (parts.length !== 3 || parts[0] !== "admin") return false;
  const payload = `${parts[0]}.${parts[1]}`;
  return constantTimeEqual(signSession(payload), parts[2]!) && Number(parts[1]) > Date.now();
}

export async function clearAdminSession(): Promise<void> {
  const store = await cookies();
  store.delete(ADMIN_COOKIE);
  store.delete(LEGACY_ADMIN_COOKIE);
}

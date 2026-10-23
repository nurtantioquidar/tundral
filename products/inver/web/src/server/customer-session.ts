import "server-only";
import type { CustomerAccount } from "@tundral/contracts/customer";
import { cookies } from "next/headers";
import { constantTimeEqual, sessionCookieOptions, signSession } from "./session-signing";

const CUSTOMER_COOKIE = "inver_customer";
/* Cookies of the pre-Tundral names, cleared at sign-out. */
const LEGACY_CUSTOMER_COOKIES = ["monet_customer", "monet_key"];
const CUSTOMER_SESSION_MS = 30 * 24 * 60 * 60 * 1000;

export type CustomerSession = CustomerAccount;

export async function createCustomerSession(customer: CustomerSession): Promise<void> {
  const payload = Buffer.from(JSON.stringify({
    ...customer,
    expiresAt: Date.now() + CUSTOMER_SESSION_MS,
  })).toString("base64url");
  (await cookies()).set(CUSTOMER_COOKIE, `v1.${payload}.${signSession(payload)}`, {
    ...sessionCookieOptions,
    maxAge: Math.floor(CUSTOMER_SESSION_MS / 1000),
  });
}

export async function readCustomerSession(): Promise<CustomerSession | null> {
  const value = (await cookies()).get(CUSTOMER_COOKIE)?.value;
  if (!value) return null;
  const [version, payload, signature] = value.split(".");
  if (version !== "v1" || !payload || !signature || !constantTimeEqual(signSession(payload), signature)) {
    return null;
  }
  try {
    const parsed = JSON.parse(Buffer.from(payload, "base64url").toString("utf8")) as CustomerSession & {
      expiresAt?: unknown;
    };
    if (
      typeof parsed.customerId !== "string" ||
      typeof parsed.name !== "string" ||
      typeof parsed.email !== "string" ||
      typeof parsed.expiresAt !== "number" ||
      parsed.expiresAt <= Date.now()
    ) {
      return null;
    }
    return {
      customerId: parsed.customerId,
      name: parsed.name,
      email: parsed.email,
    };
  } catch {
    return null;
  }
}

export async function clearCustomerSession(): Promise<void> {
  const store = await cookies();
  store.delete(CUSTOMER_COOKIE);
  for (const name of LEGACY_CUSTOMER_COOKIES) store.delete(name);
}

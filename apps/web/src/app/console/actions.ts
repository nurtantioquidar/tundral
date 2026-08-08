"use server";

import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";
import { GatewayError, gatewayCustomer } from "@/server/gateway-client";
import { clearCustomerSession, readCustomerSession } from "@/server/customer-session";

export interface CustomerActionState {
  error?: string;
  ok?: string;
  revealedKey?: string;
}

async function requireCustomer() {
  const session = await readCustomerSession();
  if (!session) redirect("/signin");
  try {
    const customer = await gatewayCustomer.getCustomer(session.customerId);
    if (customer.active !== 1 || customer.email !== session.email) {
      await clearCustomerSession();
      redirect("/signin");
    }
  } catch {
    await clearCustomerSession();
    redirect("/signin");
  }
  return session;
}

function budget(raw: string): string | null | { error: string } {
  const value = raw.trim();
  if (!value) return null;
  if (!/^\d+(?:\.\d{1,6})?$/.test(value) || Number(value) <= 0) {
    return { error: "Budget must be a positive USD value with at most 6 decimal places." };
  }
  return value;
}

function message(cause: unknown, fallback: string): string {
  return cause instanceof GatewayError ? cause.message : fallback;
}

export async function createCustomerKey(
  _previous: CustomerActionState,
  formData: FormData,
): Promise<CustomerActionState> {
  const customer = await requireCustomer();
  const name = String(formData.get("name") ?? "").trim();
  if (!name || name.length > 80) return { error: "Give the key a name up to 80 characters." };
  const spendLimit = budget(String(formData.get("spendLimit") ?? ""));
  if (spendLimit !== null && typeof spendLimit === "object") return spendLimit;
  try {
    const created = await gatewayCustomer.createKey(customer.customerId, {
      name,
      spend_limit_usd: spendLimit,
    });
    revalidatePath("/console");
    return {
      ok: `Key “${name}” created. Copy it now—it won’t be shown again.`,
      revealedKey: created.apiKey,
    };
  } catch (cause) {
    return { error: message(cause, "The key could not be created.") };
  }
}

export async function updateCustomerKey(
  _previous: CustomerActionState,
  formData: FormData,
): Promise<CustomerActionState> {
  const customer = await requireCustomer();
  const keyId = String(formData.get("keyId") ?? "");
  const name = String(formData.get("name") ?? "").trim();
  if (!name || name.length > 80) return { error: "Give the key a name up to 80 characters." };
  const spendLimit = budget(String(formData.get("spendLimit") ?? ""));
  if (spendLimit !== null && typeof spendLimit === "object") return spendLimit;
  try {
    await gatewayCustomer.updateKey(customer.customerId, keyId, {
      name,
      spend_limit_usd: spendLimit,
    });
    revalidatePath("/console");
    return { ok: "Key settings saved." };
  } catch (cause) {
    return { error: message(cause, "The key could not be updated.") };
  }
}

export async function revokeCustomerKey(keyId: string): Promise<void> {
  const customer = await requireCustomer();
  try {
    await gatewayCustomer.revokeKey(customer.customerId, keyId);
  } catch {
    // The refreshed key list remains authoritative if this was already revoked.
  }
  revalidatePath("/console");
}

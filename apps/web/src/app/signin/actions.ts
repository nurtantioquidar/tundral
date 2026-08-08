"use server";

import { redirect } from "next/navigation";
import { GatewayError, gatewayCustomer } from "@/server/gateway-client";
import { clearCustomerSession, createCustomerSession } from "@/server/customer-session";

export interface SignInState {
  error?: string;
  ok?: string;
  revealedKey?: string;
}

export async function signInCustomer(_previous: SignInState, formData: FormData): Promise<SignInState> {
  const email = String(formData.get("email") ?? "").trim().toLowerCase();
  const password = String(formData.get("password") ?? "");
  if (!email || !password) return { error: "Enter your email and password." };
  try {
    const account = await gatewayCustomer.authenticateAccount(email, password);
    await createCustomerSession(account);
  } catch (cause) {
    if (cause instanceof GatewayError && cause.status === 401) {
      return { error: "The email or password is incorrect, or the account is frozen." };
    }
    return { error: "Sign-in is unavailable right now. Try again in a moment." };
  }
  redirect("/console");
}

export async function registerCustomer(_previous: SignInState, formData: FormData): Promise<SignInState> {
  const name = String(formData.get("name") ?? "").trim();
  const email = String(formData.get("email") ?? "").trim().toLowerCase();
  const password = String(formData.get("password") ?? "");
  const confirmation = String(formData.get("confirmation") ?? "");
  if (!name || name.length > 120) return { error: "Enter a name up to 120 characters." };
  if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) return { error: "Enter a valid email address." };
  if (password.length < 12 || password.length > 128) {
    return { error: "Use a password between 12 and 128 characters." };
  }
  if (password !== confirmation) return { error: "The passwords don’t match." };
  try {
    const account = await gatewayCustomer.registerAccount({ name, email, password });
    await createCustomerSession({
      customerId: account.customerId,
      name: account.name,
      email: account.email,
    });
    return {
      ok: "Your account is ready. Copy the default API key now—it won’t be shown again.",
      revealedKey: account.apiKey,
    };
  } catch (cause) {
    if (cause instanceof GatewayError && cause.status === 409) {
      return { error: cause.message };
    }
    return { error: "Registration is unavailable right now. Try again in a moment." };
  }
}

export async function signOutCustomer(): Promise<void> {
  await clearCustomerSession();
  redirect("/");
}

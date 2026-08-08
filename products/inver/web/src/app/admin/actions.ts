"use server";

import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";
import { GatewayError, gatewayAdmin } from "@/server/gateway-client";
import { clearAdminSession, createAdminSession, hasAdminSession, verifyAdminPassword } from "@/server/admin-session";

export interface ActionState {
  error?: string;
  ok?: string;
  revealedKey?: string;
  createdCustomerId?: string;
  /** Model IDs the provider reported from /v1/models, for the capability picker. */
  discoveredModels?: string[];
}

async function requireAdmin(): Promise<void> {
  if (!(await hasAdminSession())) redirect("/admin/signin");
}

function message(cause: unknown, fallback: string): string {
  return cause instanceof GatewayError ? cause.message : fallback;
}

export async function adminSignIn(_previous: ActionState, formData: FormData): Promise<ActionState> {
  const password = String(formData.get("password") ?? "");
  if (!process.env.ADMIN_CONSOLE_PASSWORD) {
    return { error: "ADMIN_CONSOLE_PASSWORD is not configured on the server. Set it, then restart the web app." };
  }
  if (!password || !verifyAdminPassword(password)) {
    return { error: "That password didn’t match. Check the deployment’s ADMIN_CONSOLE_PASSWORD." };
  }
  await createAdminSession();
  redirect("/admin");
}

export async function adminSignOut(): Promise<void> {
  await clearAdminSession();
  redirect("/");
}

export async function createCustomer(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const name = String(formData.get("name") ?? "").trim();
  if (!name) return { error: "Give the customer a name." };
  try {
    const created = await gatewayAdmin.createCustomer(name);
    revalidatePath("/admin");
    return {
      ok: `Customer “${name}” created.`,
      revealedKey: created.apiKey,
      createdCustomerId: created.customerId,
    };
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the customer.") };
  }
}

export async function issueCredit(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const customerId = String(formData.get("customerId") ?? "");
  const amount = String(formData.get("amount") ?? "").trim();
  const kind = String(formData.get("kind") ?? "promotional_credit");
  const reference = String(formData.get("reference") ?? "").trim();
  const note = String(formData.get("note") ?? "").trim();
  if (!/^\d+(?:\.\d{1,6})?$/.test(amount) || Number(amount) <= 0) {
    return { error: "Amount must be a positive USD value with at most 6 decimal places, like 10.00." };
  }
  if (!reference) {
    return { error: "A unique reference is required — repeating the same reference replays the credit instead of double-crediting." };
  }
  try {
    const result = await gatewayAdmin.credit(customerId, {
      amount_usd: amount,
      kind,
      reference,
      ...(note ? { note } : {}),
    });
    revalidatePath(`/admin/customers/${customerId}`);
    revalidatePath("/admin");
    return {
      ok: result.replayed
        ? `Reference “${reference}” was already credited — replayed without a second charge. Balance $${result.balance_usd}.`
        : `Credited $${amount}. Balance is now $${result.balance_usd}.`,
    };
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the credit.") };
  }
}

export async function createKey(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const customerId = String(formData.get("customerId") ?? "");
  const name = String(formData.get("name") ?? "").trim() || "default";
  const spendLimit = String(formData.get("spendLimit") ?? "").trim();
  if (spendLimit && (!/^\d+(?:\.\d{1,6})?$/.test(spendLimit) || Number(spendLimit) <= 0)) {
    return { error: "Key budget must be a positive USD value with at most 6 decimal places." };
  }
  try {
    const created = await gatewayAdmin.createKey(customerId, {
      name,
      spend_limit_usd: spendLimit || null,
    });
    revalidatePath(`/admin/customers/${customerId}`);
    return { ok: `Key “${name}” created — shown once, copy it now.`, revealedKey: created.apiKey };
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the key.") };
  }
}

export async function revokeKey(customerId: string, keyId: string): Promise<void> {
  await requireAdmin();
  try {
    await gatewayAdmin.revokeKey(customerId, keyId);
  } catch {
    // Already revoked or missing — the refreshed list is the source of truth.
  }
  revalidatePath(`/admin/customers/${customerId}`);
}

function optionalUsd(raw: string): string | null | { error: string } {
  const value = raw.trim();
  if (!value) return null;
  if (!/^\d+(?:\.\d{1,6})?$/.test(value) || Number(value) <= 0) {
    return { error: "Limits must be positive USD values with at most 6 decimal places." };
  }
  return value;
}

export async function updateCustomerLimits(
  _previous: ActionState,
  formData: FormData,
): Promise<ActionState> {
  await requireAdmin();
  const customerId = String(formData.get("customerId") ?? "");
  const daily = optionalUsd(String(formData.get("dailyLimit") ?? ""));
  const perRequest = optionalUsd(String(formData.get("perRequestLimit") ?? ""));
  if (daily !== null && typeof daily === "object") return daily;
  if (perRequest !== null && typeof perRequest === "object") return perRequest;
  try {
    await gatewayAdmin.updateCustomer(customerId, {
      daily_spend_limit_usd: daily,
      per_request_limit_usd: perRequest,
    });
    revalidatePath(`/admin/customers/${customerId}`);
    revalidatePath("/admin");
    return { ok: "Customer spending limits saved." };
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the limits.") };
  }
}

function parseIntegerField(raw: string, field: string, minimum: number, maximum: number): number | { error: string } {
  const value = Number(raw);
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    return { error: `${field} must be an integer between ${minimum} and ${maximum}.` };
  }
  return value;
}

function parseModels(raw: string): string[] | null {
  const models = raw
    .split(",")
    .map((model) => model.trim())
    .filter(Boolean);
  return models.length === 0 ? null : models;
}

export async function createUpstream(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const name = String(formData.get("name") ?? "").trim();
  const transport = String(formData.get("transport") || "openai_http") as "openai_http" | "codex_cli";
  const baseUrl = String(formData.get("base_url") ?? "").trim();
  if (!name || (transport === "openai_http" && !baseUrl)) {
    return { error: "Name and an HTTP provider base URL are required." };
  }
  const maxConcurrency = parseIntegerField(String(formData.get("max_concurrency") || "4"), "Max concurrency", 1, 10_000);
  if (typeof maxConcurrency === "object") return maxConcurrency;
  /* redirect() signals by throwing, so it must sit outside the try — inside, the catch below would
   * swallow it and report a phantom failure for a creation that actually succeeded. */
  let created: { id: string; applied_to_runtime: boolean };
  try {
    created = await gatewayAdmin.createUpstream({
      name,
      transport,
      ...(transport === "openai_http" ? { base_url: baseUrl } : {}),
      enabled: formData.get("enabled") === "on",
      max_concurrency: maxConcurrency,
    });
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the provider.") };
  }
  revalidatePath("/admin/upstreams", "layout");
  /* Registration continues on the provider's own page, where the key and model forms live. The
   * query flag only selects a notice; it carries no secret. */
  redirect(`/admin/upstreams/providers/${created.id}?created=${created.applied_to_runtime ? "live" : "staged"}`);
}

export async function updateUpstreamSettings(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const id = String(formData.get("id") ?? "");
  const revision = Number(formData.get("revision"));
  const maxConcurrency = parseIntegerField(String(formData.get("max_concurrency") ?? ""), "Max concurrency", 1, 10_000);
  if (typeof maxConcurrency === "object") return maxConcurrency;
  try {
    await gatewayAdmin.updateUpstream(id, {
      max_concurrency: maxConcurrency,
      revision,
    });
    revalidatePath("/admin/upstreams", "layout");
    return { ok: "Settings saved." };
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the update.") };
  }
}

export async function setUpstreamEnabled(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const id = String(formData.get("id") ?? "");
  const revision = Number(formData.get("revision"));
  const enabled = formData.get("enabled") === "true";
  try {
    const updated = await gatewayAdmin.updateUpstream(id, { enabled, revision });
    revalidatePath("/admin/upstreams", "layout");
    return {
      ok: `${enabled ? "Enabled" : "Disabled"}.${updated.applied_to_runtime ? "" : " Stored only — the running pool still uses environment configuration."}`,
    };
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the change.") };
  }
}

export async function createUpstreamCredential(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const providerId = String(formData.get("provider_id") ?? "");
  const name = String(formData.get("name") ?? "").trim();
  const apiKey = String(formData.get("api_key") ?? "").trim();
  if (!name || !apiKey) return { error: "Credential name and API key are required." };
  const weight = parseIntegerField(String(formData.get("weight") || "1"), "Weight", 1, 100);
  if (typeof weight === "object") return weight;
  const maxConcurrency = parseIntegerField(String(formData.get("max_concurrency") || "4"), "Max concurrency", 1, 10_000);
  if (typeof maxConcurrency === "object") return maxConcurrency;
  try {
    await gatewayAdmin.createUpstreamCredential(providerId, {
      name,
      api_key: apiKey,
      enabled: true,
      weight,
      max_concurrency: maxConcurrency,
      models: parseModels(String(formData.get("models") ?? "")),
    });
    revalidatePath("/admin/upstreams", "layout");
    return { ok: `Credential “${name}” added. The key will not be shown again.` };
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the credential.") };
  }
}

export async function rotateUpstreamKey(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const id = String(formData.get("id") ?? "");
  const revision = Number(formData.get("revision"));
  const apiKey = String(formData.get("api_key") ?? "").trim();
  if (!apiKey) return { error: "Paste the replacement key first." };
  try {
    const rotated = await gatewayAdmin.rotateUpstreamCredential(id, apiKey, revision);
    revalidatePath("/admin/upstreams", "layout");
    return { ok: `Key rotated — credential version is now v${rotated.credential_version}. The old key is gone from storage.` };
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the rotation.") };
  }
}

export async function deleteUpstreamKey(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const id = String(formData.get("id") ?? "");
  const revision = Number(formData.get("revision"));
  try {
    await gatewayAdmin.deleteUpstreamCredential(id, revision);
    revalidatePath("/admin/upstreams", "layout");
    return { ok: "Key deleted from storage." };
  } catch (cause) {
    if (cause instanceof GatewayError && cause.status === 409) {
      return { error: "This key changed since the page loaded. Reload and try again." };
    }
    return { error: message(cause, "The gateway rejected the deletion.") };
  }
}

export async function deleteUpstream(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const id = String(formData.get("id") ?? "");
  const revision = Number(formData.get("revision"));
  /* redirect() signals by throwing, so it must sit outside the try — inside, the catch below would
   * swallow it and report a phantom failure for a deletion that actually succeeded. */
  let failure: string | null = null;
  try {
    await gatewayAdmin.deleteUpstream(id, revision);
  } catch (cause) {
    failure = cause instanceof GatewayError && cause.status === 409
      ? "This provider changed since the page loaded. Reload and try again."
      : message(cause, "The gateway rejected the deletion.");
  }
  if (failure) return { error: failure };
  revalidatePath("/admin/upstreams", "layout");
  // The provider's own page no longer exists, so return to the list rather than a 404.
  redirect("/admin/upstreams/providers");
}

export async function createUpstreamModel(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const providerId = String(formData.get("provider_id") ?? "");
  const canonicalModelId = String(formData.get("canonical_model_id") ?? "").trim();
  const upstreamModelId = String(formData.get("upstream_model_id") ?? "").trim();
  if (!canonicalModelId || !upstreamModelId) {
    return { error: "Canonical and provider model IDs are required." };
  }
  const paths: Array<"chat_completions" | "responses"> = [];
  if (formData.get("chat_completions") === "on") paths.push("chat_completions");
  if (formData.get("responses") === "on") paths.push("responses");
  if (paths.length === 0) return { error: "Select at least one API path." };
  try {
    await gatewayAdmin.createUpstreamModel(providerId, {
      canonical_model_id: canonicalModelId,
      upstream_model_id: upstreamModelId,
      api_paths: paths,
      enabled: true,
    });
    revalidatePath("/admin/upstreams", "layout");
    return { ok: `Model capability “${canonicalModelId}” added.` };
  } catch (cause) {
    return { error: message(cause, "The gateway rejected the model capability.") };
  }
}

export async function discoverProviderModels(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const providerId = String(formData.get("provider_id") ?? "");
  try {
    const models = await gatewayAdmin.discoverUpstreamModels(providerId);
    if (models.length === 0) {
      return { error: "The provider reported no models. Add the model manually instead." };
    }
    return { discoveredModels: models };
  } catch (cause) {
    return { error: message(cause, "The provider's model list could not be read.") };
  }
}

/* The picker submits one checkbox per discovered model (`model:<id>`), plus an optional
 * `map:<id>` naming the Monet model it serves when the provider uses its own name for it. */
export async function addDiscoveredModels(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const providerId = String(formData.get("provider_id") ?? "");
  const selections: Array<{ canonical: string; upstream: string }> = [];
  for (const [field, value] of formData.entries()) {
    if (!field.startsWith("model:") || value !== "on") continue;
    const upstream = field.slice("model:".length);
    const mapped = String(formData.get(`map:${upstream}`) ?? "").trim();
    selections.push({ canonical: mapped || upstream, upstream });
  }
  if (selections.length === 0) return { error: "Tick at least one model to add." };
  const added: string[] = [];
  const failed: string[] = [];
  for (const selection of selections) {
    try {
      await gatewayAdmin.createUpstreamModel(providerId, {
        canonical_model_id: selection.canonical,
        upstream_model_id: selection.upstream,
        api_paths: ["chat_completions", "responses"],
        enabled: true,
      });
      added.push(selection.upstream);
    } catch (cause) {
      failed.push(`${selection.upstream} (${message(cause, "rejected")})`);
    }
  }
  revalidatePath("/admin/upstreams", "layout");
  if (failed.length > 0) {
    return {
      error: `${added.length > 0 ? `Added ${added.join(", ")}. ` : ""}Not added: ${failed.join("; ")}.`,
    };
  }
  return { ok: `Added ${added.length} model${added.length === 1 ? "" : "s"}: ${added.join(", ")}.` };
}

export async function testUpstream(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const id = String(formData.get("id") ?? "");
  /* No fallback model. Defaulting here silently tested a different model than the operator selected —
   * and when the picker had no options at all, it invented one that may not exist on this gateway. */
  const model = String(formData.get("public_model_id") ?? "").trim();
  if (!model) {
    return { error: "Choose which customer-facing model to test this capability as." };
  }
  try {
    const result = await gatewayAdmin.testUpstreamModel(id, model);
    revalidatePath("/admin/upstreams", "layout");
    return {
      ok: `Test passed — ${result.public_model_id} returned HTTP ${result.upstream_status} with ${result.usage.input_tokens} in / ${result.usage.output_tokens} out tokens.`,
    };
  } catch (cause) {
    return { error: message(cause, "The provider test failed.") };
  }
}

/* The route editor submits the whole ordered fallback chain as one JSON field: position in the list
 * is the priority tier, and rows sharing a tier split traffic by weight. Sending the complete
 * snapshot matches the gateway's publish contract, which validates and swaps atomically. */
interface RouteTargetInput {
  capability_id: string;
  priority: number;
  weight: number;
}

function parseRouteTargets(raw: string): RouteTargetInput[] | { error: string } {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw || "[]");
  } catch {
    return { error: "The route order could not be read. Reload the page and try again." };
  }
  if (!Array.isArray(parsed)) return { error: "The route order could not be read. Reload the page and try again." };
  const targets: RouteTargetInput[] = [];
  for (const entry of parsed) {
    if (typeof entry !== "object" || entry === null) return { error: "The route order contained an invalid row." };
    const row = entry as Record<string, unknown>;
    const capabilityId = typeof row.capability_id === "string" ? row.capability_id.trim() : "";
    if (!capabilityId) return { error: "The route order contained a row with no provider." };
    const priority = parseIntegerField(String(row.priority ?? ""), "Priority", 1, 10_000);
    if (typeof priority === "object") return priority;
    const weight = parseIntegerField(String(row.weight ?? "1"), "Weight", 1, 100);
    if (typeof weight === "object") return weight;
    if (targets.some((target) => target.capability_id === capabilityId)) {
      return { error: "A provider appears twice in the route. Remove the duplicate and publish again." };
    }
    targets.push({ capability_id: capabilityId, priority, weight });
  }
  return targets;
}

export async function publishModelRoute(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const publicModelId = String(formData.get("public_model_id") ?? "");
  const revision = Number(formData.get("revision") || 0);
  const parsed = parseRouteTargets(String(formData.get("targets") ?? ""));
  if (!Array.isArray(parsed)) return parsed;
  if (parsed.length === 0) {
    return { error: "Add at least one provider to the route before publishing." };
  }
  try {
    const route = await gatewayAdmin.publishModelRoute(publicModelId, {
      enabled: true,
      revision,
      targets: parsed.map((target) => ({ ...target, max_concurrency: 10_000, enabled: true })),
    });
    revalidatePath("/admin/upstreams", "layout");
    const tiers = new Set(parsed.map((target) => target.priority)).size;
    return {
      ok: route.applied_to_runtime
        ? `Published ${publicModelId} — revision ${route.revision}, ${parsed.length} provider${parsed.length === 1 ? "" : "s"} across ${tiers} tier${tiers === 1 ? "" : "s"}, live now.`
        : `Published ${publicModelId} — revision ${route.revision}. Stored only: the gateway still routes from environment configuration.`,
    };
  } catch (cause) {
    if (cause instanceof GatewayError && cause.status === 409) {
      return {
        error:
          "Someone else published this model while you were editing. Reload the page to pick up their order, then reapply your change — publishing now would silently overwrite it.",
      };
    }
    return { error: message(cause, "The gateway rejected the model route.") };
  }
}

export async function deleteUpstreamModel(_previous: ActionState, formData: FormData): Promise<ActionState> {
  await requireAdmin();
  const id = String(formData.get("id") ?? "");
  const revision = Number(formData.get("revision"));
  try {
    await gatewayAdmin.deleteUpstreamModel(id, revision);
    revalidatePath("/admin/upstreams", "layout");
    return { ok: "Model removed from this provider." };
  } catch (cause) {
    if (cause instanceof GatewayError && cause.status === 409) {
      return { error: "This model changed since the page loaded. Reload and try again." };
    }
    return { error: message(cause, "The gateway rejected the removal.") };
  }
}

export async function setCustomerActive(customerId: string, active: boolean): Promise<void> {
  await requireAdmin();
  await gatewayAdmin.updateCustomer(customerId, { active });
  revalidatePath(`/admin/customers/${customerId}`);
  revalidatePath("/admin");
}

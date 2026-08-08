import { GatewayDatabase } from "../database";
import { hashPassword, validatePassword, verifyPassword } from "../accounts/password";
import { formatUsd, MODEL_RATES, publicModel, RATE_CARD_VERSION } from "../billing/pricing";
import { routeUpstreamAdmin } from "./upstream-admin";
import type { UpstreamRegistry } from "../upstreams/upstream-registry";
import { apiError as error, bearerToken as bearer, parseUsdMicros, tokenMatches } from "./response";

export interface InternalRouterDependencies {
  db: GatewayDatabase;
  adminApiKey: string;
  webBffApiKey: string;
  upstreamRegistry: UpstreamRegistry | null;
  setGenerationEnabled(enabled: boolean): void;
}

function isWebBffRoute(url: URL, method: string): boolean {
  const path = url.pathname;
  if (path === "/internal/accounts/register" && method === "POST") return true;
  if (path === "/internal/accounts/authenticate" && method === "POST") return true;
  if (path === "/internal/pricing" && method === "GET") return true;
  if (/^\/internal\/customers\/[^/]+$/.test(path) && method === "GET") return true;
  if (/^\/internal\/customers\/[^/]+\/keys$/.test(path) && (method === "GET" || method === "POST")) {
    return true;
  }
  if (
    /^\/internal\/customers\/[^/]+\/keys\/[^/]+$/.test(path) &&
    (method === "PATCH" || method === "DELETE")
  ) {
    return true;
  }
  if (/^\/internal\/customers\/[^/]+\/usage$/.test(path) && method === "GET") return true;
  return /^\/internal\/customers\/[^/]+\/reconciliation$/.test(path) && method === "GET";
}

export async function routeInternalRequest(
  req: Request,
  url: URL,
  dependencies: InternalRouterDependencies,
): Promise<Response | null> {
  const { db, adminApiKey, webBffApiKey, upstreamRegistry, setGenerationEnabled } = dependencies;
  if (url.pathname.startsWith("/internal/")) {
    const token = bearer(req);
    const isAdmin = tokenMatches(adminApiKey, token);
    const isWebBff = tokenMatches(webBffApiKey, token);
    if (!isAdmin && !isWebBff) return error("Unauthorized", 401, "authentication_error");
    if (!isAdmin && !isWebBffRoute(url, req.method)) {
      return error(
        "This service credential cannot access the requested operation",
        403,
        "authorization_error",
      );
    }
    if (
      url.pathname === "/internal/upstreams" ||
      url.pathname.startsWith("/internal/upstreams/") ||
      url.pathname.startsWith("/internal/upstream-credentials/") ||
      url.pathname === "/internal/upstream-models" ||
      url.pathname.startsWith("/internal/upstream-models/") ||
      url.pathname === "/internal/model-routes" ||
      url.pathname.startsWith("/internal/model-routes/")
    ) {
      if (!upstreamRegistry) {
        return error(
          "Upstream registry is unavailable while UPSTREAM_MODE=codex-cli",
          409,
          "conflict_error",
        );
      }
      const upstreamResponse = await routeUpstreamAdmin(req, url, upstreamRegistry);
      if (upstreamResponse) return upstreamResponse;
    }

    if (url.pathname === "/internal/customers" && req.method === "GET") {
      return Response.json({ data: db.listCustomers() });
    }
    if (url.pathname === "/internal/customers" && req.method === "POST") {
      const body = await req.json() as { name?: string };
      try {
        if (!body.name) return error("name is required", 400);
        return Response.json(db.createCustomer(body.name), { status: 201 });
      } catch (cause) {
        return error(cause instanceof Error ? cause.message : "Invalid customer", 400);
      }
    }
    if (url.pathname === "/internal/accounts/register" && req.method === "POST") {
      const body = await req.json() as Record<string, unknown>;
      const password = typeof body.password === "string" ? body.password : "";
      try {
        validatePassword(password);
      } catch (cause) {
        return error(cause instanceof Error ? cause.message : "Invalid password", 400);
      }
      try {
        const passwordHash = await hashPassword(password);
        const registered = db.registerCustomerAccount(
          String(body.name || ""),
          String(body.email || ""),
          passwordHash,
        );
        return Response.json(registered, { status: 201 });
      } catch (cause) {
        const message = cause instanceof Error && cause.message.includes("UNIQUE constraint failed")
          ? "An account with that email already exists"
          : cause instanceof Error
            ? cause.message
            : "Invalid account";
        return error(message, 409, "account_error");
      }
    }
    if (url.pathname === "/internal/accounts/authenticate" && req.method === "POST") {
      const body = await req.json() as Record<string, unknown>;
      const email = typeof body.email === "string" ? body.email : "";
      const password = typeof body.password === "string" ? body.password : "";
      const account = db.accountForAuthentication(email);
      const valid = account ? await verifyPassword(password, account.passwordHash) : false;
      if (!account || !valid || account.active !== 1) {
        return error("Invalid email or password", 401, "authentication_error");
      }
      return Response.json({
        customerId: account.customerId,
        name: account.name,
        email: account.email,
      });
    }
    if (url.pathname === "/internal/pricing" && req.method === "GET") {
      const available = upstreamRegistry
        ? new Set(upstreamRegistry.publicModels())
        : new Set(Object.keys(MODEL_RATES));
      return Response.json({
        rate_card_version: RATE_CARD_VERSION,
        currency: "USD",
        unit: "million_tokens",
        data: Object.values(MODEL_RATES).filter((rate) => available.has(rate.id)).map(publicModel),
      });
    }
    /* The complete rate card, including models that are not currently routable.
     *
     * `/internal/pricing` deliberately narrows to what the gateway can actually serve, which is right
     * for customers but useless for an operator publishing a model's first route: before that route
     * exists the model is unroutable, so it would be absent from the very screen used to create it.
     * Admin-only — it is not in the web-BFF allowlist, because customers must not see models the
     * gateway cannot serve. */
    if (url.pathname === "/internal/rate-card" && req.method === "GET") {
      return Response.json({
        rate_card_version: RATE_CARD_VERSION,
        currency: "USD",
        unit: "million_tokens",
        data: Object.values(MODEL_RATES).map(publicModel),
      });
    }
    const customerMatch = url.pathname.match(/^\/internal\/customers\/([^/]+)$/);
    if (customerMatch && req.method === "GET") {
      const customer = db.getCustomer(customerMatch[1]!);
      return customer ? Response.json(customer) : error("Customer not found", 404);
    }
    if (customerMatch && req.method === "PATCH") {
      const body = await req.json() as Record<string, unknown>;
      try {
        return Response.json(db.updateCustomer(customerMatch[1]!, {
          active: typeof body.active === "boolean" ? body.active : undefined,
          dailySpendLimitMicros: body.daily_spend_limit_usd === undefined
            ? undefined
            : body.daily_spend_limit_usd === null
              ? null
              : parseUsdMicros(body.daily_spend_limit_usd),
          perRequestLimitMicros: body.per_request_limit_usd === undefined
            ? undefined
            : body.per_request_limit_usd === null
              ? null
              : parseUsdMicros(body.per_request_limit_usd),
        }));
      } catch (cause) {
        return error(cause instanceof Error ? cause.message : "Invalid customer update", 400);
      }
    }
    const keysMatch = url.pathname.match(/^\/internal\/customers\/([^/]+)\/keys$/);
    if (keysMatch && req.method === "GET") {
      return Response.json({ data: db.listApiKeys(keysMatch[1]!) });
    }
    if (keysMatch && req.method === "POST") {
      const body = await req.json() as Record<string, unknown>;
      try {
        const expiresAt = body.expires_at === null || body.expires_at === undefined
          ? null
          : Date.parse(String(body.expires_at));
        return Response.json(db.createApiKey(keysMatch[1]!, {
          name: typeof body.name === "string" ? body.name : undefined,
          maxConcurrency: body.max_concurrency === undefined
            ? undefined
            : Number(body.max_concurrency),
          rateLimitPerMinute: body.rate_limit_per_minute === undefined
            ? undefined
            : Number(body.rate_limit_per_minute),
          spendLimitMicros: body.spend_limit_usd === undefined || body.spend_limit_usd === null
            ? null
            : parseUsdMicros(body.spend_limit_usd),
          expiresAt,
        }), { status: 201 });
      } catch (cause) {
        return error(cause instanceof Error ? cause.message : "Invalid API key", 400);
      }
    }
    const customerKeyMatch = url.pathname.match(
      /^\/internal\/customers\/([^/]+)\/keys\/([^/]+)$/,
    );
    if (customerKeyMatch && req.method === "PATCH") {
      const body = await req.json() as Record<string, unknown>;
      try {
        return Response.json(db.updateApiKey(customerKeyMatch[1]!, customerKeyMatch[2]!, {
          name: body.name === undefined ? undefined : String(body.name),
          spendLimitMicros: body.spend_limit_usd === undefined
            ? undefined
            : body.spend_limit_usd === null || body.spend_limit_usd === ""
              ? null
              : parseUsdMicros(body.spend_limit_usd),
        }));
      } catch (cause) {
        const message = cause instanceof Error ? cause.message : "Invalid API key update";
        return error(message, message === "API key not found" ? 404 : 400);
      }
    }
    if (customerKeyMatch && req.method === "DELETE") {
      return db.revokeCustomerApiKey(customerKeyMatch[1]!, customerKeyMatch[2]!)
        ? new Response(null, { status: 204 })
        : error("Active API key not found", 404);
    }
    const keyMatch = url.pathname.match(/^\/internal\/keys\/([^/]+)$/);
    if (keyMatch && req.method === "DELETE") {
      return db.revokeApiKey(keyMatch[1]!)
        ? new Response(null, { status: 204 })
        : error("Active API key not found", 404);
    }
  const creditMatch = url.pathname.match(/^\/internal\/customers\/([^/]+)\/credits$/);
  if (creditMatch && req.method === "POST") {
    const body = await req.json() as Record<string, unknown>;
    try {
      const kind = String(body.kind || "promotional_credit") as "promotional_credit" | "offline_payment" | "manual_adjustment";
      if (!["promotional_credit", "offline_payment", "manual_adjustment"].includes(kind)) return error("Invalid credit kind", 400);
      const result = db.creditManual({
        customerId: creditMatch[1]!, amountMicros: parseUsdMicros(body.amount_usd, kind === "manual_adjustment"),
        kind, reference: String(body.reference || ""), note: typeof body.note === "string" ? body.note : undefined,
      });
      return Response.json({ ...result, creditedMicros: result.creditedMicros.toString(), balance_usd: formatUsd(db.balanceMicros(creditMatch[1]!)) }, { status: result.replayed ? 200 : 201 });
    } catch (cause) { return error(cause instanceof Error ? cause.message : "Invalid credit", 409, "credit_error"); }
  }
  const usageMatch = url.pathname.match(/^\/internal\/customers\/([^/]+)\/usage$/);
  if (usageMatch && req.method === "GET") {
    if (!db.getCustomer(usageMatch[1]!)) return error("Customer not found", 404);
    return Response.json({ data: db.usage(usageMatch[1]!, Number(url.searchParams.get("limit") || 100)) });
  }
  if (url.pathname === "/internal/usage/summary" && req.method === "GET") {
    return Response.json(db.usageSummary(Number(url.searchParams.get("days") || 30)));
  }
  const reconciliationMatch = url.pathname.match(/^\/internal\/customers\/([^/]+)\/reconciliation$/);
  if (reconciliationMatch && req.method === "GET") {
    try { return Response.json(db.reconcile(reconciliationMatch[1]!)); }
    catch (cause) { return error(cause instanceof Error ? cause.message : "Reconciliation failed", 404); }
  }
  if (url.pathname === "/internal/recovery" && req.method === "POST") return Response.json(db.recoverStale());
  if (url.pathname === "/internal/audit" && req.method === "GET") return Response.json({ data: db.auditEvents(Number(url.searchParams.get("limit") || 100)) });
  if (url.pathname === "/internal/generation" && req.method === "PATCH") {
    const body = await req.json() as { enabled?: boolean };
    if (typeof body.enabled !== "boolean") return error("enabled is required", 400);
    setGenerationEnabled(body.enabled);
    db.recordAudit("generation.update", "system", "gateway", { enabled: body.enabled });
    return Response.json({ enabled: body.enabled });
  }
  if (url.pathname === "/internal/usdc/deposits" && req.method === "POST") {
    const body = await req.json() as Record<string, unknown>;
    try {
      const result = db.creditUsdc({
        customerId: String(body.customer_id),
        chain: String(body.chain),
        txHash: String(body.tx_hash),
        logIndex: Number(body.log_index ?? 0),
        amountAtomic: BigInt(String(body.amount_atomic)),
        confirmations: Number(body.confirmations),
      });
      return Response.json({ ...result, creditedMicros: result.creditedMicros.toString() }, { status: 201 });
    } catch (cause) {
      return error(cause instanceof Error ? cause.message : "Invalid deposit", 409, "deposit_error");
    }
  }
  return error("Not found", 404);
}

  return null;
}

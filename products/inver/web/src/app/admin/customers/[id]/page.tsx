import type { Metadata } from "next";
import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { adminSignOut } from "@/app/admin/actions";
import {
  CreditForm,
  CustomerLimitsForm,
  FreezeButton,
  NewKeyForm,
  RevokeKeyButton,
} from "@/features/admin/AdminForms";
import { ConsoleShell } from "@/components/ConsoleShell";
import { GatewayError, gatewayAdmin } from "@/server/gateway-client";
import { hasAdminSession } from "@/server/admin-session";
import { day, timestamp, tokens, usd } from "@/lib/format";

export const metadata: Metadata = { title: "Customer — Inver operator console" };

export default async function CustomerDetail({ params }: { params: Promise<{ id: string }> }) {
  if (!(await hasAdminSession())) redirect("/admin/signin");
  const { id } = await params;

  let customer;
  try {
    customer = await gatewayAdmin.getCustomer(id);
  } catch (cause) {
    if (cause instanceof GatewayError && cause.status === 404) notFound();
    throw cause;
  }

  const [reconciliation, usage, keys] = await Promise.all([
    gatewayAdmin.reconciliation(id),
    gatewayAdmin.usage(id, 50),
    gatewayAdmin.listKeys(id),
  ]);

  const isActive = customer.active === 1;

  return (
    <ConsoleShell
      area="Operator"
      links={[
        { href: "/admin", label: "Overview" },
        { href: "/admin/upstreams", label: "Routing" },
        { href: "/admin/upstreams/providers", label: "Providers" },
      ]}
      actions={
        <form action={adminSignOut}>
          <button type="submit" className="btn btn--ghost">
            Sign out
          </button>
        </form>
      }
    >
      <p style={{ marginBottom: "var(--space-md)" }}>
        <Link className="link-cta" href="/admin">
          ← All customers
        </Link>
      </p>

      <div className="console-head">
        <div>
          <h1 className="console-head__title">{customer.name}</h1>
          <p className="console-head__sub">
            {customer.email ?? "No console account"} · <code>{customer.id}</code> · created {day(customer.created_at)} ·{" "}
            {isActive ? "active" : "frozen — requests are rejected"}
          </p>
        </div>
        <FreezeButton customerId={customer.id} active={isActive} />
      </div>

      <section className="panel">
        <div className="panel__head">
          <h2 className="panel__title">Account guardrails</h2>
          <span className="meta">enforced before upstream execution</span>
        </div>
        <div className="panel__body">
          <CustomerLimitsForm
            customerId={customer.id}
            dailyLimitMicros={customer.daily_spend_limit_micros}
            perRequestLimitMicros={customer.per_request_limit_micros}
          />
        </div>
      </section>

      <div className="cards">
        <div className="card">
          <span className="card__label meta">Available balance</span>
          <p className="card__value">{usd(reconciliation.availableBalanceMicros)}</p>
          <p className="card__hint">{usd(reconciliation.activeHoldsMicros)} in active holds</p>
        </div>
        <div className="card">
          <span className="card__label meta">Lifetime credits</span>
          <p className="card__value">{usd(reconciliation.totalCreditsMicros)}</p>
        </div>
        <div className="card">
          <span className="card__label meta">Lifetime usage</span>
          <p className="card__value">{usd(reconciliation.usageChargesMicros)}</p>
          <p className="card__hint tnum">{tokens(reconciliation.usageCount)} settled requests</p>
        </div>
        <div className="card">
          <span className="card__label meta">Ledger</span>
          <p className="card__value" style={{ fontSize: "var(--text-md)" }}>
            {reconciliation.reconciled ? (
              <span className="chip chip--ok">reconciled</span>
            ) : (
              <span className="chip chip--error">{reconciliation.missingUsageLedgerEntries} missing entries</span>
            )}
          </p>
          <p className="card__hint">usage charges vs ledger debits</p>
        </div>
      </div>

      <section className="panel">
        <div className="panel__head">
          <h2 className="panel__title">Add credit</h2>
          <span className="meta">idempotent by reference</span>
        </div>
        <div className="panel__body">
          <CreditForm customerId={customer.id} />
        </div>
      </section>

      <section className="panel">
        <div className="panel__head">
          <h2 className="panel__title">API keys</h2>
          <span className="meta">secrets shown once at creation</span>
        </div>
        <div className="panel__body" style={{ borderBottom: keys.length ? "1px solid var(--color-rule)" : undefined }}>
          <NewKeyForm customerId={customer.id} />
        </div>
        {keys.length === 0 ? (
          <div className="empty">
            <p className="empty__title">No keys.</p>
            <p className="empty__body">Issue a key above to let this customer call the gateway.</p>
          </div>
        ) : (
          <div className="panel__body--flush table-wrap">
            <table className="data-table">
              <thead>
                <tr>
                  <th scope="col">Prefix</th>
                  <th scope="col">Name</th>
                  <th scope="col">Status</th>
                  <th scope="col" className="num">
                    Spent
                  </th>
                  <th scope="col" className="num">
                    Budget
                  </th>
                  <th scope="col" className="num">
                    Remaining
                  </th>
                  <th scope="col">Created</th>
                  <th scope="col" aria-label="Actions" />
                </tr>
              </thead>
              <tbody>
                {keys.map((key) => (
                  <tr key={key.id}>
                    <td>
                      <code>{key.key_prefix}…</code>
                    </td>
                    <td>{key.name}</td>
                    <td>
                      {key.active === 1 ? (
                        <span className="chip chip--ok">active</span>
                      ) : (
                        <span className="chip chip--muted">revoked</span>
                      )}
                    </td>
                    <td className="num">{usd(key.spent_micros)}</td>
                    <td className="num">{key.spend_limit_micros === null ? "No cap" : usd(key.spend_limit_micros)}</td>
                    <td className="num">
                      {key.spend_limit_micros === null
                        ? "—"
                        : usd(Math.max(0, key.spend_limit_micros - key.spent_micros - key.active_holds_micros))}
                    </td>
                    <td>{day(key.created_at)}</td>
                    <td style={{ textAlign: "right" }}>
                      {key.active === 1 ? <RevokeKeyButton customerId={customer.id} keyId={key.id} /> : null}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className="panel">
        <div className="panel__head">
          <h2 className="panel__title">Usage</h2>
          <span className="meta">latest 50, newest first</span>
        </div>
        {usage.length === 0 ? (
          <div className="empty">
            <p className="empty__title">No settled usage.</p>
            <p className="empty__body">Requests appear here once they settle against the ledger.</p>
          </div>
        ) : (
          <div className="panel__body--flush table-wrap">
            <table className="data-table">
              <thead>
                <tr>
                  <th scope="col">Time (UTC)</th>
                  <th scope="col">Model</th>
                  <th scope="col">Key</th>
                  <th scope="col" className="num">
                    Input
                  </th>
                  <th scope="col" className="num">
                    Cached
                  </th>
                  <th scope="col" className="num">
                    Output
                  </th>
                  <th scope="col" className="num">
                    Cost
                  </th>
                  <th scope="col" className="num">
                    Balance after
                  </th>
                </tr>
              </thead>
              <tbody>
                {usage.map((event) => (
                  <tr key={event.request_id}>
                    <td>{timestamp(event.created_at)}</td>
                    <td>
                      <code>{event.model}</code>
                    </td>
                    <td>{event.api_key_name}</td>
                    <td className="num">{tokens(event.input_tokens)}</td>
                    <td className="num">{tokens(event.cached_input_tokens)}</td>
                    <td className="num">{tokens(event.output_tokens)}</td>
                    <td className="num">{usd(event.cost_micros)}</td>
                    <td className="num">{event.balance_after_micros === null ? "—" : usd(event.balance_after_micros)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </ConsoleShell>
  );
}

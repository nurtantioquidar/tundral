import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { adminSignOut } from "@/app/admin/actions";
import { NewCustomerForm } from "@/features/admin/AdminForms";
import { ConsoleShell } from "@/components/ConsoleShell";
import { UsageChart } from "@/features/usage/UsageChart";
import { gatewayAdmin } from "@/server/gateway-client";
import { hasAdminSession } from "@/server/admin-session";
import { day, tokens, usd } from "@/lib/format";

export const metadata: Metadata = { title: "Operator console — Monet" };

export default async function AdminDashboard() {
  if (!(await hasAdminSession())) redirect("/admin/signin");

  const [customers, summary] = await Promise.all([gatewayAdmin.listCustomers(), gatewayAdmin.usageSummary(30)]);
  const postedTotal = customers.reduce((sum, customer) => sum + customer.posted_balance_micros, 0);
  const activeCount = customers.filter((customer) => customer.active === 1).length;

  return (
    <ConsoleShell
      area="Operator"
      links={[
        { href: "/admin", label: "Overview", current: true },
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
      <div className="console-head">
        <div>
          <h1 className="console-head__title">Gateway overview</h1>
          <p className="console-head__sub">Last {summary.days} days, UTC. Balances are posted ledger totals.</p>
        </div>
      </div>

      <div className="cards">
        <div className="card">
          <span className="card__label meta">Customers</span>
          <p className="card__value tnum">{customers.length}</p>
          <p className="card__hint">{activeCount} active</p>
        </div>
        <div className="card">
          <span className="card__label meta">Posted balances</span>
          <p className="card__value">{usd(postedTotal)}</p>
          <p className="card__hint">outstanding service-credit liability</p>
        </div>
        <div className="card">
          <span className="card__label meta">Requests · {summary.days}d</span>
          <p className="card__value tnum">{tokens(summary.totals.requests)}</p>
        </div>
        <div className="card">
          <span className="card__label meta">Tokens · {summary.days}d</span>
          <p className="card__value tnum">{tokens(summary.totals.input_tokens + summary.totals.output_tokens)}</p>
          <p className="card__hint">
            {tokens(summary.totals.input_tokens)} in · {tokens(summary.totals.output_tokens)} out
          </p>
        </div>
        <div className="card">
          <span className="card__label meta">Billed · {summary.days}d</span>
          <p className="card__value">{usd(summary.totals.cost_micros)}</p>
        </div>
      </div>

      <section className="panel">
        <div className="panel__head">
          <h2 className="panel__title">Tokens per day</h2>
          <span className="meta">stacked input + output</span>
        </div>
        <div className="panel__body">
          {summary.totals.requests === 0 ? (
            <div className="empty">
              <p className="empty__title">No settled usage in this window.</p>
              <p className="empty__body">Charges appear here as customer requests settle against the ledger.</p>
            </div>
          ) : (
            <UsageChart summary={summary} />
          )}
        </div>
      </section>

      {summary.models.length > 0 ? (
        <section className="panel">
          <div className="panel__head">
            <h2 className="panel__title">By model · {summary.days}d</h2>
          </div>
          <div className="panel__body--flush table-wrap">
            <table className="data-table">
              <thead>
                <tr>
                  <th scope="col">Model</th>
                  <th scope="col" className="num">
                    Requests
                  </th>
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
                    Billed
                  </th>
                </tr>
              </thead>
              <tbody>
                {summary.models.map((row) => (
                  <tr key={row.model}>
                    <td>
                      <code>{row.model}</code>
                    </td>
                    <td className="num">{tokens(row.requests)}</td>
                    <td className="num">{tokens(row.input_tokens)}</td>
                    <td className="num">{tokens(row.cached_input_tokens)}</td>
                    <td className="num">{tokens(row.output_tokens)}</td>
                    <td className="num">{usd(row.cost_micros)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      ) : null}

      <section className="panel">
        <div className="panel__head">
          <h2 className="panel__title">Customers</h2>
        </div>
        <div className="panel__body" style={{ borderBottom: "1px solid var(--color-rule)" }}>
          <NewCustomerForm />
        </div>
        {customers.length === 0 ? (
          <div className="empty">
            <p className="empty__title">No customers yet.</p>
            <p className="empty__body">Create the first pilot customer above — the initial API key is shown once.</p>
          </div>
        ) : (
          <div className="panel__body--flush table-wrap">
            <table className="data-table">
              <thead>
                <tr>
                  <th scope="col">Name</th>
                  <th scope="col">Account</th>
                  <th scope="col">Status</th>
                  <th scope="col" className="num">
                    Active keys
                  </th>
                  <th scope="col" className="num">
                    Posted balance
                  </th>
                  <th scope="col" className="num">
                    Daily limit
                  </th>
                  <th scope="col">Created</th>
                </tr>
              </thead>
              <tbody>
                {customers.map((customer) => (
                  <tr key={customer.id}>
                    <td>
                      <Link href={`/admin/customers/${customer.id}`}>{customer.name}</Link>
                    </td>
                    <td>{customer.email ?? <span className="meta">managed only</span>}</td>
                    <td>
                      {customer.active === 1 ? (
                        <span className="chip chip--ok">active</span>
                      ) : (
                        <span className="chip chip--error">frozen</span>
                      )}
                    </td>
                    <td className="num">{customer.active_key_count}</td>
                    <td className="num">{usd(customer.posted_balance_micros)}</td>
                    <td className="num">
                      {customer.daily_spend_limit_micros === null ? "—" : usd(customer.daily_spend_limit_micros)}
                    </td>
                    <td>{day(customer.created_at)}</td>
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

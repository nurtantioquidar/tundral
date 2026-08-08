import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { ConsoleShell } from "@/components/ConsoleShell";
import { CustomerKeySettings, CustomerNewKeyForm } from "@/features/api-keys/CustomerKeyForms";
import type { UsageEvent } from "@monet/contracts/customer";
import { GatewayError, gatewayCustomer } from "@/server/gateway-client";
import { clearCustomerSession, readCustomerSession } from "@/server/customer-session";
import { day, timestamp, tokens, usd } from "@/lib/format";
import { signOutCustomer } from "@/app/signin/actions";

export const metadata: Metadata = { title: "Console — Monet" };

interface ModelRollup {
  model: string;
  requests: number;
  inputTokens: number;
  cachedTokens: number;
  outputTokens: number;
  costMicros: number;
}

function rollupByModel(events: UsageEvent[]): ModelRollup[] {
  const byModel = new Map<string, ModelRollup>();
  for (const event of events) {
    const row = byModel.get(event.model) ?? {
      model: event.model,
      requests: 0,
      inputTokens: 0,
      cachedTokens: 0,
      outputTokens: 0,
      costMicros: 0,
    };
    row.requests += 1;
    row.inputTokens += event.input_tokens;
    row.cachedTokens += event.cached_input_tokens;
    row.outputTokens += event.output_tokens;
    row.costMicros += event.cost_micros;
    byModel.set(event.model, row);
  }
  return [...byModel.values()].sort((a, b) => b.costMicros - a.costMicros);
}

export default async function ConsolePage() {
  const session = await readCustomerSession();
  if (!session) redirect("/signin");

  let events: UsageEvent[];
  let customer;
  let keys;
  let reconciliation;
  let rateCardVersion = "";
  try {
    [customer, events, keys, reconciliation] = await Promise.all([
      gatewayCustomer.getCustomer(session.customerId),
      gatewayCustomer.usage(session.customerId, 100),
      gatewayCustomer.listKeys(session.customerId),
      gatewayCustomer.reconciliation(session.customerId),
    ]);
    const pricing = await gatewayCustomer.pricing();
    rateCardVersion = pricing.rate_card_version;
    if (customer.active !== 1 || customer.email !== session.email) {
      await clearCustomerSession();
      redirect("/signin");
    }
  } catch (cause) {
    if (cause instanceof GatewayError && (cause.status === 401 || cause.status === 404)) {
      await clearCustomerSession();
      redirect("/signin");
    }
    throw cause;
  }

  const spentMicros = events.reduce((sum, event) => sum + event.cost_micros, 0);
  const inputTotal = events.reduce((sum, event) => sum + event.input_tokens, 0);
  const outputTotal = events.reduce((sum, event) => sum + event.output_tokens, 0);
  const models = rollupByModel(events);

  return (
    <ConsoleShell
      area="Console"
      actions={
        <form action={signOutCustomer}>
          <button type="submit" className="btn btn--ghost">
            Sign out
          </button>
        </form>
      }
    >
      <div className="console-head">
        <div>
          <h1 className="console-head__title">Your account</h1>
          <p className="console-head__sub">
            {customer.name} · {session.email} · rate card <code>{rateCardVersion}</code>
          </p>
        </div>
      </div>

      <div className="cards">
        <div className="card">
          <span className="card__label meta">Available balance</span>
          <p className="card__value">{usd(reconciliation.availableBalanceMicros)}</p>
          <p className="card__hint">posted ledger minus active holds</p>
        </div>
        <div className="card">
          <span className="card__label meta">Spend · last {events.length} requests</span>
          <p className="card__value">{usd(spentMicros)}</p>
        </div>
        <div className="card">
          <span className="card__label meta">Input tokens</span>
          <p className="card__value">{tokens(inputTotal)}</p>
          <p className="card__hint">includes cached input</p>
        </div>
        <div className="card">
          <span className="card__label meta">Output tokens</span>
          <p className="card__value">{tokens(outputTotal)}</p>
        </div>
      </div>

      <section className="panel">
        <div className="panel__head">
          <div>
            <h2 className="panel__title">API keys</h2>
            <p className="panel__sub">Use a separate key for each device or workload. Budgets cap lifetime key spend against your shared balance.</p>
          </div>
          <span className="meta">{keys.filter((key) => key.active === 1).length} active</span>
        </div>
        <div className={`panel__body${keys.length ? " panel__body--divided" : ""}`}>
          <CustomerNewKeyForm />
        </div>
        {keys.length === 0 ? (
          <div className="empty">
            <p className="empty__title">No API keys yet.</p>
            <p className="empty__body">Create a key above to call the gateway.</p>
          </div>
        ) : (
          <div className="key-grid">
            {keys.map((key) => {
              const remaining = key.spend_limit_micros === null
                ? null
                : Math.max(0, key.spend_limit_micros - key.spent_micros - key.active_holds_micros);
              return (
                <article className="key-card" key={key.id}>
                  <div className="key-card__head">
                    <div>
                      <p className="key-card__name">{key.name}</p>
                      <code>{key.key_prefix}…</code>
                    </div>
                    {key.active === 1 ? (
                      <span className="chip chip--ok">active</span>
                    ) : (
                      <span className="chip chip--muted">revoked</span>
                    )}
                  </div>
                  <div className="key-card__stats">
                    <div>
                      <span className="meta">Spent</span>
                      <p>{usd(key.spent_micros)}</p>
                    </div>
                    <div>
                      <span className="meta">Budget left</span>
                      <p>{remaining === null ? "No cap" : usd(remaining)}</p>
                    </div>
                    <div>
                      <span className="meta">Created</span>
                      <p>{day(key.created_at)}</p>
                    </div>
                  </div>
                  <CustomerKeySettings apiKey={key} />
                </article>
              );
            })}
          </div>
        )}
      </section>

      {models.length > 0 ? (
        <section className="panel">
          <div className="panel__head">
            <h2 className="panel__title">By model</h2>
            <span className="meta">last {events.length} requests</span>
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
                    Cost
                  </th>
                </tr>
              </thead>
              <tbody>
                {models.map((row) => (
                  <tr key={row.model}>
                    <td>
                      <code>{row.model}</code>
                    </td>
                    <td className="num">{tokens(row.requests)}</td>
                    <td className="num">{tokens(row.inputTokens)}</td>
                    <td className="num">{tokens(row.cachedTokens)}</td>
                    <td className="num">{tokens(row.outputTokens)}</td>
                    <td className="num">{usd(row.costMicros)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      ) : null}

      <section className="panel">
        <div className="panel__head">
          <h2 className="panel__title">Usage</h2>
          <span className="meta">newest first</span>
        </div>
        {events.length === 0 ? (
          <div className="empty">
            <p className="empty__title">No usage yet.</p>
            <p className="empty__body">
              Point your harness at the gateway with any active key and requests will appear here as they settle.
            </p>
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
                {events.map((event) => (
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

"use client";

import { useActionState, useEffect, useRef, useState } from "react";
import {
  adminSignIn,
  createCustomer,
  createKey,
  issueCredit,
  revokeKey,
  setCustomerActive,
  updateCustomerLimits,
  type ActionState,
} from "@/app/admin/actions";
import { CopyButton } from "@/components/CopyButton";

// Feature-local forms keep operator workflows out of the shared component layer.

function StatusLine({ state }: { state: ActionState }) {
  return (
    <p
      className={`field__help${state.error ? " field__help--error" : ""}`}
      aria-live="polite"
      style={state.ok ? { color: "var(--color-ok)", fontWeight: 500 } : undefined}
    >
      {state.error ?? state.ok ?? ""}
    </p>
  );
}

function KeyReveal({ apiKey }: { apiKey: string }) {
  return (
    <div className="key-reveal" role="status">
      <span style={{ overflowWrap: "anywhere" }}>{apiKey}</span>
      <CopyButton value={apiKey} />
    </div>
  );
}

export function AdminSignInForm() {
  const [state, action, pending] = useActionState<ActionState, FormData>(adminSignIn, {});
  return (
    <form action={action}>
      <div className="field">
        <label className="field__label" htmlFor="password">
          Operator password
        </label>
        <input
          id="password"
          name="password"
          className="input"
          type="password"
          autoComplete="current-password"
          required
          disabled={pending}
          aria-invalid={state.error ? "true" : undefined}
          aria-describedby="password-help"
        />
        <p id="password-help" className={`field__help${state.error ? " field__help--error" : ""}`} aria-live="polite">
          {state.error ?? "Matches ADMIN_CONSOLE_PASSWORD on the web server. Sessions last 12 hours."}
        </p>
      </div>
      <button type="submit" className="btn btn--primary" data-state={pending ? "loading" : undefined} disabled={pending}>
        Open operator console
      </button>
    </form>
  );
}

export function NewCustomerForm() {
  const [state, action, pending] = useActionState<ActionState, FormData>(createCustomer, {});
  const formRef = useRef<HTMLFormElement>(null);

  useEffect(() => {
    if (state.ok) formRef.current?.reset();
  }, [state]);

  return (
    <div>
      <form ref={formRef} action={action} className="form-row">
        <div className="field">
          <label className="field__label" htmlFor="new-customer-name">
            Managed customer name
          </label>
          <input
            id="new-customer-name"
            name="name"
            className="input"
            placeholder="acme-pilot"
            maxLength={120}
            required
            disabled={pending}
            aria-invalid={state.error ? "true" : undefined}
          />
        </div>
        <button type="submit" className="btn btn--primary" data-state={pending ? "loading" : undefined} disabled={pending}>
          Create managed customer
        </button>
      </form>
      <StatusLine state={state} />
      {state.revealedKey ? (
        <div style={{ display: "grid", gap: "var(--space-xs)", marginTop: "var(--space-xs)" }}>
          <span className="meta">No console login · initial API key shown once</span>
          <KeyReveal apiKey={state.revealedKey} />
        </div>
      ) : null}
    </div>
  );
}

export function CreditForm({ customerId }: { customerId: string }) {
  const [state, action, pending] = useActionState<ActionState, FormData>(issueCredit, {});
  const formRef = useRef<HTMLFormElement>(null);

  useEffect(() => {
    if (state.ok) formRef.current?.reset();
  }, [state]);

  return (
    <div>
      <form ref={formRef} action={action} className="form-row">
        <input type="hidden" name="customerId" value={customerId} />
        <div className="field" style={{ flexBasis: "7rem", flexGrow: 0 }}>
          <label className="field__label" htmlFor="credit-amount">
            Amount (USD)
          </label>
          <input
            id="credit-amount"
            name="amount"
            className="input tnum"
            inputMode="decimal"
            placeholder="10.00"
            required
            disabled={pending}
            aria-invalid={state.error ? "true" : undefined}
          />
        </div>
        <div className="field" style={{ flexBasis: "14rem", flexGrow: 0 }}>
          <label className="field__label" htmlFor="credit-kind">
            Kind
          </label>
          <span className="select">
            <select id="credit-kind" name="kind" className="input" disabled={pending} defaultValue="promotional_credit">
              <option value="promotional_credit">Promotional credit</option>
              <option value="offline_payment">Offline payment</option>
            </select>
          </span>
        </div>
        <div className="field">
          <label className="field__label" htmlFor="credit-reference">
            Unique reference
          </label>
          <input
            id="credit-reference"
            name="reference"
            className="input"
            placeholder="early-access-acme-001"
            required
            disabled={pending}
          />
        </div>
        <div className="field">
          <label className="field__label" htmlFor="credit-note">
            Note <span style={{ color: "var(--color-muted)", fontWeight: 400 }}>(optional)</span>
          </label>
          <input id="credit-note" name="note" className="input" disabled={pending} />
        </div>
        <button type="submit" className="btn btn--primary" data-state={pending ? "loading" : undefined} disabled={pending}>
          Credit
        </button>
      </form>
      <StatusLine state={state} />
    </div>
  );
}

export function CustomerLimitsForm({
  customerId,
  dailyLimitMicros,
  perRequestLimitMicros,
}: {
  customerId: string;
  dailyLimitMicros: number | null;
  perRequestLimitMicros: number | null;
}) {
  const [state, action, pending] = useActionState<ActionState, FormData>(updateCustomerLimits, {});
  return (
    <div>
      <form action={action} className="form-row">
        <input type="hidden" name="customerId" value={customerId} />
        <div className="field">
          <label className="field__label" htmlFor="daily-limit">
            Daily account limit (USD)
          </label>
          <input
            id="daily-limit"
            name="dailyLimit"
            className="input tnum"
            inputMode="decimal"
            placeholder="No limit"
            defaultValue={dailyLimitMicros === null ? "" : (dailyLimitMicros / 1_000_000).toFixed(6)}
            disabled={pending}
          />
        </div>
        <div className="field">
          <label className="field__label" htmlFor="request-limit">
            Per-request limit (USD)
          </label>
          <input
            id="request-limit"
            name="perRequestLimit"
            className="input tnum"
            inputMode="decimal"
            placeholder="No limit"
            defaultValue={perRequestLimitMicros === null ? "" : (perRequestLimitMicros / 1_000_000).toFixed(6)}
            disabled={pending}
          />
        </div>
        <button type="submit" className="btn btn--ghost" data-state={pending ? "loading" : undefined} disabled={pending}>
          Save limits
        </button>
      </form>
      <StatusLine state={state} />
    </div>
  );
}

export function NewKeyForm({ customerId }: { customerId: string }) {
  const [state, action, pending] = useActionState<ActionState, FormData>(createKey, {});
  const formRef = useRef<HTMLFormElement>(null);

  useEffect(() => {
    if (state.ok) formRef.current?.reset();
  }, [state]);

  return (
    <div>
      <form ref={formRef} action={action} className="form-row">
        <input type="hidden" name="customerId" value={customerId} />
        <div className="field">
          <label className="field__label" htmlFor="key-name">
            New key name
          </label>
          <input id="key-name" name="name" className="input" placeholder="ci-runner" disabled={pending} />
        </div>
        <div className="field">
          <label className="field__label" htmlFor="key-spend-limit">
            Lifetime budget (USD)
          </label>
          <input
            id="key-spend-limit"
            name="spendLimit"
            className="input tnum"
            inputMode="decimal"
            placeholder="No cap"
            disabled={pending}
          />
        </div>
        <button type="submit" className="btn btn--ghost" data-state={pending ? "loading" : undefined} disabled={pending}>
          Issue key
        </button>
      </form>
      <StatusLine state={state} />
      {state.revealedKey ? (
        <div style={{ display: "grid", gap: "var(--space-xs)", marginTop: "var(--space-xs)" }}>
          <span className="meta">API key — shown once, stored only as a hash</span>
          <KeyReveal apiKey={state.revealedKey} />
        </div>
      ) : null}
    </div>
  );
}

/* Two-step revoke: first click arms, second confirms; disarms after 3s. Revocation is permanent. */
export function RevokeKeyButton({ customerId, keyId }: { customerId: string; keyId: string }) {
  const [armed, setArmed] = useState(false);
  const [pending, setPending] = useState(false);

  useEffect(() => {
    if (!armed) return;
    const timer = setTimeout(() => setArmed(false), 3000);
    return () => clearTimeout(timer);
  }, [armed]);

  return (
    <button
      type="button"
      className="btn btn--danger"
      style={{ minHeight: 32, paddingInline: "var(--space-sm)", fontSize: "var(--text-xs)" }}
      data-state={pending ? "loading" : undefined}
      disabled={pending}
      onClick={async () => {
        if (!armed) {
          setArmed(true);
          return;
        }
        setPending(true);
        await revokeKey(customerId, keyId);
        setPending(false);
        setArmed(false);
      }}
    >
      {armed ? "Confirm revoke" : "Revoke"}
    </button>
  );
}

export function FreezeButton({ customerId, active }: { customerId: string; active: boolean }) {
  const [pending, setPending] = useState(false);
  return (
    <button
      type="button"
      className={active ? "btn btn--danger" : "btn btn--ghost"}
      data-state={pending ? "loading" : undefined}
      disabled={pending}
      onClick={async () => {
        setPending(true);
        await setCustomerActive(customerId, !active);
        setPending(false);
      }}
    >
      {active ? "Freeze customer" : "Unfreeze customer"}
    </button>
  );
}

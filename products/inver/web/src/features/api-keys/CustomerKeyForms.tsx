"use client";

import { useActionState, useEffect, useRef, useState } from "react";
import {
  createCustomerKey,
  revokeCustomerKey,
  updateCustomerKey,
  type CustomerActionState,
} from "@/app/console/actions";
import { CopyButton } from "@/components/CopyButton";
import type { ApiKeyRow } from "@tundral/contracts/customer";

function StatusLine({ state }: { state: CustomerActionState }) {
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

export function CustomerNewKeyForm() {
  const [state, action, pending] = useActionState<CustomerActionState, FormData>(createCustomerKey, {});
  const formRef = useRef<HTMLFormElement>(null);

  useEffect(() => {
    if (state.ok) formRef.current?.reset();
  }, [state.ok]);

  return (
    <div>
      <form ref={formRef} action={action} className="form-row">
        <div className="field">
          <label className="field__label" htmlFor="customer-key-name">
            Key name
          </label>
          <input
            id="customer-key-name"
            name="name"
            className="input"
            placeholder="local-dev"
            maxLength={80}
            required
            disabled={pending}
          />
        </div>
        <div className="field">
          <label className="field__label" htmlFor="customer-key-budget">
            Lifetime budget (USD)
          </label>
          <input
            id="customer-key-budget"
            name="spendLimit"
            className="input tnum"
            inputMode="decimal"
            placeholder="No cap"
            disabled={pending}
          />
        </div>
        <button type="submit" className="btn btn--primary" data-state={pending ? "loading" : undefined} disabled={pending}>
          Create key
        </button>
      </form>
      <StatusLine state={state} />
      {state.revealedKey ? (
        <div className="key-created">
          <span className="meta">Shown once—copy this key now</span>
          <div className="key-reveal" role="status">
            <span style={{ overflowWrap: "anywhere" }}>{state.revealedKey}</span>
            <CopyButton value={state.revealedKey} />
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function CustomerKeySettings({ apiKey }: { apiKey: ApiKeyRow }) {
  const [state, action, pending] = useActionState<CustomerActionState, FormData>(updateCustomerKey, {});
  return (
    <form action={action} className="key-settings">
      <input type="hidden" name="keyId" value={apiKey.id} />
      <div className="field">
        <label className="field__label" htmlFor={`key-name-${apiKey.id}`}>
          Name
        </label>
        <input
          id={`key-name-${apiKey.id}`}
          name="name"
          className="input"
          defaultValue={apiKey.name}
          maxLength={80}
          required
          disabled={pending || apiKey.active !== 1}
        />
      </div>
      <div className="field">
        <label className="field__label" htmlFor={`key-budget-${apiKey.id}`}>
          Lifetime budget (USD)
        </label>
        <input
          id={`key-budget-${apiKey.id}`}
          name="spendLimit"
          className="input tnum"
          inputMode="decimal"
          placeholder="No cap"
          defaultValue={apiKey.spend_limit_micros === null ? "" : (apiKey.spend_limit_micros / 1_000_000).toFixed(6)}
          disabled={pending || apiKey.active !== 1}
        />
      </div>
      <div className="key-settings__actions">
        <button
          type="submit"
          className="btn btn--ghost"
          data-state={pending ? "loading" : undefined}
          disabled={pending || apiKey.active !== 1}
        >
          Save
        </button>
        {apiKey.active === 1 ? <CustomerRevokeKeyButton keyId={apiKey.id} /> : null}
      </div>
      <StatusLine state={state} />
    </form>
  );
}

function CustomerRevokeKeyButton({ keyId }: { keyId: string }) {
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
      data-state={pending ? "loading" : undefined}
      disabled={pending}
      onClick={async () => {
        if (!armed) {
          setArmed(true);
          return;
        }
        setPending(true);
        await revokeCustomerKey(keyId);
        setPending(false);
      }}
    >
      {armed ? "Confirm revoke" : "Revoke"}
    </button>
  );
}

"use client";

import Link from "next/link";
import { useActionState } from "react";
import { registerCustomer, signInCustomer, type SignInState } from "@/app/signin/actions";
import { CopyButton } from "@/components/CopyButton";

export function SignInForm() {
  const [state, action, pending] = useActionState<SignInState, FormData>(signInCustomer, {});

  return (
    <form action={action}>
      <div className="field">
        <label className="field__label" htmlFor="email">
          Email
        </label>
        <input
          id="email"
          name="email"
          className="input"
          type="email"
          autoComplete="email"
          required
          disabled={pending}
          aria-invalid={state.error ? "true" : undefined}
        />
      </div>
      <div className="field">
        <label className="field__label" htmlFor="password">
          Password
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
          aria-describedby="signin-help"
        />
        <p id="signin-help" className={`field__help${state.error ? " field__help--error" : ""}`} aria-live="polite">
          {state.error ?? ""}
        </p>
      </div>
      <button type="submit" className="btn btn--primary" data-state={pending ? "loading" : undefined} disabled={pending}>
        Sign in
      </button>
    </form>
  );
}

export function RegisterForm() {
  const [state, action, pending] = useActionState<SignInState, FormData>(registerCustomer, {});

  if (state.revealedKey) {
    return (
      <div className="auth__success">
        <p className="notice notice--ok">{state.ok}</p>
        <div className="key-reveal" role="status">
          <span className="key-reveal__secret">{state.revealedKey}</span>
          <CopyButton value={state.revealedKey} />
        </div>
        <Link className="btn btn--primary" href="/console">
          Continue to console
        </Link>
      </div>
    );
  }

  return (
    <form action={action}>
      <div className="field">
        <label className="field__label" htmlFor="register-name">
          Name
        </label>
        <input
          id="register-name"
          name="name"
          className="input"
          autoComplete="name"
          maxLength={120}
          required
          disabled={pending}
        />
      </div>
      <div className="field">
        <label className="field__label" htmlFor="register-email">
          Email
        </label>
        <input
          id="register-email"
          name="email"
          className="input"
          type="email"
          autoComplete="email"
          required
          disabled={pending}
        />
      </div>
      <div className="field">
        <label className="field__label" htmlFor="register-password">
          Password
        </label>
        <input
          id="register-password"
          name="password"
          className="input"
          type="password"
          autoComplete="new-password"
          minLength={12}
          maxLength={128}
          required
          disabled={pending}
        />
        <p className="field__help">At least 12 characters.</p>
      </div>
      <div className="field">
        <label className="field__label" htmlFor="register-confirmation">
          Confirm password
        </label>
        <input
          id="register-confirmation"
          name="confirmation"
          className="input"
          type="password"
          autoComplete="new-password"
          minLength={12}
          maxLength={128}
          required
          disabled={pending}
          aria-invalid={state.error ? "true" : undefined}
          aria-describedby="register-help"
        />
        <p id="register-help" className={`field__help${state.error ? " field__help--error" : ""}`} aria-live="polite">
          {state.error ?? ""}
        </p>
      </div>
      <button type="submit" className="btn btn--primary" data-state={pending ? "loading" : undefined} disabled={pending}>
        Create account
      </button>
    </form>
  );
}

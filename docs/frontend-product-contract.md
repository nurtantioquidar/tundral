# Monet frontend product contract

## Purpose

This document defines the product behavior and service contracts that a frontend implementation
must preserve. It is intentionally not a pixel-level design specification. A frontend agent may
replace the current visual language, layouts, components, navigation, and information architecture
when that produces a clearer product, provided the route responsibilities, security boundaries,
money semantics, and workflows below remain intact.

The target experience is a credible developer-finance product:

- simple enough for a new customer to understand without knowing the ledger architecture;
- precise enough for an operator to make consequential billing and routing changes safely;
- calm, legible, and trustworthy rather than decorative or dashboard-dense;
- usable on mobile, but optimized for developers and operators working on laptop and desktop.

Do not treat the existing React markup or CSS as the design brief. Treat this contract and the
gateway responses as the brief.

## Product model

Monet has four concepts that must remain visually and verbally distinct:

1. **Account** — the customer's browser identity, shared prepaid balance, and account-wide limits.
2. **API key** — a revocable workload credential with an optional lifetime spending cap.
3. **Usage** — settled requests attributed to a model and API key.
4. **Upstream route** — an operator-owned mapping from a public Monet model to one or more provider
   capabilities.

An API-key spending cap is not a separate balance. It is a guardrail over the shared account
balance. Do not use wallet, allocation, sub-account, or transferable-balance language for it.

Money is represented by the backend as integer micro-USD. The UI may format it as USD, but must
never perform billing calculations or introduce floating-point values into gateway commands.

## System boundary

```text
Browser
  │
  │ customer/admin session + server actions
  ▼
apps/web (Next.js backend-for-frontend)
  │
  │ scoped GATEWAY_WEB_KEY / GATEWAY_ADMIN_KEY, server-side only
  ▼
apps/inference-gateway /internal/* contracts
  │
  └── sole writer for accounts, keys, credits, limits, usage, routes, and ledger

Customer harness ── customer API key ──► apps/inference-gateway /v1/*
```

Browser code must never fetch the gateway, hold the gateway admin key, or receive upstream
credentials. `apps/web/src/server/gateway-client.ts` is the executable BFF adapter;
`packages/contracts` is the source of truth for cross-application JSON shapes. If an HTTP contract
changes, update the contracts package, gateway producer, BFF adapter, this document's semantics, and
every consuming view in the same change.

Customer registration, authentication, console reads, and API-key mutations use the restricted
`GATEWAY_WEB_KEY`. Operator pages use `GATEWAY_ADMIN_KEY`. Customer routes must never borrow the
admin transport merely because both target `/internal/*`.

## Route contract

### `/` — public landing page

Goal: explain the product and get a qualified customer to registration without requiring them to
understand reservations, reconciliation, micro-USD, routing, or provider internals.

Required content:

- one plain-language value proposition;
- OpenAI-compatible positioning;
- the three-step path: register, receive/add credit, connect a client;
- current public models and pricing;
- primary registration CTA and secondary sign-in CTA.

Rules:

- keep detailed ledger and routing explanations out of the first viewport;
- do not advertise integrations or capabilities the gateway does not implement;
- do not invent context windows, availability claims, or provider identities;
- the configured public gateway base URL comes from `NEXT_PUBLIC_GATEWAY_URL`;
- pricing copy must agree with the gateway rate card. Prefer a server-side pricing read when the
  public landing page becomes dynamic; never silently let static prices drift.

### `/register` — customer registration

Required fields: name, email, password, password confirmation.

Required states:

- idle;
- submitting;
- validation failure;
- duplicate-account or gateway failure;
- success with the initial API-key secret;
- explicit acknowledgement that the secret is shown once;
- a clear continuation into `/console`.

Registration creates the account and one default API key. The one-time secret may exist only in the
immediate server-action result. It must not appear in a URL, persistent browser storage, logs,
analytics, or a later server-rendered page.

### `/signin` — customer sign-in

Required fields: email and password.

Rules:

- never ask for an API key as the browser login credential;
- use one generic invalid-credentials message for incorrect credentials and frozen accounts;
- preserve an accessible error association and a disabled/loading submit state;
- successful sign-in redirects to `/console`.

### `/console` — customer console

Primary questions, in order:

1. How much shared balance is available?
2. Which API keys can spend it?
3. How much has each key spent and how much of its cap remains?
4. What recent usage caused the spend?

Required capabilities:

- show available balance, recent spend, and token totals;
- list active and revoked API keys;
- create more than one API key;
- optionally set, edit, or remove a lifetime spending cap;
- rename and permanently revoke a key;
- show a new secret exactly once;
- attribute usage rows to API-key name and model;
- sign out.

Required key presentation:

- name and masked prefix;
- active/revoked status;
- settled spend;
- active holds when they materially affect remaining cap;
- cap and calculated remaining amount, or an explicit “No cap” state;
- creation date;
- destructive revocation confirmation.

The console must remain useful for an empty account: zero balance, zero usage, and no keys are valid
product states, not errors.

### `/admin/signin` — operator sign-in

Required field: operator password.

This session is independent of the customer session. Do not merge customer and operator identity or
navigation. Operator auth failures must not reveal deployment configuration values.

### `/admin` — operator overview

Primary questions:

1. Is the service financially and operationally coherent?
2. Which customers need action?
3. What is the recent usage and liability?

Required capabilities:

- customer count and active/frozen state;
- posted balance liability and recent usage totals;
- customer list with account email when registered and “managed only” when not;
- active-key count and customer limits;
- managed-customer creation with one-time initial key;
- navigation to customer detail and upstream routing.

Avoid vanity charts. Every visualization must answer an operator question and have a useful empty
state.

### `/admin/customers/:id` — operator customer detail

Required capabilities:

- available balance, active holds, lifetime credits and usage, and reconciliation state;
- freeze/unfreeze;
- account-wide daily and per-request guardrails;
- idempotent promotional/offline credit issuance with a unique reference;
- key creation, spend/cap/remaining visibility, and revocation;
- usage attributed to model and API key.

Risky actions need explicit language. Credit references are idempotency identifiers, not free-form
notes. Never imply that changing a limit rewrites historical usage.

### `/admin/upstreams` — provider pool and model routing

This is a guided configuration workflow, not a generic settings form.

The UI must expose these stages in order:

1. **Runtime state** — configuration source, routability, targets, and in-flight work.
2. **Provider** — name, transport (`openai_http` or `codex_cli`), base URL when applicable,
   enabled state, and concurrency.
3. **Credential** — write-only provider key, credential name, weight, concurrency, and optional
   canonical-model restriction. Credentials do not apply to `codex_cli`.
4. **Capability** — canonical model, provider model ID, and supported API paths. When an HTTP
   provider has a stored credential, prefer offering the provider's own reported model list
   (`GET /internal/upstreams/:id/available-models`) over hand-typed IDs, with manual entry as the
   fallback; a discovered model Monet does not sell needs an explicit mapping before it can be
   declared.
5. **Route publication** — select semantically compatible capabilities, then set priority and
   weight for each public model.

Rules:

- database-backed configuration requires `UPSTREAM_CONFIG_SOURCE=database` and a stable
  `UPSTREAM_CREDENTIAL_ENCRYPTION_KEY`;
- an incompatible runtime mode must render actionable guidance, never a framework error page;
- creating a provider reveals the credential and capability forms for that provider;
- lower priority numbers run first; equal priorities split traffic by weight;
- never offer a capability whose canonical model is incompatible with the public model;
- provider tests can spend real provider credit and must say so before invocation;
- credentials are write-only: show metadata and version, never the stored secret;
- show whether a successful edit was applied to the running registry or only staged;
- revision conflicts should tell the operator to refresh instead of silently overwriting changes;
- no route is a valid initial state. Explain that generation remains unavailable until compatible
  routes are published.

## BFF data contract

The UI consumes these semantic shapes from `@monet/contracts` through
`src/server/gateway-client.ts`.

### Customer

- identity: `id`, `name`, optional `email`;
- state: `active`, `created_at`, `updated_at`;
- limits: `daily_spend_limit_micros`, `per_request_limit_micros`;
- overview-only: `posted_balance_micros`, `active_key_count`.

### API key

- identity: `id`, masked `key_prefix`, `name`;
- state: `active`, `created_at`, optional expiry/revocation timestamps;
- traffic limits: `max_concurrency`, `rate_limit_per_minute`;
- financial guardrail: optional `spend_limit_micros`;
- derived backend values: `spent_micros`, `active_holds_micros`.

The UI may calculate display-only remaining cap as:

```text
max(0, spend_limit_micros - spent_micros - active_holds_micros)
```

It must not persist that derived value or treat it as ledger balance.

### Usage event

- request identity and timestamp;
- public model and immutable rate-card version;
- API-key ID and display name;
- input, cached-input, cache-write, and output tokens;
- settled micro-USD cost and balance snapshots;
- upstream HTTP status.

Do not expose private route-target IDs or provider topology in customer usage.

### Upstream routing

- runtime summary describes whether database configuration is applied;
- providers define transport and provider-wide concurrency;
- credentials define write-only secrets, weights, concurrency, and optional model restrictions;
- capabilities map canonical models to provider-specific model IDs and API paths;
- routes are revisioned snapshots with priority/weight targets.

## Interaction and state contract

Every data-dependent route must deliberately handle:

- loading or pending mutation;
- empty but healthy state;
- field validation;
- authentication expiry;
- gateway unavailable;
- unauthorized or frozen customer;
- conflict/revision mismatch;
- successful mutation;
- destructive-action confirmation.

Do not allow an exception to fall through to the Next.js error overlay for an expected operational
state. Convert known `GatewayError` statuses into page-level notices or field-level action results.
Unexpected errors may still use the application error boundary, but must not include secret-bearing
gateway response bodies.

Server-rendered data is authoritative after each mutation. Revalidate the smallest affected routes
and do not maintain a second client-side copy of balances, usage, or routing configuration.

## Content contract

Use customer language:

- “available balance,” not “posted ledger minus reservations” as the primary label;
- “API-key budget” or “lifetime spending cap,” not “sub-balance”;
- “provider,” “model,” and “route” in operator UI;
- “cached input,” not backend field names;
- “shown once” beside every newly revealed secret.

Technical detail belongs in supporting copy, tooltips, or operator-only contexts. Error messages
must say what happened and what the person can do next.

## Visual and responsive contract

The current tokens and CSS are implementation material, not immutable brand direction. A redesign
may change them coherently.

Regardless of visual direction:

- use a small semantic token system for color, type, spacing, radius, elevation, focus, success,
  warning, and error;
- do not scatter raw color values or one-off layout constants through components;
- reserve monospace for code, masked secrets, IDs, and tabular numeric data;
- establish one obvious primary action per page or panel;
- use progressive disclosure on operator forms; do not render every advanced field at equal weight;
- use tables for dense comparisons and cards/forms for individual objects and actions;
- preserve meaningful content order without CSS;
- meet WCAG AA contrast, keyboard access, visible focus, label association, and status/error
  announcement requirements;
- support at least 360px-wide mobile, a common tablet width, and desktop without clipped actions or
  unreadable tables;
- respect reduced motion and avoid motion that blocks task completion.

Avoid inline style objects for ordinary layout. Prefer reusable classes or locally scoped styles.
Inline values are acceptable only when they are genuinely data-dependent.

## Definition of done for frontend work

A frontend change is complete only when:

- it preserves the service and money boundaries in this contract;
- every affected route has healthy, empty, error, pending, success, and mobile behavior considered;
- one-time secrets remain one-time;
- expected gateway failures render product UI rather than framework errors;
- keyboard-only navigation can complete the changed workflow;
- responsive behavior has been inspected at mobile and desktop widths;
- `bun run typecheck` and `bun run build` pass;
- auth or data-flow changes are exercised against a disposable gateway database;
- the changed browser workflow is tested end to end, including the post-mutation authoritative
  server state.

If a visual redesign changes the component model or tokens, update this contract only when product
behavior or durable design principles changed. Do not record screenshots, iteration history, or a
temporary design rationale here.

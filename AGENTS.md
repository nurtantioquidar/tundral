# Monet Agent Guide

## Repository purpose

Monet is a monorepo for metered AI products. It contains independently deployable applications and
the small set of packages that genuinely need to be shared between them. The active product consists
of the prepaid OpenAI-compatible inference gateway and one web product shell.

Build new capabilities as product features, not as arbitrary packages. A feature becomes a separate
application only when it has its own runtime, deployment lifecycle, trust boundary, data ownership,
or scaling profile.

## Instruction hierarchy

- This file governs the whole monorepo.
- Every deployable app and non-trivial shared package must have its own `AGENTS.md` describing its
  purpose, ownership boundaries, commands, invariants, and verification requirements.
- The closest `AGENTS.md` to a changed file adds to or overrides broader guidance.
- `AGENTS.md` is canonical. Each directory containing one must keep `CLAUDE.md` as a relative
  symlink to it; never maintain copied instruction files.
- Update instructions only for durable knowledge. Do not add task history, temporary status, or
  speculative implementation details.

## Repository layout

```text
monet/
├── apps/
│   ├── inference-gateway/  # Customer model API, pilot ledger, and upstream routing
│   └── web/                # Landing page, customer console, and operator BFF
├── packages/
│   └── contracts/          # Versioned JSON transport contracts shared by both apps
└── docs/                   # Cross-application architecture and operating decisions
```

Expected future deployables, created only when implementation begins:

- `apps/control-plane`: identity, organizations, entitlements, payments, and operator workflows if
  those concerns outgrow the web backend or inference gateway.
- `apps/<feature-name>`: a feature-specific backend such as a Sprite-like workload only when it
  needs an independent runtime or scaling boundary.

Possible future shared packages, created only when there are real consumers:

- `packages/ui`: shared design-system primitives used by multiple frontends.
- `packages/config`: narrowly shared lint or TypeScript configuration.

Do not create generic `utils`, `common`, or `shared` dumping grounds. Keep code local until sharing
is proven.

## Product and frontend architecture

- Use one web application as the product shell. Users should experience Monet as one product even
  when features are backed by different services.
- `docs/frontend-product-contract.md` defines the durable route, interaction, content, responsive,
  and BFF contract for `apps/web`. Frontend agents must read it before redesigning product surfaces.
- Organize the web app by feature routes and feature-local components. Do not build a separate
  frontend for every backend service unless it is intentionally a separate product.
- Browser clients must never call internal gateway administration routes. They call the web backend
  or control plane, which performs authorization and invokes private service APIs.
- Coding harnesses call the inference gateway's public OpenAI-compatible endpoints directly using
  scoped customer API keys.
- Keep public API contracts explicit and versioned. Do not share database models as API types.

## Service and data boundaries

- Each service owns its tables and is the only writer to its database. Never query another
  service's database directly.
- During the pilot, `apps/inference-gateway` remains the single owner of customer password accounts,
  API keys, credits, reservations, usage events, and its immutable ledger. `apps/web` owns signed
  browser sessions but persists no customer credentials or balance data. Do not create a second
  balance or ledger implementation in a new app.
- If identity, payments, or the ledger later move to a control plane, perform an explicit migration
  with one authoritative writer, idempotent commands, reconciliation, and a rollback plan.
- Cross-service writes use authenticated APIs or durable events with stable idempotency keys.
- Monetary values use integer micro-USD end to end. Never use floating-point arithmetic for money.
- Preserve historical rate-card versions; never reprice settled usage in place.

## Dependency rules

- Applications may depend on packages; packages must not depend on applications.
- Applications must not import source files from sibling applications.
- Cross-application JSON types and runtime boundary validators belong in `@monet/contracts`; database
  rows, domain services, sessions, and UI components do not.
- Shared packages must expose intentional public entry points and remain runtime-agnostic unless
  their name and instructions explicitly say otherwise.
- Prefer HTTP or event contracts between deployables over in-process imports that couple releases.
- Avoid a distributed system until the operational boundary is real. A modular monolith is valid
  for the web/control plane while the team and product are small.

## Security and privacy

- Never commit API keys, upstream credentials, customer prompts, source code, payment secrets,
  database files, or authentication state.
- Store secrets in environment variables or the deployment platform's secret store.
- Public reverse proxies must block every `/internal/*` route.
- Use least-privilege service credentials and scoped customer keys. Secrets are displayed once and
  stored only as cryptographic hashes where supported.
- Do not log authorization headers, prompts, source code, raw payment payloads, or complete upstream
  diagnostics.

## Workspace commands

Run from the repository root:

```bash
bun run check
bun run test
bun run dev:inference
```

Root commands orchestrate module commands; module `AGENTS.md` files define any additional required
verification. A root check must remain safe and deterministic and must not use a live billing
database or invoke a paid upstream.

## Change discipline

- Keep changes scoped to the owning module. If a change crosses modules, state the contract being
  changed and verify every consumer.
- Add a new workspace only for a concrete implementation, not merely a possible roadmap item.
- Record cross-cutting architectural decisions in `docs/`; keep module-specific operations inside
  that module.
- Database migrations must be additive or explicitly staged, preserve settled financial history,
  and include reconciliation checks.
- Before moving ownership across services, document source of truth, migration order, failure
  recovery, and rollback.

## Deployment principles

- Deploy each app independently from its own directory and health checks.
- The inference gateway requires persistent storage and a long-lived Codex process; do not deploy it
  to ephemeral serverless storage.
- Keep production data outside the source tree and back it up off-host.
- Start with one inference-gateway instance and SQLite. Introduce Postgres, Redis, queues, or
  multiple instances only after measured concurrency requires them.

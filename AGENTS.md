# Monet Agent Guide

## Repository purpose

Monet is a monorepo for a line of small, metered products. It contains independently deployable
applications and the small set of packages that genuinely need to be shared between them. Each
product is reached on its own subdomain, owns its own accounts and data, and ships on its own
schedule. `monet.sh` is only the index that routes a visitor to the right product.

The active products are Inver (a prepaid OpenAI-compatible inference gateway, `inver.monet.sh`) and
Regretz (an anti-portfolio, `regretz.monet.sh`, currently a placeholder surface).

Build new capabilities as features of an existing product, not as arbitrary packages. A capability
becomes a new product only when it has its own audience and brand; it becomes a separate deployable
within a product only when it has its own runtime, deployment lifecycle, trust boundary, data
ownership, or scaling profile.

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
│   └── hub/                    # monet.sh — product-line index; owns no accounts or data
├── products/
│   ├── inver/
│   │   ├── gateway/            # Customer model API, pilot ledger, and upstream routing
│   │   └── web/                # inver.monet.sh — landing, customer console, operator BFF
│   └── regretz/
│       └── web/                # regretz.monet.sh — anti-portfolio placeholder
├── packages/
│   ├── contracts/              # Versioned JSON transport contracts shared across deployables
│   └── ui/                     # Monet design-token layer shared by every frontend
├── deploy/                     # Edge routing for every host
└── docs/                       # Cross-product architecture and operating decisions
```

`apps/` holds deployables that belong to no single product. Everything a single product deploys
lives under `products/<name>/`. A product directory is created only when the product is real, and a
workspace inside it only for a concrete implementation.

Expected future deployables, created only when implementation begins:

- `apps/control-plane`: identity, organizations, entitlements, payments, and operator workflows if
  those concerns outgrow a product's own backend.
- `products/<product>/<service>`: a product-specific backend only when it needs an independent
  runtime or scaling boundary.

Possible future shared packages, created only when there are real consumers:

- `packages/config`: narrowly shared lint or TypeScript configuration.

Do not create generic `utils`, `common`, or `shared` dumping grounds. Keep code local until sharing
is proven. `packages/ui` is the token layer, not a component grab-bag; a component moves there only
when at least two frontends genuinely render it.

## Product and frontend architecture

- Each product has exactly one web application, on its own subdomain. Users should experience a
  product as one coherent thing even when its features are backed by different services.
- Do not build a separate frontend for every backend service. A new frontend is justified only by a
  new product — a separate brand, audience, and release lifecycle — not by a new service.
- `apps/hub` is an index, not a shell. It links to products and must never acquire accounts,
  sessions, balances, product data, or a second copy of product content such as pricing.
- Products do not share sessions or accounts. A signed-in Inver customer is not signed in anywhere
  else. Do not introduce cross-subdomain cookies without an explicit identity design.
- `docs/frontend-product-contract.md` defines the durable route, interaction, content, responsive,
  and BFF contract for `products/inver/web`. Frontend agents must read it before redesigning Inver's
  product surfaces.
- Organize each web app by feature routes and feature-local components.
- Every frontend consumes design tokens from `packages/ui`. Do not fork the token layer per product;
  if a product needs its own identity, extend the shared layer deliberately.
- Browser clients must never call internal gateway administration routes. They call the web backend
  or control plane, which performs authorization and invokes private service APIs.
- Coding harnesses call the inference gateway's public OpenAI-compatible endpoints directly using
  scoped customer API keys.
- Keep public API contracts explicit and versioned. Do not share database models as API types.

## Service and data boundaries

- Each service owns its tables and is the only writer to its database. Never query another
  service's database directly.
- During the pilot, `products/inver/gateway` remains the single owner of customer password accounts,
  API keys, credits, reservations, usage events, and its immutable ledger. `products/inver/web` owns signed
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
bun run dev:hub            # monet.sh          :3000
bun run dev:inver          # inver.monet.sh    :3001
bun run dev:regretz        # regretz.monet.sh  :3002
bun run dev:inver-gateway  # Inver public API  :8091
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
- Start with one Inver gateway instance and SQLite. Introduce Postgres, Redis, queues, or
  multiple instances only after measured concurrency requires them.

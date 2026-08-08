# Monet architecture

## Shape of the product

Monet is a line of independent products, not one product with multiple capabilities. Each product
has its own subdomain, brand, accounts, and release schedule. `monet.sh` is an index that routes a
visitor to the right one; it holds no state. The codebase is a monorepo so contracts, product
changes, and migrations can be reviewed together, but deployables retain clear runtime and data
boundaries.

```text
                    monet.sh (hub) ── links only ──► product subdomains
                                                      │
Customer coding harness ───────────────► Inver gateway public API
                                                      │
Browser ─► inver.monet.sh ─► Control plane ───────────┤ private, authenticated commands
   │              │              │                     │
   │              │              │                     └► Future feature services
   │              │              └► identity, entitlements, payments
   │              └► onboarding, usage, keys, feature UIs
   └► never calls /internal service routes directly

Browser ─► regretz.monet.sh (placeholder; no backend, no accounts)
```

Products share the design-token layer in `packages/ui` and nothing else at runtime. They do not
share sessions, accounts, or databases. A signed-in Inver customer is not signed in to Regretz.

The control plane is a target boundary, not a requirement for the current pilot. Today the inference
gateway remains the source of truth for its customer password accounts, keys, credits, reservations,
usage, and ledger. The web backend owns signed browser sessions and invokes the gateway through
narrow internal contracts. See [customer accounts and API keys](customer-accounts-and-api-keys.md).

`packages/contracts` is the executable transport boundary between the applications. It contains
JSON response/request types and narrow runtime validators, not database rows or shared business
logic. The gateway remains the producer and the web BFF remains a consumer; both are verified when a
contract changes.

Within the gateway, capability directories separate accounts, billing, generation, upstream
routing, HTTP transport, and database migration concerns. Migrations are ordered and recorded in
`schema_migrations`; application startup may apply pending additive migrations transactionally but
must not rewrite settled financial history.

## Why a monorepo

- Billing and entitlement changes commonly affect the web product and one or more services.
- API contracts and migrations can be updated atomically in one review.
- A small team avoids repository and release-management overhead while retaining independent app
  deployments.
- Feature code can remain local instead of being prematurely generalized.

The monorepo is not permission to create tightly coupled services. Database ownership and runtime
failures must remain isolated.

## Frontend direction

Use one web application per product, on that product's own subdomain, with feature-oriented routes.
Inver's, for example:

```text
/
/models
/usage
/keys
/billing
/features/<feature-name>
/admin/*
```

The hub at `monet.sh` is deliberately not one of these. It has a single route, no backend, and no
session. Its only content is the product list; anything else duplicated there would drift from the
product that owns it.

The durable route behavior, interaction states, content hierarchy, and BFF data semantics for Inver
are defined in the [frontend product contract](frontend-product-contract.md). That contract
deliberately allows the visual system and component architecture to be redesigned.

A product's web backend acts as a backend-for-frontend. It authenticates browser sessions and calls
private service APIs with service credentials. It must not expose operator credentials or relay
unrestricted paths to internal services.

The web BFF uses separate gateway credentials for customer self-service and operator administration.
The customer credential cannot issue credits, freeze customers, enumerate all customers, or change
upstream routing. Production deployments fail startup when strict credential scoping is enabled and
the gateway credentials are missing or equal.

The owner UI for upstream capacity follows the same boundary. The inference gateway remains the
owner and sole writer of providers, encrypted credentials, provider model capabilities, and
revisioned per-model priority routes. The web backend authorizes each owner action and invokes the
gateway's narrow internal contracts; the browser never receives the gateway admin token, provider
ciphertext, or provider API key. Public model routes may fail over only between capabilities with
the same canonical model identity.

A feature may contribute pages, components, and backend calls inside its product's web app. Create a
separate frontend only when the work is genuinely a separate product — its own brand, audience,
security boundary, and release lifecycle. That test is what makes Regretz a product rather than a
route inside Inver.

## Evolution stages

### Stage 1: controlled pilot

- One Inver gateway process.
- SQLite ledger on persistent disk.
- Offline promotional/payment credits.
- Operator administration over localhost or a private tunnel.
- Customer harnesses use public API keys.

### Stage 2: product shell

- Use `products/inver/web` for signup, usage, API-key management, documentation, and operator workflows.
- Keep the inference ledger authoritative; access it through authenticated internal commands.
- Maintain shared request/response schemas and boundary validators in `packages/contracts`.

### Stage 3: multiple paid features

- Introduce a control plane when identity, entitlements, payments, and wallet behavior need to span
  multiple feature services.
- Choose one owner for wallet and ledger mutations. Feature services submit idempotent usage or
  reservation commands; they do not maintain independent customer balances.
- Add a feature service only when it needs its own scaling or runtime boundary.

### Stage 4: measured scale

- Move from SQLite/in-memory coordination to Postgres and Redis or a durable queue before running
  multiple Inver gateway instances.
- Add capacity-aware queues, monitoring, alerting, and upstream-pool health.
- Scale based on observed active concurrency, not registered-user count.

## Non-negotiable invariants

- One authoritative writer for each monetary balance.
- Every credit, reservation, settlement, and cross-service command is idempotent.
- Historical usage retains the rate card used at settlement.
- Internal administration is never publicly routable.
- Feature services cannot read or write each other's databases.
- Customer prompts and source code are not persisted by the platform unless a future feature
  explicitly requires it and defines retention and consent.

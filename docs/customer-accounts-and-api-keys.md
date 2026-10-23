# Customer accounts and API keys

## Current pilot boundary

Tundral separates browser identity from inference credentials:

- customers register and sign in to `products/inver/web` with an email address and password;
- the web backend keeps a signed, `httpOnly` customer session containing the gateway customer ID;
- API keys are used only by coding harnesses against the inference gateway's public `/v1/*`
  endpoints;
- `products/inver/web` calls narrow `/internal/*` gateway contracts with its server-held admin credential.

The browser never receives the gateway admin credential and never calls an internal gateway route.
The inference gateway remains the sole writer for customer accounts, API keys, balances,
reservations, usage events, and ledger entries during the pilot.

## Registration and sign-in

Registration creates three records in one gateway transaction:

1. a customer;
2. a password account with a normalized, unique email address;
3. one default API key.

The gateway hashes passwords with Argon2id. It never stores or returns a plaintext password. The
default API-key secret is returned once after registration and is stored only as a SHA-256 hash.
The customer must copy it before continuing.

Operator-created managed customers do not automatically receive a browser account. They remain
useful for service accounts and pilot customers that are provisioned directly by an operator.

The current pilot does not yet provide email verification, password reset, organizations, or
external identity providers. Those should move behind an explicit control-plane identity contract
when they are implemented rather than being independently recreated in another service.

## API-key budgets

Customers may create more than one API key and give every key an optional lifetime spending cap in
integer micro-USD. This is a guardrail over the shared customer balance, not a second balance or
sub-ledger.

Before reserving a request, the gateway enforces:

```text
settled usage for key + active holds for key + new reservation <= key lifetime cap
```

The customer balance, customer daily limit, customer per-request limit, key cap, key concurrency,
and key request-rate limit are all checked before upstream execution. A key with no cap may spend
from the shared balance subject to the other account guardrails.

Changing a cap never rewrites historical usage. Setting a cap below a key's historical spend leaves
the history intact and blocks new reservations until the cap is raised or removed. Revocation is
permanent; a replacement key receives a new secret and independent cap.

## Console responsibilities

The customer console shows:

- available shared balance and recent settled usage;
- active and revoked keys;
- settled spend, active holds, and remaining budget for each key;
- one-time secrets immediately after key creation.

The operator console shows the same gateway-owned key budget data, customer account status,
customer-wide daily and per-request limits, credits, reconciliation, and upstream routing.

## Internal contracts

The web backend uses these gateway routes for account and key workflows:

- `POST /internal/accounts/register`
- `POST /internal/accounts/authenticate`
- `GET /internal/pricing`
- `GET|POST /internal/customers/:id/keys`
- `PATCH|DELETE /internal/customers/:id/keys/:keyId`

Every route is protected by the gateway admin credential and must remain unreachable through the
public reverse proxy.

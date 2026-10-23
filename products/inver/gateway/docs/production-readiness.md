# Production readiness

This checklist separates code readiness from deployment readiness. Complete it for every production
environment; passing unit tests alone is not sufficient.

## Required secrets

Generate independent, long random values for:

- `ADMIN_API_KEY`: full operator and accounting access;
- `WEB_BFF_API_KEY`: registration and customer self-service only;
- `METRICS_API_KEY`: loopback metrics scraping;
- `UPSTREAM_CREDENTIAL_ENCRYPTION_KEY`: stable AES key for database-managed provider credentials;
- the web app’s `ADMIN_CONSOLE_PASSWORD` and `SESSION_SECRET`.

Set `REQUIRE_SCOPED_INTERNAL_KEYS=true`. The gateway then refuses to start when the admin and web
credentials are missing or equal. Never send any of these values to browser code.

## Network boundary

- Bind the gateway to loopback.
- Terminate HTTPS at Caddy or an equivalent proxy.
- Return `404` for `/internal/*` and `/metrics` at the public proxy.
- Allow administration and metrics only from the host, a private network, or an authenticated
  tunnel.
- Apply outbound network policy for configured HTTP providers; URL validation is not a complete
  DNS-rebinding defense.

Verify from an external machine that `/internal/customers` and `/metrics` are unreachable.

## Upstream readiness

For a local Codex subscription with database configuration:

```bash
GATEWAY_URL=http://127.0.0.1:8080 \
ADMIN_API_KEY="$ADMIN_API_KEY" \
bun run bootstrap:codex
```

The script is idempotent, does not overwrite existing routes, and does not make a paid generation
request. For HTTP providers, add credentials through the operator UI or internal API, verify model
identity, and publish compatible routes.

Before enabling customers:

1. `/ready` returns `200`.
2. Every public model has at least one compatible available target.
3. A deliberate provider failure releases its billing reservation.
4. A real low-value probe settles against the expected rate card.

## Metrics and alerting

Scrape `GET /metrics` on loopback with `Authorization: Bearer $METRICS_API_KEY`. Alert when:

- `inver_gateway_database_ready` is `0`;
- `inver_gateway_upstream_configured` is `0`;
- `inver_gateway_generation_enabled` unexpectedly becomes `0`;
- readiness fails for more than two health intervals;
- process restarts or disk usage increase unexpectedly.

Metrics intentionally contain no customer IDs, prompts, keys, balances, or provider credentials.

## Reconciliation, backup, and recovery

- Run `scripts/backup-db.sh` on a schedule to storage outside the source tree.
- Back up `UPSTREAM_CREDENTIAL_ENCRYPTION_KEY` separately; the database backup is unusable for
  provider credentials without it.
- Run SQLite integrity checks on every backup.
- Restore a backup into a disposable environment at least monthly.
- Verify every customer reconciliation before and after a restore.
- Keep the database, WAL, and failed copies for incident investigation.

## Verification gates

Run:

```bash
bun run check
bun run test:integration
```

The integration test starts a disposable gateway, SQLite database, and upstream stub. It verifies
credential scoping, registration, crediting, generation, settlement, idempotent replay, usage,
reconciliation, and reservation release.

Also verify the web production build and customer/operator browser workflows against a disposable
gateway.

## Deferred public-product capabilities

The controlled pilot still uses manual/offline credits. A public paid launch additionally requires:

- an independently verified chain watcher or payment-provider integration;
- password reset/change and email verification;
- named operator identities and role-based authorization;
- an incident-response and customer-support process.

Do not treat `POST /internal/usdc/deposits` as a blockchain verifier.

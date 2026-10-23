# Early-access runbook

## Deployment boundary

Run the gateway on loopback and expose only public `/v1/*`, `/health`, and `/ready` routes through
Caddy. The supplied Caddyfile returns 404 for `/internal/*` and `/metrics`; administer and scrape
the service from the host or through an authenticated tunnel.

The systemd unit expects:

- the Tundral repository at `/opt/tundral`;
- service state and the SQLite database under `/var/lib/tundral/inference-gateway`;
- environment configuration at `/etc/tundral-inference-gateway.env`;
- Bun and Codex installed at the paths declared in the environment file;
- the `tundral` service user authenticated with Codex when using `UPSTREAM_MODE=codex-cli`.

Set `GATEWAY_DOMAIN` for Caddy and verify `https://$GATEWAY_DOMAIN/ready` before onboarding anyone.
Use distinct `ADMIN_API_KEY`, `WEB_BFF_API_KEY`, and `METRICS_API_KEY` values and set
`REQUIRE_SCOPED_INTERNAL_KEYS=true`.

For database-backed Codex routing, run `bun run bootstrap:codex` with `GATEWAY_URL` and
`ADMIN_API_KEY` set. It creates missing capabilities and routes without overwriting existing routes
or making a generation probe.

## Upstream pool operations

With `UPSTREAM_MODE=openai`, keep provider API keys in separate environment variables and reference
them with `api_key_env` entries in `OPENAI_UPSTREAMS_JSON`. The pool is immutable for the life of
the process, so restart after adding, removing, or rotating a key. Never put this environment file
in the repository.

Each key has its own `max_concurrency`. Set `MAX_GLOBAL_CONCURRENT_REQUESTS` no higher than the
usable aggregate provider capacity unless deliberate queue-free rejection is acceptable. Use
`models` when a vendor does not serve every upstream model in the Inver rate card.

Before production use, exercise each key directly against its configured endpoint, then test the
gateway with one key temporarily returning a rate limit. The request should succeed through another
key, and a failed request must still release its billing reservation. `/ready` confirms that a pool
is configured; it does not make a paid probe or guarantee that every provider is healthy.

## Create a friend account

Create a customer. The initial key is returned once:

```bash
curl -sS -X POST http://127.0.0.1:8080/internal/customers \
  -H "Authorization: Bearer $ADMIN_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"name":"Alice"}'
```

Add a promotional balance or record an offline payment. Use a unique, stable reference:

```bash
curl -sS -X POST http://127.0.0.1:8080/internal/customers/CUSTOMER_ID/credits \
  -H "Authorization: Bearer $ADMIN_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"amount_usd":"10.00","kind":"promotional_credit","reference":"early-access-alice-001","note":"Initial allocation"}'
```

Use `offline_payment` instead of `promotional_credit` when money was received outside the gateway.
Repeating the exact request is safe; changing the customer, amount, or note for an existing
reference is rejected.

## Limits and emergency controls

Set customer limits with decimal USD strings:

```bash
curl -sS -X PATCH http://127.0.0.1:8080/internal/customers/CUSTOMER_ID \
  -H "Authorization: Bearer $ADMIN_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"daily_spend_limit_usd":"3.00","per_request_limit_usd":"0.50"}'
```

Freeze a customer with `{"active":false}`. Stop all new generations globally with:

```bash
curl -sS -X PATCH http://127.0.0.1:8080/internal/generation \
  -H "Authorization: Bearer $ADMIN_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"enabled":false}'
```

This kill switch is process-local; set `GENERATION_ENABLED=false` before restarting if the stop
must persist.

## Keys

List a customer's keys with `GET /internal/customers/:id/keys`. Create another with:

```json
{
  "name": "alice-laptop",
  "max_concurrency": 1,
  "rate_limit_per_minute": 30,
  "expires_at": "2026-08-15T00:00:00Z"
}
```

Revoke with `DELETE /internal/keys/:key_id`. Secrets are returned only at creation.

## Reconciliation and recovery

Inspect `GET /internal/customers/:id/reconciliation` after every manual credit and when investigating
a balance. `reconciled` must be true. Trigger recovery with `POST /internal/recovery`; the service
also runs it at startup and every `RECOVERY_INTERVAL_MS`.

Expired active reservations are released, and matching in-progress idempotency records become
retryable. Set `RESERVATION_TTL_MS` longer than the maximum upstream timeout.

Scrape authenticated loopback `/metrics` and alert on database readiness, upstream configuration,
generation disablement, and repeated process restarts. See
[`production-readiness.md`](production-readiness.md) for the complete launch gate.

## Backup and restore

Run `scripts/backup-db.sh` from cron or a systemd timer. It uses SQLite's online backup command,
checks the copy, and removes backups older than `BACKUP_RETENTION_DAYS`.

To restore: stop the gateway, retain the damaged database and WAL files for investigation, copy a
verified backup into `DATABASE_PATH`, set ownership to the service account, start the gateway, then
check `/ready` and every customer reconciliation report.

## Friend acceptance checklist

Before sharing a key:

1. `/ready` returns 200 through HTTPS.
2. The customer has exactly the intended credit and limits.
3. The customer has a unique key; never reuse the operator pilot key.
4. OpenCode lists Luna low, Luna medium, and GPT-5.5.
5. Streaming and one complete tool-call/result loop succeed.
6. Usage records show the current rate-card version and balance snapshots.
7. Freezing the customer immediately rejects the key.
8. A database backup passes its integrity check.

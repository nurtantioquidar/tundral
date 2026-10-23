# Inver Inference Gateway

Prepaid, OpenAI-compatible GPT gateway pilot with per-token billing. Early access uses promotional
credits or operator-verified offline payments; automated USDC and QRIS settlement are deferred.

## Pilot models and prices

| Model | Input / 1M | Cache write / 1M | Cache read / 1M | Output / 1M |
|---|---:|---:|---:|---:|
| `gpt-5.6-luna` | $1.00 | $1.25 | $0.10 | $6.00 |
| `gpt-5.5` | $5.00 | $5.00 | $0.50 | $30.00 |

Reasoning effort is chosen per request by the caller (`reasoning_effort` on Chat Completions,
`reasoning.effort` on Responses; `minimal`/`low`/`medium`/`high`) and does not change the price.

Balances are denominated in USD micro-units. One atomic USDC unit (six decimals) credits one USD micro-unit.

## Run

```bash
cp .env.example .env
bun install
bun run start
```

### Internal credentials

Use separate credentials for operator administration and the customer-facing web backend:

```bash
ADMIN_API_KEY=<operator-only secret>
WEB_BFF_API_KEY=<web-backend secret>
REQUIRE_SCOPED_INTERNAL_KEYS=true
METRICS_API_KEY=<monitoring-only secret>
```

The web credential can register and authenticate customers and access customer-console data, but it
cannot manage providers, routing, credits, or other customers. The admin credential retains the
complete internal API. `ADMIN_API_KEY` remains a development fallback for the web backend during
migration; production should require distinct keys. `/metrics` is disabled unless
`METRICS_API_KEY` is configured.

### Upstream provider pool

For one OpenAI-compatible provider, the existing variables still work:

```bash
UPSTREAM_MODE=openai
UPSTREAM_BASE_URL=https://api.openai.com
UPSTREAM_API_KEY=...
```

For multiple API keys or endpoints, configure a startup pool. Each entry is one selectable key, so
repeat `base_url` when several keys use the same endpoint. A base URL may end at the host or at
`/v1`.

```bash
UPSTREAM_MODE=openai
VENDOR_A_KEY_1=...
VENDOR_A_KEY_2=...
VENDOR_B_KEY=...
OPENAI_UPSTREAMS_JSON=[{"id":"vendor-a-1","base_url":"https://api.vendor-a.example/v1","api_key_env":"VENDOR_A_KEY_1","weight":2,"max_concurrency":2},{"id":"vendor-a-2","base_url":"https://api.vendor-a.example/v1","api_key_env":"VENDOR_A_KEY_2","weight":2,"max_concurrency":2},{"id":"vendor-b-1","base_url":"https://api.vendor-b.example","api_key_env":"VENDOR_B_KEY","weight":1,"max_concurrency":4}]
```

`api_key_env` is recommended; `api_key` is also accepted for secret-management systems that inject
the complete JSON. Provider IDs must be unique and URL-safe. Optional fields are:

- `weight`: weighted round-robin share, default `1`;
- `max_concurrency`: simultaneous requests allowed for that key, default
  `UPSTREAM_MAX_CONCURRENCY_PER_PROVIDER`;
- `models`: upstream model IDs this provider can serve. Omit it to allow every configured model;
- `enabled`: set to `false` to leave an entry out of the startup pool.

The gateway skips saturated or cooling keys, retries another key after provider-specific
authentication, quota, rate-limit, endpoint, network, or 5xx failures, and cools a failed key for
`UPSTREAM_FAILURE_COOLDOWN_MS`. It makes at most `UPSTREAM_MAX_ATTEMPTS` attempts. Request-validation
4xx responses are not retried. Environment configuration is loaded once at startup; restart the
service after changing that pool. A separate upstream load balancer also works as a single pool
entry.

### Database-managed provider registry

For an owner/admin UI, set `UPSTREAM_CONFIG_SOURCE=database` and provide a deployment-held
credential-encryption key:

```bash
UPSTREAM_CONFIG_SOURCE=database
UPSTREAM_CREDENTIAL_ENCRYPTION_KEY=<base64-encoded 32-byte key>
```

Generate the encryption key once with `openssl rand -base64 32`. Store it in the deployment secret
store and back it up separately from SQLite. Losing it makes stored provider API keys
undecryptable. The database is authoritative in this mode, including when every provider is
disabled; the gateway never silently falls back to environment credentials.

The owner flow creates a provider and transport/base URL, declares supported model mappings, adds
one or more API keys, and publishes an ordered route for each public model. Database routes may mix
OpenAI-compatible HTTP providers and a Codex subscription. Lower priority numbers run first;
equal-priority targets share traffic by weight. A route can only select capabilities with the same
canonical model, so the gateway never silently substitutes a semantically different model.

Published changes atomically replace the runtime snapshot for new requests. Requests already in
flight finish against their previous snapshot. Provider keys are encrypted with AES-256-GCM,
accepted only on credential creation or rotation, and never returned by read APIs. See
[the provider and routing documentation](docs/upstream-provider-registry.md) for the internal API,
frontend boundary, migration workflow, and operating rules.

To idempotently create the standard Codex subscription provider, capabilities, and public model
routes in database mode, run:

```bash
bun run bootstrap:codex
```

The command never replaces existing routes and does not issue a paid generation request.

For a local pilot backed by the authenticated Codex CLI, set:

```bash
UPSTREAM_MODE=codex-cli
CODEX_EXECUTABLE=codex
CODEX_WORKDIR=.
```

Confirm the machine is authenticated with `codex login status`. The adapter invokes Codex in
ephemeral, read-only mode and meters the `turn.completed` token usage returned by Codex. For a
normal production API upstream, use either the single-provider variables or the provider pool
above.

Create a pilot customer:

```bash
curl -X POST http://localhost:8080/internal/customers \
  -H "Authorization: Bearer $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name":"pilot"}'
```

For early access, add a promotional credit or record an offline payment with a unique reference:

```bash
curl -X POST http://localhost:8080/internal/customers/CUSTOMER_ID/credits \
  -H "Authorization: Bearer $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"amount_usd":"10.00","kind":"promotional_credit","reference":"early-access-alice-001"}'
```

Use `offline_payment` when money was received. Repeating the exact credit is idempotent. Do not use
the USDC endpoint for cash, bank transfer, or QRIS credits.

After a future external chain watcher independently verifies a canonical USDC transfer, credit it idempotently:

```bash
curl -X POST http://localhost:8080/internal/usdc/deposits \
  -H "Authorization: Bearer $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"customer_id":"...","chain":"base","tx_hash":"0x...","log_index":0,"amount_atomic":"10000000","confirmations":10}'
```

The deposit endpoint is an accounting boundary, not a chain verifier. Never call it from a client-supplied transaction hash without independently verifying the chain, canonical USDC contract, recipient, amount, finality, and log index.

Use the returned customer key:

```bash
curl http://localhost:8080/v1/models -H "Authorization: Bearer sk-cg-..."

curl http://localhost:8080/v1/responses \
  -H "Authorization: Bearer sk-cg-..." \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-5.6-luna","input":"Write a TypeScript function","max_output_tokens":1000,"reasoning":{"effort":"medium"}}'
```

## Implemented safeguards

- API keys are stored as SHA-256 hashes and shown only once.
- USDC credits are unique by chain, transaction hash, and log index.
- Customer balances come from an immutable ledger.
- Requests reserve their estimated maximum cost before upstream execution.
- Actual upstream token usage settles the reservation.
- Cached input is subtracted from total input before normal-input billing.
- Per-key concurrency is enforced.
- Customers can be frozen, keys can be created/revoked/expired, and each key has concurrency and
  per-minute request limits.
- Customer-level per-request and UTC-daily spending limits are enforced atomically with holds.
- A global concurrency limit and emergency generation switch protect pooled capacity.
- Expired reservations and crashed in-progress requests are recovered automatically.
- Usage records contain rate-card versions and exact posted balances before and after settlement.
- Manual promotional/offline credits are idempotent and recorded in the admin audit log.
- Only explicitly listed models are accepted.
- OpenAI-style function tools are translated into structured Chat Completions or Responses tool calls.
- `stream: true` uses Codex app-server deltas for genuine incremental text, sends heartbeats while
  waiting, and settles usage even if the client disconnects.

The Codex adapter uses `codex app-server --stdio`. Text arrives through
`item/agentMessage/delta`, tool selection through `item/tool/call`, and billing through
`thread/tokenUsage/updated`. Responses include `x-gateway-stream-mode: live`.

HTTP upstreams are currently converted to non-streaming upstream calls. Environment-mode HTTP
requests use buffered SSE. Database-routed streams use a unified SSE response: Codex targets emit
live deltas, while HTTP targets emit their buffered result through the same stream after completion.
The router never changes targets after any output has been emitted.

Customer harnesses remain responsible for executing returned tool calls and sending tool results in
the next request. The gateway never executes customer-declared tools.

The Codex-backed adapter currently supports one tool call per HTTP turn and rejects
`parallel_tool_calls: true`. Sequential tool loops are supported: preserve the assistant
`tool_calls` entry and send the tool result with its matching `tool_call_id` in the next request.
The adapter converts that history into native `function_call` and `function_call_output` items
inside a new ephemeral Codex thread, so the gateway remains stateless between requests.

For safely retryable billing, send a stable `Idempotency-Key` header with the same request bytes.
A completed duplicate replays its stored response without another charge; a concurrent duplicate
returns HTTP 409, and reusing a key for different request bytes is rejected.
For streaming requests, a completed retry is returned as a buffered semantic replay with
`x-idempotent-replay: true`; event IDs, chunk boundaries, and timing need not match the original
live stream byte-for-byte.

`MAX_OUTPUT_TOKENS_PER_REQUEST` limits accepted requests. HTTP upstreams receive the native output
limit. Codex app-server does not expose a native token-limit field, so its limit is best-effort for
generation but strict for customer billing: output beyond the requested limit is never charged to
the customer.

## Operations

- Liveness: `GET /health`
- Readiness: `GET /ready`
- Prometheus metrics: authenticated `GET /metrics`
- Current pricing: authenticated `GET /v1/pricing`
- Reconciliation: `GET /internal/customers/:id/reconciliation`
- Recovery: `POST /internal/recovery`
- Emergency stop: `PATCH /internal/generation` with `{"enabled":false}`

See [the early-access runbook](docs/early-access-runbook.md) for account onboarding, limits, key
rotation, Caddy/systemd deployment, backup, restore, and acceptance checks. The supplied production
Caddyfile deliberately returns 404 for `/internal/*`.

Run `bun run check` for deterministic unit and build checks. Changes to HTTP authentication,
generation, or billing must also pass `bun run test:integration`, which uses disposable local
servers and a temporary SQLite database. See the [public API guide](docs/public-api.md) and
[production-readiness checklist](docs/production-readiness.md) for integration and rollout details.

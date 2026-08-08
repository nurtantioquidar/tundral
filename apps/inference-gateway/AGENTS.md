# Monet Inference Gateway Agent Guide

## Project purpose

This module is Monet's prepaid, OpenAI-compatible coding-model gateway. It authenticates customers,
meters upstream token usage, settles charges against USD-denominated balances, and records verified
USDC deposits. The models sold are `gpt-5.6-luna`, `gpt-5.6`, `gpt-5.5`, and `gpt-5.4`.

Follow the repository-root `AGENTS.md` first. Keep this application independently deployable and do
not import source files from sibling applications.

## Instruction files

- The repository-root `AGENTS.md` defines cross-module architecture, security, and workspace rules.
- `AGENTS.md` is the canonical project instruction file.
- `CLAUDE.md` must remain a relative symlink to `AGENTS.md` so Codex and Claude Code use the same
  guidance.
- Update only `AGENTS.md`; changes are automatically visible through `CLAUDE.md`.
- Do not replace the symlink with a copied file, because duplicate instruction files will drift.
- After making a meaningful project change, review `AGENTS.md` and update it when the change adds or
  alters durable project knowledge: architecture boundaries, billing invariants, security rules,
  supported models, rate-card semantics, required commands, testing procedures, or operational
  constraints.
- Keep `AGENTS.md` accurate and concise. Record enduring guidance, not implementation history,
  temporary debugging notes, task status, or a running change log.
- If no durable guidance changed, leave `AGENTS.md` untouched.

## Module ownership

This application owns:

- the public OpenAI-compatible model API;
- upstream Codex/OpenAI execution and response translation;
- API-key authentication, pilot password accounts, and gateway-local customer/key limits;
- model rates, reservations, token metering, usage settlement, and reconciliation;
- the pilot customer, credit, and immutable-ledger records until an explicit control-plane migration.

This application does not own:

- browser UI, browser sessions, product navigation, or feature discovery;
- general Monet identity, organizations, marketing, or customer support workflows;
- payment-provider checkout or unverified blockchain observation;
- execution of customer-declared coding tools;
- unrelated future feature runtimes.

Future web or control-plane applications must use authenticated contracts rather than importing this
module's database classes. Do not duplicate balances in another module. Any transfer of ledger
ownership requires an explicit migration with one authoritative writer and reconciliation.

## Source organization

Keep code inside the owning capability:

- `src/accounts/`: password and account-domain behavior;
- `src/billing/`: rate cards, ledger, reservation, settlement, and reconciliation behavior;
- `src/generation/`: usage extraction and public streaming serialization;
- `src/upstreams/`: routing, registry state, credential protection, and transport adapters;
- `src/http/`: public/internal HTTP parsing, authorization boundaries, and response envelopes;
- `src/database/`: the SQLite facade, transport-independent database types, and ordered migrations;
- `src/config.ts`: validated process configuration;
- `src/server.ts`: runtime composition and metered request orchestration.

Do not add another unrelated endpoint directly to `server.ts`; place internal endpoint parsing in an
`src/http/` router and keep domain behavior outside the router. Do not add new schema mutations to
the database constructor or the gateway database facade. Add the next monotonically numbered,
transactional migration under `src/database/migrations/`.

`src/database/gateway-database.ts` remains a compatibility facade while repositories are extracted.
New database-heavy capabilities should receive a narrow repository rather than making that facade
larger. Never expose repository or SQLite row types through HTTP; cross-app JSON belongs in
`@monet/contracts`.

## Product boundaries

- The customer harness owns conversation history, repository context, memory, retries, and
  compaction.
- The gateway owns authentication, model availability, upstream execution, token metering,
  reservations, settlement, limits, and usage reporting.
- Requests are stateless. Do not silently store, summarize, compact, reorder, or discard customer
  messages.
- Do not add an `auto/coding` route. Expose only explicitly available models.
- Additional models require an explicit rate card and a working upstream adapter.
- `stream: true` uses Codex app-server `item/agentMessage/delta` notifications for genuine
  incremental text. It uses `item/tool/call` for customer tool selection and
  `thread/tokenUsage/updated` for settlement. Upstream execution and settlement must continue after
  a client disconnect.
- Customer-declared function tools are selected by the model and returned in OpenAI-compatible
  structured form. The customer harness executes them; the gateway must never execute those tools.
- The Codex adapter supports one customer tool call per HTTP turn. Reject
  `parallel_tool_calls: true`; the harness may continue an arbitrary sequential tool loop by
  returning each tool result in the next stateless request.
- Chat Completions serialization must preserve assistant `tool_calls` and the matching tool
  message `tool_call_id`. For Codex, convert them to native `function_call` and
  `function_call_output` Responses items and inject them into the ephemeral thread before starting
  the next turn. A text or JSON transcript is not sufficient and causes repeated tool calls.

## Rate card

Rates are USD per one million tokens:

| Model | Input | Cache write | Cache read | Output |
|---|---:|---:|---:|---:|
| `gpt-5.6-luna` | $1.00 | $1.25 | $0.10 | $6.00 |
| `gpt-5.6` | $5.00 | $6.25 | $0.50 | $30.00 |
| `gpt-5.5` | $5.00 | $5.00 | $0.50 | $30.00 |
| `gpt-5.4` | $2.50 | $2.50 | $0.25 | $15.00 |

`gpt-5.6` is OpenAI's alias for `gpt-5.6-sol`; Monet sells the alias ID because providers report
it from `/v1/models`. The card is flat: OpenAI's long-context surcharge (above 272K input tokens,
2x input / 1.5x output upstream) is not passed through.

Reasoning effort is a per-request customer choice, never part of a public model ID. Generation
requests accept an optional effort (`reasoning_effort` on Chat Completions, `reasoning.effort` on
Responses) limited to `minimal`, `low`, `medium`, or `high`; it is validated before reservation,
forwarded to the upstream, and bills at the same rate regardless of value. The retired
`gpt-5.6-luna-low`/`-medium` aliases (rate-card versions before 2026-08-02.1) must keep their
original version string on settled usage; stored routes for retired public IDs load as dormant
rather than blocking startup.

Treat `input_tokens` as total input, including cached input. Subtract cached and cache-write tokens
before charging the normal input rate. Never double-charge cached tokens.

Reasoning tokens are part of output-token consumption when the upstream includes them in its output
total. Preserve the upstream usage object and expose the gateway-calculated charge.

## Money and ledger rules

- Never use floating-point arithmetic for money.
- Store USD amounts as integer micro-USD. `1 USD = 1,000,000` micro-USD.
- USDC has six decimals; one canonical USDC atomic unit maps to one micro-USD after verification.
- Customer balances are derived from immutable ledger entries minus active reservations.
- Early-access promotional and offline-payment credits must use idempotent manual-credit ledger
  entries with unique operator references. Never represent them as fake USDC deposits.
- Reserve the estimated maximum request cost before invoking an upstream.
- Codex CLI reservations include a configurable scaffold-overhead allowance. Settlement must never
  exceed the held reservation; a rare estimate overrun is gateway risk, not customer overdraft.
- Settle against actual upstream usage and release unused reserved value.
- A caller-supplied `Idempotency-Key` is scoped to its API key and request body. Settlement and
  storage of the replayable successful response must occur in the same SQLite transaction.
  Concurrent duplicates return a conflict, completed duplicates replay without another charge,
  and failed attempts may be retried with the same key.
- Enforce `MAX_OUTPUT_TOKENS_PER_REQUEST` during request validation. With HTTP upstreams, forward
  the limit natively. Codex app-server has no native output-token cap, so pass it as an instruction
  and never bill output tokens beyond the requested limit; upstream excess is gateway risk.
- Release the reservation when the upstream fails before billable usage is obtained.
- Reservations carry the request ID and an expiry. Startup and periodic recovery release expired
  holds and mark matching in-progress idempotency records retryable. Keep the reservation TTL longer
  than the upstream timeout.
- Make deposits and usage settlement idempotent using stable external references.
- A customer deposit is initially a service-credit liability, not recognized revenue.
- A per-key lifetime spending cap is a reservation guardrail over the shared customer balance, not
  a sub-ledger. Enforce settled key usage plus active key holds plus the new reservation against the
  cap without rewriting historical usage.
- Every usage event stores the immutable rate-card version and posted balance before/after the
  charge. Reconciliation compares usage charges with their ledger entries; historical rows retain
  `legacy` rather than being repriced.

## USDC boundary

`POST /internal/usdc/deposits` is an internal accounting boundary, not a blockchain verifier.

Before calling it, an external watcher must independently verify:

- supported chain;
- canonical USDC contract;
- receiving address;
- transfer amount and log index;
- required confirmations/finality;
- transaction has not already been credited.

Never trust a transaction hash or token contract supplied by a customer without independent
on-chain verification. Support one configured chain at a time during the pilot.

## Upstream adapters

- `UPSTREAM_MODE=openai` uses immutable runtime snapshots of OpenAI-compatible endpoint/API-key
  entries. `UPSTREAM_CONFIG_SOURCE=environment` loads `OPENAI_UPSTREAMS_JSON`, with the legacy
  `UPSTREAM_BASE_URL` and `UPSTREAM_API_KEY` pair as its single-provider fallback.
- `UPSTREAM_CONFIG_SOURCE=database` makes the gateway-owned provider, credential, model-capability,
  and model-route registry authoritative. Database routes may mix `openai_http` and `codex_cli`
  targets; `UPSTREAM_MODE` applies only to environment mode.
- Each public model has an explicit, revisioned route. Lower priority numbers run first; targets in
  the same priority tier use weighted selection. Never route to a capability whose canonical model
  differs from the public model's rate-card upstream model.
- Route publication validates the complete proposed snapshot before committing, then atomically
  swaps it for new requests. In-flight requests retain the previous snapshot. Once streaming output
  starts, never fail over to a different target.
- `UPSTREAM_MODE=codex-cli` uses the locally authenticated Codex app-server over stdio, with one
  ephemeral read-only thread per customer request.
- Keep adapter-specific transport and response translation outside billing code.
- The HTTP pool may select several keys for the same endpoint, different endpoints, or an external
  load balancer. It uses weighted round-robin selection, per-key concurrency, bounded failover, and
  temporary failure cooldowns. Health and cooldown state are process-local, not configuration.
- Prefer `api_key_env` references in environment pool JSON. Database-managed upstream credentials
  use AES-256-GCM with `UPSTREAM_CREDENTIAL_ENCRYPTION_KEY`, which must remain outside SQLite and be
  backed up separately. Keys are accepted only at creation/rotation and never returned by reads.
- Database-managed HTTP provider URLs require HTTPS and reject literal private-network destinations
  unless explicit local-development overrides are set. Production also needs egress restrictions;
  validation alone is not a complete DNS-rebinding defense.
- Never return provider IDs, credentials, or pool topology through customer endpoints.
- Bill only usage reported by the selected upstream adapter; do not invent actual token counts from
  character estimates.
- Character-based estimates may be used only to reserve a conservative maximum before execution.
- Do not read, print, copy, or persist the contents of `~/.codex/auth.json`.
- Do not expose upstream credentials, account identifiers, internal pool identifiers, or raw command
  diagnostics to customers.

The Codex CLI adds agent scaffolding to requests. Its reported input and cached-input tokens are
billable gateway usage and may vary between otherwise identical calls. Keep this visible in usage
records rather than concealing or normalizing it.

## API behavior

Public customer endpoints:

- `GET /v1/models`
- `POST /v1/responses`
- `POST /v1/chat/completions`
- `GET /v1/usage`
- `GET /v1/pricing`

Internal administrative endpoints:

- `POST /internal/customers`
- `GET /internal/customers`
- `GET|PATCH /internal/customers/:id`
- `GET|POST /internal/customers/:id/keys`
- `PATCH|DELETE /internal/customers/:id/keys/:keyId`
- `DELETE /internal/keys/:id`
- `POST /internal/accounts/register`
- `POST /internal/accounts/authenticate`
- `GET /internal/pricing`
- `GET /internal/rate-card`
- `POST /internal/customers/:id/credits`
- `GET /internal/customers/:id/usage`
- `GET /internal/usage/summary`
- `GET /internal/customers/:id/reconciliation`
- `POST /internal/recovery`
- `GET /internal/audit`
- `PATCH /internal/generation`
- `POST /internal/usdc/deposits`
- `GET|POST /internal/upstreams`
- `GET|PATCH|DELETE /internal/upstreams/:id`
- `GET|POST /internal/upstreams/:id/credentials`
- `PATCH|DELETE /internal/upstream-credentials/:id`
- `POST /internal/upstream-credentials/:id/rotate-key`
- `GET|POST /internal/upstreams/:id/models`
- `GET /internal/upstreams/:id/available-models`
- `GET /internal/upstream-models`
- `PATCH|DELETE /internal/upstream-models/:id`
- `POST /internal/upstream-models/:id/test`
- `GET /internal/model-routes`
- `GET|PUT /internal/model-routes/:publicModelId`

Internal authorization has two service roles:

- `ADMIN_API_KEY` may call every internal route;
- `WEB_BFF_API_KEY` may call registration, authentication, pricing, and customer-scoped read/key
  routes only.

Production must set distinct values and `REQUIRE_SCOPED_INTERNAL_KEYS=true`. `GET /metrics` uses a
third `METRICS_API_KEY`, is intended for loopback scraping, and must be blocked by the public proxy.

Use standard OpenAI-style error envelopes. Reject unsupported models, insufficient balances, and
concurrency excesses before upstream execution. API keys must be shown only once and stored as
cryptographic hashes.

Customer freeze, key revocation/expiry, per-key concurrency/rate limits, customer request/daily
spending limits, per-key lifetime spending caps, the global concurrency limit, and the emergency
generation switch must be checked before upstream execution. API key secrets are returned only at
creation. Customer passwords are Argon2id hashes and must never be returned by an API.

Generation endpoints accept an optional `Idempotency-Key` header containing 1-200 URL-safe
characters. Reusing a key with different request bytes is an error.

Model metadata should eventually declare actual upstream context windows and capabilities. Do not
guess model limits. Until verified, omit uncertain limits rather than publishing inaccurate ones.

## Code conventions

- Runtime: Bun.
- Language: strict TypeScript using ES modules.
- Persistence: `bun:sqlite` with foreign keys and WAL enabled.
- Prefer small modules with explicit interfaces over adding logic directly to `server.ts` or
  `database/gateway-database.ts`.
- Use `bigint` for monetary calculations and validate before converting values for SQLite.
- Validate all customer-controlled JSON before using it in money, database, process, or upstream
  operations.
- Preserve OpenAI-compatible response fields and add gateway metadata without replacing upstream
  usage.
- Avoid logging prompts, source code, authorization headers, API keys, or complete upstream errors.

## Documentation

- Create project documentation under the `docs/` directory when needed.
- Use lowercase, kebab-case filenames, for example `docs/billing-flow.md`.
- Production deployments bind Bun to loopback, terminate HTTPS at Caddy, and keep `/internal/*`
  unreachable from the public reverse proxy. Use `scripts/backup-db.sh` for consistent SQLite
  backups and verify reconciliation before onboarding users.
- Cross-application decisions belong in the repository-root `docs/`; gateway-specific API and
  operations documentation remains here.

## Required verification

Run both commands after every behavior change:

```bash
bun run typecheck
bun test
```

From the repository root, `bun run check` runs the module's complete required check.

For HTTP routing, authorization, billing orchestration, or settlement changes, also run:

```bash
bun run test:integration
```

This test requires permission to bind disposable loopback ports. It uses a temporary database and
stub upstream and must never point at live billing data or a paid provider.

For changes to HTTP routing, authentication, reservations, settlement, or adapters, also run a local
smoke test covering:

1. create one customer;
2. credit a uniquely referenced test deposit;
3. authenticate with the returned customer key;
4. call an allowed model;
5. compare upstream usage with the stored usage event;
6. verify the exact balance deduction;
7. repeat the request and inspect cached-input billing;
8. verify a failed request releases its reservation.

Use disposable databases under `/tmp` for smoke tests. Never reuse a transaction reference across
tests. Stop all temporary gateway processes after verification.

## Safety checks for changes

Before considering billing work complete, verify these invariants:

- no successful request can spend more than the customer's available balance;
- concurrent reservations cannot overdraw the same balance;
- the same deposit cannot be credited twice;
- the same usage event cannot be charged twice;
- cached input is not charged as both cached and uncached input;
- failed upstream calls do not leave permanent holds;
- customer-visible balances reconcile exactly to ledger entries and active reservations.

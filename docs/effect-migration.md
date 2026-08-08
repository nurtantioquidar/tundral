# Effect migration — Inver gateway

## Decision

`products/inver/gateway` migrates to [Effect](https://effect.website) as its runtime foundation:
typed errors, dependency injection through `Layer`, resource safety through `Scope`, and structured
concurrency in place of ad-hoc promise orchestration.

This is a rewrite of a live, money-handling service, not a refactor. It is staged deliberately. The
gateway settles real charges against an immutable ledger; a big-bang rewrite would put every
invariant in `products/inver/gateway/AGENTS.md` at risk in a single unreviewable change.

## Why Effect here

The gateway's hardest problems are the ones Effect is built for:

- **Reservation lifetime.** A reservation must be released on every failure path, including client
  disconnect and upstream timeout. Today that is manual `try`/`finally` discipline. `Effect.acquireRelease`
  makes release structural — it cannot be forgotten by an early return.
- **Typed failure.** Insufficient balance, frozen customer, revoked key, cap exceeded, and upstream
  failure are all distinct outcomes that today share `Error`. Effect puts them in the type signature,
  so a missing case is a compile error rather than a 500.
- **Concurrency.** Global concurrency limits, per-key limits, and weighted upstream failover are
  hand-rolled. `Effect.Semaphore` and structured concurrency replace that with primitives that
  cancel correctly.
- **Configuration.** `Effect.Config` gives validated, typed, testable configuration with a redacted
  type for secrets — directly useful given `ADMIN_API_KEY`, `WEB_BFF_API_KEY`, and
  `UPSTREAM_CREDENTIAL_ENCRYPTION_KEY`.

## Non-negotiable constraints

The migration inherits every invariant in `products/inver/gateway/AGENTS.md`. The following must
hold at the end of **every** stage, not just at the end:

- money stays `bigint` micro-USD; no floating point enters a billing path;
- no successful request spends more than available balance, under concurrency;
- no deposit or usage event is charged twice;
- failed upstream calls leave no permanent hold;
- settlement and the replayable idempotency record commit in the same SQLite transaction;
- customer-visible balances reconcile exactly to ledger entries minus active reservations;
- settled usage keeps its original rate-card version and is never repriced.

**The existing test suite is the contract.** 69 tests currently pass. A stage is complete only when
all of them still pass, unchanged in intent. Tests may be rewritten to call an Effect API, but a
test may not be weakened, skipped, or deleted to make a stage go green.

## Staging

Stages run bottom-up through the dependency graph, so each one lands behind already-migrated code.
Each stage is a separate commit with the full check suite green.

### Stage 1 — foundation and configuration

- Add `effect` to the gateway.
- Re-express `src/config.ts` with `Effect.Config`, keeping `loadGatewayConfig` as a synchronous
  adapter so no caller changes yet.
- Secrets (`ADMIN_API_KEY`, `WEB_BFF_API_KEY`, `METRICS_API_KEY`) become `Config.redacted`.

Exit: `config.test.ts` passes unchanged in intent; nothing else in the gateway imports Effect.

### Stage 2 — error channel

- Define the gateway's tagged error algebra (`InsufficientBalance`, `CustomerFrozen`, `KeyRevoked`,
  `SpendCapExceeded`, `UpstreamFailure`, `IdempotencyConflict`, …) as `Data.TaggedError`.
- Map each to its existing HTTP envelope in one place.

Exit: HTTP responses byte-identical to today. This stage adds vocabulary, not behavior.

### Stage 3 — database layer

- Wrap `bun:sqlite` access in a `Layer`-provided service.
- Transactions become `Effect.acquireUseRelease` so a failure cannot leave one open.
- Migrations stay ordered and transactional; **no migration is rewritten**.

Exit: `database.test.ts` and `migrations.test.ts` pass. Ledger schema untouched.

### Stage 4 — billing

The highest-risk stage. Reservation, settlement, and reconciliation move to Effect, with
reservation lifetime expressed as `acquireRelease`.

Exit: `pricing.test.ts` passes, plus a concurrency test proving parallel reservations against one
balance cannot overdraw it, and a disposable-database smoke test covering the eight steps in the
gateway's required verification.

### Stage 5 — upstreams and streaming

- Adapters become services behind a `Layer`; the HTTP pool's weighted selection, per-key
  concurrency, and cooldowns move to `Semaphore` and `Ref`.
- Streaming moves to `Stream`, preserving the rule that upstream execution and settlement continue
  after client disconnect, and that failover never happens once output has started.

Exit: `codex-adapter.test.ts`, `openai-upstream-pool.test.ts`, `streaming.test.ts`,
`model-router.test.ts`, `upstream-registry.test.ts` pass.

### Stage 6 — HTTP surface and composition

- `src/server.ts` becomes a `Layer` composition run by one `ManagedRuntime`.
- Routes move to `@effect/platform` `HttpApi` only if it preserves every existing path and envelope
  exactly; otherwise the existing router stays and only its internals become Effect.

Exit: `http-integration.test.ts` passes, and the public OpenAI-compatible contract is unchanged —
verified against `products/inver/gateway/docs/public-api.md`.

## Rules for every stage

- One stage per commit. Never mix a stage with unrelated work.
- Never migrate a module and change its behavior in the same commit.
- `packages/contracts` stays framework-neutral. It is consumed by `products/inver/web`, a Next.js
  app that is **not** migrating to Effect; putting Effect types in the transport boundary would
  force Effect into the browser bundle. Effect Schema may be used *inside* the gateway to parse
  contract types, not to define them.
- `products/inver/web`, `apps/hub`, and `products/regretz/web` are out of scope.
- If a stage cannot land green, revert it. Do not carry a red suite into the next stage.

## Rollback

Each stage is one commit on top of a green tree, so rollback is `git revert` of that commit. The
database schema is not modified by any stage; a revert therefore needs no data migration. Should a
stage reach production and misbehave, revert first and diagnose afterwards — the ledger is
authoritative and must not be reasoned about live.

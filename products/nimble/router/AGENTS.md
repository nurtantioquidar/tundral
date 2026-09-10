# Nimble Router Agent Guide

## Purpose

`nimble` is the Nimble product: an intelligent model router exposed as one OpenAI-compatible
endpoint. Clients send Chat Completions requests with `model: "nimble"` (routed) or a concrete
model id (passthrough); Nimble picks the model and provider, protects prompt-cache locality per
session, tracks cost with integer micro-USD, watches provider health, escalates after verified
failures, and learns from task feedback. The design is `products/nimble/NIMBLE_DESIGN.md`; the
optional client contract is `docs/butterfly-nimble-contract.md` (`nimble/v1`).

Nimble is not an agent framework: it never sees tools, repositories, plans, or verification, and
it never tells a client which files to edit, which commands to run, or whether tests pass.

## Ownership boundaries

This module owns model selection, provider selection, session/model affinity, the cache-affinity
ledger, cost estimation, provider health, routing decisions and their stored score components,
telemetry, and feedback statistics — all in one SQLite file (`NIMBLE_DB_PATH`). It does not own
prompts, message contents, tools, or any customer account; it holds provider keys only from the
environment. It never imports Butterfly and Butterfly never imports it; the only coupling is the
versioned JSON contract in `@monet/contracts` (`nimble.ts`).

## Layout

```text
nimble.config.json  # providers, model pool (pricing, capabilities, tiers, evidence priors, cache TTL), weights, health
src/config.ts       # JSON config parsing + env overrides (NIMBLE_HOST/PORT/DB_PATH/CONFIG)
src/pricing.ts      # micro-USD price cards, UTC tiers, actual + expected cost
src/usage.ts        # usage normalization (DeepSeek vs OpenAI cache fields), prefix hash, estimates
src/health.ts       # per-provider circuit breaker + latency EWMA
src/store.ts        # bun:sqlite: sessions, cache_ledger, requests, decisions, feedback, stats
src/router.ts       # deterministic scorer (capability, history, cache, cost, latency, risk, switch)
src/upstream.ts     # OpenAI-compatible upstream POST; error classification for failover
src/server.ts       # /v1/chat/completions, /v1/models, /v1/feedback, /v1/routing/decisions, /v1/stats, /health
src/main.ts         # process entry
scripts/            # fake-upstream.ts + loopback-smoke.sh (end-to-end with Butterfly, no keys)
test/               # offline tests with an in-memory store and a fake upstream fetch
```

## Commands

```bash
bun run check            # typecheck + tests (offline; never calls a provider)
bun run dev              # http://127.0.0.1:8092 with hot reload
bun run start
curl -s localhost:8092/v1/models
curl -s localhost:8092/v1/routing/decisions?limit=5
scripts/loopback-smoke.sh   # Butterfly → Nimble → fake upstream, no keys: proves the nimble/v1 wiring
```

Root: `bun run dev:nimble`, and the root `test` / `typecheck` / `check` chains include this module.

## Invariants

1. **One virtual model.** The routed id is exactly `nimble`. No `nimble/auto`, `auto-fast`,
   `auto-max`, or per-tier aliases; economics and escalation are the router's job.
2. **Messages pass through untouched** except for removing the `nimble` extension, setting the
   selected model id, and forcing `stream_options.include_usage`. Nimble never stores prompts,
   messages, tool schemas, or keys — only ids, counts, durations, hashes, and kinds.
3. **Money is integer micro-USD.** Price cards are versioned and carry `verified`; an unverified
   card marks every cost it produces (`price_verified` on the request row) and must never be
   presented as real money.
4. **Every routed decision is explainable.** All score components for every candidate are stored
   with the decision (`/v1/routing/decisions`). Weights are configuration until enough clean
   feedback exists to fit them; no learned router before that.
5. **Stickiness is the default.** A session stays on its current model unless another model's
   score beats it by more than the switch penalty (cache locality lost + a fixed cost), a
   provider is unhealthy, or escalation is decisive after repeated verified failure.
6. **Provider faults fail over; request faults do not.** 429/5xx/transport/timeout before any
   bytes stream trigger failover for routed requests (bounded by `max_failovers`); 4xx and
   explicit-model requests are proxied as-is. Health is per provider with cooldown doubling.
7. **The contract is optional in both directions.** Requests without the extension and clients
   that ignore the response extension must work unchanged. Unknown contract versions are ignored.
8. Tests use an in-memory SQLite store and a fake `fetch`; the suite never touches the network.

## Verification

Run `bun run check` after every change. Changes to scoring must keep the router tests (first
request, stickiness, decisive escalation, failover) green and update `NIMBLE_DESIGN.md` §8–§9 when
the formula changes.

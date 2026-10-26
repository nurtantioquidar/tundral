---
name: nimble-product
description: Nimble (products/nimble/router, Bun, port 8092) is the model router product; one virtual model "nimble"; contract nimble/v1 with Butterfly; never couple the two
metadata:
  type: project
---

Nimble is a separate Monet product at `products/nimble/router` (`@monet/nimble-router`, Bun +
bun:sqlite, default `127.0.0.1:8092`, config `nimble.config.json`). It exposes one virtual model
id `nimble` (no `nimble/auto`, `auto-fast`, `auto-max`), passes explicit model ids through, and
routes with a deterministic, explainable scorer (capability, evidence-weighted history, cache
affinity, cost, latency, risk, switch penalty; decisive escalation after repeated verified
failure). Providers declare `dialect: chat | responses`; OpenAI uses `responses` (function tools +
reasoning are only available there) with encrypted reasoning round-tripped as `nimble_replay`.
Pool verified live 2026-09-04: glm-5.3-flash (Z.ai), deepseek-v4-flash, gpt-5.6-luna (OpenAI).
Design: `products/nimble/NIMBLE_DESIGN.md`. Contract with Butterfly:
`docs/butterfly-nimble-contract.md` (`nimble/v1`), types in `packages/contracts/src/nimble.ts`.

**Why:** the user wants Butterfly and Nimble each excellent alone and strongest together, with no
lock-in in either direction.

**Status 2026-09-16:** frozen at v4, not a standalone product — see
`docs/nimble-routing-decision.md` and [[butterfly-nimble-direction]]. The contract module
`packages/contracts/src/nimble.ts` had never been committed (router did not build from a clean
checkout); committed 2026-09-16 along with the router's place in the root check chain.

**How to apply:** keep the router building but do not extend it; Nimble never imports Butterfly and vice versa; only the JSON contract couples
them, and it must stay optional. Weights stay configuration until enough clean feedback exists;
do not build a learned router before that. Costs from unverified price cards must be starred.
See [[butterfly-product]].

---
name: monet-effect-migration
description: "The Inver gateway is mid-migration to Effect TS, staged bottom-up; stage order matters"
metadata: 
  node_type: memory
  type: project
  originSessionId: 660592c9-4f70-4a1e-8eb5-8d019c294064
  modified: 2026-08-08T08:50:03.369Z
---

The user asked on 2026-08-08 for a **full Effect TS migration of the Inver gateway**
(`products/inver/gateway`, ~5.5k lines), after being told it is a rewrite of a live money-handling
service rather than a refactor. They accepted the staged sequencing.

The plan lives in `docs/effect-migration.md`: 6 stages, bottom-up — config → error algebra →
database → billing → upstreams/streaming → HTTP composition. **Stage 1 (config) is done.**

**Why:** The user chose the largest of four offered scopes with the risk stated up front, so don't
re-litigate whether to do it — but also don't accelerate it. The 69-test suite is the migration's
contract: a stage is complete only when all of them pass, and a test may never be weakened or
skipped to make a stage go green.
**How to apply:** Before writing Effect code in the gateway, read `docs/effect-migration.md` and
work the current stage only. One stage per commit; never mix a migration with a behavior change.
Effect must stay out of `packages/contracts` — it is consumed by the Next.js app at
`products/inver/web`, which is not migrating. See [[monet-product-line-structure]].

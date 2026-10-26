---
name: monet-frontend-direction
description: Design + stack decisions the user chose for the Monet web frontends
metadata:
  node_type: memory
  type: project
  originSessionId: d7fbc199-ecd1-486a-8024-96b48d98a3b6
  modified: 2026-08-08T08:49:45.525Z
---

For Monet's frontend the user chose **Next.js**, tone **utilitarian**, and audience **developer-first
with team-buyer trust signals**. The discount engine was explicitly deferred — promotional credits
are the only discount lever for now.

**Restructured 2026-08-08 into a product line** (see [[monet-product-line-structure]]). The single
`apps/web` shell is gone. Each product has its own Next.js app on its own subdomain, and design
tokens moved to the shared `packages/ui/tokens.css`.

**Corrected 2026-07-25 — Inver's landing page primary action is `/register`, not sign-in.** The
original 2026-07-24 decision was sign-in-led with operator-created accounts only; self-serve
registration has since shipped and `docs/frontend-product-contract.md` mandates a primary
registration CTA with a secondary sign-in CTA. When memory and that contract disagree about route
behaviour, the contract wins.

**Theme rotated 2026-07-25 Cobalt → Coral, then 2026-08-03 → "terminal-dark"** at the user's
request; they choose to rotate rather than preserve a palette. Terminal-dark is near-black paper +
one warm-amber accent + Geist/Geist Mono.

**Hub diverged 2026-10-08/10:** `apps/hub` (monet.sh) no longer uses `@monet/ui` or terminal-dark. After several rejected designs the user chose modern pixel art: a pixel Giverny pond with a robot painting at an easel, JetBrains Mono, coral pill CTA, water-lily pixel logo. The hub owns its tokens; `apps/hub/DESIGN.md` is its design law and `apps/hub/design-notes.md` records every rejection in the user's words. Inver web still uses terminal-dark from `packages/ui`.

**Why:** The audience/tone/stack answers came from explicit AskUserQuestion answers — don't re-ask
those. The CTA direction and each theme rotation were also explicit choices, made later.
**How to apply:** Reuse the token system in `packages/ui/tokens.css` for Inver rather than introducing raw
values. For the hub, follow `apps/hub/DESIGN.md`. Do not use the Hallmark skill (user rejected it 2026-10-08; it is disabled); ignore `.hallmark/` history. A *warm* accent makes the semantic error colour easy to confuse with the
brand; `packages/ui/AGENTS.md` records the required separation.

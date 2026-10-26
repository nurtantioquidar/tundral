---
name: monet-product-line-structure
description: "Monet is a product line of independent products on subdomains, not one product shell"
metadata: 
  node_type: memory
  type: project
  originSessionId: 660592c9-4f70-4a1e-8eb5-8d019c294064
  modified: 2026-08-08T08:49:57.391Z
---

Decided 2026-08-08: Monet is a **line of independent products**, each on its own subdomain, not one
product with multiple capabilities.

- `monet.sh` → `apps/hub` — index only. No accounts, no backend, no product data. Since 2026-10-08 a static export on Cloudflare Pages (`monet-hub`), not a Node process behind Caddy. See [[monet-domain]].
- `inver.monet.sh` → `products/inver/web` + `products/inver/gateway` — the prepaid
  OpenAI-compatible inference gateway, formerly `apps/web` + `apps/inference-gateway`.
- Regretz (anti-portfolio, `regretz.monet.sh`) was **retired 2026-10-12** at the user's request: web app, Caddy route, docs and root wiring removed (commit dd39ed5). Don't reintroduce it unless asked.

Layout is grouped by product (`products/<name>/<workspace>`); `apps/` holds only deployables
belonging to no single product. Dev ports: hub 3000, inver web 3001, gateway 8080, nimble 8092.

The user chose "hub separate, products separate" over one host-routed Next.js app, and product
grouping over a flat `apps/*` tree — both explicit AskUserQuestion answers.

**Why:** This reverses the repo's earlier "use one web application as the product shell" rule, which
still appears in older doc prose. Products deliberately share **no** sessions, accounts, or
databases — only the `packages/ui` token layer. See [[monet-frontend-direction]] and
[[monet-effect-migration]].
**Standalone products (2026-10-10):** unrelated products such as titikapi (Indonesian fire/air monitor, Python + Vite, separate repo) stay in their own repos and may later be served on a `*.monet.sh` subdomain by DNS alone. Only products that share code with Monet belong in this monorepo.

**How to apply:** Don't propose merging surfaces back into one shell or adding cross-subdomain auth.
A new frontend needs a new *product* (own brand, audience, release lifecycle) — not just a new
service. Never move Inver's landing content, pricing, or sign-in onto the hub; it would drift.

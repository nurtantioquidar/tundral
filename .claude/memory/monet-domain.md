---
name: monet-domain
description: monet.sh domain, DNS, email forwarding, Pages hosting, and the Claude for Startups application
metadata:
  type: project
---

monet.sh is the public domain for the Monet product line. Set up 2026-10-08 (in a session started from ~/.config/nix) to apply to Anthropic's Claude for Startups program, which needs a company email on the website's domain.

- Registrar: Namecheap → Cloudflare transfer paid 2026-10-08; registry still showed `pendingTransfer` on 2026-10-09. Expect expiry to move to 2028-01-10. Check with `whois -h whois.nic.sh monet.sh`.
- DNS on Cloudflare: zone e35a012e9ca5ea03ac02f87c10dfd579, account c47064b0a58c24f05601121615a7e302. `ccproxy.monet.sh` is the user's own Cloudflare Tunnel CNAME; leave it alone.
- Hosting: `apps/hub` is a static export deployed to Cloudflare Pages project `monet-hub` (custom domains monet.sh, www). Deploy command is in apps/hub/AGENTS.md. Not on charon; charon keeps no public ports.
- Email: Cloudflare Email Routing, forward-only, tio@monet.sh → the user's personal Gmail. Delivery was never test-confirmed by the user.
- Cloudflare API token: 1Password Personal `api-token__cloudflare__monet-sh__claude` (field `token-value`), expires 2026-11-30. Scopes: zone read, DNS edit, Email Routing rules/addresses, Pages edit. It cannot enable Email Routing settings (needs a dashboard click).
- Claude for Startups application submitted 2026-10-08 with a Console account on tio@monet.sh. API credits must be claimed on the Console Startups page and expire 6 months after grant.
- The monet git repo has no remote (as of 2026-10-10); commits stay local until the user names one or asks to create it.

**Why:** prerequisites for the startup program and the live public site.
**How to apply:** hub look and history live in apps/hub/DESIGN.md and design-notes.md, not here. Related: [[monet-product-line-structure]], [[monet-frontend-direction]].

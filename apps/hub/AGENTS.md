# Monet Hub Agent Guide

## Project purpose

This module serves `monet.sh`: the index of the Monet product line. Its entire job is to say what
Monet is and route a visitor to the right product subdomain.

Follow the repository-root `AGENTS.md` first.

## Instruction files

- `AGENTS.md` is the canonical project instruction file for this module.
- `CLAUDE.md` must remain a relative symlink to `AGENTS.md`.
- Record durable knowledge only; no task history or temporary status.

## Ownership boundary

This application owns:

- the product-line statement and the list of products with their hosts and status.

This application does not own, and must never acquire:

- accounts, sessions, cookies, sign-in, or registration;
- balances, usage, pricing, or any other product data;
- calls to the Inver gateway or any other product backend;
- a second copy of product content that already lives on a product surface.

The hub is a static index. If a change here needs a server-side fetch, a session, or a secret, the
change belongs on a product surface instead. The strongest reason for this boundary is drift: any
product detail duplicated here will eventually contradict the product that owns it.

## Source organization

- `src/app/page.tsx` holds the product list. Adding a product means adding an entry, not a route,
  and a mark in `src/app/marks.tsx`.
- `src/app/sunrise.tsx` is the self-painting hero (the only client component). A seed fully
  determines the picture; keep every random draw outside the draw closures so seeds stay stable.
- `src/app/marks.tsx` holds the painted tool marks; each has exactly one red region with a stated
  meaning.
- `src/app/globals.css` holds layout; `src/app/tokens.css` holds every colour, size, and font.
- `DESIGN.md` is the hub's design law and `design-notes.md` records what the owner rejected. Read
  both before changing how the hub looks. The hub owns its tokens instead of `@monet/ui` so
  monet.sh reads as a studio; never import the shared token layer here.

## Environment

`NEXT_PUBLIC_INVER_URL` is optional and not a secret. When set, Inver's card links to it (for
example a local dev port). Leave it unset in production until `inver.monet.sh` is served: a product
without a URL renders as an unlinked card, so the index never links to a host that does not resolve.

## Commands

```bash
bun run dev        # dev server on :3000
bun run typecheck  # required verification
bun run build      # static export to out/
```

## Deployment

The hub builds with `output: "export"` and is served from the Cloudflare Pages project `monet-hub`,
which owns the `monet.sh` and `www.monet.sh` custom domains. There is no Node process in production,
so `next start` does not apply. Deploy the built directory:

```bash
bunx wrangler pages deploy out --project-name monet-hub --branch main
```

Wrangler reads `CLOUDFLARE_API_TOKEN` (Pages edit on the Monet account) and `CLOUDFLARE_ACCOUNT_ID`
from the environment; never commit either.

## Required verification

Run both after any change:

```bash
bun run typecheck
bun run build
```

Check the rendered page at a 380px mobile width and at desktop, after the hero finishes painting,
and confirm every product link resolves to the intended host. Keyboard focus must be visible on
every link and on "Paint another".

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

- `src/app/page.tsx` holds the product list. Adding a product means adding an entry, not a route.
- `src/app/globals.css` holds the hub's layout and component styles.
- Design tokens come from `@monet/ui/tokens.css`. Never introduce raw colour, spacing, radius, or
  duration values.

## Environment

Both variables are optional and exist so a local checkout can point at dev ports instead of
production subdomains. Neither is a secret.

- `NEXT_PUBLIC_INVER_URL` — defaults to `https://inver.monet.sh`
- `NEXT_PUBLIC_REGRETZ_URL` — defaults to `https://regretz.monet.sh`

## Commands

```bash
bun run dev        # dev server on :3000
bun run typecheck  # required verification
bun run build      # production build
```

## Required verification

Run both after any change:

```bash
bun run typecheck
bun run build
```

Check the rendered page at a 360px mobile width and at desktop, and confirm every product link
resolves to the intended host. Keyboard focus must be visible on every link.

# Regretz Web Agent Guide

## Project purpose

This module serves `regretz.monet.sh`: Regretz, an anti-portfolio — the things that were built and
did not work, and what each one actually cost to learn.

Follow the repository-root `AGENTS.md` first.

## Current state

This application is a **placeholder**. It renders one holding page and has no data model, no
backend, and no accounts. It exists so the product-line routing is provable end to end before the
anti-portfolio's content model is designed.

Do not grow features incrementally on top of the placeholder. The next substantive change should
decide the entry shape — what was built, what it cost, what it taught — and replace the page.
Record that decision in `docs/` when it is made, and rewrite this section when it is no longer a
placeholder.

## Instruction files

- `AGENTS.md` is the canonical project instruction file for this module.
- `CLAUDE.md` must remain a relative symlink to `AGENTS.md`.
- Record durable knowledge only; no task history or temporary status.

## Ownership boundary

Regretz is a separate product from Inver. It shares no accounts, sessions, balance, or data with
Inver, and must never call the Inver gateway. If Regretz ever needs an account system, it gets its
own or consumes a future control plane — it does not borrow Inver's.

Content is Regretz's own. Do not import copy, components, or server code from a sibling application.

## Source organization

- `src/app/` holds Next.js route entry points and composition.
- Design tokens come from `@monet/ui/tokens.css`. Never introduce raw colour, spacing, radius, or
  duration values.

## Environment

- `NEXT_PUBLIC_HUB_URL` — optional, defaults to `https://monet.sh`. Not a secret.

## Commands

```bash
bun run dev        # dev server on :3002
bun run typecheck  # required verification
bun run build      # production build
```

## Required verification

```bash
bun run typecheck
bun run build
```

Check the rendered page at a 360px mobile width and at desktop.

# Monet Web Agent Guide

## Project purpose

This module is Monet's product shell: the public landing page, the customer console, and the
operator (admin) console for the inference gateway. It is a backend-for-frontend — the browser
never calls the gateway directly; Next.js server code does, with server-held credentials.

Follow the repository-root `AGENTS.md` first. Keep this application independently deployable and do
not import source files from sibling applications.

## Instruction files

- `AGENTS.md` is the canonical project instruction file for this module.
- `CLAUDE.md` must remain a relative symlink to `AGENTS.md`.
- Read `../../docs/frontend-product-contract.md` completely before changing frontend information
  architecture, page layout, components, copy, interaction behavior, or gateway data presentation.
- The frontend product contract is the behavioral brief. Existing JSX and CSS are implementation,
  not a visual specification, and may be replaced by a coherent redesign.
- Record durable knowledge only; no task history or temporary status.

## Module ownership

This application owns:

- the public landing page and product navigation;
- signed browser sessions for customers and operators;
- server-side proxying to the gateway's public and internal APIs.
- customer registration, password sign-in, and self-service API-key workflows over the gateway's
  authenticated internal contracts;
- the operator presentation and server-action workflow for gateway-owned providers, write-only
  credentials, model capabilities, and model-route publication.

This application does not own:

- customers, API keys, credits, usage, reservations, or the ledger — the inference gateway owns
  those; this app reads and commands them only through the gateway's HTTP contracts;
- upstream provider configuration or any billing computation.

Never duplicate balances or usage into a second store. Render what the gateway returns.

## Source organization

- `src/app/` owns Next.js route entry points, metadata, route-local server actions, and composition.
- `src/features/<feature>/` owns product-specific forms, views, and interaction components.
- `src/components/` contains genuinely reusable presentation and layout primitives only.
- `src/server/gateway-client.ts` is the sole BFF transport adapter for gateway HTTP calls.
- `src/server/admin-session.ts` and `src/server/customer-session.ts` keep the two identity contexts
  separate; shared signing mechanics stay in `src/server/session-signing.ts`.
- `src/lib/` is limited to small application-local, runtime-neutral helpers such as formatting.
- Shared gateway JSON shapes come from `@monet/contracts`, never from copied interfaces or sibling
  application source.

Keep client components feature-local unless at least two features genuinely reuse them. A route
should orchestrate server data and feature components; do not turn route files into a second
gateway client or duplicate authoritative billing state in React state.

## Security boundaries

- `GATEWAY_ADMIN_KEY` and `GATEWAY_WEB_KEY` are server-side only. They must never reach client
  components, serialized props, cookies, or logs. The admin key is for operator workflows;
  registration and customer-console calls must use the restricted web key through `gatewayCustomer`.
- Customer registration and password verification happen inside the inference gateway through
  authenticated internal routes. The web app stores only a signed customer ID/name/email session
  in an expiring, `httpOnly` cookie; it never stores customer passwords or API-key secrets.
- API-key secrets are displayed only in the immediate registration or creation result and must
  never be placed in a session, URL, log, or serialized server-component prop after that result.
- Operator sign-in compares against `ADMIN_CONSOLE_PASSWORD` (constant-time) and issues an
  HMAC-signed, expiring, httpOnly session cookie.
- All gateway calls happen in server components, server actions, or route handlers. No fetch to
  the gateway from client components.
- Import `gatewayAdmin` and `gatewayCustomer` only from `src/server/gateway-client.ts`. Customer
  routes must not use `gatewayAdmin`. Client components may import contract types with `import type`,
  but must never import the server transport.
- Do not log API keys, passwords, session cookie values, or complete gateway error bodies.

## Environment

See `.env.example`: `GATEWAY_URL`, `NEXT_PUBLIC_GATEWAY_URL`, `GATEWAY_ADMIN_KEY`,
`GATEWAY_WEB_KEY`, `ADMIN_CONSOLE_PASSWORD`, `SESSION_SECRET`. The app fails closed when required
credentials are unset. `GATEWAY_WEB_KEY` may fall back to the admin key only during migration;
production must provide the scoped value.

## Design system

The current implementation uses tokens in `src/app/tokens.css`, but its theme, component styling,
and page composition are not contractual. A frontend redesign may replace them if it remains
coherent across public, customer, and operator surfaces.

- Preserve the route behavior, content hierarchy, state handling, money semantics, and security
  boundaries in `../../docs/frontend-product-contract.md`.
- Do not treat the current markup, dashboard cards, tables, or Hallmark metadata as mandatory
  patterns.
- Keep raw colors, fonts, spacing, focus, status, and elevation values behind semantic tokens.
- Avoid ordinary layout in inline style objects; use reusable or locally scoped styles.
- Progressive disclosure is required for complex operator workflows, especially upstream routing.
- Expected gateway modes and failures must render actionable UI rather than a Next.js error page.
- Verify changed workflows visually and interactively at mobile and desktop widths.

`.hallmark/log.json` may remain as implementation history, but it is not a product requirement.

## Commands

```bash
bun run dev        # dev server on :3000 (expects the gateway on GATEWAY_URL)
bun run typecheck  # required verification
bun run build      # production build
```

## Required verification

Run both commands after frontend changes:

```bash
bun run typecheck
bun run build
```

For changed routes, test the browser workflow at desktop and mobile widths, including empty,
pending, validation, gateway-error, and success states relevant to the change. For changes to auth,
gateway proxying, or console data flow, also run the app against a disposable gateway database and
verify: registration, password sign-in, customer-session rejection after freeze, balance/usage
rendering, customer key creation/cap updates/revocation, admin sign-in, credit issuance,
managed-customer creation, and any affected upstream-routing workflow.

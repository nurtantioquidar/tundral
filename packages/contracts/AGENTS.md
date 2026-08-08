# Monet Contracts Agent Guide

## Purpose

`@monet/contracts` contains the explicit, transport-level HTTP contracts shared by independently
deployable Monet applications. It exists because both `apps/inference-gateway` and `apps/web`
consume the same request and response shapes.

Follow the repository-root `AGENTS.md` first. `CLAUDE.md` must remain a relative symlink to this
file.

## Ownership boundary

This package owns:

- JSON-serializable public, customer, admin, pricing, and upstream-routing contract types;
- small runtime validators for envelopes or payloads that cross an application boundary.

This package does not own:

- SQLite rows, repository interfaces, domain services, React components, sessions, or environment
  configuration;
- shared business logic or generic helpers;
- source imports from either application.

Keep contract field names identical to the HTTP JSON, including deliberate snake_case fields.
Never export a database class or make an application depend on another application through this
package.

## Change rules

- Contract changes must update the producing gateway endpoint and every consumer in the same
  change.
- Prefer additive changes. Breaking changes require an explicit versioning and migration plan.
- Monetary fields remain integer micro-USD or decimal strings as defined by the endpoint; never
  introduce floating-point billing commands.
- Secret-bearing creation responses may describe a one-time secret field, but validators and tests
  must never contain real credentials.
- Add runtime validation where untrusted JSON enters an application. Static TypeScript types alone
  do not validate network responses.

## Verification

Run from this directory:

```bash
bun run check
```

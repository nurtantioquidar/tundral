# Monet

Monet is a monorepo for metered AI products. Its first deployable application is the prepaid,
OpenAI-compatible inference gateway.

## Current workspace

| Workspace | Purpose | Status |
|---|---|---|
| `apps/inference-gateway` | Model access, streaming, metering, reservations, settlement, and pilot administration | Active |
| `apps/web` | Landing page, customer console, and operator console (backend-for-frontend over the gateway) | Active |
| `packages/contracts` | Shared JSON contracts and boundary validators for the gateway and web app | Active |

Future control-plane and feature-specific applications should be added only when their
implementation and ownership boundaries are concrete. See [the architecture](docs/architecture.md).

## Commands

```bash
bun run check
bun run dev:inference
bun run dev:web
```

Module-specific setup and operations live in each workspace's README and `AGENTS.md`.

Gateway references:

- [Public inference API](apps/inference-gateway/docs/public-api.md)
- [Production readiness](apps/inference-gateway/docs/production-readiness.md)
- [Early-access operations](apps/inference-gateway/docs/early-access-runbook.md)

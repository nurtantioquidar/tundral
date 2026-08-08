# Monet

Monet is a monorepo for a line of small, metered products. Each product is reached on its own
subdomain and owns its own accounts, balance, and deployment lifecycle. `monet.sh` is only the index
that routes a visitor to the right one.

## Product line

| Host | Workspace | Purpose | Status |
|---|---|---|---|
| `monet.sh` | `apps/hub` | Product-line index. Links to each product; owns no accounts or data. | Active |
| `inver.monet.sh` | `products/inver/web` | Inver's landing page, customer console, and operator console (backend-for-frontend over the gateway) | Active |
| — (private API domain) | `products/inver/gateway` | Inver's model access, streaming, metering, reservations, settlement, and pilot administration | Active |
| `regretz.monet.sh` | `products/regretz/web` | Regretz, an anti-portfolio. Placeholder surface only. | Placeholder |

Shared workspaces:

| Workspace | Purpose |
|---|---|
| `packages/contracts` | Versioned JSON transport contracts and boundary validators shared by Inver's gateway and web app |
| `packages/ui` | The Monet design-token layer (`tokens.css`) shared by every frontend |

## Layout

```text
monet/
├── apps/
│   └── hub/                  # monet.sh
├── products/
│   ├── inver/
│   │   ├── gateway/          # public OpenAI-compatible API + ledger
│   │   └── web/              # inver.monet.sh
│   └── regretz/
│       └── web/              # regretz.monet.sh
├── packages/
│   ├── contracts/
│   └── ui/
├── deploy/                   # edge routing for every host
└── docs/                     # cross-product architecture and decisions
```

A product directory groups everything a single product deploys. Add `products/<name>/` only when the
product is real; add a workspace inside it only for a concrete implementation.

## Ports

Local development uses one port per surface. `deploy/Caddyfile` maps the same ports to hosts.

| Port | Surface |
|---|---|
| 3000 | hub |
| 3001 | inver web |
| 3002 | regretz web |
| 8080 | inver gateway (8091 in `dev`) |

## Commands

Run from the repository root:

```bash
bun run check
```

```bash
bun run test
```

Per-surface development:

```bash
bun run dev:hub
```

```bash
bun run dev:inver
```

```bash
bun run dev:inver-gateway
```

```bash
bun run dev:regretz
```

Module-specific setup and operations live in each workspace's README and `AGENTS.md`.

Inver references:

- [Public inference API](products/inver/gateway/docs/public-api.md)
- [Production readiness](products/inver/gateway/docs/production-readiness.md)
- [Early-access operations](products/inver/gateway/docs/early-access-runbook.md)

Cross-product references:

- [Architecture](docs/architecture.md)
- [Frontend product contract](docs/frontend-product-contract.md)

# Monet

Monet is a monorepo for a line of small, metered products. Web products are reached on their own
subdomains; developer products may first ship as local tools. Each product owns its own runtime,
accounts and data when present, and deployment lifecycle. `monet.sh` is only the public index.

## Product line

| Host | Workspace | Purpose | Status |
|---|---|---|---|
| `monet.sh` | `apps/hub` | Product-line index. Links to each product; owns no accounts or data. | Active |
| `inver.monet.sh` | `products/inver/web` | Inver's landing page, customer console, and operator console (backend-for-frontend over the gateway) | Active |
| — (private API domain) | `products/inver/gateway` | Inver's model access, streaming, metering, reservations, settlement, and pilot administration | Active |
| — (local CLI) | `products/butterfly/cli` | Butterfly (`butterfly`), a fast DeepSeek-native, cache-first coding agent | Alpha |
| — (local CLI) | `products/cicada/cli` | Cicada (`cicada`), one concurrent balance check for model API providers | Alpha |
| — (local CLI) | `products/cocoon/cli` | Cocoon (`cocoon`), a credential firewall that keeps upstream secrets outside AI agents | Prototype |
| — (local CLI) | `products/dijkstral/cli` | Dijkstral (`dijkstral`), a code knowledge graph for AI agents, faster than Graphify | Prototype |

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
│   ├── butterfly/
│   │   └── cli/              # butterfly: DeepSeek-native cache-first coding agent
│   ├── cicada/
│   │   └── cli/              # cicada: unified provider balance checker
│   ├── cocoon/
│   │   └── cli/              # cocoon: local agent credential firewall
│   └── dijkstral/
│       ├── cli/              # dijkstral: code knowledge graph for AI agents
│       └── bench/            # head-to-head benchmark against Graphify
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
bun run dev:butterfly -- --help
```

```bash
bun run dev:cicada -- --help
```

```bash
bun run dev:cocoon -- --help
```

```bash
bun run dev:dijkstral -- --help
```

Module-specific setup and operations live in each workspace's README and `AGENTS.md`.

Inver references:

- [Public inference API](products/inver/gateway/docs/public-api.md)
- [Production readiness](products/inver/gateway/docs/production-readiness.md)
- [Early-access operations](products/inver/gateway/docs/early-access-runbook.md)

Cross-product references:

- [Architecture](docs/architecture.md)
- [Frontend product contract](docs/frontend-product-contract.md)
- [Butterfly feature map](docs/butterfly-features.md)
- [Cicada CLI](products/cicada/cli/README.md)
- [Cocoon design](products/cocoon/COCOON_DESIGN.md)
- [Dijkstral design](products/dijkstral/DIJKSTRAL_DESIGN.md)
- [Harness implementation comparison](docs/harness-implementation-comparison.md)

# Dijkstral hosted: design

Status: proposed (2026-10-20). Nothing here is built yet. The local CLI (`DIJKSTRAL_DESIGN.md`)
is unchanged by it and stays the free, offline product.

## 1. Goal and scope

A hosted Dijkstral at `dijkstral.monet.sh`: connect a GitHub repository once, and its graph is
rebuilt on every push and served to any assistant over a hosted MCP endpoint. No local build, no
local index, the same `context`, `query`, `explain`, `path` and `affected` answers.

`dijkstral.monet.sh` is permanent: MCP URLs are pasted into assistants' configurations and a
redirect does not reliably carry an MCP request, so if Dijkstral later gets a domain of its own,
both hosts keep serving the same service.

Released free. There is no billing until Midtrans is ready; limits exist only to keep one VPS
healthy and to stop abuse (§6), not as plan tiers.

| Phase | Scope | Models |
|---|---|---|
| 1 | GitHub sign-in, GitHub App install, rebuild on push, hosted MCP with tokens, web console | none |
| 2 | Graph-aware PR review: what a PR reaches (`affected`), overlapping open PRs, merge order, posted as one PR comment | none (deterministic); optional prose later |
| 3 | Edit verification (per-function verdict on whether a change alters behaviour), a merge check | research first |
| later | Paid plans through Midtrans; MCP OAuth; semantic pass for docs; Jev reranking (`cli/docs/dijkstral-context-rerank.md`) | per feature |

Phase 1 needs no model API: code extraction never calls a model. A model key (DeepSeek, later
through Inver) is needed only for features that write prose or read documents (the semantic
pass, a PR summary), and each such feature is opt-in per repository.

## 2. Deployables

Following the monorepo rules (one web app per product, a service only for its own runtime and
data):

- `products/dijkstral/service` (Rust, new workspace beside `cli/`, depending on its crates):
  - **Webhook receiver**: GitHub App events (`installation`, `installation_repositories`,
    `push`), verified by HMAC (`X-Hub-Signature-256`) before parsing.
  - **Build queue and worker**: one build per repository at a time; pushes arriving during a
    build coalesce into one follow-up build of the newest commit.
  - **Graph store**: per repository, the latest `graph.dij` and the shallow checkout `context`
    reads source lines from.
  - **Hosted MCP**: Streamable HTTP at `/mcp/<owner>/<repo>`, the local server's tools over the
    stored graph through `dijkstral-serve`'s MCP server in hosted mode (`mcp::Options::hosted`):
    every call uses that repository's graph (`project_path` is neither offered nor honoured, so
    no tenant reaches another's directory), the PR tools are not offered (they run `gh` with the
    server's credentials), and `context` is offered first.
  - **Private API** for the web app (`/internal/*`, never routed publicly).
  - **SQLite**: accounts, GitHub installations, repositories, builds, MCP tokens. The service is
    its only writer.
- `products/dijkstral/web` (Next.js, `packages/ui` tokens): landing, sign in with GitHub, repo
  list with build state and node counts, MCP token creation (shown once), copy-paste setup for
  Claude Code and Codex. It owns the signed browser session and persists nothing else; it calls
  the service's private API.
- JSON shared between web and service is versioned in `@monet/contracts` (`dijkstral/v1`), with
  the Rust side checked against the same fixtures.

Dijkstral accounts are its own: no shared session with Inver or the hub.

## 3. Flows

1. **Sign in**: GitHub's user OAuth through the GitHub App (user-to-server). Kept: GitHub user
   id, login, and the installations the user can see. No GitHub user token is stored past the
   sign-in request.
2. **Connect**: the user installs the GitHub App on chosen repositories. Permissions phase 1:
   Contents read, Metadata read; phase 2 adds Pull requests read and write (comments) and the
   `pull_request` event.
3. **Build**: on install and on each push to the default branch, the worker mints a short-lived
   installation token, fetches the commit shallowly (`--depth 1`, default branch only) into
   `/srv/dijkstral/repos/<repo-id>/`, runs the code build (`extract`, no semantic pass), writes
   `graph.dij` beside it, and swaps it in atomically. Queries never see a half-built graph.
4. **Query**: an assistant calls `/mcp/<owner>/<repo>` with `Authorization: Bearer <token>`. The
   token is checked against its hash and against the user's current access to the repository
   (installation still covers it). Codex: `mcp_servers.<name>.url` plus `bearer_token_env_var`;
   Claude Code: `claude mcp add --transport http … --header "Authorization: Bearer …"`. The
   remote MCP server runs outside the assistant's sandbox, which is also where a networked
   feature such as reranking would run.
5. **Disconnect**: uninstalling the app or removing a repository deletes its checkout and graph
   (§7).

## 4. Hosting: charon

Charon (NixOS, Singapore, 2 vCPU, 8 GB RAM, 80 GB SSD; `~/.config/nix`, `docs/charon-setup.md`)
is the internet-facing host meant for untrusted input. Dijkstral becomes an app module
`nix/hosts/charon/apps/dijkstral.nix` in that repository, following `apps/aiostreams.nix`:

- Two pinned OCI containers (service, web) on `127.0.0.1`, non-root, read-only root, all
  capabilities dropped, memory-capped, `partOf` `charon-apps.target`, conditioned on
  `!/var/lib/charon/restore-pending`.
- State in `/srv/dijkstral` (`db/`, `repos/`); secrets in root-only
  `/var/lib/secrets/dijkstral/env` (GitHub App id and private key, webhook secret, session key),
  generated or pasted on charon and saved with `save-secrets.sh`.
- **Public ingress through a Cloudflare Tunnel** (`cloudflared`, outbound only) for
  `dijkstral.monet.sh`, rather than opening TCP 80/443 for Caddy: charon keeps its "only UDP
  41641 open" model and the zone is already on Cloudflare. The tunnel routes only the web app and
  the public service paths (`/mcp/*`, `/github/webhook`); `/internal/*` is never routed.
  `deploy/` records the routing as it does for Caddy.
- **Backups**: restic backs up `/srv/dijkstral/db` only. `repos/` (customer code and graphs) is
  excluded: it is rebuildable from GitHub, and customer code never leaves charon.
- Images are built by CI from the Monet repository's remote and pulled by pinned tag or digest.

Capacity on one 2 vCPU host: the code build takes 0.2 s for ripgrep, 0.6 s for fastapi and
17 s for Kubernetes (256k nodes), so builds are cheap; disk is the constraint (Kubernetes'
shallow checkout plus graph is about 1.5 GB). Builds run one at a time with a timeout.

## 5. Untrusted input

Every repository is untrusted. `dijkstral extract` only parses files (tree-sitter and its own
parsers); it never runs build scripts, hooks, package managers or the code itself. In addition:

- The build step runs with no network (the fetch is a separate step), a memory cap, a CPU time
  limit and a wall-clock timeout; it writes only inside the repository's directory.
- `git` runs with hooks disabled, no submodules and no LFS smudge, and the checkout is never
  executed.
- The ignore rules and sensitive-file filter of the CLI apply (no `.env`, keys or credential
  files are extracted).
- Webhooks are rejected before parsing without a valid signature; MCP requests are rate-limited
  per token.

## 6. Limits while free

Not plans; protections for one VPS. Each is configuration and shown in the console.

| Limit | Initial value |
|---|---|
| Repositories per account | 5 |
| Repository size (shallow checkout) | 1 GB |
| Graph size | 300,000 nodes |
| Builds | one at a time per host, coalesced per repository, at most 50 per repository per day |
| Build timeout | 5 minutes |
| MCP requests | 120 per minute per token |
| Idle repositories | checkout and graph removed after 30 days without a query; rebuilt on the next one |

## 7. Customer code and data (technical policy)

- **Stored**: a shallow checkout of each connected repository's default branch at its latest
  built commit, its graph, and build metadata (commit, time, node and edge counts, errors).
  Account data: GitHub user id and login, installation and repository ids, hashed MCP tokens.
- **Not stored**: other branches, history, PR contents (phase 2 reads a PR's diff at review time
  and keeps only the posted comment's id), GitHub user tokens, MCP query contents.
- **Logs**: request path, repository id, tool name, status and latency. Never source code,
  queries, answers, tokens or authorization headers.
- **Deletion**: a repository's checkout and graph are deleted immediately when it is removed or
  the app is uninstalled, and account data on account deletion; nothing of it is in backups.
- **Location and processors**: charon in Singapore (Tencent Cloud, resold by SumoPod) stores the
  data; Cloudflare terminates TLS for the tunnel and therefore carries traffic, including MCP
  answers containing code lines; GitHub is the source.
- **Access**: only the operator, over Tailscale SSH. No model provider receives customer code in
  phase 1; any later feature that sends code to a model (semantic pass, prose review, reranking)
  is off by default and enabled per repository by its owner.
- **Encryption**: TLS in transit. At rest, the VPS disk is the provider's; encrypting
  `/srv/dijkstral/repos` (LUKS on the data path, unlocked at boot from the secrets file) is an
  open item before private repositories are accepted (§9).

A short public privacy note on the site states this in plain words; the legal text is the
owner's.

## 8. Delivery order

1. Merge `feature/dijkstral-code-graph` into `main`; give the repository a remote and CI (check
   on every push; image build on `main`).
2. `service`: webhook verification, build queue and worker, graph store, hosted MCP over the
   existing `crates/serve`, SQLite schema, tokens. Tested offline with recorded webhook payloads
   and a local bare git repository as the GitHub stand-in; end to end locally with Compose
   (Opossum on this Mac).
3. `web`: sign in, repositories, tokens, setup instructions.
4. GitHub App registered on the owner's account (a personal account can own one); charon app
   module, tunnel, DNS record `dijkstral.monet.sh`, secrets.
5. Launch with public repositories; private repositories once §7's encryption item is done.
6. Phase 2 PR review.

## 9. Open decisions

- **Private repositories at launch**: recommended only after at-rest encryption of
  `repos/` (§7); public repositories first.
- **Own domain**: `dijkstral.dev` and `.com` were unregistered on 2026-10-21; adding one later
  adds a host, never replaces `dijkstral.monet.sh`.
- **MCP OAuth**: tokens first; OAuth (the MCP authorization spec) when a client needs it.

## 10. Models and money

Dijkstral never sells model tokens or holds balances. A feature that uses a model calls Inver's
public OpenAI-compatible API, Inver's ledger being the only one (root `AGENTS.md`): either with
Dijkstral's own service key, billed to Dijkstral wholesale and offered to customers as a plan
allowance that Dijkstral counts (counts, not money), or with a customer's own scoped Inver key
(or another provider's key), stored encrypted and used only for that customer. Payments for
credits stay in Inver; Dijkstral billing sells plans only.

# Dijkstral Service Agent Guide

## Purpose

`dijkstral-service` is hosted Dijkstral's backend (`../DIJKSTRAL_HOSTED_DESIGN.md`): it receives
the GitHub App's webhooks, rebuilds a repository's code graph when its default branch changes,
serves each repository's graph over MCP (Streamable HTTP, JSON responses) to holders of an MCP
token, and gives the Dijkstral web app a private API for accounts, tokens and repository state.

## Layout

A Cargo workspace of its own (its HTTP and database dependencies stay out of the CLI's lockfile),
depending on the CLI's `dijkstral-core` (builds) and `dijkstral-serve` (the MCP server in hosted
mode, `mcp::Options::hosted`).

- `src/app.rs`: the router. Public: `POST /github/webhook`, `POST /mcp/{owner}/{repo}`,
  `GET /healthz`. Private, bearer internal token: `/internal/*`.
- `src/webhook.rs`: `X-Hub-Signature-256` verification and the events acted on.
- `src/builder.rs`: the build queue (one build at a time, coalesced per repository), the build
  steps and the atomic swap of `<data>/repos/<id>/current`; `child_main` is the build child.
- `src/git.rs`: fetching a default branch with `git`, hardened; credentials reach `git` only
  through its environment.
- `src/github.rs`: the GitHub App's JWT and cached installation tokens.
- `src/db.rs`: the SQLite schema and queries (`<data>/db/service.sqlite`).
- `src/config.rs`: environment configuration and the free-tier limits.

## Ownership boundaries

The service owns hosted Dijkstral's accounts, GitHub installations, repositories, builds and MCP
tokens, and is the only writer of its database. The web app owns browser sessions and calls
`/internal/*`; browsers never do. It never imports another Monet product. Model features, when
added, call Inver's public API (`../DIJKSTRAL_HOSTED_DESIGN.md` §10).

## Invariants

- Every repository is untrusted. Its code is parsed, never run: builds run `dijkstral-service
  build <dir>` as a child with a cleared environment and a timeout; `git` runs with hooks off,
  no submodules, no LFS content and no user or system configuration. A repository's own
  `dijkstral-out/` and `graphify-out/` are deleted before its build.
- One tenant never reaches another's data: MCP servers run in hosted mode (no `project_path`, no
  PR tools); a token reaches a repository only through an installation its account can access;
  unknown and inaccessible repositories answer alike (404).
- Webhooks are rejected before parsing without a valid signature.
- Tokens are stored only as SHA-256 hashes and shown once. Never log tokens, authorization
  headers, webhook bodies, source code, queries or answers. Errors from `git` name the
  repository, never the remote URL (it may carry a credential).
- A repository's checkout and graph are deleted as soon as it is removed or the app uninstalled.
- Only `<data>/db` is backed up; `<data>/repos` is rebuildable and holds customer code.
- Code builds never call a model or the network beyond the fetch.

## Commands

From this directory:

```bash
cargo build --release
cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

From the repository root: `bun run --filter @monet/dijkstral-service check`.

## Verification

Every change passes the module `check`. Tests are offline: `tests/e2e.rs` drives install, build,
MCP queries, a push rebuild, token revocation and uninstall against a local bare repository
standing in for GitHub, with signed webhooks; `tests/github_app.rs` checks the app JWT, token
minting and caching, and the credential's path into `git` against a fake GitHub API with a key
generated at test time. A change to tenant isolation, authentication or the build sandbox needs a
test that fails without it.

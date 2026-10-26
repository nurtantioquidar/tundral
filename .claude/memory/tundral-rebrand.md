---
name: tundral-rebrand
description: "2026-10-23 company renamed Monet -> Tundral (tundral.ai); product line cut to Butterfly, Inver, Dijkstral"
metadata:
  node_type: memory
  type: project
  originSessionId: ec8aa4f3-bd29-4a61-bf8d-d0a3ac062b7f
  modified: 2026-10-23T09:16:00.192Z
---

On 2026-10-23 the umbrella was renamed from Monet to **Tundral**; the user bought `tundral.ai`
(`tundral.com` is someone else's). Repo commit `bb4db6b` on main: `@tundral/*` packages, hosts
`tundral.ai` (hub), `inver.tundral.ai`, `dijkstral.tundral.ai` (the permanent hosted-Dijkstral
host, replacing the never-launched `dijkstral.monet.sh`). Inver's UI is branded "Inver".

Product focus: **Butterfly, Inver, Dijkstral**. Nimble, Cicada and Cocoon were moved out to
standalone repos with history at `~/Projects/repository/github.com/nurtantioquidar/{nimble,cicada,cocoon}`
(each passes its own check; Nimble vendors the nimble/v1 contract). Butterfly keeps its optional
nimble/v1 client dialect.

Deliberately kept "monet": Inver's credential-cipher context `monet:upstream-provider:` (stored
ciphertexts depend on it), Cloudflare Pages project `monet-hub`, `tio@monet.sh` email, the user's
1Password item names and `~/.config/monet`, Dijkstral bench corpus id `monet`, test fixtures.
The hub still uses the Monet-painter pond concept and has no Dijkstral card: it awaits a Tundral
redesign with the user.

The repo directory was moved to `.../nurtantioquidar/tundral` on 2026-10-25 (memory moved with it;
the `claude/nimble-products-status-aa6205` worktree was relinked with `git worktree repair`). Remote: private `github.com/nurtantioquidar/tundral` (SSH remote `origin`; the
gh token lacks `workflow` scope, so push over SSH), main pushed 2026-10-23 after the user revoked the
ccode.dev key that sits in history `84c10ef..96d0177~`; only `main` is pushed, other branches local.
Kept the name Dijkstral, not Dijkstra (famous person's name; `dijkstra` taken on crates/PyPI/npm).
See [[dijkstral-product]], [[monet-domain]], [[monet-product-line-structure]].

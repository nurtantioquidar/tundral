# Dijkstral CLI Agent Guide

## Purpose

`dijkstral` turns a repository into a queryable knowledge graph for AI coding agents and is a
faster, more accurate drop-in for Graphify. The product design, compatibility contract and delivery
stages are in `../DIJKSTRAL_DESIGN.md`; the head-to-head benchmark is in `../bench/`.

## Layout

A Cargo workspace. Crates are added when their stage begins, never ahead of an implementation:

- `crates/cli`: the `dijkstral` binary and command surface; the cross-repo commands are
  `multirepo_cmd.rs` (`merge-graphs`, `global`, `extract --global`) and `clone_cmd.rs`.
- `crates/core`: discovery (`discover.rs`), per-file extraction (`extract/<lang>.rs`), the
  content-addressed cache (`cache.rs`), the cross-file build stage (`resolve/mod.rs` plus
  `resolve/<lang>.rs`), the graph fold and `graph.json` writer (`graph.rs`), Leiden clustering
  through Microsoft's `network_partitions` (`cluster.rs`), the analysis behind
  `.graphify_analysis.json` (`analyze.rs`), `GRAPH_REPORT.md` (`report.rs`) and Python-exact JSON
  output (`pyjson.rs`), the memory-mapped query index `graph.dij` with its in-memory loader
  for any Graphify `graph.json` (`index.rs`), NetworkX node-link graphs with Graphify's
  insertion and iteration order (`nodelink.rs`), and the cross-repo merge and global graph
  (`multirepo.rs`).
- Extra build inputs, only when their flag is given (`docs/graphify-inputs.md`): the Cargo crate
  graph (`core/src/inputs/cargo.rs`, `--cargo`, manifests read without running cargo) and a
  PostgreSQL schema mapped from catalog rows (`core/src/inputs/postgres.rs`); both enter the
  build digest through `core::build_inputs`. The only database client is
  `crates/cli/src/inputs_cmd.rs` (`--postgres`, `--no-gitignore`). Google Workspace exports run
  through the `gws` CLI inside the semantic pass (`crates/semantic/src/gws.rs`). Custom
  OpenAI-compatible providers (`dijkstral provider`, `crates/cli/src/provider_cmd.rs`) live in
  `crates/semantic/src/run/backend/provider.rs`: dijkstral's `~/.dijkstral/providers.json` plus
  Graphify's file read only; a provider stores the name of its key's environment variable, never
  a key.
- `crates/serve`: the query engine behind `query`, `path`, `explain`, `affected`, `god-nodes` and
  `benchmark`: structured answers plus Graphify-format renderers, shared by the CLI
  (`crates/cli/src/query_cmds.rs`) and the stdio MCP server (`mcp.rs`, with the graph summaries
  of `insights.rs`; `crates/cli/src/mcp_cmd.rs`).
- `crates/agents`: assistant integrations — the `PreToolUse` hook guard (`guard.rs`, Bash command
  lexer in `bash.rs`) and the Claude Code / Codex installers (`install.rs`), which edit settings
  JSON by splicing text (`jsonedit.rs`) and instruction files by section (`section.rs`). All prose
  an assistant sees is in `text.rs` and is dijkstral's own. The guard runs before clap parses
  (`crates/cli/src/agents_cmd.rs`), never opens the graph, fails open, and is measured by
  `../bench/hook.py`.
- `crates/export`: exporters over dijkstral's build or any Graphify `graph.json` (`view.rs`, in
  `graph.json` order): `graph.html` (`html.rs`; the page in `page.html` is dijkstral's own and
  embeds Graphify's data shape, vis-network up to the node limit and sigma.js/WebGL above), the
  Markdown wiki (`wiki.rs`) and GraphML (`graphml.rs`). Builds run them through
  `core::pipeline::Exporter` (`crates/cli/src/export_cmd.rs`), so an unchanged build is skipped only
  while their outputs exist. The export-only formats read the graph as NetworkX loads it
  (`nx.rs`, raw attributes, Graphify's graph kinds): `cypher.txt` (`cypher.rs`) and `--push` to
  Neo4j over Bolt (`bolt.rs`) or FalkorDB over RESP (`push.rs`), both dijkstral's own plain-TCP
  clients that connect only to the URI given; the Obsidian vault and canvas (`obsidian.rs`);
  `graph.svg` (`svg.rs`); the Mermaid call-flow page (`callflow.rs`) and the D3 tree
  (`tree.rs`), whose pages are dijkstral's own (`*.html`, CDN scripts pinned with integrity).
  Their commands are in `crates/cli/src/exports/`.
- `crates/semantic`: Graphify's semantic pass (files sent, chunking, request and reply schema,
  merge, cache, model labelling) and the LLM-free markdown structure of `update`, specified in
  `docs/graphify-semantic.md`, with goldens and recorded model replies in
  `tests/fixtures/semantic/` (replayable through `../bench/llm_shim.py`). Stages: `ingest`, `run`
  (backends, cache, naming), `merge` (replies into the graph; its fold is
  `core::graph::Graph::fold_semantic`) and `markdown`; `pass.rs` runs ingest → run → merge as
  `extract`'s core `Documents` pass and implements `label`; `add.rs` is `dijkstral add <url>`.
  Their commands are in `crates/cli/src/semantic_build.rs` and `semantic_cmd.rs`.
  `crates/semantic/tests/merge_goldens.rs` feeds recorded and crafted replies through merge and
  must match Graphify's graphs, sidecars and reports field for field; `crates/cli/tests/
  semantic_cli.rs` drives the commands offline with the `replay` backend. Tests never call a
  model; `../bench/semantic.py` is the same-model head-to-head.
- `crates/git`: git integration, specified in `docs/graphify-git.md` with Graphify's goldens in
  `tests/fixtures/git/` (`../bench/git_goldens.py`). The `post-commit`/`post-checkout` block
  (`hooks.rs`) is dijkstral's own text between marker lines and leaves foreign hook content
  intact; what it runs (`rebuild.rs`) starts a detached `dijkstral update` that serialises on a
  lock and coalesces triggers, and never fails git. The `graph.json` merge driver (`merge.rs`) is
  three-way and writes a build's canonical order; `attributes.rs` and `install.rs` wire it into
  the local git config and `.gitattributes`, in the repository `repo.rs` resolves through `git`
  (`core.hooksPath`, linked worktrees). Commands are in `crates/cli/src/git_cmd.rs`; `watch`
  (`crates/cli/src/watch_cmd.rs`) runs `update` when files of the corpus change, through
  `notify`. `crates/git/tests/merge_goldens.rs` compares merges with Graphify's as data, applying
  each deliberate difference explicitly; `crates/cli/tests/git_cli.rs` and `watch_cli.rs` drive
  the commands in scratch repositories with the user's git config shut out
  (`GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`) and bounded waits.

Compatibility is defined by `docs/graphify-format.md`, the black-box specification of Graphify's
output, and enforced by `crates/core/tests/golden_<lang>.rs` against the goldens in
`tests/fixtures/`. Each golden test's `ALLOWED` list names the Graphify quirks dijkstral does not
reproduce; a new entry needs a comment with the quirk id or reason. The same tests check community
agreement, re-cluster Graphify's own graph (`dijkstral cluster-only`, which must reproduce
`graphify cluster-only --no-label` byte for byte) and compare the report goldens (§17.6).
The query, MCP, hook-guard and installer surfaces are specified the same way in
`docs/graphify-query.md`, with goldens in `tests/fixtures/query/`; `crates/cli/tests/query_goldens.rs`
runs every CLI case on the JSON load path and on `graph.dij`, and must stay at 100%;
`crates/cli/tests/mcp_goldens.rs` replays every MCP transcript against `dijkstral mcp` the same way,
with its deliberate differences (§10.9) listed in the test. The exporters are specified in
`docs/graphify-exports.md` and checked by `crates/export/tests/goldens.rs` against Graphify's own
exports of the fixtures in `tests/fixtures/exports/` (GraphML and wiki byte for byte, page data
record for record); `escaping.rs` keeps untrusted strings out of the page's markup. Each
export-only format has its own test file there (`cypher.rs`, `push.rs` against fake servers,
`obsidian.rs`, `svg.rs`, `callflow.rs`, `tree.rs`) with Graphify's outputs, or the data embedded in
its pages, as goldens. Query commands write nothing next to the graph (Graphify's
`cache/last_query_stamp` is read by nothing). The cross-repo commands (`merge-graphs`, `global`,
`extract --global`, `clone`) are specified in `docs/graphify-multirepo.md`, with Graphify's outputs
in `tests/fixtures/multirepo/` (case table and crafted inputs in its `regen.py`);
`crates/cli/tests/multirepo_goldens.rs` must reproduce every merged graph, global graph and
manifest byte for byte apart from its `ALLOWED` cases, and `multirepo_cli.rs` covers
dijkstral-built graphs, queries on them, `extract --global` and `clone` offline.
The extra inputs, walk flags and custom providers are specified in `docs/graphify-inputs.md`, with
goldens in `tests/fixtures/inputs/`: `crates/core/tests/inputs_goldens.rs` must match Graphify's
`--cargo` and `--postgres` graphs record for record (communities exact; its `ALLOWED` list names
the quirks not reproduced), building PostgreSQL graphs offline from the catalog snapshots
(`<db>.catalog.json`) that the `#[ignore]`d live test in `crates/cli/src/inputs_cmd.rs` checks
(`DIJKSTRAL_PG_TEST_DSN`); `crates/cli/tests/inputs_cli.rs` drives the flags with a scratch `HOME`,
a fake `gws` and a fake model server. `docs/graphify-cli-checklist.md` maps every Graphify command
and flag to dijkstral; a new command or flag updates it.

## Ownership boundaries

This module owns local graph construction, its on-disk index and outputs under `dijkstral-out/`,
its user-wide state under `$DIJKSTRAL_HOME` (default `~/.dijkstral`: the global graph and
`clone`'s repositories), the query and MCP surfaces, and assistant integration files it installs.
It must not import another Monet product (Butterfly's `atlas` included) or Graphify code, prompts
or skill text; only Graphify's documented formats and observable behaviour are reproduced.

## Invariants

- Builds are deterministic: identical input produces byte-identical `graph.json` regardless of
  thread count or run.
- `graph.json` stays readable as Graphify schema 1; dijkstral-only data lives under
  `graph.dijkstral` or in the native index.
- Code extraction never calls a model or the network. The semantic pass runs only when requested
  and never sends files excluded by ignore rules or the sensitive-file filter.
- Never log or store prompts, API keys or file contents outside the cache under `dijkstral-out/`.
- Tests never touch a real home or the network: they set `DIJKSTRAL_HOME`, and `clone` tests use a
  local repository through the test-only `DIJKSTRAL_CLONE_BASE`.

## Commands

From this directory:

```bash
cargo run -p dijkstral -- --help
cargo build --release
cargo fmt --all --check && cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

From the repository root:

```bash
bun run dev:dijkstral -- --help
bun run --filter @monet/dijkstral-cli check
```

## Verification

Every change must pass the module `check` command. Tests are offline and use small fixture
repositories. An extraction change must keep every golden test at its threshold and should be
checked against the frozen Graphify references with `../bench/parity.py` (see
`../bench/README.md`). A change that affects build speed, memory, graph shape or query behaviour
must also be measured with `../bench/run.py` against the pinned Graphify baseline, and the result
recorded in `../bench/README.md` with its date and pins.

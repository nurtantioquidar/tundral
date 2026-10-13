# Dijkstral CLI Agent Guide

## Purpose

`dijkstral` turns a repository into a queryable knowledge graph for AI coding agents and is a
faster, more accurate drop-in for Graphify. The product design, compatibility contract and delivery
stages are in `../DIJKSTRAL_DESIGN.md`; the head-to-head benchmark is in `../bench/`.

## Layout

A Cargo workspace. Crates are added when their stage begins, never ahead of an implementation:

- `crates/cli`: the `dijkstral` binary and command surface.
- `crates/core`: discovery (`discover.rs`), per-file extraction (`extract/<lang>.rs`), the
  content-addressed cache (`cache.rs`), the cross-file build stage (`resolve/mod.rs` plus
  `resolve/<lang>.rs`), the graph fold and `graph.json` writer (`graph.rs`), Leiden clustering
  through Microsoft's `network_partitions` (`cluster.rs`), the analysis behind
  `.graphify_analysis.json` (`analyze.rs`), `GRAPH_REPORT.md` (`report.rs`) and Python-exact JSON
  output (`pyjson.rs`).
- `crates/serve`, `crates/export`, `crates/semantic`: later stages.

Compatibility is defined by `docs/graphify-format.md`, the black-box specification of Graphify's
output, and enforced by `crates/core/tests/golden_<lang>.rs` against the goldens in
`tests/fixtures/`. Each golden test's `ALLOWED` list names the Graphify quirks dijkstral does not
reproduce; a new entry needs a comment with the quirk id or reason. The same tests check community
agreement, re-cluster Graphify's own graph (`dijkstral cluster-only`, which must reproduce
`graphify cluster-only --no-label` byte for byte) and compare the report goldens (§17.6).
The query, MCP, hook-guard and installer surfaces are specified the same way in
`docs/graphify-query.md`, with goldens in `tests/fixtures/query/`.

## Ownership boundaries

This module owns local graph construction, its on-disk index and outputs under `dijkstral-out/`,
the query and MCP surfaces, and assistant integration files it installs. It must not import another
Monet product (Butterfly's `atlas` included) or Graphify code, prompts or skill text; only
Graphify's documented formats and observable behaviour are reproduced.

## Invariants

- Builds are deterministic: identical input produces byte-identical `graph.json` regardless of
  thread count or run.
- `graph.json` stays readable as Graphify schema 1; dijkstral-only data lives under
  `graph.dijkstral` or in the native index.
- Code extraction never calls a model or the network. The semantic pass runs only when requested
  and never sends files excluded by ignore rules or the sensitive-file filter.
- Never log or store prompts, API keys or file contents outside the cache under `dijkstral-out/`.

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

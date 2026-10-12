# Dijkstral CLI Agent Guide

## Purpose

`dijkstral` turns a repository into a queryable knowledge graph for AI coding agents and is a
faster, more accurate drop-in for Graphify. The product design, compatibility contract and delivery
stages are in `../DIJKSTRAL_DESIGN.md`; the head-to-head benchmark is in `../bench/`.

## Layout

A Cargo workspace. Crates are added when their stage begins, never ahead of an implementation:

- `crates/cli`: the `dijkstral` binary and command surface.
- `crates/core` (stage 1): discovery, extraction, cache, resolution, graph store, clustering.
- `crates/serve`, `crates/export`, `crates/semantic`: later stages.

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
repositories. A change that affects build speed, memory, graph shape or query behaviour must also be
measured with `../bench/run.py` against the pinned Graphify baseline, and the result recorded in
`../bench/README.md` with its date and pins.

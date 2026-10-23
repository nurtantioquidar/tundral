# Dijkstral design

## Product statement

Dijkstral turns a repository into a knowledge graph that an AI coding agent queries instead of
grepping. It extracts code structure locally with tree-sitter, adds documents, papers, images and
recordings through an optional model pass, clusters the result, and answers questions, paths and
explanations with `file:line` citations from a CLI, an MCP server, and assistant hooks.

It is a clean-room, drop-in alternative to [Graphify](https://github.com/Graphify-Labs/graphify)
(`graphifyy`), the reference product it is measured against. Dijkstral must reach feature parity
and then beat Graphify on build speed, memory, query latency, and graph accuracy, proven by the
head-to-head benchmark in `bench/`. The name honours Dijkstra; shortest paths are a first-class
query.

## Where the reference product loses time and accuracy

Measured or documented in Graphify itself (issue numbers refer to Graphify-Labs/graphify):

- A Python process starts for every CLI call and every assistant hook; the hook guard costs about
  170 ms per Bash/Grep/Read/Glob call.
- Queries load JSON into NetworkX and score every node: p50 428 ms at 54k nodes, 3.7 s at 271k
  (#1297).
- JS/TS files bypass the AST cache and every tree is held in memory (6.5 GiB peak, #3011).
- Cross-file resolution is single-threaded after a pickling process pool; one crashed worker
  breaks the pool (#943).
- A 450k-node build spent about 5 min building NetworkX, 15 min in Leiden and 90+ min in
  betweenness (#341).
- The graph keeps one edge per node pair, so co-occurring relations are lost; call resolution
  guesses by label and path proximity with no ground truth.
- Ignore patterns accumulate across the walk (#2834).

## Pipeline

```text
discover ─► extract ─► cache ─► resolve ─► graph store ─► cluster ─► analyze ─► export
 ignore      tree-sitter  blake3    import-scoped  CSR multigraph   Leiden     cycles,     graph.json (Graphify v1)
 parallel    one walk     redb      per-language   rkyv index       seed 42    god nodes,  GRAPH_REPORT.md, html,
 walker      rayon        stat path module rules   mmap queries                betweenness wiki, graphml, cypher…
                                                        │
                                       serve: query / path / explain / MCP / hook-guard
                                       semantic: docs, PDF, office, images, A/V (skill-driven or headless)
```

- **Discover**: the `ignore` crate's parallel walker with per-directory `.gitignore` scoping, plus
  `.dijkstralignore` and `.graphifyignore`. Default skip directories, lockfiles and the sensitive
  file filter match Graphify's so both tools index the same corpus.
- **Extract**: tree-sitter 0.25 in parallel, one parser per thread and language; each language is
  a single walk over the tree (`crates/core/src/extract/<lang>.rs`) that emits nodes, in-file edges
  and the facts the cross-file pass needs. Trees are dropped as soon as facts are extracted,
  bounding memory regardless of language. JSON, TOML and shell use their own small parsers.
- **Cache**: per-file facts keyed by blake3 of content and relative path plus a grammar and query
  fingerprint, in one redb file, behind an mtime+size stat fast path with a racy-clean guard.
  Every language is cached.
- **Resolve**: parallel, import-scoped, with real module resolution per language. Edges carry
  Graphify's provenance tiers: `EXTRACTED`, `INFERRED`, `AMBIGUOUS`.
- **Graph store**: a CSR multigraph that keeps every relation, written to `dijkstral-out/graph.dij`
  as an mmap-able index with a trigram name index and precomputed degree, community and IDF tables.
- **Cluster and analyze**: Leiden through Graphify's own engine (graspologic-native's
  `network_partitions`) with its parameters and input order (resolution 1.0, seed 42, oversize and
  low-cohesion re-splits, hub-based labels), so communities match Graphify's for the same graph;
  god nodes, surprising connections, short import cycles and sampled parallel betweenness for
  `.graphify_analysis.json` and `GRAPH_REPORT.md` (`cli/docs/graphify-format.md` §17).
- **Serve**: `query`, `path`, `explain` and an MCP server with Graphify's tool names and exit codes
  (0 answer, 3 ambiguous, 4 no match); a native `hook-guard` targeting under 5 ms.
- **Semantic pass**: skill-driven (the host assistant writes validated JSON fragments, no key
  needed) or headless through OpenAI-compatible, Anthropic, Gemini and Bedrock backends; local
  whisper.cpp transcription; a cache keyed by content hash, model and prompt version.

## Compatibility contract

`dijkstral-out/graph.json` is readable by anything that reads Graphify's `graph.json` schema 1:
NetworkX node-link data with `links`, Graphify's node-id recipe, its one-edge-per-pair collapse
rules, `community`, `norm_label` and `confidence_score`, and canonical key and array order.
Dijkstral-only data lives under `graph.dijkstral` and in the native index. Formats and behaviour
are reimplemented from documentation and observed output; Graphify source, prompts and skill text
are never copied.

Graphify has two no-LLM build paths with different outputs: `extract --code-only` indexes code
only, while `update` also adds structural markdown nodes. Dijkstral mirrors both, and the benchmark
compares like with like.

## Benchmark

`bench/` measures both tools on a pinned corpus (see `bench/README.md`): build time, CPU and
process-tree peak RSS for cold, warm and one-file-update builds; query and hook latency; graph
parity; precision and recall against SCIP indexes; and agent task outcome. Only the last costs
money and it runs only when explicitly triggered.

## Delivery stages

Each stage ends with a benchmark run and a review before the next begins.

| # | Stage | Exit criterion |
|---|-------|----------------|
| 0 | Product scaffold, bench harness, Graphify baseline | baseline recorded; `bun run check` green |
| 1 | Code core for Python, TS/JS, Rust, Go, Java: discover, extract, cache, resolve, compatible `graph.json`, `update` | faster and lower RSS than Graphify on every corpus repo; parity diff explained |
| 2 | SCIP accuracy oracle, Leiden, analysis, `GRAPH_REPORT.md` | precision ≥ Graphify at recall ≥ Graphify, per relation |
| 3 | Native index, `query`/`path`/`explain`, MCP, hook guard, Claude Code and Codex installs | query p95 and hook latency far below Graphify |
| 4 | Remaining languages, rationale nodes, all exporters | language coverage matches Graphify, golden fixtures |
| 5 | Semantic pass, both modes and every backend and input type | parity on mixed corpora; cache hit on rerun |
| 6 | PR tools, git hooks, watch, merge, global graph, remaining assistant installs | CLI checklist mirrors Graphify |
| 7 | Agent-outcome benchmarks (key facts, LOCOMO, LongMemEval) | published comparison |

### Deferred: assistant installs beyond Claude Code and Codex

Stage 6 ships the Claude Code and Codex integrations only. Graphify's other assistant targets are
deferred and must be completed before the CLI checklist counts as mirroring Graphify. Each needs its
footprint probed (files, sections, hooks or plugins written; user-wide or project), specified
alongside Claude Code and Codex in `cli/docs/graphify-query.md` §12, and added to `install`,
`uninstall` and their own `<platform> install|uninstall` commands, with dijkstral's own text:

| Target | Graphify's footprint, from `graphify --help` |
|---|---|
| `gemini` | `GEMINI.md` section and a `BeforeTool` hook (Gemini CLI) |
| `cursor` | `.cursor/rules/graphify.mdc` |
| `codebuddy` | `CODEBUDDY.md` section and a `PreToolUse` hook |
| `opencode` | `AGENTS.md` section and a `tool.execute.before` plugin |
| `kilo` | native Kilo skill, command, `AGENTS.md` and a `.kilo` plugin |
| `aider`, `claw` (OpenClaw), `droid` (Factory Droid), `trae`, `trae-cn` | `AGENTS.md` section |
| `copilot` | skill under `~/.copilot/skills` (GitHub Copilot CLI) |
| `vscode` | skill and `.github/copilot-instructions.md` (VS Code Copilot Chat) |
| `antigravity` | `.agents/rules`, `.agents/workflows` and a skill (Google Antigravity) |
| `hermes` | skill under `~/.hermes/skills/` |
| `kiro` | skill under `.kiro/skills/` and a steering file |
| `pi` | skill under `~/.pi/agent/skills/` |
| `devin` | skill under `~/.config/devin/skills/` |
| `agents`, `amp`, `windows` | `install --platform` skill targets |

`uninstall --purge` (also delete the output directory) belongs with this work.

## Relationship to other Tundral products

Dijkstral is an independent local product. It does not import Butterfly, whose `atlas` crate is
prior art for tree-sitter extraction and repository ranking; a later integration would be a
versioned CLI or MCP contract, never a source import. Headless semantic extraction can target any
OpenAI-compatible endpoint, including Inver, like any other client.

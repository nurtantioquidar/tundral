# Dijkstral vs Graphify benchmark

Head-to-head measurement of `dijkstral` against [Graphify](https://github.com/Graphify-Labs/graphify)
(`graphifyy`), the product it replaces. This is not a Bun workspace; it is plain scripts, and
nothing here runs in `bun run check`. Corpus checkouts, the Graphify install, and all outputs live
outside the source tree under `$DIJKSTRAL_BENCH_HOME` (default `~/.cache/dijkstral-bench`);
`results/` is gitignored.

## Files

| File | Purpose |
|---|---|
| `corpus.toml` | Pinned corpus: repo, commit, size tier, languages, the file the `update` scenario edits |
| `setup.sh` | Installs pinned Graphify (`graphifyy[leiden,mcp]`, Python 3.13 so Leiden uses graspologic-native) into an isolated venv and fetches each repo at its pinned commit |
| `run.py` | Build benchmark: cold / warm / one-file update, wall, CPU, process-tree peak RSS, node and edge counts → `results/build-<epoch>.json`; `report` prints the markdown table. Tools: `graphify`, `graphify-nocluster`, `dijkstral` |
| `reference.sh` | Freezes Graphify's cold `extract --code-only` graph per repo as `$DIJKSTRAL_BENCH_HOME/reference/<repo>.graph.json` |
| `parity.py` | Compares two `graph.json` files as sets: nodes by kind, links by relation, field agreement, samples of each difference; for clustered graphs, community agreement (same id, NMI, ARI) and array order |
| `accuracy/` | Stage-2 SCIP oracle: precision and recall of both tools' graphs against compiler-grade indexes ([accuracy/README.md](accuracy/README.md)) |
| `query.py` | Stage-3 query benchmark: `latency` (fresh process per call, p50/p95, peak RSS; Graphify on its reference graph, dijkstral on its own `graph.dij` and on Graphify's `graph.json`) → `results/query-<epoch>.json`; `parity` (both tools on Graphify's reference graph, seeds, NODE lists, EDGE sets and full output compared) → `results/query-parity-<epoch>.json`; `mcp` (MCP servers over stdio: spawn → `tools/list`, first and warm `query_graph` calls, peak RSS) → `results/mcp-<epoch>.json`; `report` prints the tables |
| `hook.py` | Hook-guard latency: one fresh process per Claude Code `PreToolUse` payload for both tools, interleaved with the exec floor (`/usr/bin/true`) → `results/hook-<epoch>.json` |
| `llm_shim.py` | Stage-5 semantic pass: localhost OpenAI-compatible server that answers through `codex exec` (ChatGPT subscription, no API key), replays recorded replies, or serves static ones; writes clean-room request transcripts ([semantic/README.md](semantic/README.md)) |
| `semantic.py` | Stage-5 same-model semantic benchmark: Graphify and dijkstral through one Codex-backed shim (`--max-requests` caps Codex calls, `--dry-run` uses static replies) → `results/semantic-<epoch>.json` and a table |
| `semantic/` | Semantic fixture binaries (`make_binaries.py`), markers, and how to run Graphify's model pass through the shim |

Later stages add query latency, graph parity, SCIP precision/recall, and the paid
agent-outcome runs (see `../DIJKSTRAL_DESIGN.md`).

## Run

```bash
cd products/dijkstral
bench/setup.sh                       # idempotent; `bench/setup.sh fastapi` for one repo
uv run --no-project bench/run.py run --tools graphify --runs 3
uv run --no-project bench/run.py run --tools graphify,dijkstral --repos fastapi,ripgrep
uv run --no-project bench/run.py report bench/results/build-<epoch>.json
uv run --no-project bench/hook.py    # hook-guard latency, both tools (needs a release build)
bench/reference.sh fastapi           # freeze Graphify's graph for parity
cli/target/release/dijkstral extract ~/.cache/dijkstral-bench/corpus/fastapi --code-only --out /tmp/fa
uv run --no-project bench/parity.py /tmp/fa/dijkstral-out/graph.json \
  ~/.cache/dijkstral-bench/reference/fastapi.graph.json --ext .py
# clustering alone: re-cluster a copy of the reference (cluster-only rewrites the graph it reads)
mkdir -p /tmp/co/dijkstral-out && cp ~/.cache/dijkstral-bench/reference/fastapi.graph.json /tmp/co/dijkstral-out/graph.json
cli/target/release/dijkstral cluster-only /tmp/co
uv run --no-project bench/parity.py /tmp/co/dijkstral-out/graph.json \
  ~/.cache/dijkstral-bench/reference/fastapi.graph.json --samples 0
```

## Method

- **Pins**: Graphify `graphifyy==0.9.86` (git `a5ba60dd`, Python 3.13, graspologic-native Leiden).
  Corpus commits are in `corpus.toml`. Record the dijkstral commit and binary sha256 with every
  dijkstral row.
- **Commands compared**:

  | Scenario | Graphify | Dijkstral |
  |---|---|---|
  | cold | `rm -rf graphify-out && graphify extract <repo> --code-only --max-workers N` | `rm -rf dijkstral-out && dijkstral extract <repo> --code-only --threads N` |
  | warm | same command, nothing changed | same command, nothing changed |
  | update | untimed `graphify update`, append one comment line to `edit_file`, timed `graphify update <repo>` | the same with `dijkstral update` |

  `N` is the machine's core count for both tools. Graphify's `extract --code-only` output and its
  `update` output differ: `update` also adds structural markdown nodes. The update scenario
  therefore primes with one untimed update so the timed run is truly incremental.
- **Memory** is the peak of the summed RSS of the tool's whole process tree, sampled every 50 ms,
  so Graphify's worker pool is counted. CPU is user+sys of the whole tree.
- **Caches**: "cold" means a cold tool cache; the OS page cache is warm for every timed run.
- **Quiet machine**: run nothing else heavy (no cargo builds) during a run.

## Observations about the reference

- `graphify update` indexes markdown structurally (no LLM), so on `monet` its graph has 4,375
  nodes against 3,454 for `extract --code-only`; the two headless paths are different products.
- A warm `extract --code-only` on the Graphify repo itself yields 16,782 nodes against 16,790 from
  a cold build of the same commit: its cache path is not equivalent to a fresh build.

## Results

### Graphify 0.9.86 baseline — 2026-10-12

Apple M5, 10 cores, 16 GB, macOS; `--max-workers 10`; 3 runs per scenario (median, min–max);
Kubernetes 1 run. Raw: `results/build-1791798879.json`, `results/build-1791799386.json`,
`results/build-1791802101.json`.

| repo | tier | scenario | wall s | cpu s | peak tree RSS MB | nodes | edges |
|---|---|---|---|---|---|---|---|
| monet | small | cold | 2.07 (1.99–2.26) | 6.4 | 686 | 3,454 | 7,671 |
| | | warm | 1.12 (1.11–1.13) | 1.3 | 109 | 3,454 | 7,671 |
| | | update | 1.95 (1.95–2.04) | 5.4 | 621 | 4,375 | 8,574 |
| graphify | small | cold | 9.26 (9.10–9.86) | 22.6 | 964 | 16,790 | 37,510 |
| | | warm | 4.98 (4.92–6.02) | 5.9 | 290 | 16,782 | 37,495 |
| | | update | 7.71 (7.70–7.99) | 11.7 | 894 | 20,272 | 41,434 |
| fastapi | medium | cold | 6.80 (6.79–7.47) | 16.3 | 707 | 9,268 | 20,101 |
| | | warm | 3.01 (3.00–3.01) | 3.5 | 162 | 9,268 | 20,101 |
| | | update | 9.51 (9.36–9.58) | 11.0 | 617 | 26,782 | 39,755 |
| ripgrep | medium | cold | 1.70 (1.68–1.77) | 5.4 | 640 | 4,232 | 8,995 |
| | | warm | 0.97 (0.95–0.98) | 1.1 | 101 | 4,232 | 8,995 |
| | | update | 1.02 (1.00–1.03) | 1.1 | 115 | 4,330 | 9,071 |
| excalidraw | large | cold | 3.98 (3.91–4.09) | 11.6 | 759 | 4,990 | 19,721 |
| | | warm | 1.98 (1.95–2.52) | 2.4 | 161 | 4,978 | 19,708 |
| | | update | 3.92 (3.81–3.96) | 11.5 | 791 | 5,523 | 20,281 |
| erpnext | large | cold | 22.44 (22.41–23.53) | 57.2 | 1,132 | 24,633 | 61,108 |
| | | warm | 10.44 (10.43–10.58) | 12.8 | 464 | 24,633 | 61,108 |
| | | update | 21.41 (20.61–23.03) | 30.9 | 1,451 | 25,091 | 61,295 |
| kubernetes | huge | cold | 861.97 | 1,184.8 | 2,281 | 256,424 | 1,401,514 |
| | | warm † | 184.54 | 231.4 | 3,144 | 256,424 | 1,401,514 |
| | | update † | 735.97 | 886.4 | 4,713 | 260,554 | 1,405,196 |

† With default settings both fail: Graphify refuses to load its own 570 MB graph above
`GRAPHIFY_MAX_GRAPH_BYTES` (512 MiB). These rows were re-run with `GRAPHIFY_MAX_GRAPH_BYTES=4GB`.

Targets for dijkstral, read from this table:

- **Update is not incremental.** A one-file update costs about as much as a cold build on every
  repo with JS/TS or many files (erpnext 21.4 s vs 22.4 s cold; excalidraw 3.9 s vs 4.0 s).
- **No-change rebuilds cost 1–10 s**, dominated by interpreter start-up and re-reading caches.
- **Kubernetes**: a 14.4 min cold build producing a 570 MB `graph.json` (256k nodes, 1.4M edges).
  With default settings Graphify cannot rebuild or update it at all (graph-size cap). With the cap
  raised, a no-change rebuild takes 3.1 min and a one-file update 12.3 min at 4.7 GB peak RSS.

### Stage 1: dijkstral vs Graphify `--no-cluster` — 2026-10-12

Like-for-like: dijkstral does not cluster yet (stage 2), so it is compared with Graphify's
`--no-cluster` builds. Same machine and method as the baseline; dijkstral at `0c31649` plus the
memory changes in the stage-1 commit (release build, mimalloc), 10 threads. Three runs per
scenario (Kubernetes: Graphify one run, dijkstral three). Raw: `results/build-1791814992.json`,
`results/build-1791815405.json` (Graphify), `results/build-1791817944.json`,
`results/build-1791818004.json` (dijkstral).

| repo | scenario | Graphify --no-cluster: wall s | dijkstral: wall s | speedup | Graphify peak RSS MB | dijkstral peak RSS MB |
|---|---|---|---|---|---|---|
| monet | cold | 1.50 | 0.13 | 12× | 695 | 54 |
|  | warm | 0.93 | 0.07 | 13× | 76 | 8 |
|  | update | 1.80 | 0.07 | 24× | 638 | 8 |
| graphify | cold | 6.60 | 0.30 | 22× | 963 | 170 |
|  | warm | 3.46 | 0.11 | 31× | 178 | 117 |
|  | update | 7.00 | 0.12 | 60× | 1,081 | 117 |
| fastapi | cold | 5.71 | 0.21 | 27× | 710 | 84 |
|  | warm | 1.79 | 0.10 | 18× | 96 | 77 |
|  | update | 8.23 | 0.11 | 73× | 841 | 83 |
| ripgrep | cold | 1.13 | 0.11 | 10× | 666 | 65 |
|  | warm | 0.40 | 0.07 | 6× | 62 | 8 |
|  | update | 0.87 | 0.07 | 12× | 127 | 8 |
| excalidraw | cold | 3.05 | 0.19 | 16× | 767 | 95 |
|  | warm | 1.52 | 0.09 | 17× | 107 | 12 |
|  | update | 3.39 | 0.10 | 36× | 795 | 12 |
| erpnext | cold | 17.81 | 0.78 | 23× | 1,115 | 258 |
|  | warm | 8.55 | 0.20 | 43× | 301 | 226 |
|  | update | 18.14 | 0.23 | 78× | 2,008 | 220 |
| kubernetes | cold | 583.19 | 5.48 | 106× | 2,172 | 2,044 |
|  | warm | 91.06 | 2.16 | 42× | 3,741 | 1,878 |
|  | update | 634.96 | 2.19 | 290× | 5,627 | 1,882 |

dijkstral is faster in every scenario (6× to 290×) and lower in peak memory in every scenario.
Kubernetes memory needed work: the first stage-1 build peaked at 3.9 GB. Streaming `graph.json`
instead of building it in memory, not retaining file contents after hashing, chunked cache
commits with a 64 MB redb cache, an exact-size edge fold on integer ids, and mimalloc brought it
to 2.0 GB with byte-identical output. The update scenario is not comparable on graph shape:
Graphify's `update` also indexes markdown, which dijkstral leaves to the semantic stage.

### Stage 1 graph parity — 2026-10-12

`parity.py` of dijkstral's `extract --code-only` against Graphify's frozen cold reference, all
extensions (recall / precision, %):

| repo | def nodes | all nodes | all links | calls | references |
|---|---|---|---|---|---|
| monet | 99.8 / 99.9 | 98.4 / 99.6 | 98.3 / 98.6 | 98.5 / 98.3 | 96.7 / 96.7 |
| graphify | 95.9 / 99.9 | 96.5 / 99.2 | 97.0 / 95.9 | 99.0 / 92.9 | 91.6 / 97.2 |
| fastapi | 100.0 / 100.0 | 98.7 / 97.9 | 93.8 / 94.7 | 97.9 / 94.5 | 81.9 / 85.7 |
| ripgrep | 99.9 / 100.0 | 97.9 / 99.4 | 95.8 / 98.2 | 100.0 / 98.0 | 83.1 / 97.9 |
| excalidraw | 100.0 / 100.0 | 98.2 / 99.9 | 97.3 / 99.5 | 97.7 / 98.6 | 99.8 / 100.0 |
| erpnext | 100.0 / 100.0 | 99.8 / 99.6 | 94.2 / 91.1 | 88.4 / 85.5 | 81.9 / 82.0 |
| kubernetes | 99.9 / 100.0 | 99.9 / 100.0 | 99.9 / 99.8 | 99.6 / 99.0 | 99.8 / 99.9 |

What the gaps are:

- **graphify def recall 95.9:** its test fixtures in Pascal, PHP, XAML, Swift, C#, C++, Kotlin…
  (518 nodes), languages that arrive in stage 4.
- **Quirks deliberately not reproduced** (`cli/docs/graphify-format.md` §15; each golden test's
  `ALLOWED` list): Q6 file-stem remap of externals (ripgrep/monet `imports_from`, erpnext's
  `frappe` imports and calls, ~3,600 links), Q14 generic-parameter stubs (ripgrep `references`),
  Q8 method binding by name only and Q9 barrel stubs (TS/JS calls and `imports`), Q12 local-module
  calls, Python decorators bound to unrelated same-named functions and keyword names turned into
  stubs (fastapi `references`).
- Where they differ, dijkstral's edges follow the actual imports and scopes; which tool is right
  per edge is measured in stage 2 against SCIP indexes, not assumed.

### Stage 2: clustering, analysis and report — 2026-10-13

Same machine; dijkstral at the stage-2 clustering commit (binary sha256 `8bc4388c434f2894…`), 10
threads, `SOURCE_DATE_EPOCH` set; peak RSS from `/usr/bin/time -l` (dijkstral is one process).

**Clustering is Graphify's.** `dijkstral cluster-only` on each frozen reference
(`$DIJKSTRAL_BENCH_HOME/reference/<repo>.graph.json`) reproduces Graphify's community of every node
and both arrays byte for byte on all seven repos, Kubernetes included (256,424 nodes, 9,417
communities): 12 s against Graphify `cluster-only`'s 278 s. The analysis sidecar is byte-identical
on monet, ripgrep, excalidraw and Kubernetes; `cli/docs/graphify-format.md` §17.7 lists what still
differs in the rest and in the report.

**End to end** (`dijkstral extract` vs the frozen reference, `parity.py` on shared nodes), the
communities follow the stage-1 graph differences: a missing or extra link moves Leiden's result and
the size-ranked ids, so same-id agreement is low while the partitions stay close.

| repo | shared nodes | same community id | NMI | ARI |
|---|---|---|---|---|
| monet | 3,400 / 3,454 | 10.3% | 0.945 | 0.820 |
| graphify | 16,198 / 16,790 | 6.6% | 0.937 | 0.706 |
| fastapi | 9,145 / 9,268 | 2.1% | 0.915 | 0.558 |
| ripgrep | 4,142 / 4,232 | 9.1% | 0.930 | 0.765 |
| excalidraw | 4,902 / 4,990 | 9.2% | 0.831 | 0.604 |
| erpnext | 24,576 / 24,633 | 2.1% | 0.904 | 0.503 |

**Cost** (cold `extract --code-only`, median of 3; Graphify's cold build clusters too):

| repo | dijkstral wall s | of which cluster / analyze / report ms | peak RSS MB | `--no-cluster` wall s / RSS MB | Graphify cold wall s / RSS MB |
|---|---|---|---|---|---|
| erpnext | 1.01 | 91 / 12 / 24 | 308 | 1.14 / 283 | 22.44 / 1,132 |
| kubernetes | 9.72 | 1,303 / 272 / 707 | 2,216 | 8.49 / 2,149 | 861.97 / 2,281 |

Clustering, analysis and the report add about 2.3 s and 70 MB on Kubernetes. Outputs are
byte-identical between `--threads 1` and the default on both repos. The stage-1 Kubernetes cold
build measured 5.5 s on a quiet machine; these runs shared the machine with other work, so compare
the two dijkstral columns with each other.

### Stage 2 head-to-head: merged build vs full Graphify — 2026-10-13

dijkstral at `2829682` (clustering, analysis, report and the accuracy fixes), quiet machine, 10
threads, 3 runs (Kubernetes 3 for dijkstral, 1 for Graphify). Graphify's column is the stage-0
baseline (full `extract --code-only` with clustering; Kubernetes warm/update with the size cap
raised). Raw: `results/build-1791848836.json`, `results/build-1791848853.json`.

| repo | scenario | Graphify (full) s | dijkstral s | speedup | Graphify peak RSS MB | dijkstral peak RSS MB |
|---|---|---|---|---|---|---|
| monet | cold | 2.07 | 0.14 | 15× | 686 | 58 |
|  | warm | 1.12 | 0.08 | 14× | 109 | 8 |
|  | update | 1.95 | 0.08 | 23× | 621 | 9 |
| graphify | cold | 9.26 | 0.37 | 25× | 964 | 187 |
|  | warm | 4.98 | 0.17 | 28× | 290 | 97 |
|  | update | 7.71 | 0.17 | 44× | 894 | 101 |
| fastapi | cold | 6.80 | 0.25 | 27× | 707 | 89 |
|  | warm | 3.01 | 0.13 | 23× | 162 | 78 |
|  | update | 9.51 | 0.15 | 64× | 617 | 70 |
| ripgrep | cold | 1.70 | 0.12 | 14× | 640 | 66 |
|  | warm | 0.97 | 0.08 | 12× | 101 | 8 |
|  | update | 1.02 | 0.08 | 12× | 115 | 8 |
| excalidraw | cold | 3.98 | 0.22 | 18× | 759 | 86 |
|  | warm | 1.98 | 0.11 | 17× | 161 | 76 |
|  | update | 3.92 | 0.12 | 34× | 791 | 77 |
| erpnext | cold | 22.44 | 0.88 | 25× | 1,132 | 298 |
|  | warm | 10.44 | 0.29 | 35× | 464 | 243 |
|  | update | 21.41 | 0.31 | 69× | 1,451 | 234 |
| kubernetes | cold | 861.97 | 7.90 | 109× | 2,280 | 2,078 |
|  | warm | 184.54 | 4.50 | 41× | 3,144 | 1,907 |
|  | update | 735.97 | 4.45 | 165× | 4,713 | 1,908 |

dijkstral is faster (12× to 165×) and lower in peak memory in all 21 scenarios. Graphify's build
also writes `graph.html`; since stage 4a dijkstral's does too (2026-10-14, cold builds: erpnext
50 ms for a 6.2 MB WebGL page, Kubernetes 544 ms for 69 MB with peak RSS unchanged, where
Graphify's `export html` fails above 5,000 communities; `cli/docs/graphify-exports.md` §7). A no-change
rebuild now reruns clustering, analysis and the report (Kubernetes warm 4.5 s against 2.2 s without
them); skipping them when no input changed is the obvious next optimisation.

Graph accuracy against SCIP is in [accuracy/README.md](accuracy/README.md): dijkstral meets
"precision ≥ Graphify and recall ≥ Graphify" in all 27 repo/relation rows. Parity with Graphify's
graph on the merged build (`parity.py`, recall / precision %, and community NMI):

| repo | def nodes | all links | community NMI |
|---|---|---|---|
| monet | 99.8 / 99.9 | 97.2 / 95.9 | 0.945 |
| graphify | 95.9 / 99.9 | 97.3 / 95.9 | 0.944 |
| fastapi | 100.0 / 100.0 | 93.8 / 94.0 | 0.915 |
| ripgrep | 99.9 / 100.0 | 94.9 / 95.2 | 0.917 |
| excalidraw | 100.0 / 100.0 | 97.3 / 99.5 | 0.831 |
| erpnext | 100.0 / 100.0 | 94.1 / 91.1 | 0.905 |
| kubernetes | 99.9 / 100.0 | 99.9 / 99.8 | 0.865 |

Link parity moved slightly since stage 1 where the accuracy fixes made dijkstral deliberately
differ from Graphify (e.g. ripgrep gains 205 correct file-level Rust imports Graphify lacks).


### Stage 3: query, path and explain — 2026-10-14

Apple M5, 10 cores, 16 GB, macOS 27.0; `bench/query.py latency --runs 20` with the argv of
`cli/tests/fixtures/query/latency.json`. Graphify 0.9.86 (`GRAPHIFY_MAX_GRAPH_BYTES=8GB`) reads
its frozen reference graph; dijkstral (worktree build on top of `c95c097`) reads its own
`extract --code-only` build of the same corpus commit (`graph.dij`) and Graphify's `graph.json`.
Every call is a fresh process; spawning `/usr/bin/true` from the same harness costs 3.9 ms p50
(7.2 ms p95), so the `graph.dij` rows are mostly process start. Raw:
`results/query-1791911392.json`, `results/query-1791912375.json` (Graphify) and
`results/query-1791912686.json` (dijkstral).

| repo | command | Graphify p50 / p95 ms | RSS | dijkstral (graph.dij) p50 / p95 ms | RSS | dijkstral on Graphify's graph.json p50 / p95 ms | RSS |
|---|---|---|---|---|---|---|---|
| fastapi | query | 418 / 426 | 115 MB | 6.6 / 7.5 | 7 MB | 44 / 45 | 51 MB |
| fastapi | path | 410 / 415 | 118 MB | 9.1 / 10.1 | 5 MB | 43 / 46 | 51 MB |
| fastapi | explain | 378 / 387 | 107 MB | 8.7 / 10.1 | 5 MB | 42 / 44 | 51 MB |
| erpnext | query | 1,022 / 1,049 | 260 MB | 10.6 / 15.0 | 11 MB | 109 / 112 | 133 MB |
| erpnext | path | 1,063 / 1,078 | 285 MB | 10.0 / 11.2 | 6 MB | 108 / 109 | 133 MB |
| erpnext | explain | 955 / 978 | 243 MB | 8.5 / 9.8 | 5 MB | 109 / 118 | 133 MB |
| kubernetes | query | 12,526 / 13,170 | 2,854 MB | 14.8 / 16.9 | 39 MB | 1,266 / 1,303 | 1,927 MB |
| kubernetes | path | 14,495 / 14,845 | 2,983 MB | 8.5 / 17.3 | 11 MB | 1,270 / 1,293 | 1,927 MB |
| kubernetes | explain | 11,601 / 14,145 | 2,639 MB | 9.0 / 13.1 | 6 MB | 1,288 / 1,341 | 1,927 MB |

- From `graph.dij` every command is 7–15 ms p50 at every size (Graphify 0.4 s, 1 s, 12–14 s),
  with 5–40 MB peak RSS against 0.1–3 GB.
- On Graphify's own `graph.json` dijkstral parses and indexes the JSON on every call, with no
  size cap: 44 ms, 109 ms and 1.3 s, about 10x faster than Graphify with 30–55 % less memory.

Parity (`bench/query.py parity --questions 60 --questions-large 20`, raw
`results/query-parity-1791913777.json`): both tools on Graphify's reference graph, with
hand-written and fixed-seed label-word questions plus `explain` and undirected `path` on sampled
ids.

| repo | cases | identical output | same seeds | same NODE lists | same EDGE sets | same exit |
|---|---|---|---|---|---|---|
| fastapi | 70 | 70 | 70 | 70 | 70 | 70 |
| erpnext | 70 | 70 | 70 | 70 | 70 | 70 |
| kubernetes | 30 | 30 | 30 | 30 | 30 | 30 |

The 74 query goldens pass on both load paths. The probes behind the stage-3 refinements of
`cli/docs/graphify-query.md` (630 `path` probes, 750 `benchmark` questions on random graphs,
`explain`, `affected` and `god-nodes` on random fastapi, erpnext and ripgrep nodes) all match
except one recorded open case (§4: the exact id of an extensionless file's node).

### Stage 3: MCP server — 2026-10-14

Apple M5, 10 cores, 16 GB, macOS 27.0; `bench/query.py mcp --starts 5 --calls 20`, raw
`results/mcp-1791916372.json`. `graphify-mcp` 0.9.86 (`GRAPHIFY_MAX_GRAPH_BYTES=8GB`,
`GRAPHIFY_QUERY_LOG_DISABLE=1`) serves its frozen reference graph; `dijkstral mcp` (worktree build
on top of `c95c097`) serves its own `extract --code-only` build (`graph.dij`) and Graphify's
`graph.json`. Each cold start sends `initialize`, `notifications/initialized` and `tools/list`
back to back and times spawn → `tools/list` reply; then one `query_graph` with the repo's
latency question at depth 2, then 20 warm calls (100 per row). Graphify loads its graph before
answering the handshake; dijkstral answers it at once and loads on a background thread, so its
first call carries the load.

| repo | server | spawn → tools/list p50 / p95 ms | 1st query_graph p50 / p95 ms | warm query_graph p50 / p95 ms | peak RSS |
|---|---|---|---|---|---|
| fastapi | graphify-mcp | 606 / 609 | 55.9 / 56.3 | 11.7 / 13.2 | 154 MB |
| fastapi | dijkstral (graph.dij) | 14.6 / 18.1 | 1.1 / 1.2 | 0.59 / 0.73 | 7 MB |
| fastapi | dijkstral (Graphify's graph.json) | 12.6 / 20.3 | 27.3 / 28.0 | 0.57 / 0.67 | 53 MB |
| erpnext | graphify-mcp | 1,198 / 1,297 | 104 / 112 | 18.9 / 20.3 | 300 MB |
| erpnext | dijkstral (graph.dij) | 15.0 / 18.0 | 1.9 / 17.0 | 1.27 / 1.36 | 12 MB |
| erpnext | dijkstral (Graphify's graph.json) | 14.7 / 21.0 | 89.9 / 90.6 | 1.27 / 1.33 | 133 MB |
| kubernetes | graphify-mcp | 10,406 / 12,472 | 2,063 / 2,248 | 101 / 122 | 3,324 MB |
| kubernetes | dijkstral (graph.dij) | 13.6 / 22.2 | 6.7 / 55.0 | 5.25 / 5.38 | 41 MB |
| kubernetes | dijkstral (Graphify's graph.json) | 14.5 / 19.6 | 1,206 / 1,227 | 5.58 / 5.74 | 1,928 MB |

- Ready for `tools/list` in 13–15 ms at every size (Graphify 0.6 s, 1.2 s, 10.4 s); a warm
  `query_graph` is 20x faster (0.6 ms, 1.3 ms, 5.3 ms against 12, 19 and 101 ms).
- On Graphify's `graph.json` both servers return the same answer (seeds, NODE list, EDGE set).
  The `graph.dij` rows answer from dijkstral's own build of the corpus, so their answers differ
  as the graphs do.
- Conformance: the 53 MCP transcripts replay in `crates/cli/tests/mcp_goldens.rs` (50 identical
  texts; the other three and every `isError` difference are the deliberate ones of
  `cli/docs/graphify-query.md` §10.9). A black-box differential run of both servers on the same
  graph (305 tool and resource calls per fixture: every resource, `graph_stats`, `god_nodes` with
  and without `exclude_hubs_percentile`, `get_community`/`get_neighbors` with budgets,
  `get_node`, BFS and DFS `query_graph` at depths 1–3, directed and undirected `shortest_path`)
  matched on all seven fixtures and on the fastapi (500 calls), ripgrep, excalidraw, graphify,
  monet, erpnext (240 each) and kubernetes (162) references, comparing EDGE lines as a set and
  ambiguity texts case-folded.
### Stage 3: hook guard latency — 2026-10-14

`hook.py`, 5 warm-up + 100 timed fresh-process runs per case, Graphify, dijkstral and the exec
floor (`/usr/bin/true`) interleaved run by run; python fixture project; Apple M5 (10 cores),
macOS 27.0; graphifyy 0.9.86, dijkstral 0.1.0 release build. p50 / p95 ms:

| case | decision | Graphify | dijkstral | exec floor |
|---|---|---|---|---|
| search: Bash `git status` | silent | 69.9 / 76.8 | 11.0 / 14.5 | 6.2 / 8.3 |
| search: Bash `rg` pipeline | nudge | 69.6 / 75.2 | 10.7 / 15.1 | 5.9 / 8.2 |
| search: Grep | nudge | 70.3 / 77.0 | 10.7 / 14.9 | 6.0 / 7.9 |
| read: Read `.py` | nudge | 68.7 / 74.0 | 13.4 / 15.5 | 7.0 / 8.5 |
| read: Read `.json` | silent | 68.8 / 72.6 | 13.9 / 15.2 | 6.9 / 8.2 |
| read: Glob `**/*.py` | nudge | 68.1 / 73.2 | 13.2 / 15.1 | 7.1 / 8.6 |
| read --strict, session already denied | nudge | 64.2 / 78.3 | 8.4 / 12.1 | 4.9 / 7.2 |
| search: Grep, no graph | silent | 64.7 / 76.4 | 8.9 / 12.4 | 5.1 / 8.2 |
| `hook-check` (Codex no-op) | silent | 65.7 / 78.4 | 8.7 / 13.9 | 5.2 / 8.2 |

Both tools return the same decision class in every case. dijkstral is 5–8× faster but misses the
5 ms p50 target on this machine, where spawning any process is slow: `/usr/bin/true` alone costs
5–7 ms p50 (an endpoint-security agent inspects every exec). The guard's own work is microseconds
(`cargo test --release -p dijkstral-agents decision_cost -- --ignored --nocapture`: 1.6–3.7 µs per
decision). The rest of dijkstral's margin over the floor is exec and load cost that a minimal Rust
binary also pays (+2.5–4 ms over `/usr/bin/true`), about 1.5 ms of it from binary size (a 12 MB
binary measured against a 0.5 MB one); a separate small hook binary would recover that part.

### Stage 4a: C, C++, C#, Kotlin, Scala, Swift, PHP, Ruby and the exporters — 2026-10-14

Eight corpus repos were added (`corpus.toml`: jq, fmt, serilog, okhttp, munit, Alamofire, guzzle,
sinatra) with frozen Graphify references. dijkstral at `c59ab6e` (clustering, report and
`graph.html` on; Graphify's `extract` writes no HTML), 10 threads, 3 runs, quiet machine. Raw:
`results/build-1791952480.json`.

| repo | scenario | Graphify s | dijkstral s | speedup | Graphify RSS MB | dijkstral RSS MB |
|---|---|---|---|---|---|---|
| jq | cold | 1.68 | 0.13 | 13× | 645 | 50 |
|  | warm | 0.75 | 0.05 | 15× | 82 | 11 |
|  | update | 0.81 | 0.08 | 10× | 83 | 11 |
| fmt | cold | 2.61 | 0.20 | 13× | 702 | 109 |
|  | warm | 1.22 | 0.05 | 24× | 118 | 10 |
|  | update | 1.46 | 0.12 | 12× | 156 | 73 |
| serilog | cold | 1.90 | 0.14 | 14× | 634 | 54 |
|  | warm | 0.91 | 0.05 | 17× | 102 | 12 |
|  | update | 1.00 | 0.10 | 10× | 98 | 12 |
| okhttp | cold | 5.46 | 0.48 | 11× | 750 | 196 |
|  | warm | 3.54 | 0.06 | 57× | 280 | 10 |
|  | update | 3.87 | 0.25 | 16× | 323 | 163 |
| munit | cold | 1.89 | 0.17 | 11× | 608 | 34 |
|  | warm | 0.85 | 0.05 | 16× | 88 | 10 |
|  | update | 0.92 | 0.09 | 10× | 88 | 10 |
| alamofire | cold | 2.86 | 0.20 | 15× | 722 | 98 |
|  | warm | 1.29 | 0.05 | 25× | 111 | 9 |
|  | update | 1.92 | 0.12 | 16× | 145 | 66 |
| guzzle | cold | 2.93 | 0.22 | 13× | 721 | 113 |
|  | warm | 1.05 | 0.05 | 20× | 102 | 8 |
|  | update | 1.58 | 0.12 | 13× | 128 | 64 |
| sinatra | cold | 1.65 | 0.11 | 15× | 616 | 43 |
|  | warm | 0.59 | 0.05 | 12× | 72 | 8 |
|  | update | 0.74 | 0.09 | 9× | 78 | 8 |

dijkstral is faster (9× to 57×) and lower in peak memory in all 24 scenarios.

Graph parity on the merged build, all 15 corpus repos (`parity.py`, all extensions; recall /
precision %, community NMI). The original seven did not regress; the graphify repo's definition
recall rose from 95.9 to 97.6 because its fixtures include the new languages.

| repo | def nodes | all nodes | all links | NMI |
|---|---|---|---|---|
| monet | 99.8 / 99.9 | 98.2 / 99.3 | 97.2 / 95.9 | 0.945 |
| graphify | 97.6 / 99.9 | 97.9 / 99.4 | 98.0 / 95.9 | 0.937 |
| fastapi | 100.0 / 100.0 | 98.7 / 97.9 | 93.8 / 94.0 | 0.915 |
| ripgrep | 100.0 / 100.0 | 97.3 / 99.2 | 94.9 / 95.2 | 0.911 |
| excalidraw | 100.0 / 100.0 | 98.2 / 99.9 | 97.3 / 99.5 | 0.831 |
| erpnext | 100.0 / 100.0 | 99.8 / 99.6 | 94.1 / 91.1 | 0.905 |
| kubernetes | 99.9 / 100.0 | 99.9 / 100.0 | 99.9 / 99.8 | 0.875 |
| jq (C) | 100.0 / 100.0 | 100.0 / 100.0 | 100.0 / 99.9 | 0.957 |
| fmt (C++) | 98.6 / 99.4 | 98.0 / 98.2 | 96.3 / 95.9 | 0.900 |
| serilog (C#) | 97.3 / 100.0 | 97.4 / 100.0 | 98.4 / 99.9 | 0.841 |
| okhttp (Kotlin) | 100.0 / 100.0 | 99.9 / 99.6 | 98.7 / 98.9 | 0.944 |
| munit (Scala) | 100.0 / 100.0 | 99.9 / 99.6 | 99.2 / 98.7 | 0.921 |
| alamofire (Swift) | 99.9 / 100.0 | 99.9 / 100.0 | 99.1 / 99.2 | 0.935 |
| guzzle (PHP) | 100.0 / 100.0 | 100.0 / 100.0 | 100.0 / 100.0 | 0.940 |
| sinatra (Ruby) | 100.0 / 100.0 | 100.0 / 100.0 | 100.0 / 100.0 | 1.000 |

serilog's whole-repo gap is non-`.cs` files (`.csproj`, `.razor`) that Graphify extracts and
dijkstral defers to stage 4b; on `.cs` alone definitions are at 100%. fmt's gaps are listed in
`cli/docs/graphify-format.md` §25 (self-calls Graphify drops in large files, include-target rules
only partly pinned).


### Stage 4b: Elixir, Erlang, OCaml and Julia — 2026-10-14

Graph parity, `parity.py` on cold `extract` outputs (spec §29–§32):

| repo | scope | nodes | links | NMI |
|---|---|---|---|---|
| ecto (Elixir) | `--ext .ex,.exs` | 100.0 / 100.0 (1954) | 100.0 / 100.0 (3943) | 1.000, same array order |
| graphify `sample.ex` + `sample.jl` | scratch copy, all | 100.0 / 100.0 (23) | 100.0 / 100.0 (27) | 1.000 |

Erlang (`.erl .hrl`) and OCaml (`.ml .mli`) contribute no nodes in Graphify 0.9.86 (optional
grammar extras not installed); dijkstral counts them as code and emits nothing. In the whole
graphify repository `sample.jl`'s ids are collision-renamed against `sample.ps1`/`sample.f90`,
so five Julia definitions differ by id until those languages are extracted.

ecto cold `extract`, single runs on a busy machine (other builds running), indicative only:
Graphify 3.55 s wall (6.53 s user), dijkstral 0.25 s wall (0.70 s user). dijkstral `--timings`
(release, 126 files): discover 9.0 ms, stat 51.8, cache-read 30.0, extract 150.8, cache-write
18.7, resolve 9.5, cluster 9.0, write 26.7, analyze 2.8, report 34.7, html 12.9.
Stage 4b, C# project/markup and grammar-less files (2026-10-15, dijkstral on base `16deb0e` plus
the stage-4b change, all extensions, same frozen references):

| repo | def nodes | all nodes | all links | NMI |
|---|---|---|---|---|
| serilog (C#) | 100.0 / 100.0 | 99.9 / 100.0 | 99.7 / 99.9 | 0.881 |
| graphify | 99.1 / 100.0 | 99.1 / 99.4 | 98.6 / 95.9 | 0.948 |

serilog's remaining node gap is its two `.ps1` scripts (PowerShell, another stage); the graphify
repository's remaining definitions are in languages other stages add (`.f90 .groovy .ps1 .psd1 .zig
.jl .sv .m .luau .ex`) plus Python/C++/JS differences already listed for those languages.

### Shared fold and analysis rules — 2026-10-15

The build's fold (spec §8.2: orientation of a pair's first edge, the structural set, absorbed call
contexts, ties of one relation versus two, repeated self-loops, `instantiates` and `re_exports`
ranks) and the analysis (§17.4–§17.5: sorted-link adjacency, hub and god-node candidates,
surprise relations and the community-bridge fallback, top-three bridges, `no_signal`, the
`Unclassified` line) were re-derived black-box and consolidated. `parity.py` of cold `extract`
(all extensions) against the frozen references, before → after; def nodes and NMI did not move:

| repo | def nodes R / P | links R / P | NMI | link field mismatches | of which `context` |
|---|---|---|---|---|---|
| monet | 99.8 / 99.9 | 97.18 / 95.95 → 97.18 / 95.95 | 0.945 | 29 → 29 | 0 → 0 |
| graphify | 99.9 / 100.0 | 98.96 / 95.89 → 98.96 / 95.89 | 0.938 | 1614 → 1612 | 13 → 11 |
| fastapi | 100.0 / 100.0 | 93.80 / 94.03 → 93.80 / 94.03 | 0.915 | 967 → 967 | 63 → 63 |
| ripgrep | 100.0 / 100.0 | 94.93 / 95.18 → 94.93 / 95.18 | 0.911 | 3 → 3 | 1 → 1 |
| excalidraw | 100.0 / 100.0 | 97.25 / 99.50 → 97.25 / 99.50 | 0.831 | 36 → 36 | 3 → 3 |
| erpnext | 100.0 / 100.0 | 94.06 / 91.11 → 94.06 / 91.11 | 0.905 | 1910 → 1907 | 395 → 392 |
| kubernetes | 100.0 / 100.0 | 99.91 / 99.84 → 99.91 / 99.84 | 0.866 | 2513 → 2512 | 2513 → 2512 |
| jq | 100.0 / 100.0 | 100.00 / 99.89 → 100.00 / 99.89 | 0.957 | 0 → 0 | 0 → 0 |
| fmt | 98.6 / 99.4 | 96.32 / 95.88 → 96.33 / 95.89 | 0.899 | 1135 → 1134 | 13 → 12 |
| serilog | 100.0 / 100.0 | 99.75 / 99.92 → 99.75 / 99.92 | 0.881 | 49 → 27 | 24 → 2 |
| okhttp | 100.0 / 100.0 | 98.73 / 98.92 → 98.73 / 98.92 | 0.944 | 65 → 39 | 42 → 16 |
| munit | 100.0 / 100.0 | 99.21 / 98.68 → 99.21 / 98.68 | 0.921 | 7 → 5 | 6 → 4 |
| alamofire | 99.9 / 100.0 | 99.14 / 99.16 → 99.14 / 99.16 | 0.935 | 133 → 118 | 104 → 89 |
| guzzle | 100.0 / 100.0 | 99.99 / 99.99 → 99.99 / 99.99 | 0.940 | 2 → 0 | 2 → 0 |
| sinatra | 100.0 / 100.0 | 99.95 / 99.95 → 99.95 / 99.95 | 1.000 | 3 → 0 | 0 → 0 |
| dart-http | 99.8 / 99.7 | 99.28 / 98.88 → 99.28 / 98.88 | 0.965 | 122 → 39 | 88 → 5 |
| ecto | 100.0 / 100.0 | 100.00 / 100.00 → 100.00 / 100.00 | 1.000 | 0 → 0 | 0 → 0 |
| penlight | 100.0 / 100.0 | 100.00 / 100.00 → 100.00 / 100.00 | 1.000 | 0 → 0 | 0 → 0 |
| afnetworking | 94.2 / 94.2 | 92.59 / 92.55 → 92.59 / 92.55 | 0.949 | 5 → 5 | 0 → 0 |

`dijkstral cluster-only` on each frozen reference reproduces `graphify cluster-only --no-label`
byte for byte: `graph.json` on all 19 repositories (before: 17), `.graphify_analysis.json` on 17
(before: 8) and `GRAPH_REPORT.md` on 14 (before: 5). The rest differ in import-cycle order
(ripgrep, excalidraw, Kubernetes, fastapi) and in `verify_inferred` questions (fastapi, erpnext;
§17.7, Open). Outputs are byte-identical across
`--threads 1` and the default on monet, fastapi, alamofire, afnetworking, guzzle, dart-http and
penlight.

### Stage 4b head-to-head: Dart, Elixir, Lua, Objective-C — 2026-10-15

Four corpus repos added for stage 4b (dart-http, ecto, Penlight, AFNetworking), frozen references,
dijkstral after the shared-rule consolidation and the call-flow stamp fix (`3296e29`), 10 threads,
3 runs, quiet machine. Raw: `results/build-1792027080.json`. Stage 4b also added Groovy/Gradle,
Erlang/OCaml (no nodes, as Graphify without those grammar extras), Julia, Zig, Fortran,
Verilog/SystemVerilog, Solidity (no nodes), Vue, Svelte, Astro, PowerShell, VB (no nodes), .NET
project/markup files, Pascal, Apex, COBOL and BYOND; their fixture goldens and parity are in
`cli/docs/graphify-format.md` §26–§44.

| repo | scenario | Graphify s | dijkstral s | speedup | Graphify RSS MB | dijkstral RSS MB |
|---|---|---|---|---|---|---|
| dart-http | cold | 16.08 | 0.30 | 54× | 688 | 196 |
|  | warm | 2.92 | 0.05 | 57× | 221 | 12 |
|  | update | 2.69 | 0.19 | 14× | 343 | 167 |
| ecto | cold | 1.25 | 0.14 | 9× | 639 | 42 |
|  | warm | 0.66 | 0.05 | 14× | 77 | 8 |
|  | update | 0.71 | 0.08 | 8× | 80 | 9 |
| penlight | cold | 1.21 | 0.10 | 12× | 586 | 32 |
|  | warm | 0.51 | 0.05 | 11× | 69 | 7 |
|  | update | 0.61 | 0.08 | 8× | 62 | 7 |
| afnetworking | cold | 1.12 | 0.10 | 11× | 613 | 53 |
|  | warm | 0.63 | 0.04 | 14× | 77 | 9 |
|  | update | 0.63 | 0.09 | 7× | 69 | 10 |

dijkstral is faster (7× to 57×) and lower in peak memory in all 12 scenarios.


### Stage 5: semantic pass, same model — 2026-10-15

`semantic.py`: both tools run `extract --backend openai` against one `llm_shim.py --responder
codex` (`gpt-6.1-sol`, reasoning effort `medium`, both passed explicitly on every `codex exec`),
scrubbed environment, dummy key, scratch copies. dijkstral goes through the shim rather than its
own Codex backend so the transport (preamble, `codex exec` flags, model, effort) is identical and
only each tool's prompt, chunking and merge differ. Corpora: the `mixed` fixture (README, two
docs, YAML, PDF, PNG, DOCX, XLSX; 3 Python files) and a ripgrep slice (`crates/globset` README
and sources, four crate READMEs, `RELEASE-CHECKLIST.md`). Graphify 0.9.86 semantic venv,
dijkstral at this commit (release). Four Codex calls in all; reruns made none. Raw:
`results/semantic-1792070444.json`.

| Corpus | Tool | Wall s | Calls | Tokens in / out | Semantic nodes | Semantic links | Docs with ≥ 1 node | Doc→code links | Hyperedges | Rerun s | Rerun calls |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| mixed | Graphify | 66.1 | 1 | 17,614 / 3,443 | 18 | 34 | 8/8 | 0 | 2 | 0.63 | 0 |
| mixed | dijkstral | 78.8 | 1 | 16,735 / 4,177 | 24 | 41 | 8/8 | 0 | 3 | 0.13 | 0 |
| ripgrep | Graphify | 83.9 | 1 | 18,759 / 4,390 | 28 | 32 | 6/6 | 0 | 2 | 0.54 | 0 |
| ripgrep | dijkstral | 82.8 | 1 | 19,542 / 4,540 | 33 | 31 | 6/6 | 0 | 3 | 0.12 | 0 |

- Wall time is the model: one request each, 65–83 s of Codex time; the tools' own work is under a
  second. Reruns served everything from the cache with no request: dijkstral 0.12–0.13 s,
  Graphify 0.54–0.63 s.
- The dijkstral `mixed` row is its recorded reply merged again from the cache after one fix (no
  extra call): dijkstral's requests frame office files by their own path, so the model attributed
  6 nodes to `office/*.docx`/`.xlsx`, which the first run dropped as out of scope (6 of 8 files,
  18 nodes, 31 links). The pass now accepts both the original and the converted path.
- Neither tool's model linked a document to a code node: both prompts show only the documents,
  so the models create their own concept nodes (`readme_jobqueue` next to the code
  `src_app_service_jobqueue`) instead of naming code ids. `update`'s backtick references
  (no model) are what connect docs to code today.
- Token counts are Codex's own usage through the shim (they include about 12k tokens of Codex's
  instructions, mostly cached); a direct API call would report less for both tools.

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
| `run.py` | Build benchmark: cold / warm / one-file update, wall, CPU, process-tree peak RSS, node and edge counts → `results/build-<epoch>.json`; `report` prints the markdown table |

Later stages add query and hook latency, graph parity, SCIP precision/recall, and the paid
agent-outcome runs (see `../DIJKSTRAL_DESIGN.md`).

## Run

```bash
cd products/dijkstral
bench/setup.sh                       # idempotent; `bench/setup.sh fastapi` for one repo
uv run --no-project bench/run.py run --tools graphify --runs 3
uv run --no-project bench/run.py run --tools graphify,dijkstral --repos fastapi,ripgrep
uv run --no-project bench/run.py report bench/results/build-<epoch>.json
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

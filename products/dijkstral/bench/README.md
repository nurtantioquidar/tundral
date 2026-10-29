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
| `setup.sh` | Installs pinned Graphify (`graphifyy[leiden,mcp]`, Python 3.13 so Leiden uses graspologic-native) into an isolated venv and fetches each repo at its pinned commit; `GRAPHIFY_VERSION=<version>` installs another release beside the pin (with the `sql` extra from 0.9.97) |
| `run.py` | Build benchmark: cold / warm / one-file update, wall, CPU, process-tree peak RSS, node and edge counts → `results/build-<epoch>.json`; `report` prints the markdown table. Tools: `graphify`, `graphify-nocluster`, `dijkstral` |
| `reference.sh` | Freezes Graphify's cold `extract --code-only` graph per repo as `$DIJKSTRAL_BENCH_HOME/reference/<repo>.graph.json` (another `GRAPHIFY_VERSION`: `reference-<version>/<repo>.graph.json`) |
| `parity.py` | Compares two `graph.json` files as sets: nodes by kind, links by relation, field agreement, samples of each difference; for clustered graphs, community agreement (same id, NMI, ARI) and array order |
| `accuracy/` | Stage-2 SCIP oracle: precision and recall of both tools' graphs against compiler-grade indexes ([accuracy/README.md](accuracy/README.md)) |
| `query.py` | Stage-3 query benchmark: `latency` (fresh process per call, p50/p95, peak RSS; Graphify on its reference graph, dijkstral on its own `graph.dij` and on Graphify's `graph.json`) → `results/query-<epoch>.json`; `parity` (both tools on Graphify's reference graph, seeds, NODE lists, EDGE sets and full output compared) → `results/query-parity-<epoch>.json`; `mcp` (MCP servers over stdio: spawn → `tools/list`, first and warm `query_graph` calls, peak RSS) → `results/mcp-<epoch>.json`; `report` prints the tables |
| `hook.py` | Hook-guard latency: one fresh process per Claude Code `PreToolUse` payload for both tools, interleaved with the exec floor (`/usr/bin/true`) → `results/hook-<epoch>.json` |
| `llm_shim.py` | Stage-5 semantic pass: localhost OpenAI-compatible server that answers through `codex exec` (ChatGPT subscription, no API key), replays recorded replies, or serves static ones; writes clean-room request transcripts ([semantic/README.md](semantic/README.md)) |
| `semantic.py` | Stage-5 same-model semantic benchmark: Graphify and dijkstral through one Codex-backed shim (`--max-requests` caps Codex calls, `--dry-run` uses static replies) → `results/semantic-<epoch>.json` and a table |
| `semantic/` | Semantic fixture binaries (`make_binaries.py`), markers, and how to run Graphify's model pass through the shim |
| `agent/` | Stage-7 agent-outcome benchmark: task files, `run.py` (`prepare`, `run`, `judge`, `report` across the `none`, `graphify` and `dijkstral` arms) → `results/agent-<epoch>.{md,json}` ([Stage 7](#stage-7-agent-outcome)) |

The paid agent-outcome runs (stage 7) are explicit commands; nothing here calls a model on its own
(see `../DIJKSTRAL_DESIGN.md`).

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

## Stage 7: agent outcome

`agent/run.py` measures what each tool does for an agent: the same Codex model answers every task of
a task file three times, and the answers are scored. Task files: `agent/questions/<repo>.jsonl`
(key facts on fastapi, ripgrep and Kubernetes, [agent/questions/README.md](agent/questions/README.md))
and `agent/tasks/{locomo,longmemeval}.jsonl` (memory suites, [agent/README.md](agent/README.md)).

| Arm | Workspace (one per corpus, reused by every task) | Attached for the call |
|---|---|---|
| `none` | a copy of the corpus | nothing |
| `graphify` | copy + `graphify extract <ws> --code-only` (semantic tasks: `graphify extract <ws> --backend openai` through `llm_shim.py --responder codex`, as `semantic.py` does) + `graphify codex install` | `graphify-mcp <ws>/graphify-out/graph.json` |
| `dijkstral` | copy + `dijkstral extract <ws> --code-only` (semantic: `--backend codex --model gpt-6.1-sol --reasoning-effort medium`) + `dijkstral codex install` | `dijkstral mcp <ws>/dijkstral-out/graph.dij` |

```bash
cd products/dijkstral
cargo build --release --manifest-path cli/Cargo.toml       # or pass --dijkstral <binary> everywhere
T=bench/agent/questions/fastapi.jsonl
uv run --no-project bench/agent/run.py prepare --tasks $T --dry-run   # print every command, call nothing
uv run --no-project bench/agent/run.py prepare --tasks $T             # build the workspaces once
uv run --no-project bench/agent/run.py run     --tasks $T --dry-run   # print each session's codex command
uv run --no-project bench/agent/run.py run     --tasks $T --fake-provider --limit 1   # end to end, no model
uv run --no-project bench/agent/run.py run     --tasks $T --jobs 2    # paid: the answering sessions
uv run --no-project bench/agent/run.py judge   --tasks $T             # paid: the blind judge
uv run --no-project bench/agent/run.py report  --tasks $T             # results/agent-<epoch>.{md,json}
uv run --no-project python -m unittest discover -s bench/agent -p 'test_*.py'
```

Every command takes `--arms`, `--limit N`, `--ids a,b`, `--jobs N` (default 2), `--model`
(default `gpt-6.1-sol`), `--effort` (default `medium`), `--timeout` (seconds per Codex call,
default 900) and `--retries` (default 2). `GRAPHIFY_VERSION=<version>` (installed by `setup.sh`
under the same variable) runs the `graphify` arm on another Graphify release: its workspace,
manifest, sessions and build state are stored as `graphify-<version>`, apart from the pin's, and
the report header names the version.

**Workspaces.** `prepare` copies the corpus (`/bin/cp -cR`, an APFS clone; the tools' own
`graphify-out/` and `dijkstral-out/` are dropped, the checkouts are never written) to
`codebench/ws/<opaque id>/<corpus_id>/` beside the bench home (or `$AGENT_BENCH_WS`), so the
working directory names neither the arm nor either tool.
Builds and installs run with a scrubbed environment and a scratch `HOME`; after the install that
`HOME` must still be empty, or `prepare` stops (verified for both tools: each writes exactly
`AGENTS.md` and `.codex/hooks.json` in the workspace). `agent/manifests/<corpus_id>.<arm>.json`
records the build and install commands and times, model calls (Graphify: shim transcripts;
dijkstral: a counting `codex` wrapper passed as `DIJKSTRAL_CODEX`), the tool's own token counts,
graph size, install footprint, hook commands and versions. The manifest is written last, so an
interrupted `prepare` resumes per corpus, reusing a finished copy and the tools' semantic caches.
Workspaces bake in the tool paths (hooks, MCP command), so `run` refuses a different `--dijkstral`.

**Sessions.** One fixed prompt for every arm: `Question: …`, `Question date: …` when the task has
one, then "Answer from this repository. Cite file paths and symbol names where relevant. Be
concise." (memory suites: "Answer from these notes (the files in this directory). Cite file paths
where relevant. Be concise."). Nothing about graphs: the tools' installed instructions do that. No
other task field (`reference`, `facts`, `source_id`, `distractor`) reaches the agent or, apart
from the reference and facts, the judge. The command, per arm:

```text
codex exec --ephemeral --ignore-user-config -m gpt-6.1-sol -c model_reasoning_effort="medium" \
  -s read-only -C <workspace> --json --skip-git-repo-check \
  --disable apps -c web_search="disabled" --color never \
  [-c mcp_servers.<tool>.command="<binary>" -c mcp_servers.<tool>.args=[...] \
   -c mcp_servers.<tool>.startup_timeout_sec=120 -c mcp_servers.<tool>.tool_timeout_sec=120 \
   -c mcp_servers.<tool>.default_tools_approval_mode="approve" \
   (graphify: -c mcp_servers.graphify.env={GRAPHIFY_QUERY_LOG_DISABLE = "1"})] \
  -o <session>.last.txt "<prompt>"
```

The environment is the caller's minus `HERDR_*`, `GRAPHIFY_*`, `DIJKSTRAL_*`, `*_API_KEY` and
`OPENAI_BASE_URL`, plus `GRAPHIFY_QUERY_LOG_DISABLE=1`; `PATH` drops any directory that already
provides `graphify` or `dijkstral`, and a graph arm gets a directory holding only its own
executables first (as an installed tool would be). Codex configuration is never written:
everything is a per-call flag. What Codex 0.162.1 does with these flags, established on a local
fake Responses-API provider (no model call; `--fake-provider` runs the same thing):

- `--ignore-user-config` skips `~/.codex/config.toml` but not instructions: the global
  `~/.codex/AGENTS.md` and the workspace's `AGENTS.md` (with each tool's installed section) are in
  the prompt of every arm.
- The ChatGPT-account apps (GitHub, Slack, …) and web search are tools by default; both would let
  an answer come from outside the corpus, so `--disable apps -c web_search="disabled"` is passed to
  every arm and to the judge.
- An MCP tool call needs approval, which `codex exec` cannot give ("MCP tool call requires
  approval, but approval policy is never"); `default_tools_approval_mode="approve"` on the
  attached server allows it, as a user who approves the tool would.
- Hooks: no project `.codex/hooks.json` hook ran in `codex exec`, with or without
  `--dangerously-bypass-hook-trust`, with or without `-c projects."<ws>".trust_level="trusted"`,
  and (for the `PreToolUse` probe) with the user config loaded; probe hooks on `SessionStart` and
  `PreToolUse` that only append to a file. Both tools
  install only a Codex `PreToolUse` `Bash` → `<tool> hook-check` entry, which prints nothing and
  exits 0 (`cli/docs/graphify-query.md` §11.7), so whether it runs changes nothing the model sees.
  The harness therefore passes neither flag, for every arm: hook trust is not bypassed anywhere.

`--json` events are parsed for the final answer, token usage (`input_tokens` including
`cached_input_tokens`, `output_tokens`, `reasoning_output_tokens`), tool calls by type, shell
commands by program (and those running `graphify`/`dijkstral`), MCP calls by `server.tool` and
failures, warnings and wall time. Raw events stay under `$DIJKSTRAL_BENCH_HOME/agent/sessions/<arm>/`
(they contain repository text). A session with a result file is skipped, so a rerun resumes; a
transient failure (non-zero exit, no answer, failed turn, timeout) is retried with backoff up to
`--retries` times, then recorded as `.failed.json` and retried on the next run. A Codex usage
limit (`You've hit your usage limit`, `usage_limit_reached`, `Quota exceeded`, rate-limit / 429
errors in the events or stderr) is not retried: no new session starts, running ones finish, and the
command prints `QUOTA EXHAUSTED — resume with the same command` and exits 75. `prepare` applies
the same rule to the semantic builds.

**Scoring.** Key facts (`agent/score.py`): a fact or alias counts when the answer names it,
case-sensitively on identifier boundaries (`get_openapi` is not credited by `get_openapi_path`;
`Foo.updatePod` and `updatePod()` credit `updatePod`) or, for paths, with either separator and
not as the tail of a longer path; the workspace prefix of absolute paths is removed first. Recall
= matched / facts; pass = matched ≥ `min_facts` (all when absent). LOCOMO and LongMemEval use
`agent/memory.py` (`locomo_score`, `longmemeval_score`). The blind judge is a separate `codex exec`
with the same flags in an empty scratch directory, no MCP: keyfacts and LOCOMO get the fixed
prompt in `run.py` (question, reference, key facts, one answer) with `--output-schema`
`{correct, score, reason}`; LongMemEval gets its official yes/no answer-check prompt
(`memory.longmemeval_judge_prompt`). Answers are judged one at a time in shuffled order
(`--seed`), without the arm, and verdicts are cached by content (`agent/judge/`).
`report` writes per suite × arm pass rate, recall, suite score, judge score and correct rate,
median and total tokens, cached input, tool, MCP and graph-CLI calls and wall time, a per-kind
breakdown and the build table to `results/agent-<epoch>.md` and `.json` (no answers or events).

## Method

- **Pins**: dijkstral's extraction follows Graphify `graphifyy[leiden,mcp,sql]==0.9.97` since
  2026-10-29 ([below](#dijkstral-at-graphify-0997-parity--2026-10-29)); run the scripts with
  `GRAPHIFY_VERSION=0.9.97` (references in `reference-0.9.97/`). Their default is still the earlier
  pin `graphifyy==0.9.86` (git `a5ba60dd`, Python 3.13, graspologic-native Leiden), so earlier
  results keep their paths and the query, export and memory surfaces, still specified against
  0.9.86, keep their baseline. `GRAPHIFY_VERSION=<version>` points `setup.sh`, `reference.sh`,
  `run.py`, `hook.py` and `agent/run.py` at any release. Corpus commits are in `corpus.toml`.
  Record the dijkstral commit and binary sha256 with every dijkstral row.
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
  18 nodes, 31 links). The pass now accepts both the original and the converted path, and
  requests now frame office files by their converted path (`327d5d4`).
- Neither tool's model linked a document to a code node: both prompts show only the documents,
  so the models create their own concept nodes (`readme_jobqueue` next to the code
  `src_app_service_jobqueue`) instead of naming code ids. `update`'s backtick references
  (no model) are what connect docs to code today.
- Token counts are Codex's own usage through the shim (they include about 12k tokens of Codex's
  instructions, mostly cached); a direct API call would report less for both tools.

### Stage 6: git, cross-repo, memory and input commands — 2026-10-16

Stage 6 adds commands without touching extraction, so its check is a regression comparison of
two release builds on this machine: the stage 5 tip (`1537eda`) against the stage 6 merge
(`653bf4a`). Every corpus repository was built cold with `extract --code-only --no-viz`
(`SOURCE_DATE_EPOCH` fixed) by both binaries.

- `graph.json`, `GRAPH_REPORT.md` and `.graphify_analysis.json` are byte-identical on all 19
  repositories. Build wall times match within noise (Kubernetes, four alternating pairs on a
  loaded machine: 12.5 s vs 12.8 s median, about 62 s CPU each).
- `query.py latency` (20 runs, graph.dij), stage 5 → stage 6 p50 ms: fastapi query 10.6 → 7.8,
  path 7.8 → 8.0, explain 7.7 → 8.0; Kubernetes query 15.0 → 15.8, path 8.0 → 10.4, explain
  6.8 → 10.9. The spawn floor moved between 3.5 and 10 ms during these runs; the differences are
  within that noise except for a steady rise of about 4–5 MB RSS per command (fastapi path 6 →
  11 MB).
- `hook.py` (100 runs per case, two rounds each): p50 stays 11.5–12.8 ms, but dijkstral's cost
  over the exec floor rises from about 4.0 ms to about 4.6 ms on every case.
- Cause of both: the binary now links four macOS frameworks that dyld loads at every start
  (`--version` max RSS 4.1 → 8.4 MB): CoreServices through `notify`'s FSEvents backend (`watch`),
  libobjc through `ctrlc` (`dispatch2`/`objc2`, `watch`), and SystemConfiguration and Foundation
  through `whoami`, which `postgres` uses for the default user (`--postgres`).
- Resolved the same day: `watch` and the PostgreSQL client moved into a second binary,
  `dijkstral-live`, which `dijkstral` runs from its own directory. A probe binary per framework
  set showed CoreServices alone, or SystemConfiguration and Foundation alone, cost the whole
  4 MB / ~0.6 ms (libobjc alone costs nothing measurable), so both had to leave. `dijkstral`
  now links only libSystem and libiconv (`crates/cli/tests/live_cli.rs`), `--version` max RSS
  is 4.4 MB, and `hook.py` (100 runs per case, two rounds) puts dijkstral 3.3–4.2 ms over the
  exec floor (mean about 3.7), back at or below stage 5. What remains over `/usr/bin/true` is
  the cost macOS charges any non-Apple binary at launch: a Rust program that does nothing
  measured the same as `dijkstral --version` and `hook-guard` (p50 about 10 ms against 8 ms).

### Stage 7: agent outcome — 2026-10-17

`agent/run.py` (setup above): Codex answers each question in three arms — no graph, Graphify
0.9.86, dijkstral — on read-only scratch copies, each graph arm with its own installed
`AGENTS.md` section and MCP server. Answers are scored by key facts on identifier boundaries
(key-fact set), by the suites' own F1 rules (LOCOMO, LongMemEval), and by a blind
`gpt-6.1-sol` judge (medium) that sees one answer at a time without its arm. Semantic graphs
(LOCOMO, LongMemEval) were built with `gpt-6.1-sol` (medium) for both tools. One run per
question; differences of a few points are within run-to-run noise.

Answering model. With `gpt-6.1-sol` the key-fact set is at ceiling for every arm (no graph 100%
pass, judge 0.96; Graphify 100%, 0.97; dijkstral 97%, 0.96), so the arms were compared with
`gpt-6-luna` (medium), which explores less: on nine callers/impact/flow questions without a graph
it passed 7 (`gpt-5.6-terra` low 8, `gpt-5.6-luna` low 9, `gpt-6-luna` low 4).

Key facts (60 questions: fastapi, ripgrep, Kubernetes), `gpt-6-luna`:

| arm | pass | fact recall | judge | judge correct | median tokens | fresh tokens, total |
|---|---:|---:|---:|---:|---:|---:|
| no graph | 70% | 0.72 | 0.76 | 60% | 47,323 | 0.73M |
| Graphify | 68% | 0.71 | 0.73 | 52% | 62,010 | 0.79M |
| dijkstral, stage 6 instructions | 72% | 0.76 | 0.80 | 62% | 90,108 | 1.04M |
| dijkstral + question-to-command guidance (`35a20b6`) | 77% | 0.77 | 0.85 | 72% | 86,737 | 0.92M |
| dijkstral + `explain --incoming` (`76b60a4`) | **83%** | **0.80** | 0.82 | 68% | 85,047 | 0.79M |

Fresh tokens are input not served from cache plus output. The no-graph and Graphify arms were
not re-run between dijkstral versions (same model, prompt and questions). What changed: Codex
sessions showed `dijkstral query` answering "who calls X" with a truncated neighbourhood (449
nodes on fastapi), after which the agent re-queried and fell back to `rg`; the installed text now
routes callers to `explain --incoming`, impact to `affected` (which also accepts `path::symbol`
now, `4e106c9`) and broad topics to `query`. Impact questions went from 67% to 83% pass. dijkstral
still spends the most tokens per session, mostly cached input from its extra graph calls.

Memory suites, `gpt-6-luna` (prompt v2):

| suite | arm | judge | judge correct | suite score | median tokens | fresh tokens, total |
|---|---|---:|---:|---:|---:|---:|
| LOCOMO (120) | no graph | 0.47 | 38% | 0.18 | 43,229 | 1.21M |
| LOCOMO (120) | Graphify | 0.53 | 43% | 0.21 | 44,561 | 1.22M |
| LOCOMO (120) | dijkstral | **0.56** | **48%** | 0.20 | 53,221 | 1.07M |
| LongMemEval-S (60) | no graph | **0.80** | **80%** | 0.26 | 36,992 | 0.50M |
| LongMemEval-S (60) | Graphify | 0.73 | 73% | 0.21 | 37,221 | 0.50M |
| LongMemEval-S (60) | dijkstral | 0.73 | 73% | 0.24 | 50,192 | 0.60M |

The first memory run (prompt v1, "answer from these notes") is discarded as a comparison:
`gpt-6-luna` often claimed it could not read the files and asked for them to be pasted (LOCOMO
18 / 33 / 27 of 120 sessions for no graph / Graphify / dijkstral), so its scores measured refusal
rates. Prompt v2 names the shell tools for every arm; refusals fell to 9 / 5 / 6 (LOCOMO) and
0 / 4 / 0 (LongMemEval). On LongMemEval's single ~105k-token chat histories plain `rg` over the
sessions remains the best tool; both graphs tie below it.

Graph builds for the semantic suites (one extraction request per ~60k-token chunk):

| suite | tool | requests | tokens in / out | build time, total |
|---|---|---:|---:|---:|
| LOCOMO (10 histories) | Graphify | 10 | 0.38M / 0.10M | 1,632 s |
| LOCOMO (10 histories) | dijkstral | 10 | 0.39M / 0.17M | 2,718 s |
| LongMemEval-S (60 histories) | Graphify | 120 | 8.33M / 1.40M | 11,823 s |
| LongMemEval-S (60 histories) | dijkstral | 120 | 8.36M / 1.89M | 15,299 s |

dijkstral's extraction writes about a third more output (79–134 nodes per LOCOMO history against
Graphify's 40–88), which is why its semantic builds take longer; build time is model time.
Raw summaries: `results/agent-1792166614.json` (gpt-6.1-sol key facts),
`agent-1792168482.json`, `agent-1792169053.json`, `agent-1792181073.json` (gpt-6-luna key facts),
`agent-1792182437.json` (LOCOMO), `agent-1792183077.json` (LongMemEval); session events stay in
the bench home.

### Stage 7 follow-up: `dijkstral context` and repeated runs — 2026-10-17

Token cost per session is set by round trips (each command re-sends the conversation), not by
output size: every arm returned about 22k characters of tool output per key-fact session, but
dijkstral sessions ran 4.8 commands against 2.5 (no graph). `dijkstral context "<question>"`
(`cli/docs/dijkstral-context.md`) returns a question's evidence in one call — named symbols (or,
for a question naming none, the functions whose code text matches it) with definition lines,
callers with call-site lines and callees; for documents, the matching facts with their source
lines and a search for every occurrence — and the installed assistant text now starts every
question with it. Offline, the share of each key-fact question's facts present in `context`'s
own output rose from 0.43 to 0.53 while tuning on fastapi and ripgrep (Kubernetes, held out:
0.39 → 0.42).

Key facts, `gpt-6-luna` (medium), three runs per arm (mean ± standard deviation):

| arm | pass | judge | judge correct | median tokens | fresh tokens per run | fresh tokens per correct answer |
|---|---:|---:|---:|---:|---:|---:|
| no graph | 60% ± 9 | 0.70 ± 0.06 | 48% ± 11 | 40,876 | 0.58M ± 0.14 | 20.0k |
| Graphify | 74% ± 5 | 0.76 ± 0.03 | 56% ± 4 | 60,209 | 0.70M ± 0.09 | 21.1k |
| dijkstral | **83% ± 3** | **0.85 ± 0.03** | **69% ± 5** | 56,235 | 0.63M ± 0.02 | **15.0k** |

dijkstral's accuracy lead is well outside the run-to-run spread, and it is cheaper than Graphify
per session. Against no graph it spends about 9% more fresh tokens per run but a quarter less per
correct answer: without a graph `gpt-6-luna` often answers without opening a file, which is cheap
and often wrong (its cost and accuracy vary most between runs).

Memory suites with `context` (single run, `gpt-6-luna`, graphs from extraction prompt v2):
LOCOMO judge 0.59 (51% correct) against no graph 0.47 and Graphify 0.53; LongMemEval-S 0.85 (85%)
against 0.80 and 0.73.

### Rust calls through crates, aliases and field types — 2026-10-17

Rust calls now resolve through the module tree (`cli/docs/graphify-format.md` §11.5, Q23):
path-qualified calls across workspace crates (`grep::cli::resolve_binary` through the `grep`
facade's `pub extern crate grep_cli as cli`), crate names from the manifests (`[lib] name`,
`package =` renames, `workspace = true`), `use … as` aliases, `type` aliases, function-body and
inline-module `use`s, `Type::f` for types declared in other files, and `self.a.b.m()` through
the fields' declared types (`should_preprocess` → `Override::matched`). Accuracy against the SCIP
oracle (`accuracy/`, rust-analyzer 2026-09-28, oracles unchanged since 2026-10-13), dijkstral
`66018d0` → `2e44122`, `extract --code-only`, precision / recall in %:

| repo | family | Graphify 0.9.86 | dijkstral before | dijkstral after |
|---|---|---|---|---|
| ripgrep | calls | 80.2 / 40.0 | 80.3 / 40.1 | **84.9 / 55.1** |
| ripgrep | references | 94.0 / 58.4 | 96.6 / 62.5 | **96.7 / 64.6** |
| ripgrep | inherits, imports | 99.1 / 93.4, 76.8 / 61.3 | 100.0 / 93.4, 99.5 / 98.5 | unchanged |
| monet (all languages) | calls | 95.5 / 61.6 | 96.3 / 61.7 | **97.0 / 75.0** |
| monet (Rust callers only) | calls | — | 96.7 / 53.5 | **97.5 / 71.2** |
| monet | references | 91.0 / 64.2 | 98.2 / 67.2 | **98.2 / 69.9** |
| monet | inherits, imports | 91.7 / 84.6, 97.9 / 90.4 | 92.9 / 100.0, 100.0 / 100.0 | unchanged |

ripgrep gains 723 confirmed call pairs and 44 constructor references, monet 416 and 32. The
only new judged false positive is ripgrep's `grep::pcre2::RegexMatcherBuilder::new()` under
`#[cfg(feature = "pcre2")]`, which rust-analyzer indexed with the feature off; two of Graphify's
wrong INFERRED calls are gone (a bare name now goes where the file's import leads). Missed Rust
call pairs by call-site syntax, before → after:

| class | ripgrep | monet |
|---|---|---|
| `recv.m()` on a local or parameter (needs local type inference) | 1,882 → 1,850 | 392 → 368 |
| `Type::f()`, type in another file | 469 → 7 | 168 → 0 |
| `self.field.m()` | 204 → 25 | 69 → 4 |
| `a::b::f()` / `crate::…` / `super::…` | 38 → 4 | 204 → 51 |
| bare `f()` across files, `use … as` aliases | 16 → 0 | 77 → 71 |
| bare same-file `f()` inside macro arguments | 137 → 137 | 180 → 180 |
| `m!()` and calls inside macros | 94 → 94 | — |
| `self.m()` (trait default methods), `Self::f()` in traits | 49 → 49 | 2 → 2 |

Most of what remains under the path and bare rows sits inside `vec![…]`, `assert!(…)` or
`format!(…)` arguments, which neither tool parses (§11.5).

Graph parity with Graphify on ripgrep `.rs` (`parity.py --ext .rs`): calls precision 99.6% →
76.7% (726 deliberate extra calls), recall 100.0% → 99.9% (the two wrong INFERRED calls). Builds
stay byte-identical across `--threads 1, 3, 12` on ripgrep and monet. Build time, Apple M5, 15
interleaved `extract --code-only` runs into a scratch `--out` (`run.py` writes into the corpus
checkout): ripgrep cold median 0.143 s → 0.148 s, warm 0.053 s → 0.053 s; monet (7 runs) cold
0.204 s → 0.221 s, warm 0.059 s → 0.057 s.

### Stage 7 follow-up: context coverage and Rust cross-crate calls — 2026-10-17

`context` was tuned on fastapi and ripgrep with Kubernetes held out, scored offline by the share
of each key-fact question's facts its own output contains: test, example and benchmark callers
summarised in one line, four code-text anchors with shorter later previews, a function's own name
weighted (`_should_embed_body_fields` for "embedded … body"), doubled consonants undone in stems,
and a map of code areas for questions about structure that name no symbol (`e4df121`, `1812f8c`,
`bd6a40f`). With the Rust call resolution above (`2e44122`), offline coverage rose from 0.43 to
0.62 (tuning 0.45 → 0.71, held out 0.39 → 0.45; callers questions 0.68 → 0.89).

Key facts, `gpt-6-luna` (medium), three runs per arm (no graph and Graphify as above):

| arm | pass | judge | judge correct | median tokens | fresh tokens per run | fresh tokens per correct answer |
|---|---:|---:|---:|---:|---:|---:|
| no graph | 60% ± 9 | 0.70 ± 0.06 | 48% ± 11 | 40,876 | 0.58M ± 0.14 | 20.1k |
| Graphify | 74% ± 5 | 0.76 ± 0.03 | 56% ± 4 | 60,209 | 0.70M ± 0.09 | 21.3k |
| dijkstral, previous | 83% ± 3 | 0.85 ± 0.03 | 69% ± 5 | 56,235 | 0.63M ± 0.02 | 15.0k |
| dijkstral, now | **89% ± 1** | **0.87 ± 0.01** | **76% ± 4** | 53,696 | 0.62M ± 0.03 | **13.7k** |

Sessions in which `context`'s first answer already held the key facts cost a median 37k tokens
(below no graph's 41k) against 71k when it was partial, so coverage is what lowers cost per
session; per run dijkstral still spends about 8% more fresh tokens than no graph, which spends
least because it often answers without opening a file.

### Method calls through local type inference — 2026-10-17

`recv.m()` now resolves through the receiver's type wherever a declaration fixes it
(`cli/docs/graphify-format.md` §11.5 Q23 for Rust, §12.2 Q24 for Go): locals bound from
constructors, struct literals, typed `let`/`var`, parameters, declared return types (builder
chains, `x, err := NewT()`), fields, `?`/`unwrap()`/`Some(x)`, `.await`, Go type switches and
assertions, package-level variables and embedded (promoted) methods, within block scopes that
respect shadowing. Trait objects, interfaces, generic parameters, closures and iterator items
resolve nothing. Accuracy against the SCIP oracle (`accuracy/`, oracles unchanged since
2026-10-13), dijkstral `8006022` → this change, `extract --code-only`, precision / recall in %:

| repo | family | Graphify 0.9.86 | dijkstral before | dijkstral after |
|---|---|---|---|---|
| ripgrep | calls | 80.2 / 40.0 | 84.9 / 55.1 | **89.0 / 81.5** |
| monet (all languages) | calls | 95.5 / 61.6 | 97.0 / 75.0 | **97.2 / 81.1** |
| monet (Rust callers only) | calls | — | 97.5 / 71.2 | **97.7 / 79.2** |
| kubernetes | calls | 88.1 / 51.2 | 88.3 / 51.7 | **92.8 / 88.6** |
| all three | references, inherits, imports | | unchanged | unchanged |

Confirmed call pairs gained: ripgrep 1,277, monet 189, kubernetes 6,884; no confirmed pair was
lost. New judged false positives: ripgrep 13, all from `HiArgs::matcher_pcre2`'s builder chain
under `#[cfg(feature = "pcre2")]`, which rust-analyzer indexed with the feature off (the class of
the previous change's single false positive); monet 0; kubernetes 3, `Backoff{..}.Timer()` and
`.DelayFunc()` in `wait/loop_test.go`, correct by the code but absent from scip-go's index at
those sites. Missed call pairs by receiver (classified from the call-site text), before → after:

| class | ripgrep | monet (Rust) | kubernetes |
|---|---|---|---|
| builder and call chains `a.b().c()` | 815 → 126 | 47 → 11 | 3,066 → 123 |
| locals from constructors, literals, calls (`let x = T::new()`, `x := NewT()`, `T{..}`) | 484 → 183 | 166 → 98 | 2,905 → 840 |
| parameters, method receivers, `var x T`, their fields | 281 → 49 | 56 → 9 | 1,892 → 579 |
| patterns, `for`/`range`, closure parameters | 148 → 123 | 60 → 26 | 432 → 336 |
| trait objects, generic parameters (not attempted) | 10 → 10 | 16 → 12 | — |
| `self.m()`, `self.f.m()` (trait defaults and the rest) | 116 → 90 | 10 → 10 | — |
| other receivers (unbound names, indexing) | 68 → 64 | 19 → 19 | 356 → 161 |
| not method syntax (calls inside macros, paths) | 249 → 249 | 305 → 305 | 369 → 97 |
| total | 2,171 → 894 | 679 → 490 | 9,020 → 2,136 |

Most of what remains in ripgrep sits inside `assert!`/`format!` arguments; 1,426 of the 2,136 left
in kubernetes are calls through interface-typed values, whose static target is the interface
method declaration (left alone). Go also gains `pkg.F[T](..)` and `pkg.F(..)` where a method of the
package shares `F`'s name (both dropped by Graphify).

Graph parity with Graphify (`parity.py`): ripgrep `.rs` calls precision 76.7% → 54.4% (1,298 more
deliberate extra calls), recall 99.9% unchanged; kubernetes `.go` calls precision 99.0% → 72.5%
(72,377 more, calls 198,958 → 271,319 of 1.47M links), recall 99.7% unchanged, with 17 of
Graphify's links now reversed (a same-file G5 edge A→B and the inferred B→A collapse to one link,
§8.2). The Go fixture gains five Q24 links (`golden_go.rs` ALLOWED); its communities are compared
without them (NMI 1.000) and its report goldens by section. Builds stay byte-identical across
`--threads 1, 3, 12` on ripgrep, monet and kubernetes.

Build time, Apple M5 shared with other jobs (load average 3–13), interleaved runs into a scratch
`--out`: ripgrep (11 runs) cold median 0.151 s → 0.158 s, warm 0.054 s → 0.054 s; monet (11 runs)
cold 0.165 s → 0.170 s, warm 0.056 s → 0.057 s; kubernetes (9 runs, default threads) cold
13.65 s → 14.31 s (+4.9%; +3.4% in an earlier session of 9 runs, 14.27 s → 14.76 s), CPU
61.7 s → 64.6 s. Per phase (3 runs) the extract phase grows 0.2–0.6 s, resolve 0.15–0.25 s
(inference runs beside the per-file Go pass) and cache-write about 0.05 s; with `--threads 1` the
wall time is unchanged within noise (29.9–31.4 s both). The facts cache grows 423 MB → 471 MB.

### Stage 7 follow-up: flows, faster code search and local type inference — 2026-10-17

`context` additions (`a9153b2`, `c5b8675`, `37ada26`): flow questions ("how does X get from A to
B", "… end up as …") are split into their two ends and answered with the shortest call chain
between them; bare CamelCase names no longer pre-empt the code search; the code is scanned once
per question, in parallel, prefiltered by substring (Kubernetes: 4.1 s → about 0.4 s per call);
structure questions zoom into the best-matching area's subdirectories; and common abbreviations
match (`config`/`cfg`, `err`, `dir`, `memory map` → `mmap`, …). Offline coverage 0.62 → 0.65
(held out 0.45 → 0.47). One candidate — crediting text outside every definition to its file —
lowered held-out flow coverage and was dropped.

Key facts, `gpt-6-luna` (medium), three runs per arm, with the type inference above (`db2c7c4`,
`23b6c8e`, `c1b0d7e`) and these `context` changes:

| arm | pass | judge | judge correct | median tokens | fresh tokens per run | fresh tokens per correct answer |
|---|---:|---:|---:|---:|---:|---:|
| no graph | 60% ± 9 | 0.70 ± 0.06 | 48% ± 11 | 40,876 | 0.58M ± 0.14 | 20.1k |
| Graphify | 74% ± 5 | 0.76 ± 0.03 | 56% ± 4 | 60,209 | 0.70M ± 0.09 | 21.3k |
| dijkstral (`8006022`) | 89% ± 1 | 0.87 ± 0.01 | 76% ± 4 | 53,696 | 0.62M ± 0.03 | 13.7k |
| dijkstral (`1e0e2c4`) | **90% ± 2** | **0.86 ± 0.02** | 72% ± 3 | 54,158 | **0.59M ± 0.04** | **13.7k** |

Accuracy and quality are level with the previous dijkstral (within the spread) and well ahead of
both other arms. Fresh tokens per run are now within no graph's own run-to-run spread (0.59M ±
0.04 against 0.58M ± 0.14); per correct answer dijkstral costs about a third less than either.
The median session stays higher than no graph's because `gpt-6-luna` without a graph often
answers in one turn without opening a file — and is wrong about half the time.

### Stage 7 follow-up: graph-aware ranking and guidance experiments — 2026-10-17

`context`: the code-area map ranks symbols by how many other files use them (`caffd6a`), a
code-text match gains a quarter of its three best-matching callers' and callees' scores
(`df28213`; held out 0.46 → 0.49, held-out architecture 0.13 → 0.33), and definition previews
and call-site lines are shorter (`d6f7c9a`, about 6% less output). Offline coverage 0.66 (tuning
0.75, held out 0.49).

Key facts, `gpt-6-luna` (medium), three runs each (no graph: pass 60% ± 9, judge correct 48% ±
11, fresh tokens per run 0.58M ± 0.14):

| dijkstral | pass | judge | judge correct | median tokens | fresh tokens per run | per correct answer |
|---|---:|---:|---:|---:|---:|---:|
| `2aa0c46` (shipped guidance) | 88% ± 2 | 0.87 ± 0.01 | 74% ± 4 | 55,447 | 0.61M ± 0.02 | 13.6k |
| + "answer straight from it; do not re-check" | 82% ± 2 | 0.83 ± 0.00 | 64% ± 3 | 51,428 | 0.48M ± 0.06 | 12.6k |
| + "one `context` call; check only what is open" | 89% ± 3 | 0.87 ± 0.01 | 73% ± 2 | 53,524 | 0.63M ± 0.05 | 14.3k |

In the shipped runs `context`'s first answer already held the key facts in 102 of 180 sessions,
yet `gpt-6-luna` ran 1.6 more commands in those (rephrased `context` calls, `rg` re-checks). Telling
it to answer straight away cut fresh tokens 21% — below no graph — but also stopped it early on
partial answers (judge correct −10 points); limiting it to one `context` call changed nothing.
Neither guidance change was kept: accuracy comes first, and the remaining cost is the model's own
verification, which only fuller `context` answers remove.

### Stage 7 follow-up: comments as descriptions — 2026-10-17

`context` now counts comment lines and Python docstrings as each function's description, at the
same weight as its code (`d1e37cf`): comments are the code described in the words questions use
("memory-map the file" beside `mmap`). Offline coverage 0.66 → 0.72 (tuning 0.75 → 0.77; held out
0.49 → 0.61, 9 → 13 of 20 questions complete); half and 1.5× weights were both worse.

Key facts, `gpt-6-luna` (medium), three runs:

| arm | pass | judge | judge correct | median tokens | fresh tokens per run | per correct answer |
|---|---:|---:|---:|---:|---:|---:|
| no graph | 60% ± 9 | 0.70 ± 0.06 | 48% ± 11 | 40,876 | 0.58M ± 0.14 | 20.1k |
| Graphify | 74% ± 5 | 0.76 ± 0.03 | 56% ± 4 | 60,209 | 0.70M ± 0.09 | 21.3k |
| dijkstral `2aa0c46` | 88% ± 2 | 0.87 ± 0.01 | 74% ± 4 | 55,447 | 0.61M ± 0.02 | 13.6k |
| dijkstral `d1e37cf` | 84% ± 3 | **0.88 ± 0.02** | **76% ± 4** | 50,855 | **0.58M ± 0.03** | **12.6k** |

Fresh tokens per run now equal no graph's mean (0.575M each) with a seventh of its spread, and
the blind judge rates the answers as good or better; the key-fact pass rate dipped 4 points
(about 1.5 standard deviations).
### Meaning-based retrieval for `context` (research prototype) — 2026-10-17

Embeddings fused with `context`'s code-text search, in a separate `dijkstral-embed` prototype
(branch `feature/dijkstral-context-embeddings-prototype`, not merged; design, every variant and
costs in `cli/docs/dijkstral-context-semantic.md`),
scored offline with `agent/context_eval.py` (tuning fastapi + ripgrep / held-out Kubernetes):

| | tuning | held-out | median chars |
|---|---|---|---|
| lexical, `bef355c` | 0.75 | 0.49 | 2464 |
| `bef355c` + `potion-code-16M-v2`, mix 0.4 | 0.73 | 0.59 | 2377 |
| lexical, `d1e37cf` (comments count) | 0.77 | 0.61 | 2514 |
| `d1e37cf` + `potion-code-16M-v2`, mix 0.4 | 0.77 | 0.64 | 2261 |
| `d1e37cf` + the same, 3 matches after named symbols | 0.81 | 0.65 | 2861 |
| `d1e37cf` + 2 code-text matches after named symbols (no embeddings) | 0.79 | 0.64 | 2994 |
| `d1e37cf` + `bge-small-en-v1.5`, mix 0.4, 3 after named | 0.81 | 0.61 | 2860 |

Counting comments took most of what embeddings added; the rest is mostly extra anchors, which
the code text alone also supplies. Not shipped; `dijkstral` is unchanged.

### Stage 7 follow-up: code-text matches after named symbols — 2026-10-17

A question that names symbols may still be about code it does not name ("which steps run when
`X` changes"), so `context` now adds its two best code-text matches after the named symbols
(`AFTER_NAMED`). Offline coverage 0.72 → 0.74 (tuning 0.77 → 0.79, held out 0.61 → 0.64); median
output 2514 → 2994 characters.

Key facts, `gpt-6-luna` (medium), three runs:

| arm | pass | judge | judge correct | median tokens | fresh tokens per run | per correct answer |
|---|---:|---:|---:|---:|---:|---:|
| no graph | 60% ± 9 | 0.70 ± 0.06 | 48% ± 11 | 40,876 | 0.58M ± 0.14 | 20.1k |
| dijkstral `d1e37cf` | 84% ± 3 | 0.88 ± 0.02 | 76% ± 4 | 50,855 | 0.58M ± 0.03 | 12.6k |
| dijkstral + 2 matches after named | **85% ± 2** | **0.88 ± 0.01** | **78% ± 3** | 53,405 | 0.60M ± 0.06 | 12.7k |

Accuracy edges up and fresh tokens move within the spread (+4% per run, level per correct
answer). Kept: accuracy first at equal cost per correct answer.

### Classifier reranking for `context` (research prototype) — 2026-10-18

The best 12 code-text candidates reordered by a hosted classifier (TypeSafe Jev, one yes/no
question per candidate), through an external reranker hook on branch
`feature/dijkstral-context-rerank-prototype` (not merged; design, every order, costs and
findings in `cli/docs/dijkstral-context-rerank.md`). Offline key-fact coverage against the
shipped lexical order (`782cbbb`):

| | lexical | Jev |
|---|---:|---:|
| benchmark questions, tuning / held out | 0.79 / 0.64 | 0.82 / 0.68 |
| complete answers (of 60) | 41 | 46 |
| agents' own `context` queries from the v12 runs, tuning / held out (121 / 86) | 0.705 / 0.529 | 0.734 / 0.560 |

About $0.0005 and 1.5–2 s per question. Not run as an agent benchmark: Codex's read-only sandbox
blocks the network the reranker needs (as it would for Codex users), and a 3-point coverage gain
needs more runs than three to show. Not shipped; a candidate for the hosted product.

### Graphify 0.9.97 re-check — 2026-10-29

Graphify `graphifyy[leiden,mcp,sql]==0.9.97` beside the 0.9.86 pin on all 19 corpus repos, no
model calls. Both sets of references were frozen the same day; the refrozen 0.9.86 references
reproduce the node and edge counts of the 2026-10-12 baseline. dijkstral at `05b387a` (release
build). Apple M4 Pro, 12 cores, 24 GB (not the M5 of the earlier sections), 12 workers and
threads, 3 runs per scenario (Kubernetes 1), with another agent session open on the machine. Raw:
`results/build-1793245487.json`, `results/build-1793246352.json` (Kubernetes).

**What changed in Graphify.** `parity.py` of the 0.9.97 reference against the 0.9.86 one, recall /
precision %, without `.yaml` and `.yml` files:

| repo | 0.9.86 nodes | 0.9.97 nodes | of them YAML | without YAML: nodes | links |
|---|---:|---:|---:|---|---|
| monet | 3,454 | 3,374 | 0 | 96.8 / 99.1 | 96.2 / 96.2 |
| graphify | 16,790 | 16,988 | 172 | 99.9 / 99.8 | 100.0 / 99.7 |
| fastapi | 9,268 | 11,670 | 2,402 | 99.9 / 99.9 | 99.9 / 99.8 |
| ripgrep | 4,232 | 4,271 | 130 | 96.7 / 98.8 | 95.1 / 95.0 |
| excalidraw | 4,990 | 5,139 | 227 | 98.3 / 99.9 | 97.6 / 99.2 |
| erpnext | 24,633 | 25,443 | 791 | 99.9 / 99.9 | 99.4 / 99.3 |
| kubernetes | 256,424 | 436,879 | 188,804 | 93.9 / 97.0 | 95.9 / 96.1 |
| jq | 1,203 | 1,618 | 404 | 97.2 / 96.3 | 90.4 / 89.7 |
| fmt | 6,292 | 6,361 | 302 | 94.4 / 98.0 | 88.3 / 95.4 |
| serilog | 1,877 | 2,277 | 31 | 100.0 / 83.6 | 95.4 / 80.5 |
| okhttp | 12,437 | 12,589 | 184 | 99.7 / 100.0 | 99.7 / 99.8 |
| munit | 2,080 | 2,169 | 116 | 98.7 / 100.0 | 97.6 / 99.9 |
| alamofire | 3,615 | 3,681 | 123 | 98.3 / 99.9 | 97.5 / 98.4 |
| guzzle | 4,217 | 4,296 | 79 | 100.0 / 100.0 | 93.8 / 75.3 |
| sinatra | 1,343 | 1,564 | 221 | 100.0 / 100.0 | 99.5 / 100.0 |
| dart-http | 16,922 | 17,891 | 960 | 100.0 / 99.9 | 99.1 / 99.8 |
| ecto | 1,954 | 2,015 | 60 | 100.0 / 99.9 | 100.0 / 100.0 |
| penlight | 905 | 946 | 41 | 100.0 / 100.0 | 100.0 / 100.0 |
| afnetworking | 1,204 | 1,271 | 56 | 95.2 / 94.2 | 90.5 / 89.8 |

- **YAML.** 0.9.97 extracts `.yaml` and `.yml` files under `--code-only`. 18 of the 19 repos have
  some, and they are most of the growth: Kubernetes gains 188,804 YAML nodes, fastapi 2,402,
  dart-http 960. dijkstral extracts no YAML.
- **Per language**, read from the differing samples:
  - Go (Kubernetes): about 15.7k of 0.9.86's nodes are gone and 7.4k new definitions appear;
    method ids and the targets of type references and embeds change, moving about 56k links each
    way.
  - C# (serilog): overloaded methods get separate ids (a hash suffix), and interface methods link
    to their implementations (`dispatches_to`): +369 nodes, +1,177 links.
  - PHP (guzzle): +2,976 calls between methods of the same class, −604 calls whose target is a
    class.
  - C and C++: fmt loses 1,414 references to header-qualified type ids; jq's references move
    about 500 each way.
  - Objective-C (AFNetworking): category ids no longer embed the checkout's absolute path (quirk
    OC1, `cli/docs/graphify-format.md` §34; 51 nodes), which dijkstral never reproduced, so
    dijkstral agrees better with 0.9.97 there.
  - Fewer source-less stub nodes: fmt −336, ripgrep −106, monet −92, excalidraw −74.
  - SQL: the `sql` extra gives the graphify repo's seven `.sql` files 34 nodes, which dijkstral
    already matches (§45).
  - Unchanged or nearly: Lua (Penlight), Elixir (ecto, one node), Ruby (sinatra, 10 links).

**dijkstral's parity**, `extract --code-only` against each reference, recall / precision %:

| repo | vs 0.9.86: nodes | links | vs 0.9.97: nodes | links | vs 0.9.97 without YAML: nodes | links |
|---|---|---|---|---|---|---|
| monet | 98.2 / 99.3 | 97.2 / 88.7 | 98.4 / 97.2 | 95.9 / 87.5 | 98.4 / 97.2 | 95.9 / 87.5 |
| graphify | 99.9 / 99.2 | 99.0 / 95.8 | 98.9 / 99.4 | 98.3 / 95.9 | 99.9 / 99.4 | 98.8 / 95.9 |
| fastapi | 98.7 / 97.9 | 93.8 / 94.0 | 78.3 / 97.8 | 83.6 / 93.9 | 98.6 / 97.8 | 93.6 / 93.9 |
| ripgrep | 97.3 / 99.2 | 94.9 / 77.3 | 93.8 / 96.5 | 90.4 / 75.0 | 96.7 / 96.5 | 92.0 / 75.0 |
| excalidraw | 98.2 / 99.9 | 97.3 / 99.5 | 95.3 / 99.8 | 97.7 / 99.5 | 99.7 / 99.8 | 98.8 / 99.5 |
| erpnext | 99.8 / 99.6 | 94.1 / 91.1 | 96.5 / 99.6 | 92.4 / 90.8 | 99.6 / 99.6 | 93.6 / 90.8 |
| kubernetes | 100.0 / 100.0 | 99.9 / 94.9 | 55.1 / 93.9 | 84.1 / 91.4 | 97.0 / 93.9 | 96.3 / 91.4 |
| jq | 100.0 / 100.0 | 100.0 / 99.9 | 72.2 / 97.2 | 82.9 / 90.3 | 96.3 / 97.2 | 89.7 / 90.3 |
| fmt | 98.0 / 98.2 | 96.3 / 95.9 | 91.6 / 92.8 | 90.2 / 85.2 | 96.2 / 92.8 | 92.4 / 85.2 |
| serilog | 100.0 / 100.0 | 99.7 / 99.9 | 82.4 / 100.0 | 79.9 / 95.3 | 83.6 / 100.0 | 80.3 / 95.3 |
| okhttp | 99.9 / 99.6 | 98.7 / 98.9 | 98.4 / 99.3 | 97.9 / 98.6 | 99.9 / 99.3 | 98.6 / 98.6 |
| munit | 99.9 / 99.6 | 99.2 / 98.7 | 94.5 / 98.3 | 95.7 / 96.3 | 99.9 / 98.3 | 99.1 / 96.3 |
| alamofire | 99.9 / 100.0 | 99.1 / 99.2 | 96.4 / 98.3 | 95.8 / 96.9 | 99.8 / 98.3 | 97.8 / 96.9 |
| guzzle | 100.0 / 100.0 | 100.0 / 100.0 | 98.1 / 100.0 | 74.7 / 93.8 | 100.0 / 100.0 | 75.3 / 93.8 |
| sinatra | 100.0 / 100.0 | 100.0 / 100.0 | 85.9 / 100.0 | 90.1 / 99.5 | 100.0 / 100.0 | 100.0 / 99.5 |
| dart-http | 99.7 / 99.8 | 99.3 / 98.9 | 94.3 / 99.7 | 94.1 / 98.0 | 99.6 / 99.7 | 99.1 / 98.0 |
| ecto | 100.0 / 100.0 | 100.0 / 100.0 | 97.0 / 100.0 | 98.4 / 100.0 | 99.9 / 100.0 | 100.0 / 100.0 |
| penlight | 100.0 / 100.0 | 100.0 / 100.0 | 95.7 / 100.0 | 97.9 / 100.0 | 100.0 / 100.0 | 100.0 / 100.0 |
| afnetworking | 95.8 / 95.7 | 92.6 / 92.6 | 94.1 / 99.3 | 94.5 / 97.9 | 98.4 / 99.4 | 97.2 / 97.9 |

Against its pin, the movements since the earlier sections are known ones: link precision in monet,
ripgrep and Kubernetes is lower from the deliberate extra calls of local type inference
(2026-10-17); ripgrep's node recall is 97.3% against 97.9% at stage 1, all of it in stub nodes
(definitions 100%); graphify's and serilog's recall is higher since their stage-4 languages
landed. Against
0.9.97, YAML is most of the gap (Kubernetes node recall
55.1% → 97.0% without it), and the rest is the per-language drift above. Catching up with 0.9.97
means YAML extraction plus the Go, C#, PHP and C/C++ changes, each specified from Graphify's
output as before. Until then the spec, the goldens and the parity claims stay on 0.9.86; the `sql`
extra is the only part taken from 0.9.97 (§45).

**Build time**, median wall s, Graphify 0.9.97 / dijkstral and the speedup, with cold peak
process-tree RSS MB:

| repo | cold s | × | warm s | × | update s | × | cold RSS MB |
|---|---|---|---|---|---|---|---|
| monet | 2.03 / 0.14 | 14× | 1.09 / 0.04 | 30× | 1.96 / 0.10 | 19× | 819 / 61 |
| graphify | 10.36 / 0.47 | 22× | 5.95 / 0.04 | 161× | 8.96 / 0.43 | 21× | 1,104 / 257 |
| fastapi | 7.45 / 0.28 | 27× | 4.72 / 0.04 | 121× | 11.03 / 0.86 | 13× | 852 / 112 |
| ripgrep | 1.63 / 0.14 | 12× | 1.05 / 0.04 | 30× | 1.08 / 0.10 | 11× | 783 / 74 |
| excalidraw | 4.71 / 0.23 | 20× | 1.91 / 0.04 | 52× | 4.38 / 0.15 | 29× | 905 / 105 |
| erpnext | 24.52 / 1.03 | 24× | 8.77 / 0.06 | 159× | 22.53 / 0.44 | 51× | 1,231 / 352 |
| kubernetes | 797.6 / 9.16 | 87× | 178.6 / 0.39 | 453× | 771.3 / 6.36 | 121× | 4,388 / 2,701 |
| jq | 1.38 / 0.11 | 12× | 0.69 / 0.04 | 19× | 0.90 / 0.21 | 4× | 774 / 58 |
| fmt | 2.30 / 0.17 | 13× | 1.30 / 0.04 | 35× | 1.48 / 0.12 | 12× | 845 / 109 |
| serilog | 1.46 / 0.12 | 12× | 0.93 / 0.04 | 26× | 1.03 / 0.09 | 12× | 748 / 58 |
| okhttp | 4.72 / 0.36 | 13× | 3.93 / 0.04 | 89× | 3.96 / 0.24 | 17× | 880 / 210 |
| munit | 1.22 / 0.14 | 8× | 0.74 / 0.04 | 20× | 0.84 / 0.08 | 11× | 736 / 37 |
| alamofire | 2.06 / 0.16 | 13× | 1.14 / 0.04 | 31× | 2.01 / 0.15 | 13× | 850 / 111 |
| guzzle | 2.17 / 0.17 | 13× | 1.16 / 0.03 | 34× | 1.54 / 0.12 | 13× | 889 / 121 |
| sinatra | 0.98 / 0.09 | 10× | 0.54 / 0.03 | 16× | 0.65 / 0.08 | 9× | 736 / 44 |
| dart-http | 16.40 / 0.31 | 53× | 3.43 / 0.04 | 90× | 3.14 / 0.20 | 16× | 812 / 213 |
| ecto | 1.06 / 0.12 | 9× | 0.64 / 0.03 | 19× | 0.70 / 0.07 | 10× | 714 / 54 |
| penlight | 0.83 / 0.08 | 10× | 0.45 / 0.03 | 13× | 0.53 / 0.07 | 8× | 668 / 35 |
| afnetworking | 0.91 / 0.10 | 9× | 0.54 / 0.04 | 14× | 0.59 / 0.09 | 7× | 676 / 52 |

dijkstral is faster (4× to 453×) and lower in peak memory in all 57 scenarios, but the two tools
no longer build the same graph: Graphify's includes the YAML files (Kubernetes 436,879 nodes
against dijkstral's 256,451), so the Kubernetes rows compare unequal work. dijkstral's `update`
indexes markdown structurally, as Graphify's does (fastapi: 26,841 nodes after the update against
9,342 cold), so its fastapi update costs more than its cold build. dijkstral's Kubernetes cold
peak is 2,701 MB against 2,078 MB on 2026-10-13 (10 threads then, and before local type
inference); this run does not separate the two causes.

### dijkstral at Graphify 0.9.97 parity — 2026-10-29

dijkstral's extraction now follows Graphify 0.9.97: `cli/docs/graphify-format.md`, the fixture
goldens (all 34 regenerated) and the code moved together, and every changed rule was found by
running 0.9.86 and 0.9.97 on the same probes. The query, MCP, export, git, memory and multi-repo
surfaces stay specified against 0.9.86; their goldens read frozen 0.9.86 fixture graphs
(`cli/tests/fixtures/graphs-0.9.86/`). The semantic pass and markdown structure follow 0.9.97
(`cli/docs/graphify-semantic.md`), with goldens re-recorded by replaying the recorded model replies
(no model call). What 0.9.97 changed, now reproduced:

- **YAML** under `--code-only` (§46): keys as nodes like JSON, multi-document files, Graphify's
  skip classes and bounds, and its broader sensitive-file filter (all file types).
- **New node fields**: `_callable`/`_callable_class` on Go and Objective-C, `_py_return_type`,
  `_py_property`, `_py_fixture`, `_ts_fields`, `_java_ret`, C# method `arity`/`signature`, `file`
  in every `unresolved_calls` entry, `confidence` on Verilog nodes.
- **Per language**: generic-parameter stubs gone (Go, Kotlin, Scala, Swift, C++); Go aliases and
  generic receivers on the real type, package-node merging and typed receiver calls; C/C++ quoted
  includes by path suffix, `extern "C"` headers as C++, typedefs and aliases as nodes; C# overload
  ids, delegates and `dispatches_to`; PHP `$this->m()`, static and typed calls; Rust aliases and
  `use … as` leaves; Kotlin typealiases; TS/JS typed member calls and `package.json` dependency
  labels; Python property reads; Fortran cross-file calls; Swift `dispatches_to`.

Builds stay byte-identical across thread counts (Kubernetes, fmt and erpnext at 1, 5 and 12
threads). Parity against the 0.9.97 references, `extract --code-only`, all extensions, recall /
precision % (before: `05b387a`, the 0.9.86-era code):

| repo | nodes, before | nodes, now | links, before | links, now |
|---|---|---|---|---|
| monet | 98.4 / 97.2 | 98.7 / 99.7 | 95.9 / 87.5 | 98.4 / 90.2 |
| graphify | 98.9 / 99.4 | 99.9 / 99.4 | 98.3 / 95.9 | 98.8 / 96.0 |
| fastapi | 78.3 / 97.8 | 98.9 / 98.3 | 83.6 / 93.9 | 94.5 / 94.6 |
| ripgrep | 93.8 / 96.5 | 97.0 / 99.5 | 90.4 / 75.0 | 94.6 / 78.3 |
| excalidraw | 95.3 / 99.8 | 99.7 / 99.9 | 97.7 / 99.5 | 99.5 / 99.5 |
| erpnext | 96.5 / 99.6 | 99.8 / 99.6 | 92.4 / 90.8 | 94.2 / 91.0 |
| kubernetes | 55.1 / 93.9 | 100.0 / 99.9 | 84.1 / 91.4 | 99.9 / 95.7 |
| jq | 72.2 / 97.2 | 99.1 / 100.0 | 82.9 / 90.3 | 99.8 / 99.9 |
| fmt | 91.6 / 92.8 | 98.0 / 98.6 | 90.2 / 85.2 | 96.9 / 96.4 |
| serilog | 82.4 / 100.0 | 100.0 / 100.0 | 79.9 / 95.3 | 99.9 / 99.9 |
| okhttp | 98.4 / 99.3 | 99.9 / 99.6 | 97.9 / 98.6 | 98.6 / 99.0 |
| munit | 94.5 / 98.3 | 99.9 / 99.6 | 95.7 / 96.3 | 99.2 / 98.7 |
| alamofire | 96.4 / 98.3 | 99.9 / 100.0 | 95.8 / 96.9 | 99.4 / 99.2 |
| guzzle | 98.1 / 100.0 | 100.0 / 100.0 | 74.7 / 93.8 | 100.0 / 100.0 |
| sinatra | 85.9 / 100.0 | 100.0 / 100.0 | 90.1 / 99.5 | 100.0 / 100.0 |
| dart-http | 94.3 / 99.7 | 99.7 / 99.8 | 94.1 / 98.0 | 99.2 / 98.9 |
| ecto | 97.0 / 100.0 | 100.0 / 100.0 | 98.4 / 100.0 | 100.0 / 100.0 |
| penlight | 95.7 / 100.0 | 100.0 / 100.0 | 97.9 / 100.0 | 100.0 / 100.0 |
| afnetworking | 94.1 / 99.3 | 100.0 / 99.9 | 94.5 / 97.9 | 100.0 / 100.0 |

What remains is listed per language in the spec. The largest parts are deliberate: the
typed-receiver calls of local type inference (Q23, Q24: ripgrep, monet and Kubernetes link
precision), TS/JS calls through `const me = this` (0.9.97 dropped them) and aliased imports. The
rest predates 0.9.97: fastapi's stub-versus-external precedence, erpnext's file-stem remap (Q6),
okhttp's Java-to-Kotlin binding, fmt's error-recovery cases, and Graphify following symlinked
files (Y1).

**Build time**, median wall s, Graphify 0.9.97 / dijkstral and the speedup, with cold peak
process-tree RSS MB and cold node counts. Same machine as the re-check above (Apple M4 Pro, 12
cores); Graphify's rows are the re-check's runs, dijkstral at `f844c0a` (release), 3 runs per
scenario including Kubernetes. Raw: `results/build-1793258302.json`.

| repo | cold s | × | warm s | × | update s | × | cold RSS MB | nodes |
|---|---|---|---|---|---|---|---|---|
| monet | 2.03 / 0.14 | 14× | 1.09 / 0.04 | 30× | 1.96 / 0.11 | 18× | 819 / 92 | 3,374 / 3,339 |
| graphify | 10.36 / 0.47 | 22× | 5.95 / 0.04 | 153× | 8.96 / 0.43 | 21× | 1,104 / 268 | 16,988 / 17,071 |
| fastapi | 7.45 / 0.30 | 25× | 4.72 / 0.04 | 110× | 11.03 / 0.88 | 12× | 852 / 139 | 11,670 / 11,747 |
| ripgrep | 1.63 / 0.15 | 11× | 1.05 / 0.04 | 29× | 1.08 / 0.11 | 10× | 783 / 102 | 4,271 / 4,161 |
| excalidraw | 4.71 / 0.24 | 20× | 1.91 / 0.04 | 50× | 4.38 / 0.15 | 29× | 905 / 96 | 5,139 / 5,129 |
| erpnext | 24.52 / 0.97 | 25× | 8.77 / 0.06 | 159× | 22.53 / 0.46 | 49× | 1,231 / 364 | 25,443 / 25,482 |
| kubernetes | 797.6 / 11.86 | 67× | 178.6 / 0.36 | 496× | 771.3 / 8.23 | 94× | 4,388 / 3,460 | 436,879 / 437,145 |
| jq | 1.38 / 0.12 | 12× | 0.69 / 0.04 | 17× | 0.90 / 0.21 | 4× | 774 / 62 | 1,618 / 1,603 |
| fmt | 2.30 / 0.18 | 13× | 1.30 / 0.04 | 35× | 1.48 / 0.12 | 12× | 845 / 109 | 6,361 / 6,326 |
| serilog | 1.46 / 0.13 | 12× | 0.93 / 0.04 | 25× | 1.03 / 0.09 | 11× | 748 / 60 | 2,277 / 2,277 |
| okhttp | 4.72 / 0.36 | 13× | 3.93 / 0.04 | 91× | 3.96 / 0.24 | 16× | 880 / 202 | 12,589 / 12,627 |
| munit | 1.22 / 0.15 | 8× | 0.74 / 0.04 | 20× | 0.84 / 0.08 | 10× | 736 / 65 | 2,169 / 2,175 |
| alamofire | 2.06 / 0.17 | 12× | 1.14 / 0.04 | 31× | 2.01 / 0.16 | 13× | 850 / 100 | 3,681 / 3,677 |
| guzzle | 2.17 / 0.19 | 12× | 1.16 / 0.04 | 32× | 1.54 / 0.13 | 12× | 889 / 126 | 4,296 / 4,295 |
| sinatra | 0.98 / 0.10 | 10× | 0.54 / 0.04 | 15× | 0.65 / 0.08 | 8× | 736 / 47 | 1,564 / 1,564 |
| dart-http | 16.40 / 0.32 | 51× | 3.43 / 0.04 | 88× | 3.14 / 0.21 | 15× | 812 / 221 | 17,891 / 17,876 |
| ecto | 1.06 / 0.12 | 9× | 0.64 / 0.04 | 18× | 0.70 / 0.07 | 9× | 714 / 55 | 2,015 / 2,014 |
| penlight | 0.83 / 0.08 | 10× | 0.45 / 0.03 | 13× | 0.53 / 0.07 | 8× | 668 / 37 | 946 / 946 |
| afnetworking | 0.91 / 0.10 | 9× | 0.54 / 0.04 | 15× | 0.59 / 0.09 | 7× | 676 / 56 | 1,271 / 1,272 |

dijkstral is faster (4× to 496×) and lower in peak memory in all 57 scenarios, and the two tools
now build graphs of the same size, so the Kubernetes rows compare equal work. Indexing Kubernetes'
YAML (180k more nodes) took dijkstral's cold build from 9.16 s and 2,701 MB to 11.86 s and
3,460 MB; the time goes to clustering, writing and caching the larger graph, not to extraction.

### Agent outcome at Graphify 0.9.97 parity — 2026-10-29

The key-fact set (60 questions: fastapi, ripgrep, Kubernetes) rerun with both graphs rebuilt:
Graphify 0.9.97 against dijkstral at `f844c0a` (the release build of the timing table above), so
the Kubernetes graphs of both tools now include its YAML. `GRAPHIFY_VERSION=0.9.97`, `gpt-6-luna`
(medium) answers, three runs per arm; the judge is `gpt-6-luna` (medium) too, codex-cli 0.165.0.

| arm | pass | fact recall | judge | judge correct | median tokens | fresh tokens per run | per correct answer |
|---|---:|---:|---:|---:|---:|---:|---:|
| no graph | 64% ± 4 | 0.68 ± 0.04 | 0.77 ± 0.03 | 66% ± 3 | 41,844 | 0.78M ± 0.08 | 19.9k |
| Graphify 0.9.97 | 81% ± 1 | 0.78 ± 0.01 | 0.86 ± 0.01 | 75% ± 0 | 71,339 | 1.13M ± 0.07 | 25.1k |
| dijkstral | **84% ± 2** | **0.85 ± 0.02** | **0.92 ± 0.01** | **87% ± 3** | 52,534 | 0.80M ± 0.09 | **15.4k** |

Mean ± standard deviation over the three runs; median tokens over all 180 sessions of an arm; per
correct answer = fresh tokens of the three runs / judged-correct answers of the three runs. By
question kind and by corpus (three runs pooled; pass / judge correct, then median tokens or fresh
tokens per correct answer):

| kind | no graph | Graphify 0.9.97 | dijkstral |
|---|---|---|---|
| locate (45) | 62% / 60%, 47,026 | 76% / 71%, 75,688 | **91% / 93%**, 54,454 |
| callers (45) | 47% / 56%, 26,090 | 71% / 56%, 54,843 | **78% / 82%**, 25,385 |
| impact (36) | 75% / 83%, 40,585 | 78% / 78%, 71,921 | **81% / 86%**, 43,364 |
| flow (27) | 52% / 59%, 63,852 | **96% / 93%**, 120,963 | 81% / 89%, 87,758 |
| architecture (27) | 93% / 74%, 72,506 | **96% / 93%**, 96,044 | 89% / 85%, 57,948 |

| corpus | no graph | Graphify 0.9.97 | dijkstral |
|---|---|---|---|
| fastapi | 65% / 63%, 16.0k | 92% / 85%, 16.7k | **95% / 93%, 11.0k** |
| ripgrep | 67% / 63%, 22.0k | 78% / 73%, 24.1k | **83% / 83%, 16.9k** |
| kubernetes | 60% / 70%, 21.5k | 73% / 67%, 37.1k | 73% / **85%, 18.6k** |

dijkstral answers the most questions correctly at the lowest cost per correct answer. Its lead
over Graphify in judged-correct answers (87% against 75%) is well outside the spread; in pass rate
(84% against 81%) it is within it. Graphify 0.9.97 does much better than 0.9.86 did (pass 74% ± 5
in the tables above) but spends 1.4× dijkstral's fresh tokens per run (4.1 tool calls per session
against 2.9) and the most per correct answer, more than no graph. It wins the flow and architecture
questions at 1.4–1.7× dijkstral's median tokens; on Kubernetes' pod-scheduling trace
(`kubernetes-k18`, 0 of 3) `context` names `scheduleOnePod`, `schedulingCycle` and `bindingCycle`
but not `findNodesThatFitPod` or `prioritizeNodes`, one call further down. The YAML does not
crowd `context`: no section of its output names a YAML file on any of the 20 Kubernetes questions.

Not comparable with the earlier tables except through pass and fact recall: the judge was
`gpt-6.1-sol` there and is `gpt-6-luna` here (no graph's judged-correct rate is 66% here against
48%, with pass 64% against 60%), and fresh tokens per run rose by about 0.2M for no graph and
dijkstral alike (0.58M → 0.78M, 0.60M → 0.80M), which points at Codex 0.165.0 rather than at
either graph. Raw summaries: `results/agent-1793260984.json`, `agent-1793262342.json`,
`agent-1793263650.json`; session events in `$DIJKSTRAL_BENCH_HOME/agent/sessions-0997-r{1,2,3}/`.

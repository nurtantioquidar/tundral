# Graphify CLI checklist

Every command and flag of `graphify --help` (PyPI `graphifyy` 0.9.86), plus the extra flags its
subcommands print (`install --help`, `save-result --help`) and the environment variables the help
names, mapped to dijkstral. Stage 6's exit criterion is that this list has no gap except the
deferred rows. Spec references: `fmt` = `graphify-format.md`, `sem` = `graphify-semantic.md`,
`qry` = `graphify-query.md`, `exp` = `graphify-exports.md`, `inp` = `graphify-inputs.md`.

Status: **done** (implemented and specified); **D** (this track: inputs, providers and small
gaps, done in the same change as this file); **A** (track A: git hooks, merge driver, watch);
**B** (track B: merge-graphs, global graph, clone, `extract --global/--as`); **C** (track C:
save-result, reflect, prs, diagnose); **deferred** (by decision, see
`../../DIJKSTRAL_DESIGN.md`, "Deferred: assistant installs beyond Claude Code and Codex").

dijkstral's own names: outputs go to `dijkstral-out/` (Graphify's `graphify-out/graph.json` is
read wherever a graph is an input); `graphify-mcp` is `dijkstral mcp`; argument errors exit 2
where Graphify sometimes ignores unknown flags.

## Build

| Graphify | dijkstral | Status | Spec |
|---|---|---|---|
| `extract <path>` | `extract <path>` | done | fmt, sem §2–§9 |
| `extract --code-only` | same | done | fmt §1 |
| `extract --no-cluster` | same (clustered shape without `community`) | done | fmt §2.4 |
| `extract --backend B` (gemini, kimi, claude, openai, deepseek, ollama, bedrock, claude-cli, azure) | `--backend` codex, gemini, kimi, anthropic/claude, openai (also Ollama, llama.cpp, vLLM, LM Studio via `OPENAI_BASE_URL`), deepseek, replay, handoff | done (bedrock, azure, claude-cli: not offered; reach them through an OpenAI-compatible provider) | sem §4, §10 |
| `extract --backend <custom provider>` | same | D | inp §6 |
| `extract --model M` | same | done | sem §4 |
| `extract --mode deep` | same | done | sem §9 |
| `extract --force`, `GRAPHIFY_FORCE=1` | `--force`, `GRAPHIFY_FORCE` | done / D (env) | sem §9.2, inp §5 W4 |
| `extract --max-workers N` | `--max-workers` (alias of `--threads`) | D | inp §5 W2 |
| `extract --token-budget N` | same | done | sem §4.2 |
| `extract --max-concurrency N` | same | done | sem §4.4 |
| `extract --api-timeout S` | same | done | sem §4.4 |
| `extract --out DIR`, `--output DIR` | `--out`, alias `--output` | done / D (alias) | inp §5 W3 |
| `extract --google-workspace` | same (`gws`, or `DIJKSTRAL_GWS`) | D | inp §4 |
| `extract --no-gitignore` | same | D | inp §5 W1 |
| `extract --postgres DSN` | same | D | inp §3 |
| `extract --cargo` | same | D | inp §2 |
| `extract --global`, `--as <tag>` | | B | |
| `OPENAI_BASE_URL`, `OPENAI_MODEL`, `ANTHROPIC_BASE_URL`, `ANTHROPIC_MODEL` | same (and `*_MODEL`/`*_BASE_URL` for the other backends) | done | sem §4 |
| `update <path>` | same | done | sem §3 |
| `update --force` (`GRAPHIFY_FORCE=1`) | accepted; dijkstral has no shrink guard to override | D | inp §5 W4 |
| `update --no-cluster` | same | done | fmt §2.1 |
| `cluster-only <path>`, `--graph` | same | done | fmt §17 |
| `cluster-only --no-label` | same (the default) | D | inp §7.1 |
| `cluster-only --no-viz` | accepted (cluster-only writes no `graph.html`) | D | inp §7.1 |
| `cluster-only --backend`, `--model`, `--max-concurrency`, `--batch-size` | same (with `--backend`: `label --missing-only`) | D | inp §7.1, §8 |
| `label <path>`, `--backend`, `--model`, `--max-concurrency`, `--batch-size` | same | done | sem §10 |
| `label --missing-only` | same | D | inp §7.2 |
| `add <url>`, `--author`, `--contributor`, `--dir` | same | done | sem §11 |
| `check-update <path>` | same | done | sem §9.3 |
| `watch <path>` | | A | |
| `provider` (usage) | same | D | inp §6.1 R2 |
| `provider list` | same | D | inp §6.1 |
| `provider show <name>` | same | D | inp §6.1 |
| `provider add <name> --base-url --default-model --env-key [--pricing-input --pricing-output]` | same | D | inp §6.1 |
| `provider remove <name>` | same | D | inp §6.1 |

## Query

| Graphify | dijkstral | Status | Spec |
|---|---|---|---|
| `query "<q>"`, `--dfs`, `--context C`, `--budget N`, `--graph` | same | done | qry §3 |
| exit codes 0 / 3 ambiguous / 4 no match | same | done | qry §2 |
| `path "A" "B"`, `--graph`, `--directed`, `--undirected` | same | done | qry §5 |
| `explain "X"`, `--all`, `--graph` | same | done | qry §6 |
| `affected "X"`, `--relation R`, `--depth N`, `--graph` | same | done | qry §7 |
| `god-nodes`, `--top N`, `--graph`, `--json` | same | done | qry §8 |
| `benchmark [graph.json]` | same | done | qry §9 |
| `graphify-mcp` (and `GRAPHIFY_API_KEY` for its HTTP transport) | `mcp` (stdio) | done (HTTP transport not offered) | qry §10 |
| `GRAPHIFY_QUERY_LOG_DISABLE`, `GRAPHIFY_MAX_GRAPH_BYTES` | not needed: no query log, no size cap | done | qry §2.3, `query_cmds.rs` |
| `tree`, `--graph`, `--output`, `--root`, `--max-children`, `--top-k-edges`, `--label` | same | done | exp §14 |

## Export

| Graphify | dijkstral | Status | Spec |
|---|---|---|---|
| `export html`, `--graph`, `--labels`, `--node-limit`, `--no-viz` | same | done | exp §2 |
| `export callflow-html [GRAPH\|DIR]`, `--graph`, `--labels`, `--report`, `--sections`, `--output`, `--lang`, `--max-sections`, `--diagram-scale` | same (`--sections` refused: format not established) | done | exp §13 |
| `export obsidian`, `--graph`, `--labels`, `--dir` | same | done | exp §11 |
| `export wiki`, `--graph`, `--labels` | same | done | exp §3 |
| `export svg`, `--graph`, `--labels` | same | done | exp §12 |
| `export graphml`, `--graph` | same | done | exp §4 |
| `export neo4j`, `--graph`, `--push URI`, `--user`, `--password`, `NEO4J_PASSWORD` | same | done | exp §10 |
| `export falkordb`, `--graph`, `--push URI`, `--user`, `--password`, `FALKORDB_PASSWORD` | same | done | exp §10 |

## Multi-repo, memory and PRs

| Graphify | dijkstral | Status | Spec |
|---|---|---|---|
| `merge-graphs <g1> <g2> …`, `--out`, `--previous` | | B | |
| `global add <graph.json>`, `--as` | | B | |
| `global remove <tag>`, `global list`, `global path` | | B | |
| `clone <github-url>`, `--branch`, `--out` | | B | |
| `merge-driver <base> <current> <other>` | | A | |
| `hook install`, `hook uninstall`, `hook status` | | A | |
| `save-result`, `--question`, `--answer`, `--answer-file`, `--type`, `--nodes`, `--outcome`, `--correction`, `--memory-dir` | | C | |
| `reflect`, `--memory-dir`, `--out`, `--graph`, `--analysis`, `--labels`, `--half-life-days`, `--min-corroboration` | | C | |
| `prs` | | C | |
| `diagnose multigraph`, `--graph`, `--json`, `--max-examples`, `--directed`, `--undirected`, `--extract-path` | | C | |

## Assistant integrations

| Graphify | dijkstral | Status | Spec |
|---|---|---|---|
| `install`, `--platform claude`, `--platform codex` | same | done | qry §12.4 |
| `install --project`, `--strict` | same | done | qry §12 |
| `install --platform` windows, codebuddy, opencode, aider, amp, agents, claw, droid, trae, trae-cn, gemini, cursor, antigravity, antigravity-windows, hermes, kiro, pi, devin, kimi, kilo, copilot | | deferred | design doc |
| `uninstall` | same (Claude Code and Codex footprints) | done | qry §12.3 |
| `uninstall --purge` | | deferred | design doc |
| `claude install`, `claude uninstall` | same | done | qry §12.1, §12.3 |
| `codex install`, `codex uninstall` | same | done | qry §12.2, §12.3 |
| `gemini`, `cursor`, `codebuddy`, `opencode`, `kilo`, `aider`, `copilot`, `vscode`, `claw`, `droid`, `trae`, `trae-cn`, `antigravity`, `hermes`, `kiro`, `pi`, `devin` `install`/`uninstall` | | deferred | design doc |
| PreToolUse hook guard | `hook-guard` | done | qry §11 |

## General

| Graphify | dijkstral | Status | Spec |
|---|---|---|---|
| `--help`, `-h` | same (clap; every subcommand has its own `--help`) | done | |
| `--version` | `--version`, `-V` | done | |
| unknown command → exit 1 | exit 2 (clap) | done | |

## Summary

76 rows (a row may cover several flags of one command): done 43; D 16, plus 2 rows "done / D"
(the flag existed, this track added its alias or environment variable); A 3; B 5; C 4; deferred
3 (the 17 per-assistant `install`/`uninstall` commands, the other `install --platform` targets,
and `uninstall --purge`). When tracks A, B and C land, the deferred rows are the only gap, as
`DIJKSTRAL_DESIGN.md` records.

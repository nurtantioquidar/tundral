# harness bench

Comparison and stress tooling for `hx` (this product) against external
harnesses (the real Codex CLI; any other agent binary via `HARNESS_BIN`).
Not a bun workspace on purpose — plain scripts.

## Head-to-head (`bench.ts`)

Runs identical fixture tasks (failing `bun test` suites under `fixtures/`)
through each system in fresh temp checkouts, then verifies with `bun test` and
reports wall-clock, token counters, cache-hit rate, and cost.

```bash
DEEPSEEK_API_KEY=... bun bench.ts                     # hx + real Codex CLI
HARNESS_BIN=/path/to/other-agent bun bench.ts         # add a third harness
```

`--only hx|codex|harness` and `--task <fixture>` narrow the run. The Codex
baseline uses `codex exec --json -s workspace-write` (optional `CODEX_MODEL`);
a third harness binary is driven via `HARNESS_BIN` / `HARNESS_PROVIDER` /
`HARNESS_MODEL`.

**Caveat:** the systems cannot share a model (hx is DeepSeek-native; Codex
runs OpenAI models). Verified-completion and cost are therefore
model-confounded; cache-hit shape, fresh-token volume, and wall-clock are the
comparable signals.

## Full-window compaction exercise (`longrun.ts`)

Fills a pinned 128k window (`--window-tokens 131072`; the model's real window
is 1M, too expensive to fill) by reading large repository files turn after turn
in one resumed session until elision/summarization fire naturally, then probes
continuity ("list every file you read", "what did the first files cover") and
scores recall. Covers what the tiny-window live test cannot: summarizer
narrative quality at ~100k-token scale and cache recovery after compaction.

```bash
DEEPSEEK_API_KEY=... bun longrun.ts [repo-root]   # default root: the monet repo
```

Both scripts write JSON reports to `results/` (gitignored).

# harness-experimental bench

Comparison and stress tooling for `hx` (this product) vs the Codex-built
`products/harness/cli`. Not a bun workspace on purpose — plain scripts.

## Head-to-head (`bench.ts`)

Runs identical fixture tasks (failing `bun test` suites under `fixtures/`)
through both CLIs in fresh temp checkouts, then verifies with `bun test` and
reports wall-clock, token counters, cache-hit rate, and cost.

```bash
DEEPSEEK_API_KEY=... bun bench.ts                     # hx side only
HARNESS_BIN=../../harness/cli/target/debug/harness \
DEEPSEEK_API_KEY=... bun bench.ts                     # both sides
```

`--only hx|harness` and `--task <fixture>` narrow the run. The harness side
uses its default ChatGPT-subscription provider unless `HARNESS_PROVIDER` /
`HARNESS_MODEL` (and the matching API key env var) are set.

The runner also drives the **real Codex CLI** (`codex exec --json -s
workspace-write`) as a production baseline — `--only codex`, optional
`CODEX_MODEL`. Note `products/harness/cli`'s subscription provider is
hardcoded read-only sandbox, so it cannot pass mutation tasks; use its
`openai`/`anthropic` provider with an API key for a fair run.

**Caveat:** the systems cannot share a model (hx is DeepSeek-native; Codex and
harness run OpenAI models). Verified-completion and cost are therefore
model-confounded; cache-hit shape, fresh-token volume, and wall-clock are the
comparable signals.

## Full-window compaction exercise (`longrun.ts`)

Fills hx's real 128k window by reading large repository files turn after turn
in one resumed session until elision/summarization fire naturally, then probes
continuity ("list every file you read", "what did the first files cover") and
scores recall. Covers what the tiny-window live test cannot: summarizer
narrative quality at ~100k-token scale and cache recovery after compaction.

```bash
DEEPSEEK_API_KEY=... bun longrun.ts [repo-root]   # default root: the monet repo
```

Both scripts write JSON reports to `results/` (gitignored).

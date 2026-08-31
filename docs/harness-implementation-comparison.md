# Harness implementation comparison (decided 2026-08-31)

**Decision: hx was adopted as the Harness product.** The former
`products/harness-experimental` tree now lives at `products/harness` (workspace
`@monet/harness-cli`, binary `hx`, sessions under `~/.local/state/hx/`). The
Codex-built implementation was removed from the working tree without ever being
committed; a full source archive is kept at
`~/monet-backups/codex-harness-20260831-2026.tar.gz`. Its best idea — the typed
working-state checkpoint — was ported into hx (with the `update_plan` tool)
before the decision.

The evidence that informed the decision follows, unchanged.

---

Two parallel Rust coding-agent implementations were built to compare approaches:

- **`products/harness/cli`** ("harness", built with Codex): 7 crates, multi-provider
  (OpenAI Responses, Anthropic, Codex-subscription), full-screen TUI + line/JSON/RPC surfaces,
  SQLite/WAL sessions, mechanical structured-checkpoint compaction.
- **`products/harness-experimental/cli`** ("hx", built with Claude Code): 2 crates,
  DeepSeek-native, line REPL with diff-preview approvals, JSONL sessions, two-stage
  cache-aware compaction (mechanical tool-result elision, then LLM summarization restart).

This document records the evidence gathered on 2026-08-30/31. The product decision — which
implementation carries the Harness product forward — is deliberately not made here.

## Empirical results

Benchmark harness: `products/harness-experimental/bench/` (fresh temp checkout per run,
`bun test` verification, identical prompts). Models differ by necessity (hx: deepseek-v4-flash;
codex CLI: subscription gpt-5.x), so completion and cost are model-confounded; wall-clock,
fresh-token volume, and cache shape are the comparable signals.

### Task completion (5 fixtures: 3 simple, 2 multi-file/search-heavy)

| system | verified | avg wall | avg input tokens | avg fresh (uncached) tokens | avg cost |
|---|---|---|---|---|---|
| hx + deepseek-v4-flash | 5/5 | 11.8s | ~16k | ~3.5k | ~$0.0008 |
| codex CLI + gpt-5.x (subscription) | 5/5 | 36.7s | ~107k | ~18k | n/a (quota) |
| harness + subscription provider | 0/3 | — | ~14k | ~1k | n/a |

`harness` 0/3 is structural, not model quality: its subscription provider hardcodes a read-only
sandbox (`codex.rs`: `"sandbox":"read-only"`), so it diagnoses correctly but cannot write. A fair
run needs its `openai`/`anthropic` provider with an API key (none available at time of writing).

### Compaction at scale (hx, pinned 128k window, real repository reading)

- Naive tuning exposed a thrash mode (2–3 summarizes/turn, cache hits pinned at ~2.5%, $0.37);
  fixed with token-budget-capped tails. Post-fix: 1 elision, 0 forced summarizes, 75–87% cache
  hits through the rewrite, 100% file-recall continuity, 5.4× cheaper per file read.
- Even the pathological pre-fix run kept 100% recall through 19 compactions — the
  checkpoint+narrative format preserves continuity well.
- `harness`'s compaction is mechanical-only (typed working-state checkpoint, no LLM pass, no
  elision stage) and its event-count tail can in principle orphan a tool result mid-group; it has
  not been exercised live here.

### Wire/provider validation (hx)

Live against api.deepseek.com: streaming, tool-call assembly, `reasoning_content` replay, cache
counters all confirmed; prefix property held (98%+ hit on request 2). Price cards verified against
the official pricing page 2026-08-31 (peak/off-peak tiers; 1M-token window).

## Architecture trade-offs

| Dimension | harness | hx |
|---|---|---|
| Providers | OpenAI/Anthropic/Codex-subscription; native-compaction hooks | DeepSeek only |
| UI | ratatui TUI + line + JSON + RPC | line REPL + JSON one-shot; diff-preview approvals |
| Sessions | SQLite/WAL, background writer | JSONL event log, byte-exact replay |
| Compaction | mechanical typed checkpoint, event-count tail | elision stage + LLM narrative, token-budget tails, live-tested |
| Token accounting | bytes/4 heuristic over re-serialized events | anchor on real usage + EWMA calibration |
| Cache layout | stable prefix + hash; known cache-buster: per-request re-injected skill context | byte-prefix property enforced by test |
| Approval gating | none observed (subscription path: sandbox-enforced) | deterministic policy + gate, session-scoped allows |
| Size | ~6.7k lines, 7 crates | ~5k lines, 2 crates |

## Worth stealing regardless of the decision

- From harness into hx: typed working-state checkpoint (objective/constraints/pending tasks
  maintained by the loop, richer than transcript scanning); multi-provider seam; RPC surface.
- From hx into harness: tool-result elision stage; anchor+drift token ledger; user-turn-boundary
  tail cuts; the per-request byte-prefix test; diff-preview approvals.

## Open items before a final decision

- Fair harness run with a write-capable provider (needs `OPENAI_API_KEY` or `ANTHROPIC_API_KEY`).
- Real-world dogfooding mileage on hx (installed as `hx`, daily-driver ready).
- A long-horizon task tier where model quality dominates (multi-hour refactors).

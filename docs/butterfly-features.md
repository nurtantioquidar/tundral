# Butterfly feature map

The canonical inventory of what Butterfly (`products/butterfly`) has, is building, and has
deliberately rejected. Update this file in the same change that lands or removes a feature; keep
statuses honest. Positioning: a fast DeepSeek-native, cache-first coding agent CLI
(binary `butterfly`, crates `butterfly-core` / `butterfly` / `butterfly-atlas`).

## Shipped

### Provider and cost
- **DeepSeek streaming provider** — SSE, tool-call delta assembly, `reasoning_content` round-trip,
  retries. Live-validated. `crates/core/src/provider/deepseek.rs`
- **Four-counter usage accounting** — input / cache-hit / cache-miss / output kept distinct end to
  end; surfaced per turn and in `/usage`.
- **Tier-aware price cards** — integer micro-USD, DeepSeek peak/off-peak UTC windows, verified
  against the official pricing page 2026-08-31; cache-blind normalized cost alongside actual.
  `crates/core/src/domain.rs`
- **Session cost caps** — `--max-cost-usd`, hard stop mid-turn.

### Context engine (the core bet)
- **Byte-stable prompt prefix** — frozen system prompt + tool schemas, append-only history,
  dynamic context only in appended messages; enforced by a byte-prefix property test. Changing
  prefix bytes requires operator sign-off (AGENTS.md invariant 9).
- **Anchor+drift token ledger** — no local tokenizer; re-anchors on real `prompt_tokens`,
  EWMA-calibrated bytes/token. `crates/core/src/context/mod.rs`
- **Two-stage compaction** — mechanical tool-result elision first (protected tail, 25% savings
  floor), cache-riding LLM summarization second; token-budget-capped tails (thrash-proof, fixed
  after a live full-window exercise); checkpoints supersede, never stack. Live-validated at 128k
  scale: 100% file recall, 5.4× cost reduction vs pre-fix.
- **Typed working state** — loop-observed files/commands/outcomes + model-declared plan via the
  `update_plan` tool; lives outside the transcript, survives elision/compaction/resume; serialized
  into compaction checkpoints. `crates/core/src/session/working_state.rs`
- **1M-token window default** (verified) with `--window-tokens` / config override.

### Tools and safety
- **Seven tools** — `read_file`, `list_files`, `search_text` (ripgrep library crates, no external
  binary), `edit` (exact-match), `write_file` (atomic), `run_command`, `update_plan`.
  `crates/core/src/tools/`
- **Path confinement** — workspace-root only, no `.git` internals, secret-file patterns blocked,
  realpath symlink-escape re-check.
- **Three-class command policy** — strict-allowlisted commands may auto-run; everything else
  prompts with the reason even in `--auto` (headless `--json` denies); `sudo`/`su`/`doas` never
  run. Argv-only, no shell, credential-stripped environments. `crates/core/src/tools/policy.rs`
- **Approval gates with rich previews** — colorized whole-line diffs for `edit` (with occurrence
  warnings), content head for `write_file`, exact argv for `run_command`; `y`/`a`(lways)/`N` with
  session-scoped allows.

### Sessions
- **JSONL event-log persistence** — append-only, byte-exact replay, 0700/0600 perms, key-based
  redaction of structured payloads; `--resume [id]`, `butterfly sessions`.
  `crates/core/src/session/store.rs`
- **State under `~/.local/state/butterfly/`**.

### CLI / REPL
- **Line-based streaming REPL** — dim reasoning stream, tool-call activity line, per-turn status
  (tokens, cache-hit %, cost, context %). `crates/cli/src/repl.rs`
- **Slash commands** — `/usage`, `/plan`, `/compact [focus]` (optional guidance threaded into the
  summarizer), `/mode`, `/model`, `/help`; backslash multi-line input; Ctrl-C turn cancellation.
- **Mid-turn steering (pi-style)** — a persistent input thread owns stdin; lines typed while a
  turn runs answer a pending approval, or queue and enter the conversation at the next
  provider-call boundary; leftovers become the next turns.
- **HTML session export** — `butterfly export [id] [--out path]`: self-contained transcript with
  reasoning/tool output collapsed and cost totals. `crates/cli/src/export.rs`
- **Layered config** — `~/.config/butterfly/config.toml` ← `<repo>/.butterfly.toml` ← flags.
- **Headless one-shot** — `-p` with `--json` machine-readable report (used by bench).

### Atlas (native code graph)
- **Extraction + incremental index** — tree-sitter definitions/references/imports for
  Rust/Python/TS/TSX/JS; blake3 content-hash refresh with mtime fast path; deterministic lookups.
  `crates/atlas/`
- **`repo_map` + `symbol_search` tools** — personalized-PageRank ranked map (working-state files
  seed the ranking) and exact-name definition/reference lookup; read-only, never gated; the
  session invalidates paths its own edits touch (new symbols findable in the same turn).
- **Size-gated by measurement** — A/B on six fixtures (2026-09-01): both arms 6/6 verified, but
  Atlas-on was slower and costlier on every small repo (worst case 43 turns vs 11). Default:
  enabled only when the repo has ≥100 indexable files; `--atlas` / `--no-atlas` / `.butterfly.toml
  atlas=` force either way; the prompt guidance line only exists when the tools do.
- **Deterministic search fusion** — the model never has to pick the right search: a `search_text`
  for a plain identifier gets Atlas definitions prepended, and a `symbol_search` index miss falls
  back to ripgrep in the same result. Behavior-only (no prompt/schema bytes changed).
- **Large-repo A/B validated** (2026-09-01, 390-file production monorepo, 3 read-only tasks, both
  arms fully correct): orientation — Atlas 10 turns / 28 tool calls vs 15 / 69 without;
  cross-service sweep — 29k vs 44k input tokens (37% cheaper); exact-string lookup — ripgrep wins
  (3 turns / $0.0006 vs 4 / $0.0017). Verdict: an orientation/sweep accelerator, not a universal
  win; prompt guidance routes by task shape (repo_map to orient, symbol_search for unknown
  symbols, search_text for exact known strings). One run per cell; no variance data yet.

### Bench and evaluation
- **Fixture benchmark** — five bun-test tasks (three simple, two multi-file/search-heavy) run
  against fresh checkouts; drives Butterfly and the real Codex CLI (plus any binary via
  `HARNESS_BIN`). `products/butterfly/bench/`
- **Full-window compaction exercise** — `longrun.ts`, pinned 128k window, recall scoring.
- **Live smoke tests** — `BUTTERFLY_LIVE_TEST=1` gated (wire round-trip, two-stage compaction).
- Results to date: 5/5 verified vs Codex CLI's 5/5 at ~3× speed and ~6× fewer fresh tokens
  (model-confounded; see `docs/harness-implementation-comparison.md`).

## In progress

- (nothing — next items come from Planned)

## Planned (ordered)

1. **Streaming command output** — long `run_command`s currently render nothing until they finish.
3. **Git-aware dynamic context** — branch/status in appended messages, never the prefix.
4. **Telemetry alarm** — flag an unexpected cache-miss request (no compaction/elision preceded it)
   as a byte-stability regression.
5. **Distribution** — versioned releases with prebuilt binaries; prerequisite for any public face.
6. **butterfly.monet.sh** — landing page (download, benchmarks, docs) at `products/butterfly/web`
   + `deploy/` route; after distribution exists.
7. **Long-horizon bench tier** — multi-hour refactors, where model quality dominates.
8. **Billing reconciliation** — check tier-priced `/usage` totals against a real DeepSeek invoice.

## Deliberately rejected (with reasons)

- **MCP** — 13.7k–18k prompt tokens of tool definitions against a cache-first design; CLI tools +
  on-demand docs win (pi's argument, adopted).
- **Embeddings/vector index** — churn and infra against 2026 evidence that grep-in-a-loop matches
  embedding retrieval; Atlas is structural instead.
- **LSP/stack-graphs/SCIP for Atlas** — external binaries / archived upstream / batch-oriented;
  contradicts the speed pillar. Heuristic name-matching à la Aider chosen deliberately.
- **YOLO-by-default (no approvals)** — gated diffs are a trust feature, not friction.
- **Multi-provider support** — DeepSeek-native is the identity; revisit only with a product reason.
- **Embedded extension runtime** — wrong cost/benefit for a Rust CLI; skills-style progressive
  disclosure is the extension path if needed.
- **Todo-tool removal (pi's stance)** — our `update_plan` is typed working state feeding
  compaction, not a freeform todo list; kept.

## History

Selected over a parallel Codex-built implementation (evidence:
`docs/harness-implementation-comparison.md`), adopted as the product 2026-08-31, renamed
hx → Butterfly the same day.

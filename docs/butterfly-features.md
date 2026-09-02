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

### CLI / TUI
- **Inline TUI** (2026-09-01) — output streams into native terminal scrollback (no alternate
  screen: copy/paste, scrollback, and tmux keep working); a redrawn-in-place bottom block holds
  the input editor, completion dropdown, and status bar. Custom crossterm editor (rustyline
  removed): multi-line input (Ctrl-J), history, word ops. Non-TTY stdin falls back to a plain
  line loop. Block-width invariant (regression, dogfood 2026-09-01): no composed block line may
  exceed the terminal width or the erase math strands stale frames in scrollback — enforced by
  per-line budgets, a unit test, and an autowrap-off guard around the block draw.
  `crates/cli/src/tui/`
- **Screen takeover with a bottom-pinned input** (dogfood 2026-09-01) — entering `butterfly`
  newline-floods the old shell screen into scrollback (never erased) and pins the input block to
  the bottom edge, Claude Code-style: content flows from the top, the prompt stays at the bottom.
  When content grows enough to meet the block (or on resize / tiny terminals), painting hands off
  one-way to the cursor-relative scrolling model. Exit erases the block and parks the shell
  prompt right after the transcript.
- **@-mentions with fuzzy completion** — typing `@` opens a live dropdown over an ignore-aware
  file/dir walk (nucleo fuzzy matching), session-recency-boosted (files the agent touched float
  up), plus Atlas symbol names in Atlas-enabled repos; directories drill down with `/`. Submitted
  mentions stay path text (path-hint semantics): a system-prompt line (sanctioned prefix change,
  operator-approved 2026-09-01) tells the model to `read_file` them — user messages stay small
  and reads stay elidable. Slash commands complete in the same dropdown.
- **Paste placeholders** — bracketed paste; multi-line or >200-char pastes collapse to
  `[pasted #N: X lines]` in the editor (backspace removes atomically) and expand verbatim on
  submit.
- **Two-line footer + live activity** (Claude Code-style, 2026-09-01) — `model | workspace |
  ctx:% | cost | cache:%` plus a mode line colored by permission mode; Shift+Tab cycles
  ask → auto → read-only (mid-turn presses apply when the turn ends — the running turn keeps the
  mode it started with). While a turn runs, a `✳ spinner · elapsed · ~tokens · tool` line sits
  above the input rule; Esc interrupts.
- **Distinct turn chrome** — submitted prompts echo as subtle elevated bands (Codex-style
  dark-gray, not harsh inverse video; same band styles dropdown selection and the overlay
  footer); agent responses
  carry a `●` block marker with aligned continuation lines; turns close with a dim
  `✳ done in Ns · …` stats line.
- **Markdown rendering, markers hidden, word-wrapped** (Codex-inspired, dogfood 2026-09-01) —
  headings/bold/inline-code render styled with the `##`/`**`/backtick syntax stripped; prose is
  word-wrapped to the terminal with hanging indents (bullets and numbered lists wrap under their
  own text; styles survive row breaks); a spacer line precedes each `●` block; code stays
  verbatim (never word-wrapped) with a 2-column indent.
- **Reasoning hidden by default** (Codex-inspired) — the TUI suppresses the reasoning stream (the
  activity line still shows progress); Ctrl+R toggles it live; config `quiet_reasoning = false`
  opts back in at startup. Hidden reasoning is still recorded for the transcript overlay. `-p`
  and non-TTY keep the old flag semantics.
- **Ctrl+T transcript overlay** — a transient alternate-screen pager over the full session
  (user bars, responses, tool lines, and reasoning even when hidden), scrollable (↑/↓, PgUp/PgDn,
  g/G), dismissed with q/Esc/Ctrl+T; the main screen underneath is untouched, and output that
  streams in while the overlay is open queues and flushes on close. 10k-entry ring buffer. The
  overlay is the one sanctioned alternate-screen use — the main flow never leaves scrollback.
- **Logical transcript + resize re-wrap** (dogfood 2026-09-01) — the transcript ring stores
  logical entries (re-wrappable prose spans, user-bar text, fixed lines), rendered to physical
  rows at display time. On resize — where terminal reflow both invalidates the erase math
  (previously stranding stale block frames, one rule line each) and re-wraps old-width lines
  raggedly — the whole visible screen repaints from the transcript, re-wrapped to the new width,
  with the block re-pinned at the bottom edge. The overlay wraps from the same source instead of
  truncating. Scrollback above the visible screen keeps its original wrap (immutable by design).
- **Arrow-key approvals** — the input block becomes a yes / always / no menu (↑↓ + Enter;
  `y`/`a`/`n`/Esc shortcuts kept), with the colorized diff preview above it.
- **Slash commands** — `/usage`, `/plan`, `/compact [focus]` (optional guidance threaded into the
  summarizer), `/mode`, `/model`, `/help`; Ctrl-C clears the line or cancels the turn.
- **Mid-turn steering (pi-style)** — the input thread owns stdin for the whole session; messages
  typed while a turn runs queue and enter the conversation at the next provider-call boundary;
  leftovers become the next turns.
- **HTML session export** — `butterfly export [id] [--out path]`: self-contained transcript with
  reasoning/tool output collapsed and cost totals. `crates/cli/src/export.rs`
- **Layered config** — `~/.config/butterfly/config.toml` ← `<repo>/.butterfly.toml` ← flags.
- **`/connect` onboarding + credential store** (2026-09-01) — first run with no key anywhere
  opens a guided wizard (provider picker — DeepSeek for now — then a masked key prompt) and saves
  to `credentials.toml` (0600, the one sanctioned persistence spot, AGENTS.md invariant 6);
  after that, sessions just start. `/connect` in-session replaces the stored key. Resolution:
  `DEEPSEEK_API_KEY` env > store > wizard; headless `-p/--json` still fails fast. The key is
  masked while typing and never enters history, transcript, or session logs.
- **Model stickiness + picker** — bare `/model` opens an arrow-key picker over the hardcoded
  model list (display names — DeepSeek V4 Flash / DeepSeek V4 Pro — current model marked;
  `/model <wire-name>` still switches directly). The last-used model persists in
  `~/.local/state/butterfly/state.toml`; new sessions start on it regardless of repo. Explicit
  pins (flag, `model =` in config files) win over the remembered value; wire names stay in
  status and logs.
- **Cursor discipline** — the terminal cursor hides while any selector owns the block (provider
  picker, model picker, approval menu) and returns for the editor and masked key entry.
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
- **Same-model harness A/B vs opencode** (2026-09-01, both on deepseek-v4-flash, six fixtures,
  6/6 verified each): Butterfly 2.3× faster wall-clock (62s vs 141s total), 4.2× less total input
  (106k vs 441k tokens), 3.2× fewer fresh tokens, ~26% cheaper. opencode's cache-hit RATE is
  slightly higher (82–92% vs 77–88%) because its ~8.6k-token baseline prompt re-reads from cache
  every turn — percentage flatters volume; fresh tokens is the honest metric. First
  non-model-confounded harness comparison. One run per cell.

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

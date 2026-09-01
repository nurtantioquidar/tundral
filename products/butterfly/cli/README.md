# butterfly — Harness

A fast interactive coding agent CLI in Rust, DeepSeek-native, built around one idea:
**maximize prompt-cache hits, and when the window fills, compact rarely and chunkily.**

```bash
export DEEPSEEK_API_KEY=...
cargo run -p butterfly                     # REPL in the current repo
cargo run -p butterfly -- --auto -p "fix the failing test"   # one-shot, no prompts
cargo run -p butterfly -- --auto --json -p "..."             # one-shot, machine-readable report
cargo run -p butterfly -- --resume         # resume the latest session
cargo run -p butterfly -- sessions         # list saved sessions
cargo run -p butterfly -- export           # latest session -> shareable HTML
```

`--json` emits one JSON object on stdout (tokens, cache counters, cost,
context utilization, compaction counts) — used by `../bench/` for the
head-to-head against `products/harness/cli` and the full-window compaction
exercise.

## Why it is fast and cheap

- **Byte-stable prompt prefix.** The system prompt (with an AGENTS.md snapshot) and tool schemas
  are frozen at session start; history is append-only. DeepSeek's automatic prefix caching then
  prices most of every request at the ~50× cheaper cache-hit rate. The suite asserts request N's
  serialized messages are a strict byte-prefix of request N+1.
- **Anchor + drift token ledger.** No local tokenizer: the last response's `prompt_tokens` is the
  exact anchor; appended bytes are estimated with an EWMA-calibrated bytes/token ratio that
  re-grounds on every response.
- **Two-stage compaction.** At 80% utilization: first mechanically elide old tool outputs (no LLM,
  protected 40k-token tail, only if ≥25% of the window is reclaimed — one bounded cache miss, not
  a permanent one); if still tight, a summarization restart whose summarizer call rides the warm
  cache, rebuilding history as system + checkpoint + verbatim recent turns.
- **Everything observable.** `/usage` and the per-turn status line report the four token counters,
  cache-hit rate, actual micro-USD cost, and context utilization.

## Safety

Read tools run freely; edits and commands prompt unless `--auto`, and `--read-only` denies them
outright. Approval prompts show **what will actually happen** — a colorized diff for `edit` (with
occurrence warnings), the content head for `write_file`, the exact argv for `run_command` — then
`y`/`a`(lways for session)/`N`. Paths are confined to the workspace root (symlink-escape checked);
commands run as argv vectors from an allowlist with credential-stripped environments — no shell.

## Install and configure

```bash
cargo install --path crates/cli --locked        # installs ~/.cargo/bin/butterfly
```

Recommended shell wrapper (key fetched from 1Password per launch, never stored):

```bash
butterfly() { DEEPSEEK_API_KEY=$(op read "op://<vault>/<item>/password") command butterfly "$@"; }
```

Config is layered: defaults ← `~/.config/butterfly/config.toml` ← `<repo>/.butterfly.toml` ← flags:

```toml
model = "deepseek-v4-flash"   # or deepseek-v4-pro
effort = "high"               # or max
mode = "ask"                  # ask | auto | read-only
max_cost_usd = "0.50"
quiet_reasoning = false
```

Interactive TTY sessions take over the screen Claude Code-style: your shell scrolls into
scrollback (never erased), content flows from the top, and the input stays pinned to the bottom
edge. Sessions get the inline TUI: type `@` for fuzzy file/dir/symbol completion
(mentions stay path text; the agent reads them with `read_file`), large pastes collapse to a
`[pasted #N: X lines]` placeholder that expands on submit, and the input area is anchored by a
rule above and a two-line footer below (`model | workspace | ctx:% | cost | cache:%`, plus a
mode line — `Shift+Tab` cycles ask → auto → read-only). Your prompts echo as highlighted bars;
agent responses carry a `●` marker and render as word-wrapped styled markdown (markers hidden,
lists hang under their own text, code verbatim). The reasoning stream is hidden by default —
`Ctrl-R` toggles it live, and `Ctrl-T` opens a scrollable full-transcript overlay (reasoning
included) that leaves the main screen untouched. Approvals are an arrow-key yes/always/no menu
(`y`/`a`/`n` still work). `Ctrl-J` inserts a newline; `Ctrl-C` clears the line or cancels the
turn; `Esc` interrupts a running turn. Type while a turn runs — messages are delivered at the next turn boundary (pi-style
steering). `/compact [focus]` compacts with optional guidance; `/model` switches models
mid-session, `/mode` switches permissions, `/plan` shows the tracked working state (objective,
tasks, files touched, command outcomes — maintained by the loop and the model's `update_plan`
tool; it survives compaction and resume by construction). Piped (non-TTY) stdin gets a plain
line-based loop with the same commands.

See [AGENTS.md](AGENTS.md) for module layout and invariants.

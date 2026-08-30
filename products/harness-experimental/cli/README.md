# hx — harness-experimental

An experimental fast interactive coding agent CLI in Rust, DeepSeek-native, built around one idea:
**maximize prompt-cache hits, and when the window fills, compact rarely and chunkily.**

```bash
export DEEPSEEK_API_KEY=...
cargo run -p hx                     # REPL in the current repo
cargo run -p hx -- --auto -p "fix the failing test"   # one-shot, no prompts
cargo run -p hx -- --resume         # resume the latest session
cargo run -p hx -- sessions         # list saved sessions
```

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

Read tools run freely; edits and commands prompt (`y`/`a`/`N`) unless `--auto`, and `--read-only`
denies them outright. Paths are confined to the workspace root (symlink-escape checked); commands
run as argv vectors from an allowlist with credential-stripped environments — there is no shell.

See [AGENTS.md](AGENTS.md) for module layout and invariants.

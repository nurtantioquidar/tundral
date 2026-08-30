# Harness-experimental CLI Agent Guide

## Purpose

`hx` is an experimental fast interactive coding agent CLI, written in Rust: a chat REPL that runs
tools inside the user's repository against DeepSeek's Chat Completions API. It is a parallel
implementation to `products/harness/cli` built for comparison; the two share **no code** and must
never import from each other. Design goals: speed (Rust, streaming), Codex-style auto-compaction,
and opencode-style prompt-cache hit rates.

## Ownership boundaries

This module owns its conversation state, tool execution policy, context management, DeepSeek
request semantics, and local session persistence under
`~/.local/state/harness-experimental/sessions/`. It calls DeepSeek directly with a user-supplied
`DEEPSEEK_API_KEY`; it never routes traffic through Inver and never imports source from another
Monet application. It is not wired into the root `check`/`test` chains while the experiment runs.

## Layout

```text
crates/core   # hx-core: UI-free engine
  domain      # messages, 4-counter usage, integer micro-USD money, price cards
  provider    # DeepSeek SSE streaming, tool-call delta assembly; scripted fake for tests
  tools       # registry + security policy (path confinement, argv-only commands)
  context     # prompt layout, anchor+drift token ledger, elision + summarize compaction
  session     # deterministic agent loop, approval gating, JSONL persistence/resume
crates/cli    # hx: rustyline REPL, streaming render, terminal approval prompts
```

## Commands

```bash
cargo test --workspace --locked                       # offline test suite
cargo fmt --check && cargo clippy --workspace --locked --all-targets -- -D warnings
bun run --filter @monet/harness-experimental-cli check  # all of the above
cargo run -p hx -- --help                             # the binary
HX_LIVE_TEST=1 DEEPSEEK_API_KEY=... cargo test -p hx-core --test live_deepseek  # paid smoke test
```

## Invariants

1. **The frozen transcript**: once a message is sent to the provider, its bytes and position never
   change — except during a compaction event, the only operation allowed to rewrite history.
   DeepSeek prefix caching is automatic; byte-stability is the only lever. Nothing dynamic
   (timestamps, git status, counters) may enter the system prompt or be re-rendered into earlier
   history slots.
2. Compaction is rare and chunky: elision must clear the `min_savings_fraction` floor, and
   summarization rides the warm cache (current history + one appended instruction). New checkpoints
   supersede old ones; they never stack.
3. `reasoning_content` is preserved on assistant tool-call messages sent back to the provider
   (absent is serialized as null, never omitted). Post-compaction tails start at a user message,
   keep tool-call/tool-result groups atomic, and retain reasoning on kept tool-call messages.
4. Deterministic code owns budgets, approval policy, tool confinement, and termination. The model
   never judges its own completion and read-only tools never prompt.
5. Tool paths are confined to the workspace root: no absolute paths, no `..`, no `.git` internals,
   no secret-bearing files, realpath re-check after resolution. Commands execute as argv vectors —
   never a shell — from an allowlist, with credential-like environment variables stripped.
6. Never log or persist provider credentials. Structured payloads persisted about config or errors
   pass key-based redaction. Conversation content IS persisted (required for `--resume`), with
   0700/0600 permissions.
7. Provider cache-hit, cache-miss, input, and output token counts remain distinct end to end.
   Money is integer micro-USD; decimal USD is presentation only. Price cards are versioned.
8. Tests use the scripted provider and temporary directories; the deterministic suite must never
   touch the network or a paid model. Live tests are opt-in via `HX_LIVE_TEST=1`.
9. The system prompt and tool schemas are the cross-session cached prefix. Any change to their
   bytes — even a beneficial one — invalidates that prefix once per repo and requires the
   operator's explicit sign-off with the trade-off stated. Never change prefix bytes as a side
   effect of an unrelated fix.

## Verification

Run `bun run --filter @monet/harness-experimental-cli check` (or the cargo commands above) after
every change. Cache-critical changes must keep the byte-prefix property test
(`requests_grow_append_only_between_provider_calls`) and the compaction invariant tests green.

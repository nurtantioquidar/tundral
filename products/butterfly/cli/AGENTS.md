# Butterfly CLI Agent Guide

## Purpose

`butterfly` is the Butterfly product: a fast interactive coding agent CLI in Rust — a chat REPL that runs
tools inside the user's repository against DeepSeek's Chat Completions API. Design pillars: speed
(Rust, streaming, lean prompts), a cache-first prompt layout (opencode-class hit rates), two-stage
auto-compaction with a typed working state, and deterministic safety (approval-gated diffs,
argv-only commands).

It was selected over a parallel implementation after a benchmarked comparison; the evidence and
the losing tree's archive location are recorded in `docs/harness-implementation-comparison.md`.

## Ownership boundaries

This module owns its conversation state, tool execution policy, context management, DeepSeek
request semantics, and local session persistence under `~/.local/state/butterfly/sessions/`. It calls
DeepSeek directly with a user-supplied `DEEPSEEK_API_KEY`; it never routes traffic through Inver
and never imports source from another Monet application.

## Layout

```text
crates/core   # butterfly-core: UI-free engine
  domain      # messages, 4-counter usage, integer micro-USD money, tiered price cards
  provider    # DeepSeek SSE streaming, tool-call delta assembly; scripted fake for tests
  tools       # registry + security policy (path confinement, argv-only commands)
  context     # prompt layout, anchor+drift token ledger, elision + summarize compaction
  session     # deterministic agent loop, approval gating, typed working state,
              # JSONL persistence/resume
crates/cli    # butterfly: rustyline REPL, streaming render, terminal approval prompts
../bench      # fixture benchmark vs external harnesses + full-window compaction exercise
```

## Commands

```bash
cargo test --workspace --locked                       # offline test suite
cargo fmt --check && cargo clippy --workspace --locked --all-targets -- -D warnings
bun run --filter @monet/butterfly-cli check             # all of the above
bun run dev:butterfly                                 # REPL via the repo root
BUTTERFLY_LIVE_TEST=1 DEEPSEEK_API_KEY=... cargo test -p butterfly-core --test live_deepseek  # paid smoke test
```

## Invariants

1. **The frozen transcript**: once a message is sent to the provider, its bytes and position never
   change — except during a compaction event, the only operation allowed to rewrite history.
   DeepSeek prefix caching is automatic; byte-stability is the only lever. Nothing dynamic
   (timestamps, git status, counters) may enter the system prompt or be re-rendered into earlier
   history slots.
2. Compaction is rare and chunky: elision must clear the `min_savings_fraction` floor, and
   summarization rides the warm cache (current history + one appended instruction). New checkpoints
   supersede old ones; they never stack. The checkpoint's structured half is the typed working
   state (loop-observed files/commands + model-declared plan via `update_plan`), which lives
   outside the transcript and survives elision, compaction, and resume by construction.
3. `reasoning_content` is preserved on assistant tool-call messages sent back to the provider
   (absent is serialized as null, never omitted). Post-compaction tails start at a user message,
   keep tool-call/tool-result groups atomic, and retain reasoning on kept tool-call messages.
4. Deterministic code owns budgets, approval policy, tool confinement, and termination. The model
   never judges its own completion and read-only tools never prompt.
5. Tool paths are confined to the workspace root: no absolute paths, no `..`, no `.git` internals,
   no secret-bearing files, realpath re-check after resolution. Commands execute as argv vectors —
   never a shell — with credential-stripped environments. Strictly-allowlisted commands may
   auto-run per the permission mode; anything else requires explicit human approval EVEN in auto
   mode (headless runs deny instead); privilege escalation (sudo/su/doas) never runs.
6. Never log or persist provider credentials. Structured payloads persisted about config or errors
   pass key-based redaction. Conversation content IS persisted (required for `--resume`), with
   0700/0600 permissions.
7. Provider cache-hit, cache-miss, input, and output token counts remain distinct end to end.
   Money is integer micro-USD; decimal USD is presentation only. Price cards are versioned and
   tier-aware (DeepSeek peak/off-peak UTC windows).
8. Tests use the scripted provider and temporary directories; the deterministic suite must never
   touch the network or a paid model. Live tests are opt-in via `BUTTERFLY_LIVE_TEST=1`.
9. The system prompt and tool schemas are the cross-session cached prefix. Any change to their
   bytes — even a beneficial one — invalidates that prefix once per repo and requires the
   operator's explicit sign-off with the trade-off stated. Never change prefix bytes as a side
   effect of an unrelated fix.

## Feature map

`docs/butterfly-features.md` is the canonical inventory of shipped / in-progress / planned /
rejected features. Update it in the same change that lands or removes a feature.

## Verification

Run `bun run --filter @monet/butterfly-cli check` (or the cargo commands above) after every change.
Cache-critical changes must keep the byte-prefix property test
(`requests_grow_append_only_between_provider_calls`) and the compaction invariant tests green.

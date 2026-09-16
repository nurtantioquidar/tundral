# Butterfly CLI Agent Guide

## Purpose

`butterfly` is the Butterfly product: a fast, model-agnostic coding agent CLI in Rust — a chat REPL
that runs tools inside the user's repository against any OpenAI-compatible Chat Completions
endpoint (DeepSeek is the reference direct model; GLM, Luna and Muse Spark are built in; Nimble is one more
provider). Design pillars: speed (Rust, streaming, lean prompts), a cache-first prompt layout
(opencode-class hit rates), two-stage auto-compaction with a typed working state, deterministic
safety (approval-gated diffs, argv-only commands), and deterministic completion (verification and
classified recovery in autonomous modes). `BUTTERFLY_ARCHITECTURE_REVIEW.md` in the product root
records the audit that set this direction.

It was selected over a parallel implementation after a benchmarked comparison; the evidence and
the losing tree's archive location are recorded in `docs/harness-implementation-comparison.md`.

## Ownership boundaries

This module owns its conversation state, tool execution policy, context management, model and
provider descriptors (capabilities, pricing, limits), provider request semantics per dialect,
verification and failure classification, telemetry, and local session persistence under
`~/.local/state/butterfly/sessions/`. It calls providers directly with user-supplied keys (one
environment variable per provider, e.g. `DEEPSEEK_API_KEY`, `ZAI_API_KEY`, `OPENAI_API_KEY`,
`NIMBLE_API_KEY`); it never imports source from another Monet application. Nimble is optional: a
session never needs it, and `products/nimble` never links Butterfly.

## Layout

```text
crates/core   # butterfly-core: UI-free engine
  domain      # messages, 4-counter usage, integer micro-USD money, versioned price cards + tiers
  model       # ModelDescriptor/Capabilities/ProviderDescriptor registry; built-ins + [[models]] config
  provider    # OpenAI-compatible SSE adapter parameterized by Dialect (DeepSeek/GLM/OpenAI/Nimble),
              # OpenAI Responses-API dialect (tools + reasoning, encrypted-reasoning replay);
              # scripted fake for tests
  tools       # registry + security policy (PolicySettings: confined default, sandboxed relaxation),
              # argv-only commands, sandbox-only `shell`
  context     # prompt layout (gated appendices only), anchor+drift token ledger, compaction
  failure     # closed FailureKind vocabulary + rule-based classifiers
  verify      # acceptance/detected/model-declared checks, git-diff check, evidence rendering
  telemetry   # per-session JSONL events + summary; key-based redaction
  session     # deterministic agent loop, run_task (attempt → verify → classify → evidence → retry),
              # approval gating, typed working state, JSONL persistence/resume
crates/cli    # butterfly: inline TUI (crossterm editor, @-mention fuzzy completion,
              # status bar, arrow-key approvals, paste placeholders, markdown render);
              # plain line loop for non-TTY stdin; -p headless path
../bench      # fixture benchmark vs external harnesses + full-window compaction exercise
```

## TUI design

The inline TUI is crossterm-only: it prints into the terminal scrollback, with no alternate screen,
no full-frame diff, and no widget tree. `.claude/skills/tui-design` (skill `tui-design`) carries the
design references — hierarchy, spacing, color restraint, states, keybinding conventions, review
checklist — and its Ratatui guide is background reading for structure only. Adopting Ratatui or the
alternate screen is a product decision, never a side effect of a design task. Visual changes stay in
`crates/cli`, must not alter cached prefix bytes (invariant 9), and must not change what an approval
prompt gates or how a keypress maps to consent (invariants 4-5).

## Commands

```bash
cargo test --workspace --locked                       # offline test suite
cargo fmt --check && cargo clippy --workspace --locked --all-targets -- -D warnings
bun run --filter @monet/butterfly-cli check             # all of the above
bun run dev:butterfly                                 # REPL via the repo root
butterfly models                                      # registry: built-ins + config
BUTTERFLY_LIVE_TEST=1 DEEPSEEK_API_KEY=... cargo test -p butterfly-core --test live_deepseek  # paid smoke test
BUTTERFLY_LIVE_TEST=1 BUTTERFLY_LIVE_MODEL=glm-5.3-flash ZAI_API_KEY=... cargo test -p butterfly-core --test live_deepseek
products/butterfly/bench/harbor/with-keys.sh <cmd>    # keys from 1Password (op) into the env, never printed
```

## Invariants

1. **The frozen transcript**: once a message is sent to the provider, its bytes and position never
   change — except during a compaction event, the only operation allowed to rewrite history.
   Prefix caching on every supported provider is automatic; byte-stability is the only lever.
   Nothing dynamic (timestamps, git status, counters) may enter the system prompt or be
   re-rendered into earlier history slots.
2. Compaction is rare and chunky: elision must clear the `min_savings_fraction` floor, and
   summarization rides the warm cache (current history + one appended instruction). New checkpoints
   supersede old ones; they never stack. The checkpoint's structured half is the typed working
   state (loop-observed files/commands + model-declared plan via `update_plan`), which lives
   outside the transcript and survives elision, compaction, and resume by construction.
3. `reasoning_content` is preserved on assistant tool-call messages sent back to the provider
   according to the model's `ReasoningReplay` capability (DeepSeek: required — absent is
   serialized as null, never omitted). Post-compaction tails start at a user message, keep
   tool-call/tool-result groups atomic, and retain reasoning on kept tool-call messages.
4. Deterministic code owns budgets, approval policy, tool confinement, and termination. The model
   never judges its own completion and read-only tools never prompt. In autonomous modes
   (`-p` with `--verify-cmd`/`--detect-checks`/`--require-diff`) a task is complete only when
   every verification check passes; failures are classified (`failure::FailureKind`) and each retry
   appends new evidence — never the same prompt twice. With `--completion-gate` a reply without
   tool calls is never an ending: the model must call `complete_task` (reviewed once, then
   recorded as a claim; the review is skipped only when the loop itself saw the proof — every
   requirement listed as met and a behavioural check it ran with exit 0 — `--always-review-claim`
   restores it). A claim, a diff, a model-declared check, a self-authored check, or an
   independent verifier's pass never yields `verified: true` or router `task_success: true`; only
   an explicit or detected check does. With `--independent-verifier` an accepted claim is reviewed
   by a fresh session (read-only tools, `report_verdict`, no memory of the work) and its unmet
   requirements become the next attempt's evidence.
5. Tool paths are confined to the workspace root: no absolute paths, no `..`, no `.git` internals,
   no secret-bearing files, realpath re-check after resolution. Commands execute as argv vectors —
   never a shell — with credential-stripped environments. Strictly-allowlisted commands may
   auto-run per the permission mode; anything else requires explicit human approval EVEN in auto
   mode (headless runs deny instead); privilege escalation (sudo/su/doas) never runs.
   `--sandboxed` is the one relaxation: for a disposable container the operator owns it disables
   path confinement and the allowlist and exposes the `shell` tool — sudo/su/doas stay forbidden.
   Never run it on a machine you care about.
6. Never log or persist provider credentials in sessions, transcripts, telemetry, or editor
   history. The ONE sanctioned location is the dedicated credential store written by `/connect`
   (`~/.config/butterfly/credentials.toml` via the platform config dir, 0600 in a 0700 dir, keyed
   by provider id; the provider's `api_key_env` variable always takes precedence when set).
   Structured payloads persisted about config or errors pass key-based redaction. Telemetry
   (`telemetry.jsonl` beside the session log) carries ids, counts, durations, tool names, call
   previews, and failure kinds — never file contents or prompts. Conversation content IS persisted
   (required for `--resume`), with 0700/0600 permissions.
7. Provider cache-hit, cache-miss, input, and output token counts remain distinct end to end.
   Money is integer micro-USD; decimal USD is presentation only. Price cards are versioned and
   tier-aware (DeepSeek peak/off-peak UTC windows) and carry `verified`; an unverified card taints
   every cost it produces (`price_verified: false` in reports) and must never be presented as real
   money.
8. Tests use the scripted provider and temporary directories; the deterministic suite must never
   touch the network or a paid model. Live tests are opt-in via `BUTTERFLY_LIVE_TEST=1`.
9. The system prompt and tool schemas are the cross-session cached prefix. Any change to their
   bytes — even a beneficial one — invalidates that prefix once per repo and requires the
   operator's explicit sign-off with the trade-off stated. Never change prefix bytes as a side
   effect of an unrelated fix. New capabilities extend the prefix only behind an explicit mode
   (Atlas line, sandbox line, `shell` schema, completion line + `complete_task` schema, lean
   line, clarify line + `ask_user` schema; the
   verifier line + `report_verdict` schema exist only in the separate verifier session) and only
   by APPENDING. The CLI turns the lean line on by default since 2026-09-16 (sanctioned,
   measured on the private suite); `--no-lean` reproduces the earlier default bytes.
10. Model-agnosticism is a capability contract, not a lowest common denominator: the loop reads
   `Capabilities` (tools, parallel_tools, reasoning + replay, prompt_cache, limits) and dialects
   own vendor syntax. Adding a model means a descriptor (built-in or `[[models]]`), never a branch
   in the loop.

## Feature map

`docs/butterfly-features.md` is the canonical inventory of shipped / in-progress / planned /
rejected features. Update it in the same change that lands or removes a feature.

## Verification

Run `bun run --filter @monet/butterfly-cli check` (or the cargo commands above) after every change.
Cache-critical changes must keep the byte-prefix property test
(`requests_grow_append_only_between_provider_calls`) and the compaction invariant tests green.

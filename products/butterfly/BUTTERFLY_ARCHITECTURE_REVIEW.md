# Butterfly architecture review

Date: 2026-09-03. Scope: `products/butterfly` as it exists at commit `91323f2` (working tree
clean for this module). Offline suite at baseline: 67 core tests + CLI/atlas tests, all green
(`cargo test --workspace --locked`).

Purpose: decide what Butterfly already is before touching it, name every DeepSeek-specific
assumption, and lay out the path to a model-agnostic harness that still runs DeepSeek as well as it
does today, benchmarks on Terminal-Bench, and co-designs with Nimble without depending on it.

Decision recorded here (2026-09; Nimble moved to its own repository on 2026-10-23): **Nimble is a separate product at `products/nimble/`**, not a workspace
inside Butterfly. The repository rule is that a capability becomes a product when it has its own
audience and brand; Nimble is a router for any client, so it qualifies. Butterfly must never link
against Nimble, and Nimble must never import Butterfly.

---

## 1. Current architecture

Three Rust crates, one Bun bench directory.

```text
crates/core   butterfly-core   5.9k lines   UI-free engine
  domain      messages, 4-counter Usage, integer micro-USD Cost, Model enum, tiered price cards
  provider    Provider trait; DeepSeek SSE adapter; ScriptedProvider for tests
  tools       registry (9 schemas, frozen order), path/command policy, fs/search/command executors
  context     TokenLedger (anchor+drift), elision, LLM summarization, system prompt builder
  session     Agent loop, approval gate, typed WorkingState, JSONL store + replay
crates/atlas  butterfly-atlas  0.9k lines   tree-sitter code graph, PageRank repo map
crates/cli    butterfly        5.0k lines   inline TUI (ui.rs alone is 2.2k), plain REPL, -p/--json
bench/        bench.ts (fixtures vs Codex/opencode), longrun.ts (128k compaction exercise)
```

Dependency direction is strictly downward: `cli → core → atlas`. Core has no UI dependency and
takes `Provider`, `ApprovalGate`, and `StreamSink` as trait objects, so the engine is already
testable offline with the scripted provider.

### 1.1 Traced execution path

```text
user request
  │  Agent::run_turn(input)                                   session/mod.rs:419
  │    push_user → SessionEvent::UserMessage → messages.push
  ▼
context construction
  │  drain steer queue (mid-turn messages become user messages)
  │  budget checks: session cost cap, max_provider_calls_per_input (50)
  │  manage_context(): utilization ≥ 0.80 → plan_elision (protected tail, 25% floor)
  │                    still ≥ 0.80 → compact_now (summarize riding warm prefix + tail rebuild)
  │  complete_once(): ProviderRequest { model, messages.clone(), tools.clone(),
  │                    max_output_tokens (65 536), effort }
  ▼
model
  │  DeepSeekProvider::stream                                 provider/deepseek.rs
  │    request_body(): thinking.enabled, reasoning_effort, stream_options.include_usage
  │    3 attempts, backoff 500ms→2s on 408/409/425/429/5xx and transport errors
  │    400 + "context"+"length" → ContextLengthExceeded → shrink tail, compact, retry once
  │    SSE → TurnAssembler (index-keyed tool-call deltas, reasoning_content, usage)
  ▼
tool calls
  │  record AssistantMessage + UsageRecorded; ledger.observe_usage (re-anchor)
  │  no tool calls → return TurnReport                       ← the ONLY completion signal
  │  else for each call, SEQUENTIALLY:
  ▼
execution
  │  execute_gated()                                         session/mod.rs:551
  │    update_plan → WorkingState (never gated)
  │    repo_map / symbol_search → Atlas (never gated)
  │    run_command → classify_command: Forbidden | NeedsApproval(reason) | AutoEligible
  │    policy_outcome(mode, kind) → Deny | NeedsReview | AutoApprove
  │    NeedsReview → session_approvals scope key → gate.review() → Approve/ApproveForSession/Deny
  │    executor.execute(): path confinement, argv-only spawn, env stripped, 60s timeout
  │    search fusion: Atlas defs prepended to search_text; ripgrep fallback for symbol misses
  │  working_state.observe(); atlas.invalidate(path) on successful edit/write
  │  push_tool_result("ERROR: …" prefix on failure)
  ▼
continuation
  │  loop back to the steer drain / budget check / manage_context / complete_once
  ▼
verification
  │  NONE. The system prompt asks the model to "validate your work" and to never claim a check
  │  passed without tool output. Nothing deterministic enforces it.
  ▼
completion
     model returns a message without tool calls, or: cancel, cost cap, 50-call turn limit,
     or a ContextLengthExceeded retry that fails.
```

Everything above the "verification" row is solid and tested. The last two rows are where Butterfly
is currently a strong chat-with-tools loop rather than a harness that owns its own completion.

## 2. Existing capabilities

| Area | State | Evidence |
|---|---|---|
| Agent loop | Deterministic, append-only, steerable mid-turn | `run_turn`, `steering_messages_enter_history_at_the_turn_boundary` |
| Model/provider abstraction | `Provider` trait exists; one real impl; `Model` is a closed 2-variant enum | `provider/mod.rs`, `domain.rs:106` |
| DeepSeek optimizations | Byte-stable prefix, reasoning replay, 4-counter cache accounting, peak/off-peak pricing, thinking budget headroom | invariants 1, 3, 7, 9 in `AGENTS.md` |
| System prompt | Frozen at session start; core rules + optional Atlas line + workspace name + AGENTS.md snapshot (≤12k chars) | `context/prompt.rs` |
| Context engineering | L0+L1 fused into one system message; L3 (working state) lives outside the transcript and surfaces only at compaction; L2 is implicit in tool results | `build_system_prompt`, `WorkingState::render` |
| Tool execution | 9 tools, JSON-schema validated locally, `deny_unknown_fields`, sequential | `tools/mod.rs` |
| Planning | `update_plan` typed tool (objective, constraints, decisions, tasks) | `working_state.rs` |
| File editing | exact-match `edit` (single occurrence unless `replace_all`), atomic `write_file`, diff preview at approval time | `fs_tools.rs`, `approval_detail` |
| Repository search | ripgrep-library `search_text`, ignore-aware `list_files`, Atlas `repo_map`/`symbol_search` with deterministic fusion | `search.rs`, `atlas/` |
| Verification | absent as a stage | — |
| Retry/recovery | HTTP-level only (3 attempts); one context-length recovery; tool errors fed back as text | `deepseek.rs:280`, `complete_once` |
| Failure handling | unstructured: `ToolOutput.is_error` + `"ERROR: "` prefix; replay re-derives success from that prefix | `store.rs:685` |
| Compaction | two-stage, live-validated at 128k; checkpoint = tracked working state + narrative; never stacks | `context/mod.rs`, `longrun.ts` |
| Token accounting | anchor+drift ledger calibrated on real `prompt_tokens`; no tokenizer | `TokenLedger` |
| Prompt caching | byte-prefix property test; sorted-key JSON; frozen tool order | `requests_grow_append_only_between_provider_calls` |
| Session state | JSONL event log, byte-exact replay, 0700/0600, redaction of structured payloads | `store.rs` |
| Streaming | provider→sink deltas for text/reasoning; tool start/result hooks; command output is not streamed | `StreamSink` |
| Concurrency | none: one provider call at a time, tools sequential, TUI input thread separate | — |
| Telemetry | per-turn `TurnReport`; `UsageRecorded` events; `--json` report; `telemetry.rs` is only redaction | `main.rs:304` |
| Tests | 67 offline core tests incl. prefix property, compaction invariants, policy, replay; 2 opt-in live tests | `cargo test` |
| Benchmark support | fixture bench with `bun test` verification; opencode/Codex comparators; longrun | `bench/` |

## 3. DeepSeek-specific assumptions

Classification: **KEEP** (correct for every target model or belongs inside the DeepSeek adapter),
**GENERALIZE** (move behind a capability/policy so other models can differ), **REMOVE**,
**UNKNOWN** (needs live verification against GLM/Luna before deciding).

| # | Assumption | Where | Class | Action |
|---|---|---|---|---|
| A1 | `Model` is a closed enum `{DeepseekV4Flash, DeepseekV4Pro}` | `domain.rs:106`, CLI picker, config, resume | GENERALIZE | `ModelDescriptor` + registry; config-defined models |
| A2 | Pricing hardcoded per enum variant; peak/off-peak UTC tiers | `domain.rs:170` | KEEP mechanism, GENERALIZE data | `PriceCard` moves into the descriptor; tier schedule optional (DeepSeek keeps it) |
| A3 | Request always sends `thinking: {type: enabled}` and `reasoning_effort` | `deepseek.rs:110` | GENERALIZE | capability `reasoning: {supported, effort_values}`; adapter emits provider syntax |
| A4 | `reasoning_content` echoed on tool-call messages, null never omitted | `deepseek.rs:62`, invariant 3 | KEEP (adapter), GENERALIZE (capability) | `reasoning_replay: Required \| Optional \| Unsupported`; compaction's "retain reasoning" rule stays |
| A5 | Usage parsed from `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens` | `deepseek.rs:262` | GENERALIZE | dialect-specific usage parser; OpenAI/GLM use `prompt_tokens_details.cached_tokens`; miss = input − hit |
| A6 | Prefix cache is automatic with no breakpoints → byte-stability is the only lever | invariant 1, `context/mod.rs` header | KEEP | true for DeepSeek, OpenAI-compatible, GLM; capability `prompt_cache: Automatic \| Explicit \| None` reserved for Anthropic-style APIs (not in scope) |
| A7 | Context window 1 000 000 for every model | `domain.rs:130`, `ContextConfig::default` | GENERALIZE | `context_limit` per descriptor; `ContextConfig` derived from it |
| A8 | Context-length error = HTTP 400 with "context" and "length" in body | `deepseek.rs:274` | GENERALIZE | per-dialect error classifier; keep this as the OpenAI-compatible default |
| A9 | `max_output_tokens` default 65 536 (reasoning shares the budget) | `SessionConfig::default` | GENERALIZE | `max_output_tokens` per descriptor; default clamps to it |
| A10 | Summarizer adds 4 096 tokens headroom for thinking | `context/mod.rs:322` | GENERALIZE | headroom only when `reasoning.supported` |
| A11 | `DEEPSEEK_API_KEY` / `DEEPSEEK_BASE_URL`; credential store keyed `"deepseek"`; `/connect` offers one provider | `main.rs:206`, `connect.rs:15`, `credentials.rs` | GENERALIZE | provider registry with `api_key_env`, `base_url`; store keyed by provider id (existing DeepSeek entries stay valid) |
| A12 | Tool-call deltas assembled by `index` | `TurnAssembler` | KEEP | OpenAI Chat Completions standard; shared by all three targets |
| A13 | `tool_choice: auto`, `stream: true`, `stream_options.include_usage` | `request_body` | KEEP | OpenAI-compatible base; `include_usage` gated by capability for servers that reject it |
| A14 | Tools executed strictly sequentially | `run_turn` | GENERALIZE (P2) | `supports_parallel_tools` + read-only tools may run concurrently; mutating stay serial |
| A15 | `ScriptedProvider` prices with `Model::DeepseekV4Flash` | `scripted.rs:82` | GENERALIZE | takes a descriptor |
| A16 | `SessionStarted.model` is a wire name; resume falls back silently if unknown | `store.rs`, `Agent::resume` | GENERALIZE | persist `provider` + `model` ids; unknown model on resume is an explicit error with a hint |
| A17 | Bench requires `DEEPSEEK_API_KEY`, caps cost at $0.10 | `bench.ts` | GENERALIZE | model matrix from env; per-model caps |
| A18 | Positioning text "DeepSeek-native" in `--about`, `AGENTS.md`, feature map | `main.rs:25`, docs | REMOVE (wording only) | "model-agnostic, cache-first; DeepSeek is the reference direct model" |
| A19 | Effort vocabulary `high \| max` | `ReasoningEffort` | GENERALIZE | descriptor lists accepted values; unknown → reject at config time |
| A20 | Off-peak pricing exists | `PriceTier` | UNKNOWN for GLM/Luna | descriptor `tiers: Option<Schedule>`; GLM/Luna ship flat until verified |
| A21 | Cached-token field shape for Luna (`gpt-5.6-luna`, served OpenAI-compatible via Inver) and GLM | — | UNKNOWN | live smoke test per provider before trusting `cache_hit` counters; fall back to `cached_tokens = 0` with a telemetry flag |
| A22 | Models double-encode argv as a JSON string; handled by `extract_argv` | `tools/mod.rs:274` | KEEP, GENERALIZE the category | this is the first instance of `model_format_failure`; keep the fix, add the classification |
| A23 | Instructions snapshot reads `AGENTS.md` then `CLAUDE.md` | `prompt.rs:860` | KEEP | model-agnostic |
| A24 | Elision placeholder and checkpoint wording are plain prose | `describe_elided`, `summarize` | KEEP | model-agnostic |

Nothing in the DeepSeek adapter is wrong; the problem is that its choices leak upward into
`domain`, `session`, `context`, and the CLI through the closed `Model` enum. The fix is to give the
adapter a descriptor to read from, not to strip the adapter.

## 4. Strengths

1. **Cache-first design enforced by tests**, not by convention. The byte-prefix property test plus
   the frozen tool order make cache locality a regression-tested property. This transfers unchanged
   to any automatic-prefix-cache provider, which is all three initial targets.
2. **Two-stage compaction validated at scale**: elision first, summarization riding the warm cache
   second, tails capped by token budget (thrash fix), checkpoints never stack. 100% file recall at
   128k in the live exercise.
3. **Typed working state outside the transcript** is the right primitive for compaction and for
   the L3 layer the target design wants. It already tracks files read/changed, command outcomes,
   plan, decisions, constraints.
4. **Deterministic safety**: argv-only, path confinement with realpath re-check, credential
   stripping, three-class command policy, approvals forced even in auto mode for non-allowlisted
   binaries, headless denies. This is what lets Butterfly run unattended in a benchmark container
   with a clear story.
5. **Honest cost accounting**: four distinct counters, integer micro-USD, cache-blind normalized
   cost beside actual cost. The bench already reports "fresh tokens" as the honest metric.
6. **Byte-exact session replay** from an append-only JSONL log, including rewrites.
7. **Lean tool surface** (9 schemas) with measured decisions (MCP rejected for prompt size, Atlas
   size-gated after an A/B). Decisions are recorded with evidence.
8. **Bench discipline already exists**: fresh checkouts, external verification with `bun test`,
   comparators (Codex, opencode), same-model A/B.

## 5. Gaps

Ordered by how much they block the Phase 2–3 goals.

1. **No verification stage.** Completion is "the model stopped calling tools". There is no
   deterministic post-condition (tests, compiler, linter, git diff non-empty, acceptance
   command). A wrong-but-confident model ends the turn successfully.
2. **No failure classification or recovery loop.** Failures are strings. The loop cannot tell a
   syntax error from a missing file from a provider timeout, so it cannot choose a recovery, and it
   cannot detect the same edit failing three times.
3. **Closed model enum.** Adding GLM or Luna today means editing `domain.rs`, `main.rs`,
   `repl.rs`, `tui/mod.rs`, `connect.rs`, `config.rs` and every price table. This is the single
   biggest blocker to "Butterfly + any model".
4. **Container/benchmark execution mode is missing.** Terminal-Bench tasks live in `/app`, need
   `apt`, `pip`, `curl`, `bash` pipelines, and absolute paths under `/tmp`, `/etc`, `/usr`. The
   current policy would deny nearly every command headlessly. A sandbox-trusting permission mode
   and a gated shell tool are required, and the shell tool changes prefix bytes (see §9 for how to
   keep the default prefix byte-identical).
5. **Context layering is only partially explicit.** L0 and L1 are fused in one system message
   (fine for caching, but L1 cannot be refreshed per session without breaking the cross-session
   prefix). Working state (L3) is invisible to the model between compactions unless it calls
   `update_plan`. There is no verification state to preserve. Git-aware dynamic context is planned
   but not built.
6. **Telemetry is not a first-class artifact.** There is no per-request record of model, provider,
   latency, retries, cached tokens, compactions, verification outcome. The bench scrapes a final
   JSON line. Nimble feedback (Phase 6) needs this as structured data.
7. **Headless mode is single-turn.** `-p` runs one `run_turn`. Long benchmark tasks need a bounded
   autonomous loop (attempt → verify → recover) with its own turn and time budgets; the 50-call
   limit and 60s default command timeout are tuned for interactive use.
8. **No streaming command output**, so a 10-minute build looks hung and cannot be interrupted
   intelligently.
9. **Sequential tools only.** Read-only fan-out (several `read_file`s) could run concurrently.
10. **Provider retry is blind.** Same request three times, no jitter, no distinction between
    rate-limit and server error, no per-model fallback.
11. **Summarizer failure is fatal** (`compact_now` propagates `ProviderError` as `SessionError`).
12. **Test gaps**: no provider retry/backoff test, no test for headless denial paths, no failure
    injection, no CLI integration test of `-p --json`.

## 6. Technical debt

- `crates/cli/src/tui/ui.rs` at 2 195 lines mixes layout, event handling, and rendering.
- `Agent` is a god object: loop, approval, Atlas fusion, and tool special-casing by string name
  (`"update_plan"`, `"repo_map"`, `"edit" | "write_file"`) live in `session/mod.rs`.
- `store::tool_result_failed` recovers success from the `"ERROR:"` text prefix; a structured
  `ToolResult { ok }` event field would be exact.
- `SessionConfig.model: Model` is an engine type leaking into config, TUI, and persistence.
- `ContextConfig::default()` bakes in the 1M window; `main.rs` overwrites it. Window should be
  derived from the model descriptor in one place.
- `ScriptedProvider` and several tests hardcode `Model::DeepseekV4Flash`.
- `ReasoningEffort` is an enum with exactly DeepSeek's two values.
- `bench.ts` duplicates JSON-report parsing for every comparator; results directory is unversioned.
- Feature map says "Multi-provider support: deliberately rejected". That decision is reversed by
  this review and the feature map must be updated when the abstraction lands.
- The credential store has no notion of provider beyond a string key; fine, but the `/connect`
  wizard hardcodes one entry.

None of this is blocking; it is listed so the refactor removes it rather than routes around it.

## 7. Target architecture

```text
                   ┌────────────────────────────────────────────┐
                   │ crates/cli   TUI · REPL · -p/--json · bench │
                   └───────────────┬────────────────────────────┘
                                   │ SessionConfig { model_id, provider_id, permission, budgets }
┌──────────────────────────────────▼───────────────────────────────────────────┐
│ crates/core                                                                  │
│                                                                              │
│  session::Agent  ── run_turn ──►  loop { context ─► model ─► tools ─► ... }  │
│        │                                                                     │
│        ├─ verify::Verifier      deterministic checks after the model yields  │
│        ├─ recover::Recovery     FailureKind → evidence → next attempt        │
│        ├─ working_state         L3: files, commands, plan, failures, verify  │
│        └─ telemetry::Recorder   per-request + per-turn JSONL                 │
│                                                                              │
│  context::Layers   L0 harness (frozen) · L1 repo (frozen per session)        │
│                    L2 task · L3 working · L4 request  (appended, elidable)   │
│                                                                              │
│  model::Registry   ModelDescriptor { id, provider, capabilities, pricing,    │
│                    limits } from built-ins + config                           │
│                                                                              │
│  provider::Provider (trait)                                                  │
│     └─ openai_compat::Adapter<Dialect>   DeepSeek · GLM · OpenAI/Luna ·      │
│                                          Nimble (dialect adds optional       │
│                                          routing metadata, reads routing     │
│                                          response)                           │
│  tools::Registry   frozen core schemas + gated extensions (shell)            │
└──────────────────────────────────────────────────────────────────────────────┘
```

Rules that do not change: the transcript stays frozen once sent; deterministic code owns
termination; money is micro-USD; read-only tools never prompt; credentials never persist outside
the store.

## 8. Model abstraction recommendation

Introduce `core::model` with:

```rust
pub struct ModelDescriptor {
    pub id: String,             // wire name, e.g. "deepseek-v4-flash", "glm-5-flash"
    pub provider: String,       // registry key, e.g. "deepseek", "zai", "openai", "nimble"
    pub display_name: String,
    pub capabilities: Capabilities,
    pub pricing: Pricing,       // PriceCard in micro-USD per 1M + optional tier schedule
    pub limits: Limits,         // context_tokens, max_output_tokens
}

pub struct Capabilities {
    pub tools: bool,
    pub parallel_tools: bool,
    pub reasoning: Option<Reasoning>,     // { effort_values: Vec<String>, replay: Required|Optional|Unsupported }
    pub structured_output: bool,
    pub prompt_cache: PromptCache,        // Automatic | None
    pub images: bool,
    pub compaction: CompactionPolicy,     // thresholds/keep_last_turns preferences
}

pub struct ProviderDescriptor {
    pub id: String,
    pub dialect: Dialect,                 // DeepSeek | Glm | OpenAi | Nimble
    pub base_url: String,
    pub api_key_env: String,
}
```

- Built-in descriptors ship for DeepSeek V4 Flash/Pro (current values), GLM Flash, and Luna via an
  OpenAI-compatible endpoint. Prices for GLM and Luna are placeholders until verified; the
  descriptor carries `price_version` so the bench can refuse to report cost with unverified cards.
- `[[models]]` and `[[providers]]` tables in `config.toml`/`.butterfly.toml` add or override
  descriptors. This is how "other models" work without a release.
- One `OpenAiCompatAdapter` parameterized by `Dialect`. The dialect owns four hooks: request
  extras (thinking/effort syntax), usage parsing (cache fields), error classification, and
  reasoning replay policy. The DeepSeek dialect keeps every current byte of behavior.
- `Model` enum is replaced by `ModelDescriptor` throughout. `Model::parse` becomes
  `Registry::resolve(id)`. Persistence stores `provider` and `model` ids.
- The agent loop reads capabilities, never dialects: reasoning headroom, effort validation,
  context limits, parallel-tool eligibility all come from `Capabilities`.

Nimble is a provider descriptor with `dialect = Nimble`, so "Butterfly + Nimble" is one config
line and zero loop changes.

## 9. Context and cache strategy

Layering, mapped onto what exists:

| Layer | Content | Where it lives | Stability |
|---|---|---|---|
| L0 harness | core instructions, tool schemas | system message + `tools` array | frozen across sessions of the same version and repo; changing bytes needs operator sign-off (invariant 9) |
| L1 repo | AGENTS.md snapshot, workspace name, Atlas guidance line | tail of the system message | frozen per session |
| L2 task | files, symbols, plan | tool results and `update_plan` | appended, elidable |
| L3 working | files touched, commands, failures, verification state | `WorkingState` outside the transcript; rendered into checkpoints and, new, into a small appended "state note" when it changes materially | appended, superseded by compaction |
| L4 request | the user message | appended | — |

Rules:

1. **Default prefix bytes do not change** in this program of work. Every new tool (shell) and
   every new prompt line is gated behind an explicit mode so a default session produces exactly
   today's bytes. This keeps the cross-session cache for every current user and honors the
   standing instruction to never change prefix bytes as a side effect.
2. Dynamic facts (git status, verification results, failure evidence) enter only as appended
   messages, never re-rendered into earlier slots.
3. Compaction preserves, verbatim from L3: goal, constraints, decisions, files changed, symbols
   touched, tests/commands run with outcomes, known failures with classification, current plan,
   verification state. The narrative covers the rest. Redundant tool output is elided first.
4. Model switches (via Nimble or `/model`) keep the transcript bytes identical; the new model pays
   one cold read. Butterfly may append a one-line "model changed" note so the model knows why
   earlier reasoning style differs, but only when the provider reports a change.
5. Per-model `context_limit` drives `ContextConfig`; thresholds stay proportional.

## 10. Verification and recovery strategy

**Verification** runs after the model yields without tool calls, before the turn is declared
complete, when the session runs in an autonomous mode (`-p`, bench, or `--verify`). Interactive
sessions surface the result instead of looping.

Sources of checks, all deterministic:

- explicit acceptance commands from the caller (`--verify-cmd`, repeatable; the bench passes the
  task's test command when known);
- repo-detected checks (`cargo check`/`cargo test`, `bun test`, `pytest`, `go test`, `tsc`), each
  only when the corresponding manifest exists and the model touched files of that ecosystem;
- `git diff --stat` non-empty when the task implies a change (a "no-op completion" is a failure);
- model-declared checks recorded via `update_plan.verification` (a new optional field) are run,
  never trusted.

**Failure classification** (`FailureKind`): `syntax_failure`, `compile_failure`, `test_failure`,
`missing_context`, `incorrect_assumption`, `tool_failure`, `environment_failure`, `timeout`,
`model_format_failure`, `repeated_patch_failure`, `uncertain_result`. Classification is
rule-based from exit codes and output patterns (compiler/test runner signatures), tool error
variants, provider errors, and repetition detection (same `edit` target + same `old_string` failing
twice → `repeated_patch_failure`).

**Recovery loop** (autonomous modes only):

```text
attempt N
  → run verification → pass: done (record verification state)
  → fail: classify → gather evidence (failing output tail, diff of files changed this attempt,
    relevant file re-read for repeated patch failures)
  → append evidence as a user message ("Verification failed: <kind>. Evidence: …. Do not repeat
    the previous patch verbatim.") → attempt N+1
bounded by --max-attempts (default 3), turn budget, cost cap, wall clock
```

Never re-send the same prompt. Every attempt appends new evidence; the prefix stays warm.

## 11. Nimble integration boundary

Butterfly ↔ Nimble is an optional, versioned extension of the OpenAI-compatible request/response.

- Transport: Nimble is an OpenAI-compatible `POST /v1/chat/completions`. Butterfly uses the same
  adapter with `dialect = Nimble`.
- Request extension (all optional): `session_id`, `task_type`, `task_complexity`, `agent_phase`,
  `attempt`, `previous_failure`, `verification_failed`, `stable_prefix_hash`,
  `estimated_stable_tokens`, `estimated_dynamic_tokens`, `context_version`. Carried as an
  `x-nimble` object in the body under a versioned key and mirrored as headers for proxies that
  strip unknown body fields.
- Response extension: `selected_model`, `model_capabilities`, `model_changed`, `cache_affinity`,
  `estimated_cached_tokens`, `context_limit`, `routing_reason`, returned in the first SSE chunk
  under `x_nimble` and in a header.
- Feedback: `POST /v1/feedback` with `session_id`, `request_id`, `task_success`,
  `verification_result`, `failure_type`, `attempt_count`. Fire-and-forget from Butterfly.
- Butterfly's use of the response is limited to: keeping the prefix stable when affinity is high,
  appending a model-changed note, sizing compaction to the returned context limit, and including
  failure evidence when `model_changed` follows a failed verification.
- Nimble never receives tool schemas' semantics, file paths as instructions, or commands to run;
  it receives the messages array like any provider. Nimble never returns instructions about files,
  commands, tests, or solutions; the adapter ignores any such field.
- Everything works with the extension absent: a plain OpenAI-compatible Nimble request is a valid
  Butterfly request, and Butterfly against a non-Nimble endpoint sends no extension.

The contract text lives at `docs/butterfly-nimble-contract.md` (versioned `v1`), with Rust types
in `butterfly-core::provider::nimble` and TypeScript validators in `@tundral/contracts`.

## 12. Terminal-Bench integration plan

Facts checked 2026-09-03: Harbor 0.22.0 installs with `uvx --from harbor harbor`; Docker 29.7.2 is
present. The public registry lists `terminal-bench@2.0` (89 tasks), `terminal-bench-sample@2.0`
(10 tasks), `terminal-bench-pro@1.0` (200 tasks). No dataset is named "4"; this plan treats
"Terminal-Bench 4" as the current Terminal-Bench track on Harbor and keeps the dataset id
configurable (`TB_DATASET`, default `terminal-bench@2.0`).

Plan:

1. **Harbor adapter** `products/butterfly/bench/harbor/butterfly_agent.py` implementing
   `BaseInstalledAgent` (`install`, `run`, `populate_context_post_run`). `install` uploads a
   statically linked Linux binary from the host (`--ak binary=…`) or downloads a release; `run`
   executes `butterfly -p --json --auto --sandboxed --root /app --model … --max-attempts …` with the
   provider key in `env`, teeing stdout to `/logs/agent/`. `populate_context_post_run` reads the
   telemetry JSONL to fill `n_input_tokens`, `n_cache_tokens`, `n_output_tokens`, `cost_usd`.
2. **Sandboxed permission mode** in Butterfly: inside a disposable container, path confinement and
   the command allowlist are relaxed (`--sandboxed`), a `shell` tool is enabled, and the policy
   still forbids privilege escalation. Prefix bytes for default sessions are untouched.
3. **Dev subset**: `bench/harbor/dev-subset.txt` pins 10–12 task ids (start from
   `terminal-bench-sample@2.0`, add two known-hard ones) for fast iteration; the full run is the
   89-task set.
4. **Matrix runner** `bench/harbor/run-matrix.sh`: `{deepseek-v4-flash, glm-flash, luna}` ×
   `{butterfly, baseline}` where baseline is Harbor's bundled `mini-swe-agent` on the same model,
   giving the "DeepSeek + minimal harness vs DeepSeek + Butterfly" uplift.
5. **Metrics extraction** `bench/harbor/summarize.ts`: pass rate, cost/task, cost/successful task,
   tokens/task, cached-token ratio, runtime, tool calls/task, retries/task, from Harbor's
   `result.json` files joined with Butterfly telemetry.
6. Repeatability: pinned Harbor version, pinned dataset version, pinned binary hash, N attempts per
   task recorded, price version recorded.

## 13. Prioritized implementation roadmap

Complexity: S (<½ day), M (1–2 days), L (3–5 days), XL (>1 week). Priority: P0 unblocks the
program, P1 completes the harness, P2 quality, P3 later.

| # | Item | Pri | Cx | Notes |
|---|---|---|---|---|
| R1 | `ModelDescriptor`, `Capabilities`, `ProviderDescriptor`, registry, config `[[models]]`/`[[providers]]`; replace `Model` enum everywhere; per-provider env keys and credential store keys | P0 | L | Prefix bytes unchanged. Unblocks GLM/Luna/Nimble |
| R2 | `OpenAiCompatAdapter<Dialect>` with DeepSeek/GLM/OpenAI dialects; usage parsers; error classifiers | P0 | M | DeepSeek behavior byte-identical; live smoke test per dialect |
| R3 | `FailureKind` + classification from tool output, provider errors, repetition | P0 | M | Feeds recovery, telemetry, Nimble feedback |
| R4 | Verification stage with acceptance commands, repo-detected checks, git-diff check | P0 | M | Autonomous modes only |
| R5 | Recovery loop: attempt → verify → classify → evidence → retry, bounded | P0 | M | Never repeats the same prompt |
| R6 | Telemetry recorder: per-request/turn JSONL + summary in `--json` | P0 | S | Model, provider, tokens, cached, latency, retries, compactions, verification |
| R7 | Sandboxed mode + gated `shell` tool + relaxed confinement inside containers | P0 | M | Required for Terminal-Bench; gated so defaults keep prefix bytes |
| R8 | Harbor adapter, dev subset, matrix runner, summarizer | P0 | M | Phase 3 |
| R9 | Working-state additions: failures, verification state, symbols; compaction checkpoint includes them | P1 | S | |
| R10 | Compaction thresholds derived from `context_limit`; reasoning headroom by capability | P1 | S | |
| R11 | Model-changed note and Nimble dialect (request/response extension, feedback post) | P1 | M | Phase 6 |
| R12 | Provider retry with jitter, rate-limit vs server distinction, per-provider budgets | P1 | S | |
| R13 | Streaming command output with incremental timeout | P1 | M | Long builds |
| R14 | Concurrent read-only tool execution when `parallel_tools` | P2 | M | Measure before default-on |
| R15 | `/connect` multi-provider wizard, `/model` picker over registry | P2 | S | |
| R16 | Structured `ToolResult.ok` event; drop `"ERROR:"` prefix heuristic in replay | P2 | S | Migration: keep reading old logs |
| R17 | Split `ui.rs`; extract Atlas fusion out of `Agent` | P2 | M | Debt |
| R18 | Summarizer fallback (mechanical checkpoint-only compaction on provider failure) | P2 | S | |
| R19 | Explicit-breakpoint prompt cache support (Anthropic-style) | P3 | M | Only if a provider needs it |
| R20 | Images capability plumbing | P3 | M | No current consumer |

Execution order for this program: R1 → R2 → R3 → R4 → R5 → R6 → R7 → R8, then Nimble design and
v1, then R11 and the combined benchmark.

### Status (2026-09-04)

Implemented in the same change set as this review, all offline-tested (`cargo test`: 94 core +
45 CLI; `bun test`: 21 Nimble + 7 contracts): R1, R2, R3, R4, R5, R6, R7, R8 (tooling; no paid
run yet), R9, R10 (window from descriptor and router facts), R11 (Nimble dialect, metadata,
facts, feedback). Open: R12–R20 and the first Terminal-Bench run. A20/A21 resolved 2026-09-04: GLM is
`glm-5.3-flash` on Z.ai (list $0.15/$0.03/$0.50 per 1M, flat), Luna is `gpt-5.6-luna` on OpenAI
($0.20/$0.02/$1.20, flat below 272K input). OpenAI's Chat Completions endpoint rejects function
tools with any reasoning effort but `none`, so Luna is served through a new OpenAI Responses-API
dialect (R21, done 2026-09-04) with encrypted-reasoning replay; all three dialects pass the live
smoke test. Default
sessions' prefix bytes are unchanged, asserted by tests.

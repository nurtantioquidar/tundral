# pi and oh-my-pi harness study (2026-09-12)

Two TypeScript coding agents read against Butterfly's equivalents. **pi** is `badlogic/pi-mono`
(MIT, `@earendil-works/pi-coding-agent` 0.85.1, commit `71dca87`, 2026-09-11); paths relative to
`packages/`. **oh-my-pi** is `can1357/oh-my-pi` (MIT, `omp` 18.1.18, commit `9f829f4`, 2026-09-12),
a pi fork with an ~80k-line Rust core; repo-relative paths. Both read, not run.

## Turn loop

- **pi** (`agent/src/agent-loop.ts` `runLoop`): sample → run all tool calls (parallel unless a
  tool is `sequential`) → repeat. **A reply with no tool calls ends the turn.** No completion
  gate, step cap, token budget or nudge; only a `shouldStopAfterTurn` hook and steering queues. A
  `length` stop with tool calls fails each call ("hit the output token limit … Re-issue the tool
  call with complete arguments") instead of executing salvaged JSON. `core/agent-session.ts`
  (`_handlePostAgentRun`) re-enters only for a retryable provider error or overflow compaction;
  print mode (`modes/print-mode.ts`) ends with the loop, exit 1 on error/abort.
- **oh-my-pi** (`packages/agent/src/agent-loop.ts`, 3.1k lines): same skeleton plus a wall-clock
  `deadline` (`--max-time`); no step cap. "Keep going" lives in `packages/coding-agent/src/
  session/turn-recovery.ts`: an empty stop gets a developer message "Stopped without actionable
  output; task incomplete. Continue…" (`EMPTY_STOP_MAX_RETRIES 3`);
  an *unexpected* stop ("I'll fix that now." then silence) gets "You said you would continue …
  MUST call the next concrete tool now" (3×), gated by `features.unexpectedStopDetection`
  (default `mechanical` = thinking-only stops; `smart` asks a tiny classifier, 4 s timeout);
  text-formatted function calls get 3 retries; a loop guard injects `<system-interrupt>` after 5
  identical calls (`model.toolCallLoopGuard.threshold`). Otherwise termination is prompt-carried
  ("NEVER yield while actionable work remains"). Neither tool has a verification gate.

## Provider errors

- **pi**: SDK layer `ai/src/utils/provider-retry.ts` (0 retries by default, honours
  `retry-after`, refuses server delays > 60 s); agent layer `ai/src/utils/retry.ts`: regex
  classifier (429/5xx/overloaded/timeout/socket/"stream ended before" retryable;
  `insufficient_quota`/billing fatal), `baseDelayMs·2^(n-1)` capped 60 s, defaults `maxRetries
  3`, `baseDelayMs 2000` (`core/settings-manager.ts:927`); HTTP idle 300 s
  (`core/http-dispatcher.ts:4`). Overflow is never retried: `ai/src/utils/overflow.ts` has ~25
  provider regexes plus two silent cases — **z.ai returns success with `usage.input >
  contextWindow`**, Xiaomi `length` with zero output — then one compact-and-retry (`:2194`).
- **oh-my-pi**: `retry.maxRetries` default **10**, `baseDelayMs 500`, backoff capped 8 s with 25 %
  downward jitter (`session/retry-fallback-chains.ts:71`). Flag-based classifier
  (`packages/ai/src/error/`); no retry once visible text or a tool call streamed.
  `retry.fallbackChains` switches model/provider with cooldown revert. Stream idle and
  first-event timeouts 300 s (`packages/ai/src/utils/idle-iterator.ts`); empty completions
  retried twice at the transport. Overflow and `length` go to compaction.

## Tools

- **pi** (`coding-agent/src/core/tools/index.ts`): `read bash edit write grep find ls
  powershell`; default set is the first four. `read {path, offset?, limit?}` head-truncates at
  2000 lines / 50 KB. `edit {path, edits:[{oldText,newText}]}` (`tools/edit-diff.ts`): exact
  `indexOf`, then one fuzzy pass in a normalised space (NFKC, per-line rstrip, smart quotes,
  Unicode dashes, NBSP-family → ASCII); match must be unique; untouched lines copied back
  byte-for-byte. `prepareArguments` accepts `edits` as a JSON string or single object ("Opus
  4.6, GLM-5.1 send edits as a JSON string"). Failure: "Could not find the exact text in {path}.
  The old text must match exactly…" / "Found N occurrences…" — **no closest-match echo**.
  `bash {command, timeout?}` (`tools/bash.ts`): blocking, **no default timeout**, tail of 2000
  lines / 50 KB (`tools/truncate.ts`), full output to a temp file named in the footer; non-zero
  exit is an error result with the output. No background sessions.
- **oh-my-pi** (`packages/coding-agent/src/tools/builtin-names.ts`): 28 built-ins — `read bash
  edit ast_grep ast_edit ask debug eval github glob grep lsp checkpoint rewind context_notes
  new_context security_scan task hub todo web_search write memory_edit retain recall reflect
  learn manage_skill` — plus hidden `yield goal think`; `grep`/`glob`/`bash` are in-process Rust.
  **Edit** has five engines (`crates/pi-edit/src/engine.rs`): `replace`, `patch`, `apply_patch`,
  `hashline` (default, `edit.mode`), `sloppy`. Hashline: `read` returns a `[path#TAG]` 4-hex
  snapshot header and `N:` lines; the model emits `PUT 3.=3:` / `PUT <N:` / `CUT` / `MV` against
  line numbers, never `-` lines (`src/edit/hashline-compact.md`); stale tags are recovered from
  stored snapshots with a warning (`modes/hashline/messages.rs:687-706`), else the error names
  both hashes. `replace`: exact → whole-block Levenshtein at `edit.fuzzyThreshold` 0.95, unique
  or dominant hit (`crates/pi-edit/src/fuzzy.rs:529`); failure echoes the best candidate: "Could
  not find a close enough match in {path}.\n\nClosest match (87% similar) at line 4:\n  - beta\n
  + gamma\nClosest match was below the 95% similarity threshold." Tree-sitter re-parses after
  an edit and an LLM auto-repair fixes a broken region (`src/edit/auto-repair.md`). **Read**
  returns a structural summary (declarations only, footer naming ranges to re-issue) for code
  without a selector (`tools/read-summary.ts`). **Bash**: persistent shell, default timeout
  **300 s**, clamped 1–3600 (`tools/tool-timeouts.ts`); foreground calls auto-background after
  60 s (`async/auto-background.ts`); tail 50 KB / 3000 lines (`session/streaming-output.ts:9`),
  raw stream spilled to `artifact://<id>`; timeout is an `isError` result, not a throw.

## Prompt

- **pi** (`coding-agent/src/core/system-prompt.ts`): ~25 lines; guidelines come from tool
  snippets ("Use read to examine files instead of cat or sed"). No planning, verification or
  re-read doctrine. Context files (`core/resource-loader.ts:72`): first of `AGENTS.override.md`,
  `AGENTS.md`, `CLAUDE.md` per directory, `~/.pi/agent` then root→cwd; no cap; no model variants.
- **oh-my-pi**: `prompts/system/system-prompt.md`, 271 Handlebars lines: RFC 2119 conventions, a
  Harmony-style `namespace functions` tool inventory, a six-phase workflow ("Tool failure/file
  change since read → re-read before acting", "NEVER re-audit applied edit … Tool results are
  verification", "NEVER yield non-trivial work without deliverable proof"), and a delivery
  contract forbidding stubs and silent scope reduction. Per-model variants are *dialects*
  (`packages/ai/src/dialect/{glm,deepseek,qwen3,kimi,minimax,…}.md`) giving each family's in-band
  tool-call syntax when run without native function calling; GLM's ends "NEVER announce a tool
  then stop". Context files come from 18 discovery providers (`src/discovery/`: `.omp/AGENTS.md`,
  `CLAUDE.md`, Codex, Gemini, Cursor…), deduped per depth, wrapped in `<repo-rules>`; deeper
  `AGENTS.md` paths (≤200, `workspace-tree.ts:16`) are listed for lazy reading.

## Context management

- **pi** (`core/compaction/compaction.ts`): trigger when `contextTokens > contextWindow −
  reserveTokens` (16 384), from the last usage report plus chars/4 for trailing messages. Keep
  the newest ~20 000 tokens, cut only at user/assistant boundaries, summarise the rest with a
  fixed Goal/Progress/Decisions/Next-Steps template; later compactions *update* the previous
  summary. Caching: Anthropic `cache_control` on system, last tool, last user message;
  OpenAI-compatible `prompt_cache_key` plus `cache_control` (`ai/src/api/openai-completions.ts`).
- **oh-my-pi** (`packages/agent/src/compaction/`): method chain `remote` (server-side) →
  `snapcompact` (history rendered to PNG frames, no LLM call) → `handoff` → `shake` (placeholder
  elision) → `soft` (`session/compaction-methods.ts:44`); pi's reserve rule, `keepRecentTokens
  20000`, mid-turn and idle passes, an `auto-continue` prompt after; `pruning.ts` drops superseded
  tool results first, guarded so it never mutates the warm cache prefix.

## Provider seam

- **pi**: `~/.pi/agent/models.json` (`coding-agent/docs/models.md`): `baseUrl`, `api ∈
  {openai-completions, openai-responses, anthropic-messages, …}`, `apiKey` literal/`$ENV`/`!cmd`.
  Built-ins `ai/src/providers/deepseek.ts` (`api.deepseek.com`, `DEEPSEEK_API_KEY`) and `zai.ts`
  (`api.z.ai/api/coding/paas/v4`, `ZAI_API_KEY`), both `openai-completions`.
- **oh-my-pi**: `~/.omp/agent/models.yml` (`docs/models.md`), same fields plus `compat` flags;
  Z.ai on both the completions and `api.z.ai/api/anthropic` endpoints, DeepSeek on
  `api.deepseek.com`. `packages/ai/src/providers/openai-completions.ts` strips leaked DeepSeek
  `<｜…｜>` tokens, heals text-wrapped tool calls (`stop`→`toolUse`), retries effort 400s.

## Non-interactive run mode

- **pi**: `pi -p "prompt"` from the target directory (no `--cwd` flag found) or `pi --mode json
  "prompt"`; `--model provider/id[:thinking]`, `--api-key`, `--tools`, `--no-session`. JSON mode
  is JSONL events (`docs/json.md`); `message_end` carries `usage {input, output, cacheRead,
  cacheWrite, totalTokens, cost}`; no latency field.
- **oh-my-pi**: `omp -p --cwd <dir> --yolo --no-session --max-time 10m --mode json "prompt"`
  (`docs/cli-reference.md`); same usage/cost shape; OTLP export (`telemetry-export-otlp.ts`).

## What oh-my-pi changes relative to pi, and why

Stated goal (README, blog "The harness problem"): make cheap models land edits first try — Grok
Code Fast 6.7 % → 68.3 %, Grok 4 Fast −61 % output tokens — via edit format, read summaries and
per-model prompts. Added: hashline; in-process Rust `grep/glob/bash`; LSP, DAP, `eval` kernels,
subagents; TTSR rules that abort a stream and inject a reminder; stop and loop guards; fallback
chains; snapcompact and pruning; 60+ providers with dialects. Kept: pi's loop and compaction.

## Ranked deltas worth trying in Butterfly (weak, non-OpenAI models)

1. **Echo the closest match with a similarity score on a failed edit** (omp `fuzzy.rs:1069`) on
   top of pi's normalisation ladder — turns "not found" into a one-shot fix for GLM/DeepSeek.
2. **Silent-overflow detection for z.ai** (`usage.input+cacheRead > contextWindow` on a
   successful reply, pi `overflow.ts` case 2) plus the regex table; one compact-and-retry.
3. **Bounded empty-stop / unexpected-stop nudges** (omp, 3 each, mechanical mode) *before*
   Butterfly's gate — cheaper than a gate round for "announced a tool and stopped"; keep the gate.
4. **Fail, don't execute, tool calls from a `length` stop** with a re-issue message, and accept
   `edits` as JSON string / single object (pi `agent-loop.ts`, `prepareArguments`).
5. **Tool-call loop guard** at 5 identical calls with a `<system-interrupt>` naming the arguments
   and last result (omp), plus a malformed-text-call retry message.
6. **Bash defaults**: 300 s timeout returned as an error *result* with output, full output
   spilled to a named file (pi/omp); omp's 60 s auto-background is the cheap Codex yield/poll.
7. **Transport hygiene for DeepSeek/GLM**: strip leaked `<｜…｜>` tokens, retry empty `stop`
   completions twice, promote `stop`→`toolUse` when a call was healed from text (omp).
8. **Prompt lines** (gated prefix only): "file changed since read → re-read", "tool results are
   verification; do not re-audit an applied edit", "bug fix → reproduce, fix, confirm" (omp).

Not adopted, deliberately: hashline as default edit format (changes every read result, needs the
snapshot store; revisit with a flash-model bench delta), the 271-line prompt and 28-tool surface,
snapcompact, advisor review, TTSR, and — as with Codex — dropping the completion gate: pi has
none, omp substitutes stop classifiers, and Butterfly measured why the gate exists.

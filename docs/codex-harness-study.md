# Codex CLI harness study (2026-09-12)

What OpenAI's Codex CLI (Apache-2.0, `openai/codex`, commit `c210f4c`, 2026-09-11) does in its
Rust core, read against Butterfly's equivalents, to decide which mechanisms are worth adopting.
Paths are relative to `codex-rs/` in the upstream checkout. This extends
`harness-implementation-comparison.md`; it does not repeat it.

## Turn loop

- One unbounded `loop` (`session/turn.rs`): pre-sampling compaction → capture context, tools and
  permissions once per request → sample → dispatch tools → repeat while a tool asked for a
  follow-up or user input is queued. **A reply with no tool calls ends the turn unconditionally.**
  No completion gate, no nudge, no attempt/retry-with-evidence, no per-turn call cap. Termination
  is carried by the prompt ("keep going until the query is completely resolved"; in
  non-interactive mode "proactively run tests, lint and do whatever you need"). The only post-turn
  hook is a user-configured Stop hook.
- Ceilings are token-based: auto-compaction at 90% of the context window; an optional session
  token budget that errors out.
- Provider errors (`responses_retry.rs`, `protocol/src/error.rs`): 5 stream retries (configurable,
  capped at 100), backoff `200ms·2^(n-1)` with jitter, server `retry_delay` honoured, 300 s idle
  timeout. Retryable: stream, rate limit, timeouts, unexpected status, connection failure, 5xx,
  io, json. Fatal: invalid request, quota/usage limit, context window exceeded, sandbox, server
  overloaded, aborted. Two extras: after retries exhaust it switches transport (WebSocket → HTTPS)
  and resets the counter; connection failures get an *unbounded* separate 5 s → 60 s ladder.
  Context-window-exceeded mid-turn triggers one compaction and continues.

## Tools

- **No read, list or grep tools.** The file surface is the shell plus `apply_patch`.
- `exec_command {cmd, workdir?, tty?, yield_time_ms?, max_output_tokens?, …}` runs in a PTY and
  returns after the yield (default 10 s, 250 ms–30 s) with a `session_id` if still running;
  `write_stdin {session_id, chars?, …}` feeds and polls it. Background processes live up to 300 s;
  up to 64 concurrent.
- Output budget default **10 000 tokens** per call (hard ceiling ~262k). Truncation is
  **middle-out** with a header stating the original token count.
- `apply_patch` is a freeform grammar tool (Lark grammar, "do not wrap the patch in JSON").
  Failures come back as ordinary tool results, never fatal, and echo the exact context block that
  did not match (`"Failed to find expected lines in {path}:\n{old_lines}"`). Matching is a
  four-pass ladder: exact → rstrip → full trim → Unicode-normalised (dashes, smart quotes, NBSP).
- `update_plan {explanation?, plan:[{step,status}]}`, `get_context_remaining → {tokens_left}`,
  `new_context` (the model can trigger its own compaction), `view_image`, `request_permissions`,
  `sleep`, `current_time`, MCP tools.
- Malformed tool-call JSON is not specially handled: the parse error is returned to the model.

## Prompt

- One base prompt (`models-manager/prompt.md`, ~275 lines) plus a server-supplied per-model
  override and a personality placeholder; legacy per-model prompt files remain in tree.
- Notable lines: "Do not waste tokens by re-reading files after calling `apply_patch` on them. The
  tool call will fail if it didn't work."; prefer `rg`; ~40 lines of plan doctrine with good and
  bad examples; validation instructions are approval-mode-conditional.
- `AGENTS.md` is assembled host-side from root to cwd, concatenated into the developer message
  with a separator, capped at 10 000 tokens.
- Reasoning effort travels as a history item (`ConfigurationUpdate`), so mid-session changes do not
  invalidate the cached prefix.

## Context management

- Auto-compact at 90% of the window. Rebuild: initial context, then the most recent user messages
  up to 20k tokens, then one summary item produced by a "context checkpoint compaction" prompt and
  replayed as another model's summary. Mid-turn compaction re-injects initial context before the
  last user message, preserving prefix stability. Compaction can be remote.

## Sandbox and approval

- Two axes: `sandbox_mode ∈ {read-only, workspace-write, danger-full-access}` and
  `approval_policy ∈ {never, on-request, unless-trusted, granular}`, each with its own prompt
  fragment. Commands are split at shell operators and each segment classified; anything with
  redirection, substitution, env prefixes or globs is never rule-matched. The model requests
  escalation inside the tool call (`sandbox_permissions`, `justification`, optional reusable
  `prefix_rule`), with broad prefixes banned. OS sandboxes: Landlock, seatbelt, bwrap, Windows.

## Provider seam

- `[model_providers]` config: base URL, key env, headers, query params, retry and idle settings;
  Ollama and LM Studio integrations. **The Chat Completions wire was removed**: Codex is
  Responses-API only, so its seam for GLM and DeepSeek is narrower than Butterfly's.

## Ranked deltas worth trying in Butterfly (weak, non-OpenAI models)

1. **Fuzzy edit matching with the failed block echoed back** — tool-level, transfers. Butterfly's
   `edit` requires an exact unique `old_string`; the rstrip/trim/normalise ladder plus echoing the
   unmatched lines turns a dead end into a one-shot correction.
2. **Middle-out truncation of tool output** — tool-level, transfers. Butterfly keeps the tail;
   test runners put the first failure at the head. Keep both ends, state the original size.
3. **Yield-and-poll command execution** — tool-level, transfers; the largest change. Return after
   ~10 s with a session id instead of blocking up to the timeout.
4. **Model-callable `get_context_remaining` / `new_context`** — tool-level, partly transfers.
5. **Separate unbounded ladder for connection failures** — loop-level, transfers.
6. **Prompt: no re-read after a successful patch; in autonomous mode run the tests yourself before
   yielding** — prompt-level, transfers; gated prefix only (default bytes are frozen).
7. **Freeform grammar patch tool, `output_schema` on tools** — Responses-API features; do not
   transfer to Chat-Completions endpoints.

Not adopted from Codex, deliberately: dropping the completion gate. Codex ends a turn on any
text-only reply and relies on the prompt; Butterfly's measured false-completion rate before the
gate (23 of 24 "verified" claims wrong on Terminal-Bench, 2026-09-04) is the reason the gate exists.

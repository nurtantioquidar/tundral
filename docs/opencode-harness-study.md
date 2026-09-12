# OpenCode harness study (2026-09-12)

What OpenCode (MIT, `anomalyco/opencode`, formerly `sst/opencode`, commit `95daf90`, 2026-09-11) does in its
Bun/Effect core, read against Butterfly's equivalents. Paths are relative to `packages/opencode/src/` unless prefixed
`packages/`; source read only, nothing run. Extends `codex-harness-study.md`.

## Turn loop

- One `while (true)` (`session/prompt.ts:1085-1336`): exit when the last assistant has a `finish` other than
  `tool-calls`/`unknown` and no tool parts; else sample → dispatch → repeat. **A text-only reply ends the turn
  unconditionally.** No completion gate, nudge, or retry-with-evidence.
- Step cap per agent, default `Infinity` (`prompt.ts:1178`; `packages/core/src/v1/config/agent.ts:34`). On the last
  step a synthetic assistant message "MAXIMUM STEPS REACHED… Respond with text only" is appended
  (`packages/core/src/session/runner/max-steps.ts`); tools are still sent — prompt-only. Doom-loop guard: 3
  consecutive identical tool calls (name + JSON input) raise a `doom_loop` permission request
  (`session/processor.ts:29,355-381`). No token budget; overflow → compaction.
- Termination is prompt-borne (beast.txt:9 "NEVER end your turn without having truly and completely solved the
  problem"; gpt.txt:19 "Persist until the task is fully handled"); default.txt, which GLM/DeepSeek get, has no such line.

## Provider errors

- `session/retry.ts`: 5 retries, `2s·2^(n-1)` +25% jitter, cap 30 s without headers; `retry-after(-ms)` honoured up to
  2^31-1 ms. Retryable: SDK `isRetryable`, any 5xx, or message/body regexes (429/5xx/524, rate limit, overloaded,
  socket/DNS errors, timeouts, "try again"). Fatal: everything else incl. `ContextOverflowError` (`retry.ts:87`). The
  whole stream is retried via `Effect.retry` (`processor.ts:674-689`); AI SDK `maxRetries: 0` (`session/llm.ts:323`).
  Fetch wrapper (`provider/provider.ts:1798-1826`): `headerTimeout` 300 s, `chunkTimeout` 300 s between SSE chunks
  (`wrapSSE`, `provider.ts:37`), optional total `timeout`.
- Context overflow (413, `context_length_exceeded`, SDK detector; `provider/error.ts:175`) sets `needsCompaction`; the
  loop summarises everything before the last user message and replays it (`processor.ts:621-633`,
  `prompt.ts:1321-1329`, `session/compaction.ts:340-356`).

## Tools

- Builtins (`tool/registry.ts:206-245`): `invalid`, `question`, `bash`, `read`, `glob`, `grep`, `edit`, `write`,
  `task`, `webfetch`, `todowrite`, `websearch` (opencode-hosted providers only, `registry.ts:58-63`), `skill`,
  `apply_patch`; experimental `lsp`, `plan_exit`, `execute`; MCP. Gating (`registry.ts:290-296`): `gpt-*` (not
  `oss`/`gpt-4`) get `apply_patch` and lose `edit`/`write`; everyone else the reverse. `read` returns 2 000 lines / 50
  KB as `N: line` (`tool/read.ts:13-16,339`).
- `edit {filePath, oldString, newString, replaceAll?}` (`tool/edit.ts`): CRLF-normalised, then a nine-replacer ladder
  (`edit.ts:697-707`): exact → line-trimmed → block-anchor (first/last-line anchors + per-line Levenshtein ≥ 0.65,
  `edit.ts:220-221,288-425`) → whitespace-normalised → indentation-flexible → escape-normalised → trimmed-boundary →
  context-aware → multi-occurrence. Spans ≥ 2× or +3 lines of `oldString` are refused (`edit.ts:709-712,727-733`).
  Failure text is generic — "Could not find oldString in the file. It must match exactly…" / "Found multiple matches…"
  (`edit.ts:721-726`) — **no candidate echo**; success returns a unified diff plus LSP diagnostics
  (`edit.ts:199-200`). Read-before-Edit is claimed in `edit.txt`, not enforced.
- `bash {command, timeout?, workdir?, description}` (`tool/shell.ts`): **blocking**, default 120 000 ms
  (`shell.ts:347`), kill at timeout+100 ms, then a `<shell_metadata>` note telling the model to retry with a larger
  timeout (`shell.ts:562-566`). Cap 2 000 lines / 50 KB (`tool/truncate.ts:14-15`, config `tool_output`), **tail
  kept** (`shell.ts:225-250,571`); past 50 KB output streams to a file and the result is prefixed "...output
  truncated... Full output saved to: <path>" (`shell.ts:497-505,577-580`). No yield/poll or background shell;
  `background: true` exists only for `task` subagents (`tool/task.ts:26-60`).
- Malformed tool calls are rewritten into `invalid {tool, error}` whose result is the parse error; wrong-case tool
  names are lower-cased (`session/llm.ts:284-300`, `tool/invalid.ts`).

## Prompt

- Selection by model-id substring (`session/system.ts:28-52`): `claude` → anthropic.txt, `gpt-4|o1|o3` → beast,
  `gpt-6` → gpt-astra, `codex` → codex, `gpt` → gpt, `gemini-` → gemini, `kimi`/moonshot → kimi, `muse` → meta, else
  default.txt (95 lines; 46–155 across variants). System order (`prompt.ts:1258-1264`): env header (model id, cwd,
  worktree, git, platform, date) → instruction files → MCP instructions → skills.
- default.txt: "fewer than 4 lines", "DO NOT ADD ANY COMMENTS", verify with tests, run lint/typecheck when known,
  prefer the Task tool for search, batch tool calls. No plan doctrine. Only beast.txt:129 has a no-re-read rule.
- Instruction files (`session/instruction.ts:60-67,115-132,155-167`): one global file (first of
  `~/.config/opencode/AGENTS.md`, `~/.claude/CLAUDE.md`), then the first name found of
  `AGENTS.md`/`CLAUDE.md`/`CONTEXT.md` walking cwd → worktree, then `config.instructions` globs/URLs; each is its own
  system string `Instructions from: <path>\n…`. No token cap (not found).

## Context management

- Trigger: last assistant total tokens ≥ input limit − min(20 000, max output) (`session/overflow.ts:8-31`), checked
  before every step (`prompt.ts:1163-1170`) and on overflow errors.
- A `compaction` agent summarises serialised history (tool outputs cut to 2 000 chars) into a fixed template —
  Objective / Important Details / Work State (Completed, Active, Blocked) / Next Move / Relevant Files — merged with
  the prior summary (`packages/core/src/session/compaction.ts:16-58`). A verbatim tail is kept: 25% of usable, clamped
  2 000–15 000 tokens (`compaction.ts:32-33,117-122`).
- Pruning (opt-in, `compaction.ts:273-318`): tool outputs older than the newest 40 000 tokens become "[Old tool result
  content cleared]" (`message-v2.ts:293`) when ≥ 20 000 tokens would be freed. This rewrites history in place and
  defeats any prefix cache.
- Prompt cache: `cache_control: ephemeral` on the first two system and last two messages, Anthropic-family only
  (`provider/transform.ts:357-395,466-480`); OpenAI `prompt_cache_key = sessionID` (`transform.ts:1310-1321`). No
  prefix-stability logic for OpenAI-compatible (not found).

## Provider seam

- Config `provider.<id>` = `{npm, api, env, options{apiKey, baseURL, headers, timeout, headerTimeout, setCacheKey},
  models{<id>{…}}}` (`packages/core/src/v1/config/provider.ts:81-115`). Catalog from `https://models.opencode.ai`
  (`packages/core/src/models-dev.ts:160`) already lists `zai` (`@ai-sdk/openai-compatible`,
  `https://api.z.ai/api/paas/v4`, `ZHIPU_API_KEY`), `zai-coding-plan` (`…/api/coding/paas/v4`) and `deepseek`
  (`https://api.deepseek.com`, `DEEPSEEK_API_KEY`) (fixture `packages/opencode/test/tool/fixtures/models-api.json`).
  Any Chat Completions endpoint: `{"provider":{"x":{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":
  "…","apiKey":"…"},"models":{"glm-4.7-flashx":{}}}}}` (docs `providers.mdx:377-403`).
- Anthropic via `@ai-sdk/anthropic` (`provider.ts:116`); Responses API for openai/xai/meta (`provider.ts:209-231`). A
  native runtime (`packages/llm/src/protocols/`) is gated to providers `openai`/`anthropic`/`opencode*`
  (`session/llm/native-runtime.ts:55-59`); GLM/DeepSeek go through the AI SDK.

## Non-interactive run mode

- `opencode run --dir <path> -m zai/glm-4.7-flashx --auto "<prompt>"` (`cli/cmd/run.ts:127-260`). `--format json`
  emits NDJSON `{type, timestamp, sessionID, …}`, `type` ∈ text, reasoning, tool_use, step_start, step_finish, error
  (`run.ts:676-786`); `step_finish` carries `tokens` and `cost` (`packages/core/src/session/projector.ts:39-40`); the
  assistant message carries `tokens{input,output,reasoning,cache{read,write}}` and `cost` (`prompt.ts:1195-1196`). No
  per-call latency (not found). Exits when `session.status` is `idle` (`run.ts:786-792`), exit code 1 on
  `session.error` (`run.ts:837-842`); without `--auto` every permission is auto-rejected (`run.ts:801-820`) and a
  denied tool ends the turn (`processor.ts:642`).

## Ranked deltas worth trying in Butterfly (weak, non-OpenAI models)

1. **`invalid` tool repair** — malformed tool-call JSON or wrong-case names become a real tool result carrying the
   parse error instead of a dropped call. Tiny; high value for GLM flash.
2. **Doom-loop interrupt** — 3 identical consecutive calls stop the turn. Trivial.
3. **Block-anchor edit matching with a disproportion guard** — anchors plus per-line Levenshtein ≥ 0.65, refusing
   spans ≥ 2× the request; pair with Codex's unmatched-block echo, which OpenCode lacks.
4. **Overflow-error compaction with last-user-message replay** — a fatal 413 becomes one summary and a continued turn.
5. **Spill-to-file truncation with a Grep/Read hint** — full output stays on disk and the model is told where.
6. **Per-model prompt by id substring** — GLM and DeepSeek variants without touching frozen bytes.

Not adopted, deliberately: no completion gate (a text-only reply ends the turn; default.txt has no "keep going" line);
blocking 2-minute shell with no yield/poll; generic edit failure text; in-place pruning that defeats prefix caching;
300 s header/chunk timeouts; no connection-failure ladder; "prefer the Task subagent for search" on cheap models.

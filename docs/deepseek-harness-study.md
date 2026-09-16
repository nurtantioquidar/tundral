# DeepSeek Harness (dsh) study (2026-09-13)

What DeepSeek's harness (MIT, `deepseek-ai/deepseek-harness`, developer preview, commit `c291e79`,
2026-09-10, Node 22+/TypeScript, ~2 800 source files in ~200 workspace packages) does, read against
Butterfly's equivalents; companion to the Codex, OpenCode and pi studies. Paths are relative to the
upstream checkout; nothing was run or installed.

## Turn loop

- `packages/core/agent-loop/src/agent.ts:285-320`: `while (true)` over steps. A reply with no tool
  calls, or a tool result flagged `concludesTurn` (`:492`), ends the turn after the serial
  `agent/turn-stopping` event unless steering input is queued. **No completion gate, step cap,
  tool-call cap, token budget or doom-loop abort** (`constants.ts` holds only
  `DEFAULT_MAX_PARALLEL_TOOL_CALLS = 10`; `agent-loop/README.md:200` "No built-in turn budget");
  `Stop` hooks may force continuation, uncapped (`hooks/hooks-claude-code/src/index.ts:268`).
- "Keep going" is a plugin: `packages/goal/goal-round-driver/src/prompt.ts:15-24` queues one
  `<goal_round>` user message per idle period while a goal is active — "Before claiming completion,
  gather evidence that the whole objective is achieved, read the current goal, and mark it complete."
  `maxGoalRounds` default 256 (`goal/goal/src/index.ts:244`); `blocked` rejected before 3 consecutive
  rounds (`tool-goal/src/index.ts:32`). Completion is model-asserted (`packages/workflow/tool-ralph`:
  reports "are not independently verified"). Repeated identical calls get a reminder at run lengths
  `[3,5,8]`, never a stop (`packages/guard/repeat-tool-reminder/src/index.ts:46`).
- Provider errors (`packages/llm/llm/src/retry-policy.ts:14-24`, `packages/llm/llm-retry`): 5 retries,
  `500ms·2^n` capped at 10 s, ±10 % jitter, `retry-after` honoured up to the cap (above it normal
  mode gives up); `always` mode retries forever. Retryable `EMPTY_RESPONSE, RATE_LIMIT, SERVER,
  TIMEOUT, TRANSPORT`; fatal `AUTH, INVALID_REQUEST, QUOTA, INVALID_CREDENTIAL,
  CONTEXT_WINDOW_EXCEEDED`. Stream idle timeout 300 s, no connect timeout
  (`llm-deepseek/src/adapter.ts:145`). Context overflow → compaction's `agent/request-error` handler:
  prune/summarise once, retry (`maxOverflowRetries` 1, `compaction-basic/src/config.ts:96`).

## Tools

- Base bundle (`packages/bundle/base/cordis.patch.yml`): `bash {command, description, timeoutMs?,
  workdir?, run_in_background?}`, `read {file_path, offset?, limit?}`, `write`, `edit {file_path,
  old_string, new_string, replace_all?}`, `read_image`, `glob {pattern, path?}`, `grep {pattern,
  path?, include?}`, `job_list/job_output/job_kill`, `skill`, `exit_plan_mode`, `todo_write`,
  `create_goal/get_goal/update_goal`, `subagent`, `subagent_fork`, `list_subagent_models`,
  `send_message/interrupt_agent/list_agents`, `workflow`, `ralph`, `web_search`, `web_fetch`, MCP.
  The `minimal` preset — the "fixed two-tool training configuration" (`preset/agent-presets/README.md`)
  — is persona plus one persistent `bash`, 300 s timeout.
- `read`: default and max 2 000 lines, 2 000 chars/line, 50 KiB/call (`fs/tool-fs/src/read.ts:15`).
- `edit` is **exact literal match**, CRLF→LF only (`fs/fs-local/src/fsio.ts:804-816`). Errors:
  `old_string was not found in "<path>"`; `old_string matched N times in "<path>"; provide a more
  specific old_string or set replace_all to true`; `file changed since it was read — re-read the
  file, then retry`; `file has not been read — read the file, then retry` (`tool-fs/src/error.ts:25-31`).
  Success returns "The file … has been updated successfully." — no snippet, no diff.
- `bash`: blocking pipe, no PTY, default 120 s, max 600 s, **tail-only** 64 000-byte window per
  stream, full output spilled to a file (≤64 MiB) whose path is appended
  (`shell/bash-local/src/index.ts:35-111`). Fresh shell per call; `run_in_background` returns a job
  id; no stdin. `spill-policy` (base `maxInlineBytes 50000`) keeps head 25 000 / tail 25 000 bytes;
  `compaction-tool-result-pruner` (8 192 → head 4 096 + tail 1 024) prunes old results at compaction.

## Prompt

- No prompt file: `packages/core/system-prompt/src/index.ts:121-153` orders registered sections —
  identity "You are an AI agent powered by DeepSeek Harness." (−1000), persona (0), plan policy
  (500), per-tool sections (1000–2900), persona suffix (10200). Shipped persona is one line, "You are
  a coding agent powered by the {{model}} model." / "Your working directory is {{cwd}}."
  (`bundle/headless/cordis.patch.yml:7-11`); ~4–7 KB rendered. **Per-model variants: not found.**
- Rules sit in tool sections: "Read the file first … unless you just created or edited it in this
  session" (`tool-fs/src/edit.ts:81`); "Check the [exit code: N] marker on every bash result;
  investigate failures before moving on" (`tool-bash/src/index.ts:238`).
- `AGENTS.md`/`CLAUDE.md` (+`.local.md`): `$DSH_HOME/AGENTS.md`, then `.git` root → cwd, delivered as
  a **user** message, cap `maxBytes 65536`, broad files dropped before the most specific is truncated
  (`packages/context/agent-instructions/src/{config,files,render}.ts`).

## Context management and caching

- `compaction-basic`: trigger at **80 %** of the routed model's window, retain newest **16 %**
  verbatim, never split an assistant/tool pair (`src/config.ts:20-23`, `src/region.ts:117-140`). Count
  = last provider `usage` + 4-chars/token estimate of the delta (`llm/token-meter/src/index.ts:167-186`).
  Summary is a fixed 8-section checkpoint injected as a user message in `<compacted-summary>`
  (`src/summarizer.ts:33-74`); runs between steps and on overflow; the model cannot trigger it.
- Cache discipline is the standout. 262 package READMEs carry a mandatory "KV Cache effect" section
  (`core/system-prompt/README.md:147`, `core/agent-loop/README.md:158`). The summarisation call replays
  the previous request's system prompt, tools and history **byte-for-byte** with the instruction as
  the final user message, so the summary itself is a cache hit (`summarizer.ts:26-31`). Plan mode keeps
  the tool catalog identical "for request-cache stability" (`bundle/base/cordis.patch.yml:309`).
  `prompt_cache_hit_tokens` → `cacheReadTokens`, disjoint from `inputTokens` (`llm-deepseek/src/translate.ts:56`).

## Sandbox, approval, provider seam

- Modes `read-only | workspace-write | danger-full-access`, default `workspace-write`, approval
  `ask | never`; the only grant is `allowed-once`, no persistent rules (`bundle/base/cordis.patch.yml:227-241`).
- Two adapters on `ctx.llm`: `llm-deepseek` (`DEEPSEEK_API_KEY`, `https://api.deepseek.com`, four
  V4 models all `contextWindow 1_000_000`; `llm-deepseek/src/index.ts:88-122`) and `llm-pi-ai` over
  `@earendil-works/pi-ai` 0.85.1 with `openai-completions`, `openai-responses`, `anthropic-messages`
  (`llm-pi-ai/src/provider.ts:47-51`). Any endpoint via `$DSH_HOME/settings.yaml`: `providers.<id>:
  {apiKeyEnv, api, baseURL, models:[{id, contextWindow, maxTokens}], compat:{thinkingFormat:
  deepseek|zai|openrouter|…}}` (`llm-pi-ai/src/config.ts:90-135`). `zai` (GLM) and `openrouter` are
  pi-ai catalog ids; Ollama: not found. Retry policy is per route.

## Non-interactive mode and telemetry

- `cd <dir> && dsh --profile headless "<task>"`; no `--cwd`, `--json` or `--output-format`
  (`apps/cli/src/args.ts`, `bundle/headless/src/startup.ts:36`). Reasoning → stderr, final text →
  stdout, exit 0/1. Structured control only via `dsh --profile sdk` (JSON-RPC stdio; Python
  `DeepSeekHarness(...).run(prompt)` → `final_response, finish_reason, events`) or `--profile acp`.
- Telemetry is the session log: `$DSH_HOME/sessions/--<cwd>--/<id>/session.v3.jsonl[.zstd]`, one
  `{type, seq, time, data}` per line; each `assistant/message` embeds the timed chunk stream and
  `usage {inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens, reasoningTokens}`
  (`core/session/src/types.ts:325-327`). **Cost in currency: not found.** The "Trajectory" view
  (`packages/client/ui-trajectory`) is a web tab over that log, not a file format.

## Plugin model

- Cordis (`vendor/cordis`, renamed `@deepseek-ai/cordis`): a plugin is a function or `Service` with
  `inject` + `apply(ctx)`; services claim `ctx.<key>`; registrations are disposable effects
  (`ctx.effect()`, `ctx.on()`); events dispatch as `emit | waterfall | parallel | serial | bail`
  (`docs/cordis-primer.md`). "Spatiotemporal composability" appears only in the arXiv citation; in
  code the plugin *tree* (rows, `isolate` realms) is patchable and hot-reloadable while *lifecycle*
  (mount, dependency wait, unwind) is framework-owned; plugins are YAML rows layered by patch files
  (`docs/architecture.md:15-37`).
- Hooks (`docs/architecture.md:133-158`): `ctx.tools.register()`, `ctx.llm.registerAdapter()`,
  `ctx.systemPrompt.section({order, text})`; waterfalls `agent/pre-step` (rewrite/reject input),
  `agent/request` + `prepareCall` (per-request model routing), `llm/stream`, `tools/pre-execute |
  execute | post-execute` (deny, ask, timeout, replace result, add context), `agent/request-error`
  (return a retry); serial `agent/turn-stopping`; `agent.inject()`; `session/event`.
- "Creator mode" is the shipped `cordis` preset (`presets/cordis/agent.cordis.yml`): standard tools
  plus `cordis_define/run/stop/undefine/inspect_*`, which evaluate model-written JS against the live
  runtime ("treat a session on this preset as shell access") — an agent that writes another agent.

## Ranked deltas worth trying in Butterfly (weak, cheap models)

1. **Cache-aligned compaction** — issue the summary request as an exact prefix of the previous
   request (same system, tools, history; instruction appended last): a 1/50-price call on DeepSeek.
2. **"KV Cache effect" as a review gate** — every change touching the request states prefix-stable
   or append-only; keep the tool catalog identical across modes, as dsh does for plan mode.
3. **Three-layer output control** — shell tail+spill (have), head/tail split for any oversized
   result, and pruning *old* tool results at compaction (8 192 → 4 096 + 1 024) with no model call.
4. **Repeat-call reminder at 3/5/8** — a graded nudge before a hard doom-loop stop.
5. **`[exit code: N]` marker + "investigate failures before moving on"** — shell section, not prefix.
6. **Goal-round wording** — "gather evidence that the whole objective is achieved" for the gate re-prompt.
7. **AGENTS.md byte budget** — drop broad files before truncating the specific one.

Not adopted, deliberately: the loop (no gate; completion model-asserted); exact-only `edit` with no
failed-block echo (worse than Codex/OpenCode for weak models); the everything-is-a-plugin Node runtime
(Butterfly's value is a small static Rust binary); the 1M window and 80 % trigger (Butterfly targets
128k–200k and should compact earlier); no monetary cost accounting (Butterfly bills in micro-USD).

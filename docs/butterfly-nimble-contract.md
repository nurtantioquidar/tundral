# Butterfly ↔ Nimble optimization contract (`nimble/v1`)

Status: v1, 2026-09-04. Optional, versioned, loosely coupled. Butterfly treats Nimble as one more
OpenAI-compatible provider; Nimble treats Butterfly as one more client that may supply richer
metadata. Every field below is optional. A request without the extension is a plain
OpenAI-compatible request; a response without the extension is a plain OpenAI-compatible
response. Either side must work when the other ignores all of this.

Canonical types: Rust in `butterfly-core::provider::nimble`, TypeScript validators in
Nimble's own repository (`router/src/contract.ts`), where the router lives since 2026-10-23.

## Transport

- Endpoint: `POST /v1/chat/completions` (Chat Completions shape, streaming or not).
- Request extension: a top-level body object `nimble` **and** mirrored headers `x-nimble-<field>`
  (kebab-case) for proxies that strip unknown body fields. Nimble removes the body field before
  forwarding upstream. Body wins over headers when both are present.
- Response extension: header `x-nimble-routing: <JSON>` on every response; on streams the first
  SSE chunk additionally carries a top-level `nimble` object with the same JSON. OpenAI clients
  ignore both.
- Version: `nimble.version = "nimble/v1"` in requests; `x-nimble-version: nimble/v1` in responses.
  A receiver that does not recognise the version ignores the extension entirely.

## Butterfly → Nimble (request)

```json
{
  "nimble": {
    "version": "nimble/v1",
    "session_id": "abc",
    "task_type": "debugging",
    "task_complexity": "high",
    "agent_phase": "implementation",
    "attempt": 2,
    "previous_failure": "test_failure",
    "verification_failed": true,
    "stable_prefix_hash": "sha256:…",
    "estimated_stable_tokens": 48000,
    "estimated_dynamic_tokens": 7000,
    "context_version": 3
  }
}
```

| Field | Type | Meaning |
|---|---|---|
| `session_id` | string | Butterfly session id; the stickiness and feedback key |
| `task_type` | `debugging \| feature \| refactor \| test \| explain \| ops \| unknown` | Coarse task class (from the user or the harness; never inferred by Nimble from prompts) |
| `task_complexity` | `low \| medium \| high` | Caller's estimate |
| `agent_phase` | `planning \| implementation \| verification \| recovery \| summarization` | What the request is for; `summarization` marks compaction requests |
| `attempt` | integer ≥ 1 | Attempt number within the task |
| `previous_failure` | Butterfly `FailureKind` string | Last classified failure, if any |
| `verification_failed` | boolean | The previous attempt failed deterministic verification |
| `stable_prefix_hash` | string | Hash of the cached prefix (system message + tool schemas + compaction checkpoint) |
| `estimated_stable_tokens` | integer | Butterfly's ledger estimate of the stable prefix |
| `estimated_dynamic_tokens` | integer | Estimate of the appended, non-prefix tokens |
| `context_version` | integer | Increments on every history rewrite (elision/compaction); a new value means the prefix changed |

## Nimble → Butterfly (response)

```json
{
  "version": "nimble/v1",
  "selected_model": "deepseek-v4-flash",
  "provider": "deepseek",
  "model_changed": false,
  "cache_affinity": 0.96,
  "estimated_cached_tokens": 48000,
  "context_limit": 1000000,
  "model_capabilities": { "tools": true, "parallel_tools": true, "reasoning": true, "prompt_cache": "automatic", "images": false },
  "routing_reason": "sticky: affinity 0.96",
  "decision_id": "d_01J…",
  "request_id": "r_01J…"
}
```

| Field | Meaning |
|---|---|
| `selected_model`, `provider` | What served the request |
| `model_changed` | Differs from the session's previous model |
| `cache_affinity` | [0,1] estimate of how much of the stable prefix was warm on the selected model |
| `estimated_cached_tokens` | Pre-request estimate; the actual value is in `usage` |
| `context_limit` | Selected model's context window |
| `model_capabilities` | Capability summary for the client's own decisions |
| `routing_reason` | Human-readable explanation; stored with the decision |
| `decision_id`, `request_id` | Keys for feedback and explainability |

## Opaque replay items (`nimble_replay`)

Some upstreams (OpenAI's Responses API) return reasoning state that must be replayed verbatim
before the function calls it produced. Nimble surfaces it to the client as an opaque
`nimble_replay` array on the final assistant chunk (and on the `message` of a non-streaming
completion); a client that echoes it back as `nimble_replay` on that assistant message gets it
replayed. Clients that drop it still work — the model re-reasons. Butterfly keeps it as
`replay_state` on the assistant message and sends it only on the Nimble dialect.

## Butterfly → Nimble (feedback)

`POST /v1/feedback`, fire-and-forget, idempotent per (`session_id`, `request_id`).

```json
{
  "version": "nimble/v1",
  "session_id": "abc",
  "request_id": "r_01J…",
  "task_type": "debugging",
  "task_success": true,
  "verification_result": "passed",
  "failure_type": null,
  "attempt_count": 2,
  "cost_microusd": 12345
}
```

`task_success` is truthful by construction: `true` only when an acceptance check the model did
not write (operator `--verify-cmd` or a manifest-detected check) passed; `false` when a check
failed; `null` when the run ended on the model's claim, a workspace diff, or model-declared checks
alone (`verification_result: "not_run"`). A router must treat `null` as no evidence, never as
success — the 2026-09-04 Terminal-Bench runs showed why: diff-only "passes" reported as success
taught the router to prefer the fastest confident model, which failed 23 of 24 tasks.

A `null` outcome can be **upgraded once** by a later report for the same session and request
that carries a verdict (`task_success` true/false): Nimble rewrites the record and updates its
per-model history. This is how a benchmark's verifier teaches the router what the harness could
not know (`bench/harbor/report-feedback.ts` posts Terminal-Bench rewards after each job). A
non-null outcome is never overwritten; a repeated report is a duplicate (HTTP 200).

## What Butterfly does with the response

- High affinity → keep the prefix byte-stable (it already does; the fact confirms the cost of a
  rewrite).
- `model_changed` → append a one-line user-visible note and, if the previous attempt failed,
  include the failure evidence the recovery loop already produces.
- `context_limit` → size compaction thresholds to the selected model.
- Nothing else. The response cannot name files, commands, tests, or solutions; any such field is
  ignored by the adapter.

## What Nimble must never do

Return instructions about which files to edit, which commands to run, whether tests pass, or how
to solve the task; alter the messages array beyond removing the `nimble` extension and forcing
`stream_options.include_usage`; persist prompts or message contents.

## Compatibility rules

- Additive changes only within `nimble/v1`; unknown fields are ignored by both sides.
- A breaking change bumps to `nimble/v2` and both sides negotiate by the `version` field.
- Absence of the extension is always valid in both directions.

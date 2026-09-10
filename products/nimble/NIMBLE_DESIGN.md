# Nimble design

Date: 2026-09-04. Status: design for v1–v4; v1 + v2 + the deterministic v3 scorer and the v4
feedback endpoint are implemented in `products/nimble/router` alongside this document. Learned
routing is explicitly deferred until enough clean telemetry exists.

Nimble is an intelligent model router. It is a product on its own (`products/nimble`,
`nimble.monet.sh` later, local service first). It is not an agent framework: it never sees
tools, repositories, plans, or verification; it sees OpenAI-compatible chat requests and decides
which model and provider should serve each one, remembering enough per session to protect
prompt-cache locality and to learn which model earns its cost.

Positioning:

```text
Nimble alone           = a model-agnostic router any OpenAI-compatible client can point at
Butterfly + Nimble     = the reference configuration; Butterfly supplies richer metadata,
                         Nimble returns richer routing facts; neither requires the other
```

Externally there is one virtual model, `nimble`. There are no `nimble/auto`, `auto-fast`,
`auto-max` tiers: economics and escalation are the router's job, not the caller's.

## 1. Architecture

```text
client (Butterfly, opencode, curl …)
   │  POST /v1/chat/completions   model: "nimble" | <concrete id>
   ▼
┌─────────────────────────────── nimble (one Bun process) ───────────────────────────────┐
│ http        OpenAI-compatible surface; strips the optional `nimble` extension and       │
│             `x-nimble-*` headers; re-emits SSE unchanged plus routing facts             │
│ router      explicit id → direct; "nimble" → score every eligible pool model, pick max, │
│             record every component of the decision                                     │
│ sessions    session_id → current model, stable-prefix hash, estimated stable tokens,   │
│             last seen, request count, recent failures                                  │
│ cache       per (provider, model, prefix hash): last served at → cache affinity        │
│ pricing     micro-USD price cards per model (same shape as Butterfly's), cost estimate │
│ health      per provider: consecutive failures, error EWMA, latency EWMA, cooldown     │
│ adapters    OpenAI-compatible upstream transport per provider (deepseek/zai/openai/…)  │
│ telemetry   SQLite (bun:sqlite): requests, decisions, sessions, feedback, stats views  │
│ feedback    POST /v1/feedback joins task outcomes back to decisions                    │
└──────────────────────────────────────────────────────────────────────────────────────────┘
   │
   ▼ upstream OpenAI-compatible providers: DeepSeek · Zhipu (GLM) · Inver (Luna) · configured
```

Single process, single SQLite file, no queue, no cache daemon. Everything is in-memory with
SQLite as the durable record; restart-safe because sessions and cache observations are persisted
on write.

Owns: model selection, provider selection, session/model affinity, prompt-cache economics, cost
and latency optimization, provider reliability, model switching, escalation. Does not own: tools,
repository context, editing, planning, verification, tests, failure classification, the agent
loop. Nimble never returns instructions about files, commands, tests, or solutions.

## 2. API

OpenAI-compatible, versioned under `/v1`.

| Method | Path | Purpose |
|---|---|---|
| POST | `/v1/chat/completions` | The only inference endpoint. `model: "nimble"` routes; a concrete id (e.g. `deepseek-v4-flash`) passes through to that model's provider. Streaming (`stream: true`) and non-streaming both supported; `stream_options.include_usage` is forced on so usage is always observed. |
| GET | `/v1/models` | The pool plus `nimble`, OpenAI list shape. |
| POST | `/v1/feedback` | Task outcome for a session/request (contract §13). |
| GET | `/v1/routing/decisions?session_id=&limit=` | Explainability: recent decisions with score components. |
| GET | `/v1/stats?task_type=` | Per-(model, task_type) success rate, cost/task, latency from feedback joins. |
| GET | `/health` | Liveness plus provider health snapshot. |

Authentication: a single bearer token (`NIMBLE_API_KEY`) when set; loopback-only by default
(`NIMBLE_HOST=127.0.0.1`). Nimble holds provider keys from the environment and never returns
them.

Request extension (optional): a top-level `nimble` object in the body (stripped before
forwarding) and/or `x-nimble-*` headers. Response extension: `x-nimble-routing` header (JSON) on
every response and, for streams, a `nimble` field on the first SSE chunk. Both are defined in
`docs/butterfly-nimble-contract.md` and are ignored safely by plain clients.

Errors are OpenAI-shaped `{ "error": { "message", "type", "code" } }`. Upstream 429/5xx trigger
failover to the next-best model within the pool when the request is routable (`model: "nimble"`)
and nothing has been streamed yet; explicit-model requests are proxied as-is.

## 3. Provider abstraction

```ts
interface ProviderConfig { id: string; base_url: string; api_key_env: string; dialect: "openai" }
interface ModelConfig {
  id: string; provider: string; display_name?: string;
  tier: number;                    // 1 = cheapest/weakest … n = strongest; drives escalation
  context_tokens: number; max_output_tokens: number;
  pricing: { cache_hit_input, cache_miss_input, output, version, verified }; // micro-USD per 1M
  capabilities: { tools, parallel_tools, reasoning, prompt_cache: "automatic" | "none", images };
  cache_ttl_seconds: number;       // how long a served prefix is assumed warm upstream
  task_affinity?: Record<string, number>; // optional prior per task_type in [0,1]
  prior?: number;                          // evidence prior in [0,1] (measured success on the
                                           // operator's suite); else tier / max_tier. Tier stays
                                           // the escalation ladder; prior is who is tried first.
}
```

Providers declare a `dialect`: `chat` forwards the body verbatim (minus the `nimble` extension,
plus forced `stream_options.include_usage`); `responses` translates to OpenAI's Responses API
(`src/responses.ts`) because OpenAI's Chat Completions endpoint refuses function tools with any
reasoning effort but `none` — input items, `instructions`, `reasoning.effort`, `store: false` with
encrypted reasoning that rides back to the client as `nimble_replay` and is replayed when echoed.
Both set the provider key and normalize usage: `prompt_cache_hit_tokens`/`prompt_cache_miss_tokens` (DeepSeek) or
`prompt_tokens_details.cached_tokens` (OpenAI/GLM) → `{ input, cached, output }`.

The initial pool is GLM-5.3 Flash (Z.ai), DeepSeek V4 Flash, GPT-5.6 Luna (OpenAI); all of it is
configuration (`nimble.config.json`), nothing is hard-coded. Prices verified 2026-09-04.

## 4. Routing state

Per session (`session_id` from the extension, else `x-session-id`, else a hash of the first
message for anonymous stickiness):

```ts
interface SessionState {
  session_id: string;
  current_model: string | null;        // last model that served this session
  prefix_hash: string | null;          // stable-prefix hash (client-provided or computed)
  stable_tokens: number;               // estimated stable-prefix tokens (client or observed)
  last_request_at: number;
  requests: number;
  switches: number;
  recent_failures: string[];           // last failure kinds reported via feedback/extension
}
```

Per (provider, model, prefix_hash): `last_served_at`, `prefix_tokens_seen` (max observed
`cached + miss` for that prefix). This is the cache-affinity ledger.

Per provider: health (§10). Per (model, task_type): success/cost statistics from feedback (§12).

## 5. Session stickiness

Default behaviour: a session stays on its current model. Switching requires the score of another
model to beat the current one by more than the **switch penalty** (§8), which encodes the
prefix-cache loss plus a fixed cost for re-priming a different model's style. Consequences:

- tiny savings never cause a switch ("DeepSeek cache ≠ GLM cache ≠ Luna cache");
- escalation after verified failure can switch, because the failure risk term outweighs the
  penalty;
- a session whose prefix cache has expired (`cache_ttl_seconds` elapsed) has no locality to lose
  and may move freely.

## 6. Cache-affinity model

For candidate model *m* and the request's stable prefix *p* with estimated size *S* tokens:

```text
warm(m,p)     = last_served_at(m,p) within cache_ttl_seconds(m)
affinity(m)   = warm ? min(1, prefix_tokens_seen(m,p) / S) : 0
est_cached(m) = affinity(m) × S
```

Sources of *S*: `estimated_stable_tokens` from the extension; otherwise the last observed
`prompt_tokens` for this session minus `estimated_dynamic_tokens` (or a byte heuristic over the
first message and tool schemas). Sources of *p*: `stable_prefix_hash` from the extension;
otherwise SHA-256 of the serialized system message + tools.

The ledger updates from real usage: after each upstream response, `prefix_tokens_seen(m,p) =
max(prev, cached + miss)` and `last_served_at = now`. Affinity is therefore observed, not
assumed, after the first request.

## 7. Pricing and cost model

Same integer micro-USD-per-million cards as Butterfly (DeepSeek verified 2026-08-31; GLM and
Luna placeholders marked `verified: false`; unverified cards taint cost in every report).

```text
expected_cost(m) = est_cached × hit(m)
                 + (S − est_cached) × miss(m)
                 + D × miss(m)                 // dynamic tokens
                 + O(m, task) × out(m)         // expected output tokens
```

`D` from `estimated_dynamic_tokens` or a byte heuristic over the non-prefix messages; `O` from
per-(model, task_type) observed output averages, else the model's default (config, 2 000).
Peak/off-peak tiers apply where the card has a schedule.

## 8. Routing score v1

Deterministic, explainable, every component stored with the decision.

```text
score(m) =   w_cap  × capability(m, task_type, complexity)
           + w_hist × historical_success(m, task_type)
           + w_cache× affinity(m)
           − w_cost × normalized_cost(m)
           − w_lat  × normalized_latency(m)
           − w_risk × failure_risk(m)
           − w_sw   × switch_penalty(m)
```

- `capability`: prior from `task_affinity[task_type]` if configured, else the model's `prior`
  (evidence prior; 2026-09-04 Terminal-Bench dev subset set GLM-5.3-Flash 1.0, Luna 0.4,
  DeepSeek 0.3), else tier / max_tier; a
  `task_complexity` of `high` and any `previous_failure`/`verification_failed` add an escalation
  bonus proportional to tier (§9).
- `historical_success`: Bayesian-smoothed success rate from feedback, `(s + 1) / (n + 2)`; with no
  data it equals 0.5 for every model and contributes nothing to the ranking. Its weight scales
  with evidence, `w_hist × (1 + 2·n/(n+5))`, so a well-measured model can outrank the tier prior.
- `affinity`: §6.
- `normalized_cost`: `expected_cost(m) / max over pool`.
- `normalized_latency`: provider latency EWMA / max over pool.
- `failure_risk`: provider health (open circuit → ineligible; degraded → penalty) plus recent
  session failures on this model, plus 0.5 for the current model when escalation is decisive (it
  has verifiably failed this task).
- `switch_penalty`: 0 for the current model; for others, `affinity(current) × cache_weight + fixed`
  — the locality you would throw away plus a constant.

Default weights (config): `cap 0.35, hist 0.20, cache 0.20, cost 0.15, lat 0.05, risk 0.30,
sw 0.25`. Ties break toward the current model, then the cheaper model.

Every decision stores: candidates, per-candidate components and total, chosen model, reason string
(`"sticky: affinity 0.96"`, `"escalated: attempt 2 after test_failure"`, `"failover: deepseek
unhealthy"`), and the estimated cached tokens returned to the client.

## 9. Escalation strategy

Escalate = prefer a higher `tier` than the current model. Triggers, all from the request
extension or feedback (never from parsing prompts): `attempt ≥ 2`, `verification_failed`,
`previous_failure` in {`test_failure`, `compile_failure`, `repeated_patch_failure`,
`incorrect_assumption`, `uncertain_result`}, or `task_complexity: "high"` on the first request.
Each trigger adds `escalation_bonus × (tier(m) / max_tier)` to capability. With a warm cache
that bonus alone rarely beats stickiness — correct for a single failure on a model ten times
cheaper. From `escalation_after_attempts` (default 2) onward, a *verified* failure
(`verification_failed` or a `previous_failure` in the escalation set) makes escalation decisive:
only tiers above the current model receive the bonus, the switch penalty is waived, and the model
that failed is benched for that decision whenever a higher rung exists, because a fix is now worth
more than cache locality. A decisive escalation fires **once per attempt** (the session records
`escalated_attempt`; the client's evidence is constant for the whole attempt, and re-firing it
against whichever model is current made a session ping-pong between two models on every request,
observed 2026-09-04). Likewise a `previous_failure` counts against a model once per attempt
(`failure_attempt`). Benching applies only when a higher rung is a credible alternative
(evidence prior ≥ half the current model's); forcing a switch to a model the evidence says is far
weaker trades a likely pass for a near-certain miss. De-escalation happens naturally on a new session
(sticky state is per session) and never mid-session unless the stronger model fails health
checks.

### Per-provider token budget

A provider may declare `tokens_per_minute` (the account's upstream cap). The health tracker keeps
a one-minute sliding window of tokens billed per provider; a candidate whose window plus the
request's estimate (stable + dynamic + expected output) would exceed its cap is ineligible for
that decision. Observed 2026-09-05: OpenAI's 500k TPM on ~100k-token requests made Luna refuse a
long session mid-stream after four calls; the budget keeps such sessions on a provider that can
carry them.

## 10. Provider health

Per provider: `consecutive_failures`, `error_rate` EWMA (α = 0.2), `latency_ms` EWMA,
`cooldown_until`. Three consecutive failures (5xx, transport, timeout) or an error EWMA above 0.5
open the circuit for `cooldown_seconds` (default 60, doubling to a cap of 600 while it keeps
failing). A probe request through the provider on the first eligible request after cooldown
half-opens it. 429 counts as a failure for failover but only half toward circuit opening.

## 11. Telemetry

SQLite tables, all timestamps epoch ms:

- `requests`: id, session_id, requested_model, selected_model, provider, routed, status,
  latency_ms, input_tokens, cached_tokens, output_tokens, cost_microusd, price_verified,
  prefix_hash, decision_id, error_kind.
- `decisions`: id, request_id, session_id, chosen, reason, components (JSON per candidate),
  model_changed, cache_affinity, estimated_cached_tokens.
- `sessions`: the state in §4.
- `cache_ledger`: provider, model, prefix_hash, last_served_at, prefix_tokens_seen.
- `feedback`: §12.

No prompts, no messages, no keys. Model ids, counts, durations, hashes, kinds only.

## 12. Feedback ingestion

`POST /v1/feedback` (contract §13): `session_id`, optional `request_id`, `task_type`,
`task_success`, `verification_result` (`passed | failed | not_run`), `failure_type`,
`attempt_count`, optional `cost_microusd` as measured by the client. Nimble joins to the last
decision for that session (or the given request) and updates `stats(model, task_type)`:
`n`, `successes`, `cost_sum`, `attempts_sum`, `latency_sum`. Feedback is idempotent per
(session_id, request_id).

Learned routing (weights fit from data, per-task priors) is deferred until at least 200 feedback
rows per model exist and are clean (verified prices, known task types). Until then the router is
the deterministic §8 scorer; the statistics are exposed via `/v1/stats` for human review.

## 13. Butterfly integration

Butterfly treats Nimble as a provider with `dialect = nimble`: the same OpenAI-compatible adapter
plus the optional extension (request metadata out, routing facts in) and a fire-and-forget
feedback post at task end. The contract is `docs/butterfly-nimble-contract.md`; the version is
`nimble/v1`. A Butterfly session never requires Nimble, and Nimble handles any client.

Butterfly uses the returned facts only for its own context decisions: keep the prefix stable
when affinity is high; append a one-line "model changed" note when `model_changed` is true;
size compaction to `context_limit`; include failure evidence when a stronger model follows a
failed verification. Nothing in the response can make Butterfly edit, run, or conclude anything.

## 14. Benchmark methodology

Three measurements, all on Terminal-Bench 4.0.0 through Harbor
(`products/butterfly/bench/harbor`), each cell repeated `-k 5`:

1. **Harness uplift**: `mini-swe-agent + model` vs `butterfly + model`, per model.
2. **Router uplift**: `butterfly + best fixed model` vs `butterfly + nimble` over the same pool.
3. **Combined**: `butterfly + nimble` vs leaderboard configurations.

Metrics: pass rate, cost/task, **cost/successful task**, success/dollar, success/second,
cache-hit ratio, latency, retries, model switches, cache tokens lost on switch (estimated stable
tokens at switch time), escalations, recovery success rate (verified after ≥1 failed attempt).
Nimble's decision log supplies switches, cache loss, and escalations; Butterfly's telemetry
supplies the rest. Unverified price cards are starred and excluded from cost comparisons.

## 15. Implementation roadmap

| Version | Scope | Status |
|---|---|---|
| v1 | OpenAI-compatible API, provider adapters (config-defined), explicit model passthrough, basic configurable routing (ordered pool, health failover), telemetry | implemented |
| v2 | session state, current-model affinity, stable-prefix tracking, cache ledger, cost estimation, stickiness with switch penalty | implemented |
| v3 | deterministic scoring with stored components, escalation, provider health circuit | implemented (weights are config, not learned) |
| v4 | `/v1/feedback`, stats joins, `/v1/stats`, historical_success in the score | implemented; learned weights deferred |
| next | Butterfly dialect end-to-end run on the dev subset; weight tuning from measured cost/success; `nimble.monet.sh` docs surface | pending benchmark data |

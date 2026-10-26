---
name: butterfly-product
description: Butterfly (products/butterfly, binary `butterfly`) is the coding agent, ex-hx; now model-agnostic with verification; never reintroduce hx/harness names
metadata:
  type: project
---

Butterfly (`products/butterfly`, crates `butterfly-core` / `butterfly` / `butterfly-atlas`, binary
`butterfly`) is Monet's coding agent, renamed from hx on 2026-08-31. On 2026-09-03/04 it was made
model-agnostic: a `ModelDescriptor`/`ProviderDescriptor` registry (built-ins deepseek-v4-flash,
deepseek-v4-pro, glm-5.3-flash (Z.ai), gpt-5.6-luna (OpenAI), nimble; `[[models]]`/`[[providers]]` config), one
OpenAI-compatible adapter parameterized by dialect, a verification + failure-classification +
recovery loop (`run_task`, `-p --max-attempts --verify-cmd --detect-checks --require-diff`),
telemetry JSONL, and `--sandboxed` mode (shell tool) for Terminal-Bench containers.

**Why:** the user's program (2026-09-03) is Butterfly + Nimble as co-designed but independent
products; DeepSeek stays the reference direct model, never a requirement.

**How to apply:** never reintroduce "hx"/"harness" names. Default sessions' prompt-prefix bytes
must stay byte-identical; new prompt lines/tool schemas are gated behind explicit modes and only
appended. GLM-5.3-Flash (Z.ai, $0.15/$0.03/$0.50 list) and GPT-5.6 Luna (OpenAI, $0.20/$0.02/$1.20)
prices were verified against vendor docs on 2026-09-04; Luna's >272K-input surcharge is not modelled.
OpenAI Chat Completions refuses function tools with any reasoning effort but `none`, so the
`openai` provider uses a Responses-API dialect (`provider/openai_responses.rs`) with encrypted
reasoning replay; the user considers Luna without reasoning useless, so never fall back to `none`. The audit lives at `products/butterfly/BUTTERFLY_ARCHITECTURE_REVIEW.md`.
See [[nimble-product]] and [[terminal-bench-harbor-facts]].

**2026-09-04 benchmark findings and priorities (user decisions):** harness work comes first, then
benchmark through Nimble rather than model by model. Terminal-Bench 4 dev subset, single attempt:
Butterfly+DeepSeek 1/14 vs mini-swe-agent 0/12 at half the cost; Butterfly+GLM-5.3-Flash 2/2 (GLM
is the only model with headroom on this suite); Nimble-routed 1/24 because the router sent 100% to
Luna on a static tier prior and Butterfly fed it diff-only "verified" successes. Fixes shipped as
`--completion-gate` (`complete_task` tool, review, nudges; gated prefix) and a truthful verdict
(`verified`/`task_success` true only from explicit or detected checks); Linux binary v5. Canonical
acceptance rule (bench/harbor/README.md): cost per successful task with guardrails uplift,
false-completion rate, harness-attributable failures; protocol dev subset, -k 3, GLM, both
harnesses. DeepSeek account hit HTTP 402 mid-run (user topping up). Nimble tiers must be set from
evidence (GLM first), not the current 1/2/3 prior.

**Result 2026-09-04 (end of day):** Butterfly v5/v6 + Nimble (GLM first) passed 4/12 dev-subset
tasks at $0.20/task, $0.60/success; the same combination was 1/24 before the completion gate,
evidence priors, and the once-per-attempt decisive-escalation fix (Nimble ping-ponged GLM↔Luna
every request before). Keys now load unattended from ~/.config/monet/bench.env (0600) via
with-keys.sh. Next: fixed-GLM vs Nimble on the same tasks with -k 3; harness lever: reject
compile-only acceptance checks in the completion review.

**Head-to-head 2026-09-05/06 (dev subset, single repeat, v7–v10):** Butterfly+Nimble 4/12 at
$0.34/task ($1.01/success) vs Butterfly+GLM fixed 5/12 at $0.38/task ($0.91/success, after the v11
distributed-dedup re-run); 9/11 pairs agree, all splits were GLM-vs-GLM variance. Router = cost/ops value, not pass-rate value, at this
sample. Infra hardening shipped: stream retry, idle/first-byte timeouts, in-stream rate-limit retry,
6x backoff windows; Nimble: evidence priors, once-per-attempt escalation, verifier feedback upgrade,
task types, TPM budget. Z.ai balance ran out 2026-09-06 (429 code 1113); DeepSeek 402 on 09-04.
Next: -k 3 repeats once balances are topped up; harness lever: shallow claims still pass the gate
when the model's own check is weak-but-not-trivially-weak (html-js-filter Nimble session).

**Muse Spark 1.3 Contributor 2026-09-08 (user focus: cheapest combo; Contributor tier only, never
standard):** fast-subset (5 GLM-passing tasks), v13, 300 calls, 1 h cap, 5-way concurrency: 1/5 at
$0.09/task. Two failure classes: (a) false completion via a self-written verification script the
gate accepted (bun-sourcemap-leak 27/36 hidden); (b) three tasks timed out still working because
Muse thinks silently for minutes per call (session-window-debug: 18 calls/hour, 196 s median), so
wall clock binds, not the call budget. The earlier 50-call cap did bind (wal-recovery converted to a
pass at 300). Levers proposed to the user, awaiting decision: EFFORT=low rerun (~$0.50) to test
whether Muse is usable inside an hour at all; then an independent verifier turn. The user is
impatient with hour-long runs; state ETAs as the hard cap and prefer the fast subset.

**2026-09-09 levers shipped (user: "proceed", all three):** requirement checklist in the final
claim, self-authored-check detection (`verification_source: self_authored`), independent verifier
(`--independent-verifier`, fresh session, `report_verdict`, unmet → next attempt's evidence, pass
stays a claim), `--verifier-model` for cross-model verification, malformed-tool-argument
sanitising (Meta HTTP 400 otherwise). Binaries v14/v15/v16. Findings: Muse low effort 2/5 at
$0.24 (vs 1/5 at $0.44 default effort) — and fixed GLM's own record on the fast subset is also 2/5
(react-lead-form, sound-change-cascade), NOT 5/5 as first stated to the user; the subset is
"discriminating", not "GLM-passing"; same-model verifier on Muse passed 3 wrong claims out of 4
(shared misreading; hidden variant tests swap the input app); sound-change-cascade is beyond Muse
(239 calls/hour, 0/7). Runs in flight at hand-off: GLM control (v16, fast subset) and Muse+GLM
verifier (v16, low effort). User delegated the GLM-control decision ("I'll leave it up to you").

**2026-09-10/11 state:** committed 3f237e8 (Butterfly loop + Nimble + harness, 73 files; Regretz
staged work left alone). Modal attempt burned $43 of the user's $50 (my cost estimate was 3–4x
low; 1-h cap + GLM runaway reasoning; user lowered the limit — never use Modal again unasked).
Local Docker arm Butterfly+GLM dev subset k=2 N=4 3-h cap: 7/24, 4 timeouts (distributed-dedup,
music-harmony ×2 each), $11.07 true spend, $1.58/success; passes: payments ×2, sound-change ×2,
interleaved-vigenere ×2, html-js-filter ×1; react-lead-form 0/2. DeepSeek V4.1 Flash
(`deepseek-flash`, registry v18, legacy id rebilled at V4.1) fast subset 1/5 at $0.81 — not the
cheaper worker. Baseline (mini_swe_agent_py311) and Muse dev-subset arms NOT run yet; user asked
to hold the queue and will say when to continue. Uncommitted: deepseek-flash registry change,
README/bench notes.

**2026-09-12 three-arm result (dev subset ×2, local):** Butterfly+GLM 7/24 $1.58/success;
mini-swe-agent+GLM 7/24 $1.35/success; Butterfly+Muse low 3/24 $1.18/success. **No pass-rate
uplift over the minimal harness on GLM** — the 0/12 baseline was a DeepSeek-specific artefact.
Told the user plainly Butterfly cannot compete with Codex/Claude Code on pass rate (model ceiling);
user pivoted to studying Codex CLI's open-source harness → `docs/codex-harness-study.md`
(ranked deltas: fuzzy edit matching + failed-block echo, middle-out truncation, yield/poll exec,
connection-retry ladder, two prompt lines). Awaiting go to implement #1 and #2. Results section
written to bench README; commit held for the user's word.

**2026-09-19 System One triage (TypeSafe Jev):** shipped optional, off-by-default failure triage
(`crates/core/src/triage`, `--triage` / `[triage] enabled`, key `TYPESAFE_API_KEY` else the
`typesafe` credential-store entry). It refines ONLY `uncertain_result` verification verdicts —
AGENTS.md invariant 11, "triage may tighten, never open". Live-validated: `test_failure` at
confidence 1.0, 856 ms round trip (RTT-dominated, effectively free at $0.042/Mtok).

**Why:** the user wants Jev used natively in Butterfly eventually ("I want to make it native in the
end"); opt-in is a deliberate first step to get a measured refinement rate, not the end state.

Offline accuracy corpus (17 real failing outputs from cargo/rustc/python/unittest/go/bun/node/jq,
ground truth by construction, production client): **13/13 correct at the 0.9 floor, zero confident
errors**; every disagreement fell below the floor. Bench arm (Muse Contributor, fast subset,
2026-09-19): 2/5 at $0.191 in 36 min — unchanged from Muse's usual 2/5 — and triage fired exactly
ONCE, correctly declining at 0.33.

**Decision 2026-09-19 (user agreed):** judge triage on the offline corpus, ship it off-by-default
as-is, and do NOT buy more bench arms looking for a pass-rate signal.

**How to apply:** **failing verification events are rare — 1-2 per 24-task job** (GLM 09-10: 2 of
22; Muse 09-11: 2 of 23), and all 34 historical jobs hold only 23 in total. A triage pass-rate A/B
is underpowered by BASE RATE, not just variance: one or two events cannot move 7/24, and repeats
do not fix it. An earlier claim in this file's lineage that "61% of failing verifications are
uncertain_result" is a true proportion over a tiny denominator — do not read it as a large
opportunity. To make the verification loop measurable at all, first raise the base rate with
`detect_checks`/`require_diff` (a benchmark-methodology decision, not a config tweak). Extend
triage only in the order in `docs/butterfly-features.md` Planned #9. Never put a Jev call in front
of the main tool-decision loop: Jev emits no tool calls, so a pre-screen adds a round trip per turn
and removes none. Jev only pays where it replaces a model turn or a heuristic miss that costs one.

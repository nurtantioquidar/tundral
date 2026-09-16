# Private task suite

SWE-bench-style tasks mined from a private repository's own history, run without Docker on the
developer's machine. It exists to measure the harness on real, unpublished code before paying for
Terminal-Bench runs: every task is a commit that changed source and tests together inside one
service; the agent starts at the parent commit with the commit's intent as the statement, and the
commit's test changes are the hidden check, copied in only after the agent finishes.

Nothing from the private repository lives in this directory. Task files hold commit hashes,
paths, and statement text, and are kept outside the monorepo (default
`~/.config/monet/private-suite/`, mode 700); the checkout is passed with `--repo` at run time.
Run outputs (`--out`) contain diffs of private code and must also stay outside the monorepo and
outside `/tmp` (macOS clears it on reboot; the 2026-09-12 round-one raw trials were lost that
way, their summaries survive below). Use `~/.config/monet/private-suite/runs/<name>`. An
interrupted run resumes with `--skip-done`.

## Rules

- **Never a model that trains on prompts.** Muse Contributor is forbidden; `run.py` refuses any
  model whose name contains `muse` before the first request. GLM (`zai/glm-5.3-flash`) and
  DeepSeek (`deepseek/deepseek-flash`) are the intended models.
- **No history leakage.** Each workspace is a single-commit repository built with `git archive`
  of the parent, so `git log` cannot reveal the fix.
- **Hidden checks decide.** A trial passes only when every hidden test passes and no test that
  passed at baseline fails. The harness's own verdict is recorded separately, so false
  completions (claimed done, hidden tests fail) are counted.

## Commands

```bash
# 1. Find candidate commits (source + tests in one service with a pyproject.toml).
python3 extract.py scan --repo ~/src/private-repo --name private --rev origin/master -o candidates.json
python3 extract.py scan ... --all-commits            # also walk non-squashed branch commits

# 2. Prove each candidate: tests pass at parent, fail with hidden tests, pass with the commit.
python3 extract.py validate --repo ~/src/private-repo --tasks candidates.json --work-dir /tmp/wt -o tasks.json

# 3. Self-test the runner: apply the reference fix instead of a model. Must be 100%.
python3 run.py --tasks tasks.json --repo ~/src/private-repo --oracle --out runs/oracle

# 4. Run a harness. Keys come from bench/harbor/with-keys.sh; results land in <out>/results.jsonl.
../harbor/with-keys.sh python3 run.py --tasks tasks.json --repo ~/src/private-repo \
    --model zai/glm-5.3-flash --out runs/glm-baseline --runs 2 --concurrency 4 --timeout 2700
# --lean is on by default (2026-09-12 A/B); --no-lean reproduces the round-one baseline.
# Outline reads (whole-file read_file of a >=400-line file returns an Atlas outline) are the
# binary's default; --outline-min-lines 0 reproduces plain reads for an A/B.
```

## What a result row carries

Verdict: `pass`, `hidden_passed/hidden_total`, `regressions`, `claimed_done`, `false_completion`.
Cost: `cost_microusd`, `tokens_in/out/cached`, `cache_hit_rate` (from telemetry, so a killed trial
still bills correctly). Speed: `wall_clock_s`, `provider_calls`, `time_to_first_edit_s`,
`model_latency_s` vs `tool_time_s`, `max_call_latency_s`. Diagnostics: `edit_failures`,
`reread_after_edit`, `command_timeouts`, `output_cap_hits`, `compactions`, `provider_errors`,
`retries`, `errors_by_kind`. Raw logs per trial: `statement.txt`, `command.txt`, `stdout.json`
(Butterfly's report), `stderr.log`, `telemetry.jsonl`, `diff.txt`, `hidden-check.log`.

## Maven modules (round two)

`extract.py` and `run.py` treat any directory (up to two levels deep) with a `pom.xml` as a
Maven service: tests run with `mvn -q -B test`, statuses come from `target/surefire-reports`
as `Class::method`, hidden test files map to classes by path, and the workspace is warmed with
`mvn test-compile` before the clock starts. Java hidden tests compile against exact signatures
and assert exact messages, so a statement must carry the interface and the strings the tests
expect; a vague statement guarantees a compile failure rather than a near miss.

## Findings log

- 2026-09-12, de-services (17 Python/Go services, 344 squash commits): 29 candidates across
  master and PR branches, 16 validated, oracle 16/16. Rejections: bootstrap commits with no
  pyproject at the parent, services with no collectable tests, test-only commits. Two tasks
  require a dependency change as part of the fix. midas (dbt) and de-infra (Terraform) are
  unusable: their checks need BigQuery or cloud credentials.
- First GLM trial (15-min cap): no edit in 15 minutes, 11 calls, single calls of 35–200 s from
  hidden reasoning, $0.02. Cap raised to 45 min for the pilot.
- Five-task GLM pilot with the raw commit titles as statements (45-min cap, $0.50/task):
  0/5 pass, $0.48 total ($0.10 per trial), wall clock 9–45 min, time to first edit 5–12 min,
  single provider calls up to 333 s, cache hit 92–97%, no edit failures. Four of five were
  Butterfly-verified false completions: three hidden tests import a helper the commit
  introduced (`_is_retryable_status`, `run_pipeline_for_event_async`, `UserRepositoryError`)
  that a six-word title cannot convey; one was a behavioural miss on an underspecified title.
  Lesson: statements must state the required interface and behaviour, as an issue with
  acceptance criteria would. Twelve of sixteen statements were rewritten from the tests and
  diff (`statement_source: rewritten`); the four `large` refactor commits keep their titles and
  are excluded from round one via `--tier small,medium`.
- Same five tasks with rewritten statements: **5/5 pass**, 0 regressions, 0 false completions,
  $0.16 total ($0.03 per trial), wall clock 6–12 min, 18–29 calls, time to first edit 35 s–6 min.
  All passes came from real source edits (the hidden tests are copied in fresh, so tampering
  with visible tests cannot help). Statement quality, not the harness, explained pilot 1.
- **Round one, GLM baseline** (12 small+medium tasks × 2 runs, final statements): 20/24 pass
  (83%), 4 false completions all on the two largest medium tasks (multi-requirement commits
  where GLM lands most checks and declares done), 0 regressions, 0 timeouts, $1.74 total,
  $0.087 per success, median wall clock 10 min, median 28 provider calls, median time to first
  edit 5 min, cache hit 94%, model latency 95% of wall clock, 2.0 re-reads of a just-edited
  file per trial, 1 edit failure and 0 command timeouts in 24 trials, worst single call 908 s.
  Nine earlier false completions in the first pass were traced to statement errors (wrong
  default, strip on the wrong function, private vs public name, missing fields) and fixed by
  correcting the statements, not the harness; those tasks then passed 6/6. Statements are
  therefore the suite's main source of noise, and every statement change must be logged.
  Reading: tool-level levers (fuzzy edit matching, middle-out truncation, yield/poll exec) have
  no room here; round trips (re-reads, orientation before the first edit) and the completion
  gate on multi-requirement tasks do.
- Lever check before building: the four residual false completions all listed every
  requirement and marked each `met`, so a stricter checklist gate (count or enumerate) would
  not have caught them; they are honest misjudgements of one requirement in multi-part
  commits. Not built. The round-trip lever (`--lean`, appended prompt block) was built instead
  and is measured as an A/B against the round-one baseline.
- **Round one, DeepSeek V4.1 Flash baseline** (same 12 tasks × 2, peak pricing): 19/24 (79%),
  5 false completions (the two hard tasks plus one-off misses on three others, e.g. mapping
  `s`→`saham` in a derived key), 0 regressions, $0.70 total, $0.037 per success, median wall
  clock 2.7 min (GLM 10 min), median 28 calls (same as GLM), median time to first edit 73 s
  (GLM 300 s), cache hit 97%, model latency 81% of wall clock, worst single call 68 s (GLM 908 s),
  3.2 re-reads of a just-edited file per trial (GLM 2.0). Same call count, four times faster:
  GLM's wall clock is reasoning latency, not extra turns.
- **`--lean` A/B, DeepSeek V4.1 Flash** (same 12 × 2): 20/24 vs 19/24, median calls 22 vs 28,
  re-reads per trial 1.8 vs 3.2, time to first edit 46 s vs 73 s, $0.028 vs $0.037 per success,
  median wall 146 s vs 164 s, 0 regressions. The 16 provider errors in the lean arm were one
  63-second api.deepseek.com outage at 21:50 local hitting four concurrent trials (GLM arm saw
  none); one of those trials also had network-bound pytest runs stall for 300 s, inflating its
  wall clock to 19 min. Retries recovered every call.
- **`--lean` A/B, GLM 5.3 Flash** (same 12 × 2, rerun in full after the reboot): 21/24 vs 20/24,
  median calls 24 vs 28, re-reads per trial 0.6 vs 2.0, $0.060 vs $0.087 per success, 0
  regressions, 0 provider errors. Median wall clock 681 s vs 606 s and time to first edit 431 s
  vs 300 s went the other way: with model latency at 95% of wall clock and single calls ranging
  from seconds to 1 008 s, wall clock on GLM is dominated by reasoning-latency variance, and
  24 trials do not resolve a ±15% difference. Calls, re-reads and cost are the robust signals,
  and all three improved on both models. The two hard tasks remain the only failures.
- **Outline reads A/B** (2026-09-15, 9 Flink tasks × 2 runs, `--outline-min-lines 400` vs `0`,
  same binary per model): GLM 18/18 vs 17/18 at $0.044 vs $0.043 per success, 15.2M vs 14.2M
  input tokens, 205 vs 202 read calls, median cache-miss 53k vs 52k per trial; DeepSeek 18/18 vs
  18/18 at $0.018 vs $0.018, 13.0M vs 13.6M input tokens, 195 vs 174 reads. Paired per-task
  deltas are noise in both directions. Cost-neutral: both models already read large files in
  ranges (DeepSeek's outline fired in 4 of 18 trials, 2 tasks) and plain reads were already
  capped at 24 KB, so the outline replaced one capped page with a 10–21 KB structure map and the
  ranged reads that followed were the same. Kept on as the default (same pass rate, whole-file
  structure instead of a truncated first page). 95% of input tokens are cache hits: the prefix
  re-sent per call is the cost, so the remaining lever is call count, not read size. The
  savings estimate the binary now reports (`avoided_input_tokens`) came to ~130k tokens per
  outline trial, inside the ±400k per-trial noise, so it is unverifiable at this n. Blind
  edits (edits to files never read) on DeepSeek: 15 vs 10 across 18 passing trials — no signal.
  Runs: `flink-{outline,plain}-{glm,deepseek}`.
- **Call-count levers** (2026-09-16, 9 Flink tasks × 2 runs per arm, control = the outline arms
  above). Where Butterfly's extra calls went, 72 trials: `complete_task` 2–4 per trial with the
  review round trip a pure re-claim in 54 of 72; 91 of 1,794 provider calls carried only
  `update_plan`; 330 carried a single edit. OpenCode (36 trials, median 21.5 steps) has no
  completion tool and calls its plan tool a third as often — the ~4-call gap is structural.
  Two changes, measured cumulatively: (1) *proven first claim* — the review is skipped when
  every requirement is listed met and a non-weak check ran in-session with exit 0; (2) the lean
  block gained "update_plan with the step's first call, never alone; independent edits in one
  reply" (approved prefix change). Results (pass, median calls, input tokens per arm, output
  tokens per trial, actual cost per trial): GLM control 18/18, 26, 15.2M, 23.9k, $0.044 →
  proven 18/18, 21.5, 14.2M, 22.4k, $0.041 → proven+lean 18/18, 19.5, 13.7M, 25.4k, $0.042.
  DeepSeek control 18/18, 23.5, 13.0M, 17.6k, $0.018 → proven 17/18, 24.5, 13.5M, 17.1k, $0.017
  → proven+lean 18/18, 17.5, 11.8M, 17.9k, $0.034 **at the peak tier** (the arm started at 01:23
  UTC, off-peak ends 00:30; the tier doubles every DeepSeek price, so compare tokens, not
  dollars, across DeepSeek arms). Solo `update_plan` calls fell 25 → 11 (GLM) and 18 → 11
  (DeepSeek); solo edits 109 → 23 on GLM. First claims qualified as proven in 5/18 GLM trials
  (GLM claims before it has run a check) and 14–16/18 DeepSeek trials. The DeepSeek miss in the
  proven arm is the coin-flip task and its claim went through the full review. Net: calls −25%,
  input tokens −10%, pass rate intact, cost −4% on GLM where output tokens (~$0.012 of $0.042
  per trial) now rival the cache-hit prefix (~$0.021). Butterfly now matches OpenCode's call
  count (19.5 vs 21.5 on GLM) with the gate still on. Runs: `flink-{proven,lean2}-{glm,deepseek}`.

## Other harnesses

`run.py --harness opencode` drives OpenCode (`opencode run --pure --auto --format json`) on the
same tasks and hidden checks. The provider must be registered in the operator's own OpenCode
(`opencode auth login`; Z.ai and DeepSeek are in its catalog); a generated provider config under a
private XDG home errored at init on 1.18.26, so nothing is overridden. stdin is closed by the
runner: `opencode run` reads stdin to EOF whenever it is not a TTY and blocks forever on an
inherited open pipe, which cost a day of "init stalls" before the cause was found.
Metrics come from the NDJSON events (`step_finish` tokens, `tool_use` parts);
cost is recomputed from tokens on Butterfly's price card so both harnesses are billed alike.
OpenCode has no completion gate, so every self-terminated run counts as a claim and a hidden-test
failure is a false completion. Its prompt, tools and defaults are its own; only the task, model,
workspace and verdict are shared.
- **OpenCode 1.18.26 on GLM 5.3 Flash** (same 12 × 2, its own prompt and tools, `--pure`,
  session titling disabled, cost on Butterfly's card): 19/24 (79%), 5 false completions, 0
  regressions, $1.24, $0.065 per success, median 20 calls, median wall 509 s, model latency 79% of
  wall clock, 0.3 re-reads per trial, 1 edit failure, 1 output-cap hit. Butterfly `--lean` on the
  same model: 21/24, 3 false completions, $0.060 per success, 24 calls, 681 s. Both fail the same
  two multi-part commits every time; OpenCode additionally missed the crawler boundary task once.
  Reading: parity within the suite's resolution, Butterfly ahead by two passes and zero
  regressions, OpenCode ahead on calls per task; GLM wall clock remains latency noise.
- 2026-09-13, de-pipeline-hub (Flink, Java 11, Maven): 28 candidates since 2026-04 across three
  modules, 25 validated (3 rejected as test-only), oracle 25/25 (85-test module runs in ~3 s
  warm). Round-two set: nine tasks with rewritten statements (`round: 2`), seven small and two
  medium, all bug fixes with exact-message assertions. Statement fidelity is the risk again.
- **Cross-model verifier A/B** (GLM worker + DeepSeek V4.1 Flash verifier, `--verifier`, the two
  hard de-services tasks plus three small controls, 2 runs each): 6/10 vs 7/10 for the same
  tasks in the lean baseline. The verifier passed all four wrong claims on the hard tasks (11/14
  and 3–4/5 hidden checks, verdict `pass` every time) and rejected two correct claims on easy
  tasks ("negative requirement" objections), each costing a second attempt and 5–10 extra
  minutes. Verifier cost $0.14 of $0.78; zero catches. Same conclusion as Terminal-Bench on
  2026-09-09: a fresh-context reviewer with the same statement shares the worker's reading of
  it. Not adopted; the misjudged-requirement failure class stays open.
- **Round two, Flink (9 tasks × 2 runs)**: Butterfly DeepSeek 17/18 at $0.020 per success and
  101 s median; Butterfly GLM 18/18 at $0.052 and 547 s; OpenCode DeepSeek 17/18 at $0.034 and
  99 s. Zero regressions anywhere; one false completion per DeepSeek arm. Java with exact-message
  assertions is tractable for flash models once the statement carries the interface; the suite
  separates harnesses on cost and false completions, not pass rate. OpenCode GLM 17/18 at $0.048
  and 332 s. Every miss across all four arms is the same task (aborted-writer recovery, 0/4 when
  missed): a model coin flip, not a harness difference. Butterfly is cheaper per success on both
  models (DeepSeek $0.020 vs $0.034, GLM $0.052 vs $0.048 with one more pass); OpenCode uses
  fewer calls (20–22 vs 24) and re-reads less on GLM.

## Clarify experiment

The largest swing in the whole program was statement quality (0/5 → 5/5 on the same harness and
model), so `--clarify` lets the model ask before its first edit. The experiment keeps the model
blind: `--raw-statements` shows the commit title and body only; `--clarify oracle` answers its
questions through `clarify_oracle.py`, which holds the rewritten statement (what a well-informed
user knows) and answers strictly from it via DeepSeek; `--clarify unanswered` measures the
prompt line alone. Compare against `--raw-statements` without clarify and against the rewritten
statements: the gap the oracle closes is the value of asking; `questions_asked` and
`answers.log` per trial show what was asked.
- **Clarify experiment** (GLM, five small de-services tasks × 2, raw commit titles): with
  `--clarify oracle` 1/10, without 0/10 (rewritten statements: 10/10). The model asked in 6 of 10
  trials, and the oracle answered accurately, but it asked about the ambiguity it perceived
  (which layout, which time zone, how far to scan) and not about the bindings the hidden tests
  need (a helper's name, a new exception class, an exact error message). The one pass is the
  trial whose question happened to name the helper. Asking recovers only known unknowns; the
  interface names a test binds to are unknown unknowns to the model, so `--clarify` is a product
  feature for interactive use (assumptions become visible and correctable) rather than a
  pass-rate lever. Statement quality stays the dominant factor and lives with the user.

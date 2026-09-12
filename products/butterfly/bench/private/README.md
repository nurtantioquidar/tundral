# Private task suite

SWE-bench-style tasks mined from a private repository's own history, run without Docker on the
developer's machine. It exists to measure the harness on real, unpublished code before paying for
Terminal-Bench runs: every task is a commit that changed source and tests together inside one
service; the agent starts at the parent commit with the commit's intent as the statement, and the
commit's test changes are the hidden check, copied in only after the agent finishes.

Nothing from the private repository lives in this directory. Task files hold commit hashes,
paths, and statement text, and are kept outside the monorepo (default
`~/.config/monet/private-suite/`, mode 700); the checkout is passed with `--repo` at run time.

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
```

## What a result row carries

Verdict: `pass`, `hidden_passed/hidden_total`, `regressions`, `claimed_done`, `false_completion`.
Cost: `cost_microusd`, `tokens_in/out/cached`, `cache_hit_rate` (from telemetry, so a killed trial
still bills correctly). Speed: `wall_clock_s`, `provider_calls`, `time_to_first_edit_s`,
`model_latency_s` vs `tool_time_s`, `max_call_latency_s`. Diagnostics: `edit_failures`,
`reread_after_edit`, `command_timeouts`, `output_cap_hits`, `compactions`, `provider_errors`,
`retries`, `errors_by_kind`. Raw logs per trial: `statement.txt`, `command.txt`, `stdout.json`
(Butterfly's report), `stderr.log`, `telemetry.jsonl`, `diff.txt`, `hidden-check.log`.

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

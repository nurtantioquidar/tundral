# Butterfly onboarding

Butterfly is a coding agent that runs in your terminal, inside your repository, against a
model you pay for directly. It is built to be measured: every autonomous run ends with a
verdict it can defend, and it never calls something verified unless a check it did not write
passed. This page gets you from zero to a first verified task in about ten minutes.

## Install

```bash
cd products/butterfly/cli
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
butterfly --help
```

## Keys

Butterfly talks to providers directly. Set one environment variable per provider you use:

| provider | variable | model id |
|---|---|---|
| DeepSeek (reference model) | `DEEPSEEK_API_KEY` | `deepseek-flash` |
| Z.ai | `ZAI_API_KEY` | `glm-5.3-flash` |

Or run `/connect` once inside a session to store a key in Butterfly's credential file (0600,
under your config directory). Keys never enter transcripts, telemetry or exports.

DeepSeek charges half price between 16:30 and 00:30 UTC (23:30 to 07:30 Jakarta). Butterfly's
cost figures already apply the tier that was in force.

## Two ways to run

**Interactive.** Start `butterfly` in the repository root. Type a request, or `@path` to
mention a file. Shift+Tab cycles the permission mode between ask, auto and read-only. Every
command that is not on the safe allowlist asks you first, even in auto mode.

**Autonomous, verified.** Give a task and let it finish on its own:

```bash
butterfly --auto --completion-gate --verify-cmd "pytest -q tests" -p "your task statement"
```

`--auto` lets allowlisted commands run without prompting; anything outside the allowlist is
denied in headless mode rather than asked. `--detect-checks` finds the repository's own test
runner when you do not want to name one. Inside an interactive session the same thing is
`/task <statement>`. The completion gate means the model must declare completion explicitly
with its evidence, and the run ends with a summary block. Add `--json` to get the same facts
as a machine-readable result.

Lean turns are on by default: no reading a file back after a successful edit, orientation
limited to the files the task involves, independent reads and edits batched. `--no-lean` turns
that off if you want to compare.

## Reading a run summary

The first line is the verdict, and it is the only line you have to trust:

- `VERIFIED` — a check the model did not write passed. Your `--verify-cmd`, or a test runner
  Butterfly detected in the repository.
- `CLAIMED` — the model says it is done and its own checks passed. Nothing independent backed
  it. Treat it as a pull request from a colleague, not a green build.
- `FAILED` — a check failed on the last attempt.
- `UNRESOLVED` — the run ended without a completion claim (budget, cancellation, or no gate).

Below it: what backed the ending, each attempt's checks with who authored them, the model's
requirement checklist, cost and calls, and any warnings. Two warnings matter most:

- `statement: thin` — the statement names too few functions, messages or paths for a test to
  bind to. See the next section.
- `N edit(s) to files never read in this session` — the model edited from a search hit or
  memory. Worth a closer look at those files.

## Writing a task statement

This is the single largest factor in whether a run succeeds. On our internal suite the same
model and harness scored 0 out of 10 when given raw commit titles and 10 out of 10 when given
statements that named what the tests would check. Write the statement the way you would brief
a careful contractor who cannot ask questions:

- Name the function, class or endpoint to add or change, with its signature if it is new.
- Quote exact messages, formats and status codes another party will depend on.
- Give the path when there is more than one plausible home for the change.
- Say what should happen in the edge cases you care about, including the ones that should
  not change.

Butterfly prints a one-line warning before an autonomous run when a statement looks thin. It
never blocks, but the warning has predicted almost every miss we have seen.

## Cost guardrails

- `--max-cost-usd 1.00` caps a session; the summary states the cap it ran under.
- `--max-calls` caps provider round trips per input (default 50).
- Typical figures on our internal tasks: DeepSeek $0.02 to $0.03 per completed task, GLM
  $0.03 to $0.06. A task that runs past $0.50 is usually a thin statement or a missing
  test runner, not a big task.

## Useful commands and flags

| what | how |
|---|---|
| show tokens, cost, cache-hit rate | `/usage` |
| show the tracked plan and files | `/plan` |
| switch model | `/model glm-5.3-flash` |
| export the session as HTML, summary on top | `butterfly export` |
| turn GLM reasoning down for cheap, simple tasks | `--effort low` (costs about two passes in 18 on hard tasks) |
| disposable container only, relax confinement | `--sandboxed` (never on a machine you care about) |

## What to send back

For any run that felt wrong, send three things: the statement you gave, the run summary, and
the `telemetry.jsonl` beside the session log under `~/.local/state/butterfly/sessions/`. The
telemetry holds counts, durations, costs and failure kinds, never code or prompts. That is
enough to reproduce the shape of the run without sharing the repository.

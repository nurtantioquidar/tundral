#!/usr/bin/env python3
"""Run a harness against the private task suite and score it with hidden checks.

    run.py --tasks tasks.json --repo PATH --model zai/glm-5.3-flash --out runs/glm-baseline \
           [--runs 2] [--concurrency 4] [--binary PATH] [--timeout 1200] [--only id,id]
    run.py ... --oracle          # apply the reference fix instead of running a model (self-test)

Per task and run:
  1. A fresh, single-commit repository is built from `git archive` of the parent commit, so
     the agent sees no history and cannot find the fix in a later commit.
  2. The service's environment is synced before the clock starts (`uv sync --all-extras`).
  3. The harness runs headlessly with the task statement and a wall-clock cap.
  4. The hidden test files are copied in from the reference commit and pytest runs once.
     pass = every hidden check passes; regression = a test that passed at baseline now fails.

Butterfly's JSON report and telemetry JSONL feed the metrics; the hidden checks decide the
verdict. Results: one JSON line per (task, run) in `<out>/results.jsonl`, plus raw logs under
`<out>/<task>/<run>/`. Model names are Harbor-form `provider/model`; anything matching the
suite's `forbidden_models` (Muse) is refused before the first request.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from io import BytesIO
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_BINARY = HERE.parent.parent / "cli" / "target" / "release" / "butterfly"
PYTEST_ARGS = ["-q", "--no-header", "-p", "no:cacheprovider", "-rA", "--tb=short"]
RESULT_LINE = re.compile(r"^(PASSED|FAILED|ERROR) (\S+)")
EDIT_TOOLS = {"edit", "write_file"}
PROVIDER_KEY_ENV = {
    "zai": "ZAI_API_KEY",
    "z-ai": "ZAI_API_KEY",
    "deepseek": "DEEPSEEK_API_KEY",
    "openai": "OPENAI_API_KEY",
    "meta": "META_API_KEY",
}

STATEMENT_TEMPLATE = """{statement}

The change belongs in the `{service}/` service of this repository. Run its tests from inside that directory with `uv run pytest`; keep the existing tests passing."""

_print_lock = threading.Lock()


def say(msg: str) -> None:
    with _print_lock:
        print(msg, flush=True)


def git(
    repo: Path, *args: str, check: bool = True, input: bytes | None = None
) -> bytes:
    result = subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, input=input
    )
    if check and result.returncode != 0:
        raise RuntimeError(
            f"git {' '.join(args)} failed: {result.stderr.decode(errors='replace').strip()}"
        )
    return result.stdout


@dataclass
class Task:
    id: str
    service: str
    commit: str
    parent: str
    statement: str
    hidden_tests: list[str]
    hidden_test_ids: list[str]
    baseline_failing: list[str]
    source_files: list[str]
    other_files: list[str]


# ---------------------------------------------------------------- workspace


def build_workspace(repo: Path, task: Task, dest: Path) -> None:
    """Single-commit repository holding the parent tree, minus the hidden test files."""
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    archive = git(repo, "archive", "--format=tar", task.parent)
    with tarfile.open(fileobj=BytesIO(archive)) as tar:
        tar.extractall(dest, filter="data")
    # Hidden tests that already exist at the parent stay (the agent may run them); the
    # commit's versions replace them at scoring time. Nothing is removed here.
    git(dest, "init", "-q", "-b", "main")
    git(dest, "config", "user.email", "bench@localhost")
    git(dest, "config", "user.name", "bench")
    git(dest, "add", "-A")
    git(dest, "commit", "-q", "-m", "baseline", "--no-gpg-sign")


def sync_env(service_dir: Path, timeout: int) -> None:
    subprocess.run(
        ["uv", "sync", "--quiet", "--all-extras"],
        cwd=service_dir,
        capture_output=True,
        timeout=timeout,
    )
    subprocess.run(
        [
            "uv",
            "run",
            "--quiet",
            "--all-extras",
            "--with",
            "pytest",
            "--with",
            "pytest-asyncio",
            "python",
            "-c",
            "0",
        ],
        cwd=service_dir,
        capture_output=True,
        timeout=timeout,
    )


def apply_hidden_tests(repo: Path, task: Task, workspace: Path) -> None:
    for f in task.hidden_tests:
        exists_after = (
            subprocess.run(
                ["git", "-C", str(repo), "cat-file", "-e", f"{task.commit}:{f}"],
                capture_output=True,
            ).returncode
            == 0
        )
        target = workspace / f
        if exists_after:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(git(repo, "show", f"{task.commit}:{f}"))
        else:
            target.unlink(missing_ok=True)


def apply_oracle(repo: Path, task: Task, workspace: Path) -> None:
    """Reference solution: every non-test file as it is at the commit (source, config,
    dependencies). Files are written whole rather than patched because `uv sync` may have
    rewritten the lock file in the workspace."""
    for f in [*task.source_files, *task.other_files]:
        target = workspace / f
        exists_after = (
            subprocess.run(
                ["git", "-C", str(repo), "cat-file", "-e", f"{task.commit}:{f}"],
                capture_output=True,
            ).returncode
            == 0
        )
        if exists_after:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(git(repo, "show", f"{task.commit}:{f}"))
        else:
            target.unlink(missing_ok=True)


def run_pytest(service_dir: Path, timeout: int) -> tuple[int, dict[str, str], str]:
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    try:
        result = subprocess.run(
            [
                "uv",
                "run",
                "--quiet",
                "--all-extras",
                "--with",
                "pytest",
                "--with",
                "pytest-asyncio",
                "pytest",
                *PYTEST_ARGS,
            ],
            cwd=service_dir,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
        )
    except subprocess.TimeoutExpired:
        return 124, {}, "timeout"
    statuses = {}
    for line in result.stdout.splitlines():
        m = RESULT_LINE.match(line)
        if m:
            statuses[m.group(2)] = m.group(1)
    return result.returncode, statuses, result.stdout + result.stderr


# ---------------------------------------------------------------- harness


def butterfly_argv(
    args: argparse.Namespace, workspace: Path, telemetry: Path, statement: str
) -> list[str]:
    model = args.model.split("/", 1)[-1]
    argv = [
        str(args.binary),
        "--auto",
        "--json",
        "--no-save",
        "--quiet-reasoning",
        "--root",
        str(workspace),
        "--model",
        model,
        "--max-attempts",
        str(args.max_attempts),
        "--max-calls",
        str(args.max_calls),
        "--max-cost-usd",
        str(args.max_cost_usd),
        "--telemetry",
        str(telemetry),
    ]
    if args.completion_gate:
        argv.append("--completion-gate")
    if args.effort:
        argv += ["--effort", args.effort]
    argv += args.extra_args
    argv += ["-p", statement]
    return argv


def run_harness(
    args: argparse.Namespace, workspace: Path, run_dir: Path, statement: str
) -> tuple[int, float, dict | None]:
    telemetry = run_dir / "telemetry.jsonl"
    argv = butterfly_argv(args, workspace, telemetry, statement)
    (run_dir / "command.txt").write_text(
        " ".join(a if " " not in a else repr(a) for a in argv[:-2])
        + " -p <statement>\n"
    )
    env = {k: v for k, v in os.environ.items()}
    started = time.time()
    try:
        proc = subprocess.run(
            argv,
            cwd=workspace,
            capture_output=True,
            text=True,
            timeout=args.timeout,
            env=env,
        )
        code, out, err = proc.returncode, proc.stdout, proc.stderr
    except subprocess.TimeoutExpired as exc:
        code, out, err = (
            124,
            (exc.stdout or b"").decode(errors="replace")
            if isinstance(exc.stdout, bytes)
            else (exc.stdout or ""),
            "wall-clock timeout",
        )
    elapsed = time.time() - started
    (run_dir / "stdout.json").write_text(out)
    (run_dir / "stderr.log").write_text(err)
    report = None
    try:
        report = json.loads(out.strip().splitlines()[-1]) if out.strip() else None
    except json.JSONDecodeError:
        report = None
    return code, elapsed, report


def telemetry_metrics(path: Path, started_ms: int | None = None) -> dict:
    m = {
        "provider_calls": 0,
        "provider_errors": 0,
        "retries": 0,
        "tool_calls": 0,
        "edit_failures": 0,
        "command_timeouts": 0,
        "compactions": 0,
        "reread_after_edit": 0,
        "time_to_first_edit_s": None,
        "output_cap_hits": 0,
        "model_latency_s": 0.0,
        "tool_time_s": 0.0,
        "errors_by_kind": {},
        "cost_microusd": 0,
        "tokens_in": 0,
        "tokens_out": 0,
        "tokens_cached": 0,
        "cache_hit_rate": None,
        "max_call_latency_s": 0.0,
    }
    if not path.exists():
        return m
    first_ts = None
    last_edit_by_path: dict[str, int] = {}
    for line in path.read_text().splitlines():
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        ts = ev.get("at_epoch_ms")
        if first_ts is None and ts:
            first_ts = ts
        kind = ev.get("event")
        if kind == "provider_call":
            m["provider_calls"] += 1
            m["retries"] += ev.get("retries", 0)
            m["model_latency_s"] += ev.get("latency_ms", 0) / 1000
            m["max_call_latency_s"] = max(
                m["max_call_latency_s"], ev.get("latency_ms", 0) / 1000
            )
            m["cost_microusd"] += ev.get("actual_microusd", 0)
            usage = ev.get("usage") or {}
            m["tokens_in"] += usage.get("input_tokens", 0)
            m["tokens_out"] += usage.get("output_tokens", 0)
            m["tokens_cached"] += usage.get("cache_hit_input_tokens", 0)
            if ev.get("finish") == "length":
                m["output_cap_hits"] += 1
        elif kind == "provider_error":
            m["provider_errors"] += 1
            k = str(ev.get("kind"))
            m["errors_by_kind"][k] = m["errors_by_kind"].get(k, 0) + 1
        elif kind == "tool_call":
            m["tool_calls"] += 1
            m["tool_time_s"] += ev.get("duration_ms", 0) / 1000
            name, _, rest = ev.get("preview", "").partition("(")
            target = rest[:-1] if rest.endswith(")") else rest
            if name in EDIT_TOOLS:
                if m["time_to_first_edit_s"] is None and first_ts and ts:
                    m["time_to_first_edit_s"] = round((ts - first_ts) / 1000, 1)
                if not ev.get("ok", True):
                    m["edit_failures"] += 1
                else:
                    last_edit_by_path[target] = ts
            elif name == "read_file" and target in last_edit_by_path:
                m["reread_after_edit"] += 1
            if name == "run_command" and str(ev.get("failure")) == "Timeout":
                m["command_timeouts"] += 1
        elif kind == "compaction":
            m["compactions"] += 1
    m["model_latency_s"] = round(m["model_latency_s"], 1)
    if m["tokens_in"]:
        m["cache_hit_rate"] = round(m["tokens_cached"] / m["tokens_in"], 3)
    m["tool_time_s"] = round(m["tool_time_s"], 1)
    return m


# ---------------------------------------------------------------- one trial


def trial(args: argparse.Namespace, repo: Path, task: Task, run: int) -> dict:
    run_dir = Path(args.out) / task.id / f"run{run}"
    run_dir.mkdir(parents=True, exist_ok=True)
    workspace = Path(args.work_dir) / f"{task.id}-run{run}"
    service_dir = workspace / task.service
    result: dict = {
        "task": task.id,
        "run": run,
        "model": args.model,
        "service": task.service,
        "mode": "oracle" if args.oracle else "harness",
    }
    try:
        build_workspace(repo, task, workspace)
        sync_env(service_dir, args.check_timeout)
        statement = STATEMENT_TEMPLATE.format(
            statement=task.statement.strip(), service=task.service
        )
        (run_dir / "statement.txt").write_text(statement + "\n")

        if args.oracle:
            started = time.time()
            apply_oracle(repo, task, workspace)
            code, elapsed, report = 0, time.time() - started, None
            metrics = telemetry_metrics(run_dir / "telemetry.jsonl")
        else:
            code, elapsed, report = run_harness(args, workspace, run_dir, statement)
            metrics = telemetry_metrics(run_dir / "telemetry.jsonl")

        # What the agent changed, before scoring touches the tree.
        diff = git(workspace, "diff", "--stat", check=False).decode(errors="replace")
        (run_dir / "diff.txt").write_bytes(
            git(workspace, "diff", check=False)
            + git(workspace, "status", "--short", check=False)
        )
        files_changed = len(
            [
                line
                for line in git(workspace, "status", "--short", check=False)
                .decode()
                .splitlines()
                if line.strip()
            ]
        )

        apply_hidden_tests(repo, task, workspace)
        pcode, statuses, output = run_pytest(service_dir, args.check_timeout)
        (run_dir / "hidden-check.log").write_text(output)
        hidden = {t: statuses.get(t, "MISSING") for t in task.hidden_test_ids}
        hidden_pass = all(s == "PASSED" for s in hidden.values())
        hidden_dirs = {f.split("/", 1)[1] for f in task.hidden_tests}
        regressions = sorted(
            t
            for t, s in statuses.items()
            if s != "PASSED"
            and t not in task.hidden_test_ids
            and t not in task.baseline_failing
            and t.split("::", 1)[0] not in hidden_dirs
        )
        claimed = None
        if report is not None:
            # Butterfly's own verdict: the loop accepted a completion claim and its checks passed.
            claimed = bool(report.get("completed")) and bool(
                report.get("checks_passed")
            )
        result.update(
            {
                "pass": hidden_pass and not regressions,
                "hidden_pass": hidden_pass,
                "hidden_passed": sum(1 for s in hidden.values() if s == "PASSED"),
                "hidden_total": len(hidden),
                "regressions": regressions,
                "claimed_done": claimed,
                "false_completion": bool(claimed) and not hidden_pass,
                "exit_code": code,
                "wall_clock_s": round(elapsed, 1),
                "files_changed": files_changed,
                "diff_stat": diff.strip().splitlines()[-1] if diff.strip() else "",
                **metrics,
            }
        )
        if report is not None:
            result["attempts"] = (
                len(report.get("attempts", []))
                if isinstance(report.get("attempts"), list)
                else report.get("attempts")
            )
            result["report_status"] = report.get("status")
            result["report_verified"] = report.get("verified")
            result["verification_source"] = report.get("verification_source")
            result["self_authored_checks"] = sum(
                a.get("self_authored_checks", 0)
                for a in report.get("attempts", [])
                if isinstance(a, dict)
            )
    except Exception as exc:  # noqa: BLE001
        result.update({"pass": False, "error": f"{type(exc).__name__}: {exc}"})
    finally:
        if not args.keep:
            shutil.rmtree(workspace, ignore_errors=True)
    with open(Path(args.out) / "results.jsonl", "a") as fh:
        fh.write(json.dumps(result) + "\n")
    mark = "PASS" if result.get("pass") else "fail"
    say(
        f"{mark} {task.id} run{run} {result.get('wall_clock_s', 0):6.0f}s hidden {result.get('hidden_passed', '?')}/{result.get('hidden_total', '?')} "
        f"reg={len(result.get('regressions', []))} calls={result.get('provider_calls', 0)} ${(result.get('cost_microusd') or 0) / 1e6:.3f}"
        + (f"  ERROR {result['error']}" if "error" in result else "")
    )
    return result


def summarize(out: Path) -> None:
    rows = [
        json.loads(line)
        for line in (out / "results.jsonl").read_text().splitlines()
        if line.strip()
    ]
    if not rows:
        return
    n = len(rows)
    passed = sum(1 for r in rows if r.get("pass"))
    cost = sum(r.get("cost_microusd") or 0 for r in rows) / 1e6
    fc = sum(1 for r in rows if r.get("false_completion"))
    reg = sum(1 for r in rows if r.get("regressions"))

    def med(key: str) -> float:
        return sorted(r.get(key) or 0 for r in rows)[n // 2]

    print(
        f"\n{out.name}: {passed}/{n} pass  false completions {fc}  regressions {reg}  cost ${cost:.2f} "
        f"(${cost / max(passed, 1):.3f} per success)  median wall {med('wall_clock_s'):.0f}s  median calls {med('provider_calls')}  "
        f"median time-to-first-edit {med('time_to_first_edit_s'):.0f}s"
    )


def main() -> int:
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    p.add_argument("--tasks", required=True)
    p.add_argument(
        "--repo",
        required=True,
        help="checkout of the private repository (read-only use)",
    )
    p.add_argument(
        "--model",
        default="oracle",
        help="Harbor form provider/model, e.g. zai/glm-5.3-flash",
    )
    p.add_argument("--out", required=True)
    p.add_argument(
        "--work-dir",
        default=None,
        help="where workspaces are built (default <out>/work)",
    )
    p.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    p.add_argument("--runs", type=int, default=1)
    p.add_argument("--concurrency", type=int, default=2)
    p.add_argument("--only", help="comma-separated task ids")
    p.add_argument(
        "--tier", help="comma-separated tiers to include (small,medium,large)"
    )
    p.add_argument(
        "--timeout", type=int, default=1200, help="wall-clock seconds per trial"
    )
    p.add_argument("--check-timeout", type=int, default=600)
    p.add_argument("--max-attempts", type=int, default=3)
    p.add_argument("--max-calls", type=int, default=300)
    p.add_argument("--max-cost-usd", default="1.00")
    p.add_argument("--effort")
    p.add_argument("--no-completion-gate", dest="completion_gate", action="store_false")
    p.add_argument("--extra-args", nargs="*", default=[])
    p.add_argument(
        "--oracle",
        action="store_true",
        help="apply the reference fix instead of running the harness",
    )
    p.add_argument("--keep", action="store_true", help="keep workspaces after scoring")
    args = p.parse_args()

    data = json.loads(Path(args.tasks).read_text())
    if not args.oracle:
        lowered = args.model.lower()
        for forbidden in data.get("forbidden_models", ["muse"]):
            if forbidden in lowered:
                print(
                    f"refusing model {args.model!r}: {forbidden} is forbidden for the private suite",
                    file=sys.stderr,
                )
                return 2
        provider = args.model.split("/", 1)[0]
        key_env = PROVIDER_KEY_ENV.get(provider)
        if key_env and not os.environ.get(key_env):
            print(
                f"{key_env} is not set; run through bench/harbor/with-keys.sh",
                file=sys.stderr,
            )
            return 2
        if not args.binary.is_file():
            print(f"binary not found: {args.binary}", file=sys.stderr)
            return 2

    only = set(args.only.split(",")) if args.only else None
    tiers = set(args.tier.split(",")) if args.tier else None
    tasks = [
        Task(**{k: t[k] for k in Task.__dataclass_fields__})
        for t in data["tasks"]
        if (not only or t["id"] in only) and (not tiers or t.get("tier") in tiers)
    ]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    args.work_dir = args.work_dir or str(out / "work")
    (out / "config.json").write_text(
        json.dumps(
            {k: (str(v) if isinstance(v, Path) else v) for k, v in vars(args).items()},
            indent=2,
        )
        + "\n"
    )
    repo = Path(args.repo).resolve()
    say(
        f"{len(tasks)} tasks x {args.runs} runs, model {args.model}, concurrency {args.concurrency} -> {out}"
    )
    jobs = [(t, r) for r in range(1, args.runs + 1) for t in tasks]
    with ThreadPoolExecutor(max_workers=args.concurrency) as pool:
        list(pool.map(lambda job: trial(args, repo, *job), jobs))
    summarize(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())

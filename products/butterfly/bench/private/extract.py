#!/usr/bin/env python3
"""Build SWE-bench-style tasks from a private repository's own history.

A task is a commit that changed source and tests together inside one service. The agent
starts at the parent commit and gets the commit's intent as its statement; the commit's
test changes are the hidden check, copied in only after the agent finishes. Nothing from
the private repository is stored here except commit hashes, paths, and the statement text
you choose to keep; the checkout is resolved at run time from `--repo`.

    extract.py scan     --repo PATH --name de-services [--max-commits N] -o candidates.json
    extract.py validate --repo PATH --tasks candidates.json --work-dir DIR -o tasks.json

`scan` lists commits whose diff touches at least one test file and one non-test source
file under the same service directory (a directory with `pyproject.toml`).

`validate` proves each candidate is a real task, with no model involved:

    1. worktree at the parent commit; the service's tests pass (`uv run pytest`);
    2. with the hidden test files checked out from the commit, pytest fails;
    3. with the whole commit applied, pytest passes.

The test node ids that fail in step 2 and pass in step 3 are recorded as the task's hidden
checks. Candidates that fail any step are kept in the output under `rejected` with the reason
so the selection is auditable. Stdlib only; no network beyond `uv sync`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path

TEST_PATH = re.compile(
    r"(^|/)(tests?|__tests__)/|(^|/)test_[^/]*\.py$|_test\.py$|(^|/)conftest\.py$"
)
SOURCE_SUFFIXES = {".py"}
FORBIDDEN_MODELS = (
    "muse",
)  # runner refuses these; kept here so tasks.json carries the rule


def git(repo: Path, *args: str, check: bool = True) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True
    )
    if check and result.returncode != 0:
        raise RuntimeError(
            f"git {' '.join(args)} failed in {repo}: {result.stderr.strip()}"
        )
    return result.stdout


@dataclass
class Candidate:
    id: str
    repo: str
    service: str
    commit: str
    parent: str
    date: str
    subject: str
    body: str
    hidden_tests: list[str]
    source_files: list[str]
    other_files: list[str]
    statement: str = ""
    statement_source: str = "commit"  # commit | rewritten
    hidden_test_ids: list[str] = field(default_factory=list)
    baseline_passed: int = 0
    baseline_failing: list[str] = field(default_factory=list)
    notes: str = ""


def service_of(path: str, services: set[str]) -> str | None:
    top = path.split("/", 1)[0]
    return top if top in services else None


def scan(args: argparse.Namespace) -> None:
    repo = Path(args.repo).resolve()
    services = {p.parent.name for p in repo.glob("*/pyproject.toml")}
    log = git(
        repo,
        "log",
        *([] if args.all_commits else ["--first-parent"]),
        "--no-merges",
        f"-{args.max_commits}",
        "--format=%x1e%H%x1f%P%x1f%as%x1f%s%x1f%b",
        args.rev,
    )
    candidates: list[Candidate] = []
    for record in log.split("\x1e")[1:]:
        sha, parents, date, subject, body = record.split("\x1f", 4)
        parent = parents.split()[0] if parents else ""
        if not parent:
            continue
        files = git(repo, "diff", "--name-only", f"{parent}", sha).split()
        by_service: dict[str, dict[str, list[str]]] = {}
        for f in files:
            svc = service_of(f, services)
            if svc is None:
                continue
            bucket = by_service.setdefault(
                svc, {"tests": [], "source": [], "other": []}
            )
            if TEST_PATH.search(f):
                bucket["tests"].append(f)
            elif Path(f).suffix in SOURCE_SUFFIXES:
                bucket["source"].append(f)
            else:
                bucket["other"].append(f)
        # One service per task; commits spanning services are ambiguous statements.
        touched = [s for s, b in by_service.items() if b["tests"] or b["source"]]
        if len(touched) != 1:
            continue
        svc = touched[0]
        bucket = by_service[svc]
        if not bucket["tests"] or not bucket["source"]:
            continue
        subject_clean = re.sub(r"\s*\(#\d+\)\s*$", "", subject).strip()
        candidates.append(
            Candidate(
                id=f"{args.name}-{sha[:10]}",
                repo=args.name,
                service=svc,
                commit=sha,
                parent=parent,
                date=date,
                subject=subject_clean,
                body=body.strip(),
                hidden_tests=sorted(bucket["tests"]),
                source_files=sorted(bucket["source"]),
                other_files=sorted(bucket["other"]),
                statement=subject_clean
                + ("\n\n" + body.strip() if body.strip() else ""),
            )
        )
    out = {
        "repo": args.name,
        "scanned_rev": args.rev,
        "forbidden_models": FORBIDDEN_MODELS,
        "candidates": [asdict(c) for c in candidates],
    }
    Path(args.output).write_text(json.dumps(out, indent=2) + "\n")
    print(
        f"{len(candidates)} candidates from {args.max_commits} commits -> {args.output}"
    )
    for c in candidates:
        print(
            f"  {c.id}  {c.date}  {c.service:28} tests={len(c.hidden_tests)} src={len(c.source_files)}  {c.subject[:60]}"
        )


# ---------------------------------------------------------------- validation

PYTEST_ARGS = ["-q", "--no-header", "-p", "no:cacheprovider", "-rA", "--tb=no"]
RESULT_LINE = re.compile(r"^(PASSED|FAILED|ERROR) (\S+)")


def run_pytest(service_dir: Path, timeout: int) -> tuple[int, dict[str, str], str]:
    """Return (exit code, {node id: status}, tail of output)."""
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
    statuses: dict[str, str] = {}
    for line in result.stdout.splitlines():
        m = RESULT_LINE.match(line)
        if m:
            statuses[m.group(2)] = m.group(1)
    tail = "\n".join((result.stdout + result.stderr).splitlines()[-15:])
    return result.returncode, statuses, tail


def worktree(repo: Path, work_dir: Path, name: str, rev: str) -> Path:
    path = work_dir / name
    git(repo, "worktree", "prune", check=False)
    if path.exists():
        git(repo, "worktree", "remove", "--force", str(path), check=False)
        shutil.rmtree(path, ignore_errors=True)
    git(repo, "worktree", "add", "--detach", str(path), rev)
    return path


def drop_worktree(repo: Path, path: Path) -> None:
    git(repo, "worktree", "remove", "--force", str(path), check=False)
    shutil.rmtree(path, ignore_errors=True)


def validate_one(
    repo: Path, work_dir: Path, c: Candidate, timeout: int
) -> tuple[bool, str]:
    wt = worktree(repo, work_dir, c.id, c.parent)
    svc = wt / c.service
    try:
        if not (svc / "pyproject.toml").exists():
            return False, "no pyproject at parent"
        # Exit 5 = no tests collected: the commit may add the service's first tests.
        # Exit 1 = pre-existing failures: tolerated, recorded, excluded from regression checks.
        code, base, tail = run_pytest(svc, timeout)
        if code not in (0, 1, 5):
            return (
                False,
                f"baseline pytest exit {code}: {tail.splitlines()[-1] if tail else ''}",
            )
        c.baseline_passed = sum(1 for s in base.values() if s == "PASSED")
        c.baseline_failing = sorted(t for t, s in base.items() if s != "PASSED")

        # Step 2: hidden tests only. Files deleted by the commit are removed.
        for f in c.hidden_tests:
            exists_after = (
                subprocess.run(
                    ["git", "-C", str(repo), "cat-file", "-e", f"{c.commit}:{f}"],
                    capture_output=True,
                ).returncode
                == 0
            )
            if exists_after:
                git(wt, "checkout", c.commit, "--", f)
            else:
                (wt / f).unlink(missing_ok=True)
        code, with_tests, tail = run_pytest(svc, timeout)
        if code == 0:
            return (
                False,
                "hidden tests already pass at parent (test-only or refactor commit)",
            )
        if code not in (1, 2):
            return (
                False,
                f"pytest exit {code} with hidden tests: {tail.splitlines()[-1] if tail else ''}",
            )
        passing_before = {t for t, s in with_tests.items() if s == "PASSED"}

        # Step 3: full commit applied to the service.
        git(wt, "checkout", c.commit, "--", c.service)
        for f in git(
            repo, "diff", "--name-only", "--diff-filter=D", c.parent, c.commit
        ).split():
            if f.startswith(c.service + "/"):
                (wt / f).unlink(missing_ok=True)
        code, after, tail = run_pytest(svc, timeout)
        if code != 0:
            return (
                False,
                f"reference solution fails pytest exit {code}: {tail.splitlines()[-1] if tail else ''}",
            )
        # Hidden checks: tests in the hidden files that pass after the commit and did not
        # pass before it (failed, errored, or did not exist / could not be collected).
        hidden_files = set(c.hidden_tests)
        flipped = sorted(
            t
            for t, s in after.items()
            if s == "PASSED"
            and t.split("::", 1)[0] in {f.split("/", 1)[1] for f in hidden_files}
            and t not in passing_before
        )
        if not flipped:
            return False, "no test flips fail->pass"
        c.hidden_test_ids = flipped
        return True, ""
    finally:
        drop_worktree(repo, wt)


def validate(args: argparse.Namespace) -> None:
    repo = Path(args.repo).resolve()
    work_dir = Path(args.work_dir).resolve()
    work_dir.mkdir(parents=True, exist_ok=True)
    data = json.loads(Path(args.tasks).read_text())
    only = set(args.only.split(",")) if args.only else None
    accepted, rejected = [], []
    for raw in data["candidates"]:
        c = Candidate(**raw)
        if only and c.id not in only:
            continue
        started = time.time()
        try:
            ok, reason = validate_one(repo, work_dir, c, args.timeout)
        except Exception as exc:  # noqa: BLE001 - one bad candidate must not stop the batch
            ok, reason = False, f"exception: {exc}"
        elapsed = time.time() - started
        mark = "ok " if ok else "REJ"
        print(
            f"{mark} {c.id} {c.service:28} {elapsed:5.0f}s  {reason or f'{len(c.hidden_test_ids)} hidden checks, {c.baseline_passed} baseline'}",
            flush=True,
        )
        (accepted if ok else rejected).append(
            {**asdict(c), **({} if ok else {"reason": reason})}
        )
    out = {
        "repo": data["repo"],
        "forbidden_models": FORBIDDEN_MODELS,
        "tasks": accepted,
        "rejected": rejected,
    }
    Path(args.output).write_text(json.dumps(out, indent=2) + "\n")
    print(f"{len(accepted)} tasks, {len(rejected)} rejected -> {args.output}")


def main() -> None:
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("scan")
    s.add_argument("--repo", required=True)
    s.add_argument("--name", required=True, help="short repo name stored in task ids")
    s.add_argument("--rev", default="HEAD")
    s.add_argument("--max-commits", type=int, default=400)
    s.add_argument(
        "--all-commits",
        action="store_true",
        help="walk every commit, not just first-parent squashes",
    )
    s.add_argument("-o", "--output", default="candidates.json")
    s.set_defaults(func=scan)
    v = sub.add_parser("validate")
    v.add_argument("--repo", required=True)
    v.add_argument("--tasks", required=True)
    v.add_argument("--work-dir", required=True)
    v.add_argument("--timeout", type=int, default=600, help="seconds per pytest run")
    v.add_argument("--only", help="comma-separated task ids")
    v.add_argument("-o", "--output", default="tasks.json")
    v.set_defaults(func=validate)
    args = p.parse_args()
    args.func(args)


if __name__ == "__main__":
    sys.exit(main())

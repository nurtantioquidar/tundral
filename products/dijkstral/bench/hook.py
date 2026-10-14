#!/usr/bin/env python3
"""Hook-guard latency: Graphify vs dijkstral, one fresh process per PreToolUse call.

    uv run --no-project bench/hook.py                     # both tools, 50 timed runs per case
    uv run --no-project bench/hook.py --tools dijkstral --runs 200

Every case pipes one representative Claude Code PreToolUse payload into
`<tool> hook-guard search|read` (or the Codex `hook-check`) from a scratch project that holds the
python fixture sources and a graph in both `graphify-out/` and `dijkstral-out/`, exactly as the
assistant would: a new process per tool call, wall clock around `subprocess.run`. Each case runs
`--warmup` untimed calls first (page cache warm). The decision each tool returned (silent, nudge,
stale, deny) is recorded too, so a latency number is never reported for a wrong answer. The
exec floor (`/usr/bin/true`, interleaved with the tools run by run) shows what any process
spawn costs on the machine.
Results go to `bench/results/hook-<epoch>.json`. Stdlib only; no network.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CLI = HERE.parent / "cli"
BENCH_HOME = Path(
    os.environ.get("DIJKSTRAL_BENCH_HOME", Path.home() / ".cache/dijkstral-bench")
)
GRAPHIFY_VERSION = os.environ.get("GRAPHIFY_VERSION", "0.9.86")
FIXTURE = CLI / "tests" / "fixtures" / "python"
FIXTURE_GRAPH = CLI / "tests" / "fixtures" / "python.clustered.graphify.json"
OUT_DIRS = {"graphify": "graphify-out", "dijkstral": "dijkstral-out"}
# The cheapest process to spawn: what any hook costs on this machine before doing anything.
FLOOR = Path("/usr/bin/true")


def tool_bins() -> dict[str, Path]:
    default = CLI / "target" / "release" / "dijkstral"
    return {
        "dijkstral": Path(os.environ.get("DIJKSTRAL_BIN", default)),
        "graphify": BENCH_HOME
        / "tools"
        / f"graphify-{GRAPHIFY_VERSION}"
        / "bin"
        / "graphify",
    }


def make_project(root: Path, with_graph: bool) -> Path:
    proj = root / ("proj" if with_graph else "nograph")
    shutil.copytree(FIXTURE, proj)
    old = time.time() - 3600
    for p in proj.rglob("*"):
        os.utime(p, (old, old))
    (proj / "package.json").write_text("{}\n")
    if with_graph:
        for out in OUT_DIRS.values():
            (proj / out).mkdir()
            shutil.copy(FIXTURE_GRAPH, proj / out / "graph.json")
    return proj


def cases(proj: Path, nograph: Path) -> list[dict]:
    service = str(proj / "pkg" / "service.py")

    def call(tool: str, tool_input: dict, session: str = "bench") -> dict:
        return {
            "session_id": session,
            "transcript_path": "/dev/null",
            "cwd": str(proj),
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": tool,
            "tool_input": tool_input,
            "tool_use_id": "toolu_bench",
        }

    bash = {"description": "bench", "timeout": 120000, "run_in_background": False}
    return [
        {
            "case": "search: Bash `git status` (silent)",
            "argv": ["hook-guard", "search"],
            "payload": call("Bash", {"command": "git status", **bash}),
        },
        {
            "case": "search: Bash `rg` pipeline (nudge)",
            "argv": ["hook-guard", "search"],
            "payload": call(
                "Bash", {"command": "cd pkg && rg -n 'def ' . | head -20", **bash}
            ),
        },
        {
            "case": "search: Grep (nudge)",
            "argv": ["hook-guard", "search"],
            "payload": call(
                "Grep",
                {"pattern": "class .*Service", "output_mode": "content", "-i": False},
            ),
        },
        {
            "case": "read: Read .py (nudge)",
            "argv": ["hook-guard", "read"],
            "payload": call("Read", {"file_path": service}),
        },
        {
            "case": "read: Read .json (silent)",
            "argv": ["hook-guard", "read"],
            "payload": call("Read", {"file_path": str(proj / "package.json")}),
        },
        {
            "case": "read: Glob **/*.py (nudge)",
            "argv": ["hook-guard", "read"],
            "payload": call("Glob", {"pattern": "**/*.py"}),
        },
        {
            "case": "read --strict: Read .py, session already denied (nudge)",
            "argv": ["hook-guard", "read", "--strict"],
            "payload": call("Read", {"file_path": service}, session="bench-strict"),
        },
        {
            "case": "search: Grep, no graph (silent)",
            "argv": ["hook-guard", "search"],
            "payload": call("Grep", {"pattern": "x"}),
            "cwd": nograph,
        },
        {
            "case": "hook-check (Codex no-op)",
            "argv": ["hook-check"],
            "payload": call("Bash", {"command": "rg x", **bash}),
        },
    ]


def outcome(stdout: bytes) -> str:
    if not stdout.strip():
        return "silent"
    out = json.loads(stdout)["hookSpecificOutput"]
    if out.get("permissionDecision") == "deny":
        return "deny"
    text = out.get("additionalContext", "")
    return "stale" if "stale" in text or "out of date" in text else "nudge"


def p95(xs: list[float]) -> float:
    return statistics.quantiles(xs, n=100, method="inclusive")[94]


def run_once(argv: list[str], data: bytes, cwd: Path, env: dict) -> tuple[float, str]:
    t0 = time.perf_counter_ns()
    r = subprocess.run(
        argv, cwd=cwd, input=data, capture_output=True, env=env, check=False
    )
    dt = (time.perf_counter_ns() - t0) / 1e6
    if r.returncode != 0 or r.stderr:
        sys.exit(
            f"{argv} failed in {cwd}: exit {r.returncode}, stderr {r.stderr[:200]!r}"
        )
    return dt, outcome(r.stdout)


def summary(samples: list[float], seen: set[str]) -> dict:
    return {
        "outcome": "/".join(sorted(seen)),
        "p50_ms": round(statistics.median(samples), 3),
        "p95_ms": round(p95(samples), 3),
        "mean_ms": round(statistics.fmean(samples), 3),
        "min_ms": round(min(samples), 3),
        "samples_ms": [round(s, 3) for s in samples],
    }


def measure_case(
    bins: dict[str, Path],
    tools: list[str],
    case: dict,
    cwd: Path,
    runs: int,
    warmup: int,
    env: dict,
) -> dict[str, dict]:
    """Runs every tool and the exec floor round-robin, so machine drift hits all of them alike."""
    data = json.dumps(case["payload"]).encode()
    argvs = {t: [str(bins[t]), *case["argv"]] for t in tools}
    argvs["floor"] = [str(FLOOR)]
    samples: dict[str, list[float]] = {t: [] for t in argvs}
    seen: dict[str, set[str]] = {t: set() for t in argvs}
    for i in range(warmup + runs):
        for t, argv in argvs.items():
            dt, kind = run_once(argv, data, cwd, env)
            seen[t].add(kind)
            if i >= warmup:
                samples[t].append(dt)
    return {t: summary(samples[t], seen[t]) for t in argvs}


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--tools",
        default="graphify,dijkstral",
        help="comma-separated: graphify, dijkstral",
    )
    ap.add_argument(
        "--runs", type=int, default=50, help="timed fresh-process runs per case (>= 50)"
    )
    ap.add_argument("--warmup", type=int, default=5)
    args = ap.parse_args()
    if args.runs < 50:
        sys.exit("--runs must be at least 50")
    bins = tool_bins()
    tools = [t.strip() for t in args.tools.split(",") if t.strip()]
    for t in tools:
        if t not in bins:
            sys.exit(f"unknown tool {t}")
        if not bins[t].exists():
            hint = (
                "run `cargo build --release` in cli/"
                if t == "dijkstral"
                else "run bench/setup.sh"
            )
            sys.exit(f"{t} not found at {bins[t]}; {hint}")

    env = dict(os.environ, GRAPHIFY_QUERY_LOG_DISABLE="1")
    results = []
    with tempfile.TemporaryDirectory(prefix="dijkstral-hook-") as tmp:
        root = Path(tmp).resolve()
        proj, nograph = make_project(root, True), make_project(root, False)
        for tool in tools:
            # The strict case measures the steady state: this session's one denial has happened.
            sessions = proj / OUT_DIRS[tool] / "cache" / "hook_sessions"
            sessions.mkdir(parents=True, exist_ok=True)
            (sessions / "bench-strict.denied").touch()
        for case in cases(proj, nograph):
            cwd = case.get("cwd", proj)
            by_tool = measure_case(bins, tools, case, cwd, args.runs, args.warmup, env)
            floor = by_tool["floor"]["p50_ms"]
            for tool, r in by_tool.items():
                results.append(
                    {"case": case["case"], "tool": tool, "argv": case["argv"], **r}
                )
                if tool == "floor":
                    continue
                print(
                    f"{case['case']:<58} {tool:<10} {r['outcome']:<7} "
                    f"p50 {r['p50_ms']:7.2f} ms  p95 {r['p95_ms']:7.2f} ms  "
                    f"(+{r['p50_ms'] - floor:6.2f} ms over exec floor {floor:.2f})",
                    flush=True,
                )

    versions = {}
    for t in tools:
        out = subprocess.run(
            [str(bins[t]), "--version"],
            capture_output=True,
            text=True,
            check=False,
            env=env,
        )
        versions[t] = {
            "bin": str(bins[t]),
            "version": (out.stdout or out.stderr).strip(),
        }
    doc = {
        "kind": "hook-latency",
        "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "machine": {
            "platform": platform.platform(),
            "processor": platform.processor() or platform.machine(),
            "cpus": os.cpu_count(),
            "python": platform.python_version(),
        },
        "method": "fresh process per call, wall clock around subprocess.run, "
        f"{args.warmup} warm-up + {args.runs} timed runs per case, tools and the exec floor "
        "(/usr/bin/true) interleaved run by run; p95 linear-interpolated",
        "tools": versions,
        "results": results,
    }
    out_dir = HERE / "results"
    out_dir.mkdir(exist_ok=True)
    path = out_dir / f"hook-{int(time.time())}.json"
    path.write_text(json.dumps(doc, indent=2) + "\n")
    print(f"wrote {path}")


if __name__ == "__main__":
    main()

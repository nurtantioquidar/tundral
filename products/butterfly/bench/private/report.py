#!/usr/bin/env python3
"""Summarise and compare private-suite runs.

    report.py runs/glm-baseline                 # per-task table + totals for one run
    report.py runs/glm-baseline runs/glm-v2     # side-by-side totals for several runs
    report.py --tasks tasks.json runs/...       # adds the tier column

Verdicts per trial: PASS (hidden checks pass, no regressions), FC (false completion: the harness
claimed done but hidden checks failed), REG (hidden checks pass but a baseline test broke),
TIME (wall-clock cap), fail (anything else).
"""

from __future__ import annotations

import argparse
import json
import statistics
from collections import defaultdict
from pathlib import Path


def load(run_dir: Path) -> list[dict]:
    f = run_dir / "results.jsonl"
    return (
        [json.loads(line) for line in f.read_text().splitlines() if line.strip()]
        if f.exists()
        else []
    )


def verdict(r: dict) -> str:
    if r.get("pass"):
        return "PASS"
    if r.get("exit_code") == 124:
        return "TIME"
    if r.get("hidden_pass") and r.get("regressions"):
        return "REG"
    if r.get("false_completion"):
        return "FC"
    return "fail"


def median(rows: list[dict], key: str) -> float:
    vals = [r.get(key) or 0 for r in rows]
    return statistics.median(vals) if vals else 0.0


def totals(rows: list[dict]) -> dict:
    n = len(rows)
    passed = sum(1 for r in rows if r.get("pass"))
    cost = sum(r.get("cost_microusd") or 0 for r in rows) / 1e6
    return {
        "trials": n,
        "pass": passed,
        "pass_rate": f"{100 * passed / n:.0f}%" if n else "-",
        "false_completions": sum(1 for r in rows if verdict(r) == "FC"),
        "regressions": sum(1 for r in rows if r.get("regressions")),
        "timeouts": sum(1 for r in rows if verdict(r) == "TIME"),
        "cost_usd": f"{cost:.2f}",
        "usd_per_success": f"{cost / passed:.3f}" if passed else "-",
        "median_wall_s": f"{median(rows, 'wall_clock_s'):.0f}",
        "median_calls": f"{median(rows, 'provider_calls'):.0f}",
        "median_first_edit_s": f"{median(rows, 'time_to_first_edit_s'):.0f}",
        "median_cache_hit": f"{100 * median(rows, 'cache_hit_rate'):.0f}%",
        "model_time_share": f"{100 * sum(r.get('model_latency_s') or 0 for r in rows) / max(sum(r.get('wall_clock_s') or 0 for r in rows), 1):.0f}%",
        "rereads_per_trial": f"{sum(r.get('reread_after_edit') or 0 for r in rows) / max(n, 1):.1f}",
        "edit_failures": sum(r.get("edit_failures") or 0 for r in rows),
        "command_timeouts": sum(r.get("command_timeouts") or 0 for r in rows),
        "output_cap_hits": sum(r.get("output_cap_hits") or 0 for r in rows),
        "provider_errors": sum(r.get("provider_errors") or 0 for r in rows),
        "max_call_s": f"{max((r.get('max_call_latency_s') or 0 for r in rows), default=0):.0f}",
    }


def per_task(rows: list[dict], tiers: dict[str, str]) -> None:
    by: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        by[r["task"]].append(r)
    print(
        f"{'task':28} {'tier':7} {'runs':18} {'wall s':>12} {'calls':>9} {'1st edit':>9} {'cost':>6} {'max call':>8}"
    )
    for task, rs in sorted(by.items(), key=lambda kv: (tiers.get(kv[0], ""), kv[0])):
        rs.sort(key=lambda r: r["run"])
        print(
            f"{task:28} {tiers.get(task, ''):7} {' '.join(f'{verdict(r):4}' for r in rs):18} "
            f"{'/'.join(f'{r.get('wall_clock_s') or 0:.0f}' for r in rs):>12} "
            f"{'/'.join(str(r.get('provider_calls') or 0) for r in rs):>9} "
            f"{'/'.join(f'{r.get('time_to_first_edit_s') or 0:.0f}' for r in rs):>9} "
            f"{sum(r.get('cost_microusd') or 0 for r in rs) / 1e6:6.3f} "
            f"{max(r.get('max_call_latency_s') or 0 for r in rs):8.0f}"
        )


def main() -> None:
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    p.add_argument("runs", nargs="+", type=Path)
    p.add_argument("--tasks", type=Path)
    args = p.parse_args()
    tiers = {}
    if args.tasks:
        tiers = {
            t["id"]: t.get("tier", "")
            for t in json.loads(args.tasks.read_text())["tasks"]
        }
    if len(args.runs) == 1:
        rows = load(args.runs[0])
        per_task(rows, tiers)
        print()
        for k, v in totals(rows).items():
            print(f"{k:22} {v}")
        return
    cols = [(r.name, totals(load(r))) for r in args.runs]
    keys = list(cols[0][1].keys())
    print(f"{'':22} " + " ".join(f"{name[:16]:>16}" for name, _ in cols))
    for k in keys:
        print(f"{k:22} " + " ".join(f"{str(t[k]):>16}" for _, t in cols))


if __name__ == "__main__":
    main()

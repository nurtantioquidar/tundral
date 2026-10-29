"""Offline key-fact coverage of one `context` answer per question (stdlib only).

Runs `<binary> context "<question>" [extra args]` in each prepared dijkstral workspace (paths from
`~/.cache/dijkstral-bench/agent/manifests/<repo>.dijkstral.json`) for the key-fact questions,
scores the output with `score.score_facts`, and prints per-kind recall/pass, tuning (fastapi,
ripgrep) and the second tuning set (erpnext, jq, serilog, alamofire: flow and architecture
questions written like the blind set's, to tune for code `context` was not shaped on) against
held-out (kubernetes), and per-kind aggregates of the second set and the held-out one. Kubernetes
is never shown per question: tune on the tuning repos only.

    python3 context_eval.py <binary> [--low] [--blind] [--arg=ARG]...

`--arg` adds an argument after the question; `{repo}` in it becomes the repository name, so an
index per repository can be passed (`--arg=--index --arg=/tmp/dj-embed/{repo}/potion-code` for
`dijkstral-embed`, `cli/docs/dijkstral-context-semantic.md`). `--low` lists the tuning questions
(both sets) below half recall. Also prints the median wall time of a call per repository.

`--blind` also scores the blind set (okhttp, excalidraw: flow and architecture questions written
after the tuning, by an author who saw neither tool's output). It is printed apart, as aggregates
only, and never enters the lines above it; never tune on it.
"""

from __future__ import annotations

import argparse
import json
import os
import statistics as st
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import score

QUESTIONS = HERE / "questions"
MANIFESTS = Path(os.path.expanduser("~/.cache/dijkstral-bench/agent/manifests"))
REPOS = ("fastapi", "ripgrep", "erpnext", "jq", "serilog", "alamofire", "kubernetes")
TUNING = ("fastapi", "ripgrep")
TUNING_2 = ("erpnext", "jq", "serilog", "alamofire")
HELD_OUT = ("kubernetes",)
BLIND = ("okhttp", "excalidraw")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("--arg", action="append", default=[])
    ap.add_argument("--low", action="store_true")
    ap.add_argument("--blind", action="store_true")
    a = ap.parse_args()
    rows = []
    times: dict[str, list[float]] = {}
    for repo in REPOS + (BLIND if a.blind else ()):
        manifest = json.loads((MANIFESTS / f"{repo}.dijkstral.json").read_text())
        ws = manifest["workspace"]
        extra = [x.replace("{repo}", repo) for x in a.arg]
        for line in (QUESTIONS / f"{repo}.jsonl").read_text().splitlines():
            t = json.loads(line)
            cmd = [a.binary, "context", t["question"], *extra]
            t0 = time.perf_counter()
            out = subprocess.run(
                cmd, cwd=ws, capture_output=True, text=True, check=False
            ).stdout
            times.setdefault(repo, []).append(time.perf_counter() - t0)
            r = score.score_facts(t["facts"], out, t.get("min_facts"))
            rows.append((t["id"], t["kind"], r["recall"], r["pass"], len(out)))
    blind = [r for r in rows if r[0].split("-")[0] in BLIND]
    rows = [r for r in rows if r[0].split("-")[0] not in BLIND]
    by: dict[str, list] = {}
    for i, k, rec, p, n in rows:
        by.setdefault(k, []).append((rec, p, n))
    for k, v in sorted(by.items()):
        print(
            f"{k:13} recall {st.mean(x[0] for x in v):.2f} pass {sum(x[1] for x in v)}/{len(v)}"
            f" chars median {int(st.median(x[2] for x in v))}"
        )
    for grp, keep in (
        ("tuning", TUNING),
        ("tuning-2", TUNING_2),
        ("held-out", HELD_OUT),
    ):
        g = [r for r in rows if r[0].split("-")[0] in keep]
        print(
            f"{grp:13} recall {st.mean(r[2] for r in g):.2f} pass {sum(r[3] for r in g)}/{len(g)}"
        )
    print(
        f"{'ALL':13} recall {st.mean(r[2] for r in rows):.2f} pass"
        f" {sum(r[3] for r in rows)}/{len(rows)} chars median {int(st.median(r[4] for r in rows))}"
    )
    for repo, ts in times.items():
        print(
            f"time {repo:10} median {st.median(ts) * 1000:.0f} ms max {max(ts) * 1000:.0f} ms"
        )
    if a.low:
        for r in rows:
            if r[2] < 0.5 and r[0].split("-")[0] in TUNING + TUNING_2:
                print("  low:", r[0], r[1], round(r[2], 2), r[4])
    # The second tuning set and the held-out one per kind (held-out: aggregates only).
    for grp, keep in (("tuning-2", TUNING_2), ("held-out", HELD_OUT)):
        gk: dict[str, list] = {}
        for i, k, rec, p, n in rows:
            if i.split("-")[0] in keep:
                gk.setdefault(k, []).append((rec, p, n))
        for k, v in sorted(gk.items()):
            print(
                f"{grp} {k:13} recall {st.mean(x[0] for x in v):.2f} pass"
                f" {sum(x[1] for x in v)}/{len(v)} chars median {int(st.median(x[2] for x in v))}"
            )

    # Blind: aggregates only, never per question.
    if blind:
        print(
            f"{'blind':13} recall {st.mean(r[2] for r in blind):.2f} pass"
            f" {sum(r[3] for r in blind)}/{len(blind)} chars median"
            f" {int(st.median(r[4] for r in blind))}"
        )
        bk: dict[str, list] = {}
        for i, k, rec, p, n in blind:
            bk.setdefault(k, []).append((rec, p, n))
        for k, v in sorted(bk.items()):
            print(
                f"blind {k:16} recall {st.mean(x[0] for x in v):.2f} pass"
                f" {sum(x[1] for x in v)}/{len(v)} chars median {int(st.median(x[2] for x in v))}"
            )


if __name__ == "__main__":
    main()

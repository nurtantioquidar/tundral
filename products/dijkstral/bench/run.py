#!/usr/bin/env python3
"""Head-to-head build benchmark: Graphify vs dijkstral on the pinned corpus.

    uv run --no-project bench/run.py run --tools graphify --repos monet,fastapi
    uv run --no-project bench/run.py report results/build-<epoch>.json

Scenarios (each tool writes <repo>/<tool>-out/ inside the corpus checkout):
  cold    delete the tool's output and caches, then build from scratch
  warm    rebuild immediately with nothing changed (cache hit path)
  update  run one untimed update (steady state), append one comment line to the repo's
          `edit_file`, time the tool's incremental update, then restore the file. Note that
          Graphify's `update` graph also contains structural markdown nodes, which its
          `extract --code-only` graph does not, so update node counts differ from cold/warm.

Measured per run: wall time, CPU time (user+sys of the whole process tree), peak RSS of the
whole process tree (sampled every 50 ms, so worker pools are counted), and the largest single
process's max RSS. The OS page cache is warm for every timed run; "cold" means cold *tool* cache.
Stdlib only; no network, no paid calls.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import resource
import shutil
import statistics
import subprocess
import sys
import threading
import time
import tomllib
from dataclasses import asdict, dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
BENCH_HOME = Path(os.environ.get("DIJKSTRAL_BENCH_HOME", Path.home() / ".cache/dijkstral-bench"))
GRAPHIFY_VERSION = os.environ.get("GRAPHIFY_VERSION", "0.9.86")
SCENARIOS = ("cold", "warm", "update")


# ---------------------------------------------------------------------------------------------
# Tools


@dataclass
class Tool:
    name: str
    version: str
    out_dir: str  # directory name inside the corpus checkout
    env: dict[str, str] = field(default_factory=dict)

    def build(self, repo: Path, workers: int) -> list[str]:
        raise NotImplementedError

    def update(self, repo: Path, workers: int) -> list[str]:
        raise NotImplementedError

    def graph_json(self, repo: Path) -> Path:
        return repo / self.out_dir / "graph.json"


class Graphify(Tool):
    def __init__(self) -> None:
        self.bin = BENCH_HOME / "tools" / f"graphify-{GRAPHIFY_VERSION}" / "bin" / "graphify"
        if not self.bin.exists():
            sys.exit(f"graphify not installed at {self.bin}; run bench/setup.sh")
        super().__init__("graphify", GRAPHIFY_VERSION, "graphify-out", {"GRAPHIFY_QUERY_LOG_DISABLE": "1"})

    def build(self, repo: Path, workers: int) -> list[str]:
        self.env["GRAPHIFY_MAX_WORKERS"] = str(workers)
        return [str(self.bin), "extract", str(repo), "--code-only", "--max-workers", str(workers)]

    def update(self, repo: Path, workers: int) -> list[str]:
        self.env["GRAPHIFY_MAX_WORKERS"] = str(workers)
        return [str(self.bin), "update", str(repo)]


class Dijkstral(Tool):
    def __init__(self) -> None:
        default = HERE.parent / "cli" / "target" / "release" / "dijkstral"
        self.bin = Path(os.environ.get("DIJKSTRAL_BIN", default))
        if not self.bin.exists():
            sys.exit(f"dijkstral not built at {self.bin}; run `cargo build --release` in cli/")
        version = subprocess.run([self.bin, "--version"], check=False, capture_output=True, text=True).stdout.strip()
        super().__init__("dijkstral", version, "dijkstral-out")

    def build(self, repo: Path, workers: int) -> list[str]:
        return [str(self.bin), "extract", str(repo), "--code-only", "--threads", str(workers)]

    def update(self, repo: Path, workers: int) -> list[str]:
        return [str(self.bin), "update", str(repo), "--threads", str(workers)]


TOOLS = {"graphify": Graphify, "dijkstral": Dijkstral}


# ---------------------------------------------------------------------------------------------
# Measurement


def _tree_rss_kb(root: int) -> int:
    """Sum RSS (KiB) of `root` and all its descendants, from one `ps` snapshot."""
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,rss="], check=False, capture_output=True, text=True).stdout
    children: dict[int, list[int]] = {}
    rss: dict[int, int] = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) != 3:
            continue
        pid, ppid, kb = map(int, parts)
        rss[pid] = kb
        children.setdefault(ppid, []).append(pid)
    total, stack = 0, [root]
    while stack:
        pid = stack.pop()
        total += rss.get(pid, 0)
        stack.extend(children.get(pid, []))
    return total


@dataclass
class Sample:
    tool: str
    version: str
    repo: str
    commit: str
    scenario: str
    run: int
    workers: int
    exit_code: int
    wall_s: float
    cpu_s: float
    peak_tree_rss_mb: float
    max_proc_rss_mb: float
    nodes: int | None
    edges: int | None
    stderr_tail: str


def measure(cmd: list[str], env: dict[str, str], timeout: float) -> tuple[int, float, float, float, float, str]:
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    peak = 0
    start = time.perf_counter()
    proc = subprocess.Popen(cmd, env={**os.environ, **env}, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    done = threading.Event()

    def sampler() -> None:
        nonlocal peak
        while not done.is_set():
            peak = max(peak, _tree_rss_kb(proc.pid))
            done.wait(0.05)

    chunks: list[bytes] = []

    def drain() -> None:
        err = proc.stderr
        assert err is not None
        for chunk in iter(lambda: err.read(65536), b""):
            chunks.append(chunk)
            del chunks[:-4]  # keep only the tail

    threads = [threading.Thread(target=f, daemon=True) for f in (sampler, drain)]
    for t in threads:
        t.start()
    killer = threading.Timer(timeout, proc.kill)
    killer.start()
    # wait4 (not communicate) so ru_maxrss is this child's own peak, not the session's maximum.
    _, status, usage = os.wait4(proc.pid, 0)
    wall = time.perf_counter() - start
    killer.cancel()
    proc.returncode = os.waitstatus_to_exitcode(status)
    done.set()
    for t in threads:
        t.join()
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)
    # ru_maxrss is bytes on macOS and KiB on Linux.
    scale = 1 / (1024 * 1024) if sys.platform == "darwin" else 1 / 1024
    tail = b"".join(chunks).decode(errors="replace")[-2000:]
    return proc.returncode, wall, cpu, peak / 1024, usage.ru_maxrss * scale, tail


def graph_counts(path: Path) -> tuple[int | None, int | None]:
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError):
        return None, None
    return len(data.get("nodes", [])), len(data.get("links", data.get("edges", [])))


# ---------------------------------------------------------------------------------------------
# Runner


def load_corpus() -> list[dict]:
    return tomllib.loads((HERE / "corpus.toml").read_text())["repo"]


def run(args: argparse.Namespace) -> None:
    corpus = {r["name"]: r for r in load_corpus()}
    repos = [corpus[n] for n in args.repos.split(",")] if args.repos else list(corpus.values())
    tools = [TOOLS[t]() for t in args.tools.split(",")]
    scenarios = args.scenarios.split(",")
    samples: list[Sample] = []
    out = HERE / "results" / f"build-{int(time.time())}.json"
    out.parent.mkdir(exist_ok=True)

    for repo_cfg in repos:
        repo = BENCH_HOME / "corpus" / repo_cfg["name"]
        if not repo.is_dir():
            sys.exit(f"missing corpus checkout {repo}; run bench/setup.sh {repo_cfg['name']}")
        for tool in tools:
            for scenario in scenarios:
                for i in range(args.runs):
                    edit = repo / repo_cfg["edit_file"]
                    original = None
                    if scenario == "cold":
                        shutil.rmtree(repo / tool.out_dir, ignore_errors=True)
                        cmd = tool.build(repo, args.workers)
                    elif scenario == "warm":
                        if not tool.graph_json(repo).exists():
                            measure(tool.build(repo, args.workers), tool.env, args.timeout)
                        cmd = tool.build(repo, args.workers)
                    else:
                        # Prime with an untimed update so the timed one starts from the update
                        # path's own steady state. Graphify's `update` also indexes markdown,
                        # which `extract --code-only` skips, so the first update is not incremental.
                        measure(tool.update(repo, args.workers), tool.env, args.timeout)
                        original = edit.read_bytes()
                        edit.write_bytes(
                            original + f"\n{repo_cfg['comment']} dijkstral-bench edit {time.time_ns()}\n".encode()
                        )
                        cmd = tool.update(repo, args.workers)
                    try:
                        code, wall, cpu, peak, maxproc, tail = measure(cmd, tool.env, args.timeout)
                    finally:
                        if original is not None:
                            edit.write_bytes(original)
                    nodes, edges = graph_counts(tool.graph_json(repo))
                    s = Sample(
                        tool.name,
                        tool.version,
                        repo_cfg["name"],
                        repo_cfg["commit"],
                        scenario,
                        i,
                        args.workers,
                        code,
                        round(wall, 3),
                        round(cpu, 3),
                        round(peak, 1),
                        round(maxproc, 1),
                        nodes,
                        edges,
                        tail if code else "",
                    )
                    samples.append(s)
                    status = "ok" if code == 0 else f"EXIT {code}"
                    print(
                        f"{tool.name:9} {repo_cfg['name']:11} {scenario:6} #{i} {wall:8.2f}s "
                        f"cpu {cpu:8.2f}s rss {peak:8.1f} MB nodes {nodes} edges {edges} {status}",
                        flush=True,
                    )
                    out.write_text(
                        json.dumps({"machine": machine(), "samples": [asdict(x) for x in samples]}, indent=1)
                    )
            # Leave the corpus checkout clean for the next tool.
            subprocess.run(["git", "-C", str(repo), "checkout", "--", "."], check=False)
    print(f"wrote {out}")


def machine() -> dict:
    def sysctl(key: str) -> str:
        return subprocess.run(["sysctl", "-n", key], check=False, capture_output=True, text=True).stdout.strip()

    info = {"platform": platform.platform(), "python": platform.python_version(), "cpus": os.cpu_count()}
    if sys.platform == "darwin":
        info["cpu"] = sysctl("machdep.cpu.brand_string")
        info["mem_gb"] = round(int(sysctl("hw.memsize") or 0) / 2**30, 1)
    return info


def report(args: argparse.Namespace) -> None:
    data = json.loads(Path(args.file).read_text())
    rows: dict[tuple[str, str, str], list[dict]] = {}
    for s in data["samples"]:
        rows.setdefault((s["repo"], s["scenario"], s["tool"]), []).append(s)
    m = data["machine"]
    print(f"Machine: {m.get('cpu', m['platform'])}, {m['cpus']} cpus, {m.get('mem_gb', '?')} GB\n")
    print("| repo | scenario | tool | wall s (median, min–max) | cpu s | peak tree RSS MB | nodes | edges | ok |")
    print("|---|---|---|---|---|---|---|---|---|")
    for (repo, scenario, tool), ss in sorted(
        rows.items(), key=lambda kv: (kv[0][0], SCENARIOS.index(kv[0][1]), kv[0][2])
    ):
        walls = [s["wall_s"] for s in ss]
        ok = sum(1 for s in ss if s["exit_code"] == 0)
        print(
            f"| {repo} | {scenario} | {tool} {ss[0]['version']} | {statistics.median(walls):.2f} "
            f"({min(walls):.2f}–{max(walls):.2f}) | {statistics.median(s['cpu_s'] for s in ss):.1f} | "
            f"{statistics.median(s['peak_tree_rss_mb'] for s in ss):.0f} | {ss[-1]['nodes']} | {ss[-1]['edges']} | "
            f"{ok}/{len(ss)} |"
        )


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--tools", default="graphify,dijkstral")
    r.add_argument("--repos", default="", help="comma-separated corpus names (default: all)")
    r.add_argument("--scenarios", default=",".join(SCENARIOS))
    r.add_argument("--runs", type=int, default=3)
    r.add_argument("--workers", type=int, default=os.cpu_count() or 1)
    r.add_argument("--timeout", type=float, default=3600)
    r.set_defaults(fn=run)
    rep = sub.add_parser("report")
    rep.add_argument("file")
    rep.set_defaults(fn=report)
    args = p.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()

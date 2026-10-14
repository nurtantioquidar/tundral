#!/usr/bin/env python3
"""Query benchmark: Graphify vs dijkstral on `query`, `path` and `explain`.

    uv run --no-project bench/query.py latency --repos fastapi,erpnext,kubernetes --runs 20
    uv run --no-project bench/query.py parity --repos fastapi,erpnext,kubernetes
    uv run --no-project bench/query.py mcp --repos fastapi,erpnext,kubernetes --starts 5
    uv run --no-project bench/query.py report bench/results/query-<epoch>.json

latency: every call is a fresh process (as an agent's CLI call is). One discarded warm-up run,
then `--runs` timed runs, wall clock around `subprocess.run`; p95 is linear-interpolated; peak
RSS comes from one extra run under `/usr/bin/time -l`. Three targets per repo:

  graphify        Graphify on its frozen reference graph.json
  dijkstral       dijkstral on its own build of the corpus repo (memory-mapped graph.dij)
  dijkstral-json  dijkstral on Graphify's reference graph.json (indexed in memory per call)

parity: both tools answer the same questions on Graphify's reference graph (dijkstral reads the
same graph.json), and their `Start:` seeds, NODE lists, EDGE sets and full output are compared;
plus `explain` and `path` on sampled labels. The question set is fixed per repo (hand-written
questions plus label-word questions drawn with a fixed seed).

mcp: the MCP servers (`graphify-mcp` on its reference graph.json; `dijkstral mcp` on its own
graph.dij, and optionally on Graphify's graph.json) over stdio. Each of `--starts` cold starts
times spawn -> `tools/list` response (initialize, the initialized notification and tools/list
sent back to back), then the first `query_graph` call (the repo's latency question at depth 2,
the CLI's depth, default budget), then `--calls` warm `query_graph` calls. Peak RSS comes from
the last start, run under `/usr/bin/time`. The answers of both tools are compared (EDGE lines as a
set).

Graphify writes `cache/last_query_stamp` next to the graph it reads, so both tools read copies
under $DIJKSTRAL_BENCH_HOME/query/. Stdlib only; no network, no paid calls.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import random
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
BENCH_HOME = Path(
    os.environ.get("DIJKSTRAL_BENCH_HOME", Path.home() / ".cache/dijkstral-bench")
)
GRAPHIFY = BENCH_HOME / "tools" / "graphify-0.9.86" / "bin" / "graphify"
GRAPHIFY_MCP = BENCH_HOME / "tools" / "graphify-0.9.86" / "bin" / "graphify-mcp"
DIJKSTRAL = Path(
    os.environ.get(
        "DIJKSTRAL_BIN", HERE.parent / "cli" / "target" / "release" / "dijkstral"
    )
)
SCRATCH = BENCH_HOME / "query"
RESULTS = HERE / "results"
GRAPHIFY_ENV = {"GRAPHIFY_QUERY_LOG_DISABLE": "1", "GRAPHIFY_MAX_GRAPH_BYTES": "8GB"}

# The argv Graphify's baseline was measured with (cli/tests/fixtures/query/latency.json).
COMMANDS = {
    "fastapi": {
        "query": ["query", "how does APIRouter add routes"],
        "path": ["path", "APIRouter", "APIRoute"],
        "explain": ["explain", "APIRouter"],
    },
    "erpnext": {
        "query": ["query", "how does SalesInvoice post GL entries"],
        "path": ["path", "SalesInvoice", "AccountsController"],
        "explain": ["explain", "SalesInvoice"],
    },
    "kubernetes": {
        "query": ["query", "how does PriorityQueue requeue unschedulable pods"],
        "path": ["path", "PriorityQueue", "SchedulingQueue"],
        "explain": ["explain", "PriorityQueue"],
    },
}

# Hand-written parity questions; label-word questions are added per repo.
FIXED_QUESTIONS = {
    "fastapi": [
        "how does APIRouter add routes",
        "dependency injection",
        "who calls solve_dependencies",
        "how are websocket routes registered",
        "OAuth2PasswordBearer",
        "request validation errors",
    ],
    "erpnext": [
        "how does SalesInvoice post GL entries",
        "stock ledger entry",
        "who calls make_gl_entries",
        "payment reconciliation",
        "BOM explosion",
    ],
    "kubernetes": [
        "how does PriorityQueue requeue unschedulable pods",
        "who calls NewScheduler",
        "kubelet pod lifecycle",
    ],
}


def percentile(xs: list[float], p: float) -> float:
    s = sorted(xs)
    pos = p / 100 * (len(s) - 1)
    lo = int(pos)
    hi = min(lo + 1, len(s) - 1)
    return s[lo] + (s[hi] - s[lo]) * (pos - lo)


def peak_rss(argv: list[str], cwd: Path, env: dict[str, str]) -> int | None:
    """Peak RSS in bytes of one run under /usr/bin/time (macOS -l, Linux -v)."""
    flag = "-l" if platform.system() == "Darwin" else "-v"
    p = subprocess.run(
        ["/usr/bin/time", flag, *argv],
        cwd=cwd,
        env=env,
        check=False,
        capture_output=True,
        text=True,
    )
    for line in p.stderr.splitlines():
        m = re.match(r"\s*(\d+)\s+maximum resident set size", line)
        if m:
            return int(m.group(1))
        m = re.match(r"\s*Maximum resident set size \(kbytes\):\s*(\d+)", line)
        if m:
            return int(m.group(1)) * 1024
    return None


def prepare(repo: str, build: bool) -> tuple[Path, Path, Path]:
    """Scratch copies: Graphify's reference graph.json and dijkstral's own build of the repo."""
    root = SCRATCH / repo
    gdir = root / "graphify"
    gdir.mkdir(parents=True, exist_ok=True)
    ref = BENCH_HOME / "reference" / f"{repo}.graph.json"
    gjson = gdir / "graph.json"
    if not gjson.exists() or gjson.stat().st_size != ref.stat().st_size:
        shutil.copyfile(ref, gjson)
    dij = root / "dijkstral-out" / "graph.dij"
    if build or not dij.exists():
        corpus = BENCH_HOME / "corpus" / repo
        subprocess.run(
            [str(DIJKSTRAL), "extract", str(corpus), "--code-only", "--out", str(root)],
            check=True,
        )
    return root, gjson, dij


def targets(
    gjson: Path, dij: Path, tools: list[str]
) -> dict[str, tuple[list[str], dict[str, str], Path]]:
    out = {}
    env = dict(os.environ)
    if "graphify" in tools:
        out["graphify"] = ([str(GRAPHIFY)], {**env, **GRAPHIFY_ENV}, gjson)
    if "dijkstral" in tools:
        out["dijkstral"] = ([str(DIJKSTRAL)], env, dij)
    if "dijkstral-json" in tools:
        out["dijkstral-json"] = ([str(DIJKSTRAL)], env, gjson)
    return out


def latency(args: argparse.Namespace) -> None:
    tools = args.tools.split(",")
    results = {
        "kind": "query-latency",
        "date": time.strftime("%Y-%m-%d"),
        "machine": machine(),
        "runs": args.runs,
        "repos": {},
    }
    # What spawning any process costs from this harness, so tool times can be read against it.
    floor = []
    for _ in range(args.runs):
        t0 = time.perf_counter()
        subprocess.run(["/usr/bin/true"], check=False, capture_output=True)
        floor.append((time.perf_counter() - t0) * 1e3)
    results["spawn_floor_ms"] = {
        "p50": round(percentile(floor, 50), 2),
        "p95": round(percentile(floor, 95), 2),
    }
    print(
        f"spawn floor (/usr/bin/true): p50 {results['spawn_floor_ms']['p50']} ms",
        flush=True,
    )
    for repo in args.repos.split(","):
        root, gjson, dij = prepare(repo, args.build)
        rep: dict = {
            "graph_json_bytes": gjson.stat().st_size,
            "graph_dij_bytes": dij.stat().st_size,
            "commands": {},
        }
        for cmd, argv in COMMANDS[repo].items():
            rep["commands"][cmd] = {"argv": argv}
            for tool, (prefix, env, graph) in targets(gjson, dij, tools).items():
                full = [*prefix, *argv, "--graph", str(graph)]
                runs = (
                    args.graphify_runs
                    if tool == "graphify" and args.graphify_runs
                    else args.runs
                )
                subprocess.run(
                    full, cwd=root, env=env, check=False, capture_output=True
                )  # warm-up
                times = []
                last = None
                for _ in range(runs):
                    t0 = time.perf_counter()
                    last = subprocess.run(
                        full,
                        cwd=root,
                        env=env,
                        check=False,
                        capture_output=True,
                        text=True,
                    )
                    times.append((time.perf_counter() - t0) * 1e3)
                assert last is not None
                rss = peak_rss(full, root, env)
                row = {
                    "n": runs,
                    "p50_ms": round(percentile(times, 50), 2),
                    "p95_ms": round(percentile(times, 95), 2),
                    "min_ms": round(min(times), 2),
                    "max_ms": round(max(times), 2),
                    "peak_rss_bytes": rss,
                    "exit": last.returncode,
                    "stdout_sha1": hashlib.sha1(last.stdout.encode()).hexdigest(),
                    "stdout_first_line": last.stdout.splitlines()[0]
                    if last.stdout
                    else "",
                }
                rep["commands"][cmd][tool] = row
                print(
                    f"{repo:10} {cmd:8} {tool:15} p50 {row['p50_ms']:9.1f} ms  p95 {row['p95_ms']:9.1f} ms  rss {fmt_mb(rss)}",
                    flush=True,
                )
        results["repos"][repo] = rep
    write(results)


class McpServer:
    """A stdio MCP server process driven line by line."""

    def __init__(self, argv: list[str], cwd: Path, env: dict[str, str]) -> None:
        self.t0 = time.perf_counter()
        self.proc = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.next_id = 0

    def send(
        self, method: str, params: dict | None = None, notify: bool = False
    ) -> None:
        msg: dict = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        if not notify:
            msg["id"] = self.next_id
            self.next_id += 1
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def recv(self) -> dict:
        assert self.proc.stdout is not None
        line = self.proc.stdout.readline()
        if not line:
            raise RuntimeError("server closed stdout")
        return json.loads(line)

    def call(self, name: str, arguments: dict) -> tuple[float, str]:
        t0 = time.perf_counter()
        self.send("tools/call", {"name": name, "arguments": arguments})
        reply = self.recv()
        ms = (time.perf_counter() - t0) * 1e3
        return ms, reply["result"]["content"][0]["text"]

    def close(self) -> str:
        """Close stdin (the server exits on EOF) and return its stderr."""
        _, err = self.proc.communicate(timeout=600)
        return err


def mcp_start(
    argv: list[str], cwd: Path, env: dict[str, str], question: str, calls: int
) -> dict:
    srv = McpServer(argv, cwd, env)
    srv.send(
        "initialize",
        {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "bench", "version": "0"},
        },
    )
    srv.send("notifications/initialized", notify=True)
    srv.send("tools/list")
    srv.recv()
    srv.recv()
    startup = (time.perf_counter() - srv.t0) * 1e3
    args = {"question": question, "depth": 2}
    first, text = srv.call("query_graph", args)
    warm = [srv.call("query_graph", args)[0] for _ in range(calls)]
    err = srv.close()
    return {
        "startup_ms": startup,
        "first_ms": first,
        "warm_ms": warm,
        "text": text,
        "stderr": err,
    }


def mcp(args: argparse.Namespace) -> None:
    tools = args.tools.split(",")
    results = {
        "kind": "mcp-latency",
        "date": time.strftime("%Y-%m-%d"),
        "machine": machine(),
        "starts": args.starts,
        "calls": args.calls,
        "repos": {},
    }
    flag = "-l" if platform.system() == "Darwin" else "-v"
    for repo in args.repos.split(","):
        root, gjson, dij = prepare(repo, args.build)
        question = COMMANDS[repo]["query"][1]
        env = dict(os.environ)
        argvs = {
            "graphify": (
                [str(GRAPHIFY_MCP), str(gjson)],
                {**env, **GRAPHIFY_ENV},
            ),
            "dijkstral": ([str(DIJKSTRAL), "mcp", "--graph", str(dij)], env),
            "dijkstral-json": ([str(DIJKSTRAL), "mcp", "--graph", str(gjson)], env),
        }
        rep: dict = {"question": question, "tools": {}}
        texts = {}
        for tool in tools:
            argv, tenv = argvs[tool]
            starts = []
            for k in range(args.starts):
                last = k == args.starts - 1
                full = ["/usr/bin/time", flag, *argv] if last else argv
                starts.append(mcp_start(full, root, tenv, question, args.calls))
            rss = None
            for line in starts[-1]["stderr"].splitlines():
                m = re.match(r"\s*(\d+)\s+maximum resident set size", line)
                if m:
                    rss = int(m.group(1))
                m = re.match(r"\s*Maximum resident set size \(kbytes\):\s*(\d+)", line)
                if m:
                    rss = int(m.group(1)) * 1024
            up = [x["startup_ms"] for x in starts]
            first = [x["first_ms"] for x in starts]
            warm = [w for x in starts for w in x["warm_ms"]]
            row = {
                "startup_p50_ms": round(percentile(up, 50), 1),
                "startup_p95_ms": round(percentile(up, 95), 1),
                "first_p50_ms": round(percentile(first, 50), 2),
                "first_p95_ms": round(percentile(first, 95), 2),
                "warm_p50_ms": round(percentile(warm, 50), 2),
                "warm_p95_ms": round(percentile(warm, 95), 2),
                "peak_rss_bytes": rss,
                "first_line": starts[-1]["text"].split("\n", 1)[0],
            }
            texts[tool] = starts[-1]["text"]
            rep["tools"][tool] = row
            print(
                f"{repo:10} {tool:15} start p50 {row['startup_p50_ms']:8.1f} ms  first {row['first_p50_ms']:8.1f} ms"
                f"  warm p50 {row['warm_p50_ms']:8.2f} / p95 {row['warm_p95_ms']:8.2f} ms  rss {fmt_mb(rss)}",
                flush=True,
            )
        if "graphify" in texts:
            ref = split_output(texts["graphify"])
            for tool, text in texts.items():
                got = split_output(text)
                # The header names the graph file each server was given.
                rep["tools"][tool]["same_answer_as_graphify"] = (
                    got["nodes"] == ref["nodes"]
                    and got["edges"] == ref["edges"]
                    and got["start"] == ref["start"]
                )
        results["repos"][repo] = rep
    write(results)


def words_of(label: str) -> list[str]:
    parts = re.findall(r"[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|\d+", label)
    return [p.lower() for p in parts if len(p) >= 3]


def questions(repo: str, nodes: list[dict], n: int) -> list[str]:
    rnd = random.Random(f"dijkstral-query-parity-{repo}")
    symbols = [
        x["label"]
        for x in nodes
        if x.get("source_file")
        and not x["label"].endswith((".py", ".go", ".js", ".ts"))
    ]
    qs = list(FIXED_QUESTIONS.get(repo, []))
    templates = [
        "{a}",
        "how does {a} {b}",
        "who calls {a}",
        "{a} {b}",
        "{a} {b} {c}",
        "what uses {a}",
        "{L}",
    ]
    while len(qs) < n:
        t = rnd.choice(templates)
        picks = [rnd.choice(symbols) for _ in range(3)]
        ws = [w for p in picks for w in words_of(p)] or ["main"]
        q = t.format(a=rnd.choice(ws), b=rnd.choice(ws), c=rnd.choice(ws), L=picks[0])
        if q not in qs:
            qs.append(q)
    return qs


def split_output(text: str) -> dict:
    lines = text.split("\n")
    start = re.search(r"Start: (\[.*?\]) \|", lines[0]) if lines else None
    return {
        "start": start.group(1) if start else None,
        "nodes": [line for line in lines if line.startswith("NODE ")],
        "edges": sorted(line for line in lines if line.startswith("EDGE ")),
        "rest": [line for line in lines if not line.startswith(("NODE ", "EDGE "))],
    }


def parity(args: argparse.Namespace) -> None:
    results = {
        "kind": "query-parity",
        "date": time.strftime("%Y-%m-%d"),
        "machine": machine(),
        "repos": {},
    }
    for repo in args.repos.split(","):
        root, gjson, _ = prepare(repo, False)
        doc = json.loads(gjson.read_text())
        nodes = doc["nodes"]
        n = args.questions_large if len(nodes) > 100_000 else args.questions
        cases = [["query", q] for q in questions(repo, nodes, n)]
        rnd = random.Random(f"dijkstral-path-parity-{repo}")
        labelled = [x for x in nodes if x.get("source_file")]
        for _ in range(args.lookups):
            cases.append(["explain", rnd.choice(labelled)["id"]])
            cases.append(
                [
                    "path",
                    rnd.choice(labelled)["id"],
                    rnd.choice(labelled)["id"],
                    "--undirected",
                ]
            )
        rows = []
        tally = {
            "cases": 0,
            "identical": 0,
            "start": 0,
            "nodes": 0,
            "edges": 0,
            "exit": 0,
        }
        for argv in cases:
            g = subprocess.run(
                [str(GRAPHIFY), *argv, "--graph", str(gjson)],
                cwd=root,
                env={**os.environ, **GRAPHIFY_ENV},
                check=False,
                capture_output=True,
                text=True,
            )
            d = subprocess.run(
                [str(DIJKSTRAL), *argv, "--graph", str(gjson)],
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
            )
            a, b = split_output(g.stdout), split_output(d.stdout)
            row = {
                "argv": argv,
                "exit": [g.returncode, d.returncode],
                "start_equal": a["start"] == b["start"],
                "nodes_equal": a["nodes"] == b["nodes"],
                "node_set_jaccard": jaccard(a["nodes"], b["nodes"]),
                "edges_equal": a["edges"] == b["edges"],
                "identical": a == b
                and g.stderr == d.stderr
                and g.returncode == d.returncode,
            }
            if not row["identical"]:
                row["graphify_head"] = g.stdout[:400]
                row["dijkstral_head"] = d.stdout[:400]
            rows.append(row)
            tally["cases"] += 1
            tally["identical"] += row["identical"]
            tally["start"] += row["start_equal"]
            tally["nodes"] += row["nodes_equal"]
            tally["edges"] += row["edges_equal"]
            tally["exit"] += g.returncode == d.returncode
            mark = "=" if row["identical"] else "x"
            print(f"{repo:10} {mark} {' '.join(argv)[:90]}", flush=True)
        results["repos"][repo] = {"tally": tally, "cases": rows}
        print(f"{repo}: {tally}")
    write(results)


def jaccard(a: list[str], b: list[str]) -> float:
    sa, sb = set(a), set(b)
    return 1.0 if not sa and not sb else round(len(sa & sb) / len(sa | sb), 4)


def machine() -> dict:
    sha = (
        hashlib.sha256(DIJKSTRAL.read_bytes()).hexdigest()
        if DIJKSTRAL.exists()
        else None
    )
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=HERE,
        check=False,
        capture_output=True,
        text=True,
    ).stdout.strip()
    return {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "dijkstral_sha256": sha,
        "commit": commit,
    }


def fmt_mb(b: int | None) -> str:
    return "?" if b is None else f"{b / 1e6:,.0f} MB"


def write(results: dict) -> None:
    RESULTS.mkdir(exist_ok=True)
    path = RESULTS / f"{results['kind'].split('-')[0]}-{int(time.time())}.json"
    if results["kind"] == "query-parity":
        path = RESULTS / f"query-parity-{int(time.time())}.json"
    path.write_text(json.dumps(results, indent=1))
    print(f"wrote {path}")


def report(args: argparse.Namespace) -> None:
    doc = json.loads(Path(args.file).read_text())
    if doc["kind"] == "mcp-latency":
        print(
            "| repo | server | spawn → tools/list p50 / p95 ms | 1st query_graph p50 ms | warm query_graph p50 / p95 ms | peak RSS | same answer |"
        )
        print("|---|---|---|---|---|---|---|")
        for repo, rep in doc["repos"].items():
            for tool, r in rep["tools"].items():
                print(
                    f"| {repo} | {tool} | {r['startup_p50_ms']:,.1f} / {r['startup_p95_ms']:,.1f} | {r['first_p50_ms']:,.1f}"
                    f" | {r['warm_p50_ms']:,.2f} / {r['warm_p95_ms']:,.2f} | {fmt_mb(r['peak_rss_bytes'])}"
                    f" | {r.get('same_answer_as_graphify', '')} |"
                )
        return
    if doc["kind"] == "query-latency":
        print("| repo | command | tool | p50 ms | p95 ms | peak RSS |")
        print("|---|---|---|---|---|---|")
        for repo, rep in doc["repos"].items():
            for cmd, rows in rep["commands"].items():
                for tool, row in rows.items():
                    if tool == "argv":
                        continue
                    print(
                        f"| {repo} | {cmd} | {tool} | {row['p50_ms']:,.1f} | {row['p95_ms']:,.1f} | {fmt_mb(row['peak_rss_bytes'])} |"
                    )
    else:
        print(
            "| repo | cases | identical | same seeds | same nodes | same edges | same exit |"
        )
        print("|---|---|---|---|---|---|---|")
        for repo, rep in doc["repos"].items():
            t = rep["tally"]
            print(
                f"| {repo} | {t['cases']} | {t['identical']} | {t['start']} | {t['nodes']} | {t['edges']} | {t['exit']} |"
            )


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = ap.add_subparsers(dest="cmd", required=True)
    lat = sub.add_parser("latency")
    lat.add_argument("--repos", default="fastapi,erpnext,kubernetes")
    lat.add_argument("--runs", type=int, default=20)
    lat.add_argument(
        "--graphify-runs", type=int, default=0, help="override --runs for Graphify"
    )
    lat.add_argument("--tools", default="graphify,dijkstral,dijkstral-json")
    lat.add_argument(
        "--build", action="store_true", help="rebuild dijkstral's graph of each repo"
    )
    par = sub.add_parser("parity")
    par.add_argument("--repos", default="fastapi,erpnext,kubernetes")
    par.add_argument("--questions", type=int, default=40)
    par.add_argument(
        "--questions-large",
        type=int,
        default=12,
        help="questions on graphs over 100k nodes",
    )
    par.add_argument(
        "--lookups", type=int, default=5, help="explain and path cases per repo"
    )
    mc = sub.add_parser("mcp")
    mc.add_argument("--repos", default="fastapi,erpnext,kubernetes")
    mc.add_argument("--starts", type=int, default=5, help="cold server starts per tool")
    mc.add_argument(
        "--calls", type=int, default=20, help="warm query_graph calls per start"
    )
    mc.add_argument("--tools", default="graphify,dijkstral,dijkstral-json")
    mc.add_argument(
        "--build", action="store_true", help="rebuild dijkstral's graph of each repo"
    )
    rep = sub.add_parser("report")
    rep.add_argument("file")
    args = ap.parse_args()
    if not GRAPHIFY.exists() and args.cmd != "report":
        sys.exit(f"graphify not installed at {GRAPHIFY}; run bench/setup.sh")
    if not DIJKSTRAL.exists() and args.cmd != "report":
        sys.exit(
            f"dijkstral not built at {DIJKSTRAL}; run `cargo build --release` in cli/"
        )
    {"latency": latency, "parity": parity, "mcp": mcp, "report": report}[args.cmd](args)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Same-model semantic-pass benchmark: Graphify vs dijkstral through one Codex-backed shim.

    uv run --no-project bench/semantic.py --dijkstral cli/target/release/dijkstral
    uv run --no-project bench/semantic.py --dry-run      # static replies: no Codex call

Both tools run `extract` (semantic pass on) with `--backend openai` against the same localhost
shim (`llm_shim.py --responder codex --model gpt-6.1-sol --reasoning-effort medium`, passed
explicitly), so every request reaches the same model, effort and `codex exec` framing; only
each tool's prompts, chunking and merge differ. dijkstral uses `--backend openai` rather than
its own Codex backend for that reason: its direct backend frames requests for `codex exec`
itself, which would change the transport, not just the tool. Both run with a scrubbed
environment (scratch HOME, dummy key) and on scratch copies of the corpora:

  mixed     tests/fixtures/semantic/mixed (README, docs, YAML, PDF, PNG, DOCX, XLSX; 3 Python files)
  ripgrep   crates/globset (README and Rust sources) plus four crate READMEs and
            RELEASE-CHECKLIST.md from the pinned ripgrep checkout

Per tool and corpus: a cold extract (empty caches) and a rerun. Reported: wall time, model calls
(requests the shim answered), tokens (the tools' own `.graphify_analysis.json` counts, which are
Codex's usage), semantic nodes and links (those without `_origin: "ast"`), document coverage
(non-code files with at least one node), links between a semantic node and a code node, and the
rerun's calls (0 expected). Results go to bench/results/semantic-<epoch>.json and a Markdown table
on stdout. `--max-requests` caps Codex calls for the whole run (the shim answers HTTP 429 past
it). Stdlib only. Never reads API keys.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CLI = HERE.parent / "cli"
BENCH_HOME = Path(
    os.environ.get("DIJKSTRAL_BENCH_HOME", Path.home() / ".cache/dijkstral-bench")
)
GRAPHIFY = BENCH_HOME / "tools/graphify-0.9.86-semantic/bin/graphify"
MODEL = "gpt-6.1-sol"
EFFORT = "medium"
DOC_EXT = {
    ".md",
    ".mdx",
    ".rst",
    ".txt",
    ".html",
    ".yaml",
    ".yml",
    ".svg",
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".webp",
    ".pdf",
    ".docx",
    ".xlsx",
}
RIPGREP_FILES = [
    "crates/globset/README.md",
    "crates/globset/Cargo.toml",
    "crates/globset/src/fnv.rs",
    "crates/globset/src/glob.rs",
    "crates/globset/src/lib.rs",
    "crates/globset/src/pathutil.rs",
    "crates/globset/src/serde_impl.rs",
    "crates/ignore/README.md",
    "crates/matcher/README.md",
    "crates/printer/README.md",
    "crates/searcher/README.md",
    "RELEASE-CHECKLIST.md",
]


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def make_corpora(work: Path) -> dict[str, Path]:
    corpora = {}
    mixed = work / "corpus/mixed"
    shutil.copytree(CLI / "tests/fixtures/semantic/mixed", mixed)
    corpora["mixed"] = mixed
    rg_src = BENCH_HOME / "corpus/ripgrep"
    if rg_src.is_dir():
        rg = work / "corpus/ripgrep"
        for rel in RIPGREP_FILES:
            (rg / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(rg_src / rel, rg / rel)
        corpora["ripgrep"] = rg
    else:
        print(
            f"skipping ripgrep: {rg_src} not found (run bench/setup.sh)",
            file=sys.stderr,
        )
    return corpora


def scrubbed_env(home: Path, port: int) -> dict[str, str]:
    home.mkdir(parents=True, exist_ok=True)
    return {
        "HOME": str(home),
        "PATH": "/usr/bin:/bin",
        "LANG": "en_US.UTF-8",
        "GRAPHIFY_QUERY_LOG_DISABLE": "1",
        "OPENAI_BASE_URL": f"http://127.0.0.1:{port}/v1",
        "OPENAI_API_KEY": "dummy",
        "OPENAI_MODEL": "codex-shim",
        "SOURCE_DATE_EPOCH": "1760400000",
    }


class Shim:
    def __init__(self, work: Path, port: int, args: argparse.Namespace) -> None:
        self.dir = work / "transcripts"
        cmd = [
            sys.executable,
            str(HERE / "llm_shim.py"),
            "--port",
            str(port),
            "--transcript-dir",
            str(self.dir),
            "--max-requests",
            str(args.max_requests),
            "--parallel",
            "2",
        ]
        if args.dry_run:
            reply = json.loads(
                (
                    CLI / "tests/fixtures/semantic/mixed.transcript/req-0001.json"
                ).read_text()
            )["response"]["text"]
            static = work / "static_reply.json"
            static.write_text(reply)
            cmd += ["--responder", "static", "--static-response", f"@{static}"]
        else:
            cmd += [
                "--responder",
                "codex",
                "--model",
                MODEL,
                "--reasoning-effort",
                EFFORT,
            ]
        self.log = open(work / "shim.log", "w")  # noqa: SIM115 - lives as long as the shim
        self.proc = subprocess.Popen(cmd, stdout=self.log, stderr=self.log)
        for _ in range(100):
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    return
            except OSError:
                time.sleep(0.1)
        raise SystemExit("shim did not start")

    def requests(self) -> int:
        return len(list(self.dir.glob("req-*.json"))) if self.dir.is_dir() else 0

    def stop(self) -> None:
        self.proc.terminate()
        self.proc.wait(timeout=10)
        self.log.close()


def run_tool(
    tool: str, binary: Path, corpus: Path, env: dict[str, str]
) -> tuple[float, str]:
    if tool == "graphify":
        cmd = [str(binary), "extract", str(corpus), "--backend", "openai"]
    else:
        cmd = [
            str(binary),
            "extract",
            str(corpus),
            "--backend",
            "openai",
            "--model",
            "codex-shim",
            "--no-viz",
        ]
    t0 = time.perf_counter()
    p = subprocess.run(cmd, env=env, capture_output=True, text=True, check=False)
    wall = time.perf_counter() - t0
    if p.returncode != 0:
        print(
            f"{tool} failed on {corpus}:\n{p.stdout[-2000:]}\n{p.stderr[-2000:]}",
            file=sys.stderr,
        )
    return wall, p.stdout + p.stderr


def code_only_ids(
    tool: str, binary: Path, src: Path, scratch: Path, env: dict[str, str]
) -> set[str]:
    """Node ids of the tool's code-only graph of `src` (no model)."""
    corpus = scratch / f"{tool}-code-only" / src.name
    shutil.copytree(src, corpus)
    cmd = [str(binary), "extract", str(corpus), "--code-only"]
    if tool == "dijkstral":
        cmd.append("--no-viz")
    subprocess.run(cmd, env=env, capture_output=True, check=True)
    out = corpus / ("graphify-out" if tool == "graphify" else "dijkstral-out")
    return {n["id"] for n in json.loads((out / "graph.json").read_text())["nodes"]}


def measure(tool: str, corpus: Path, code_ids: set[str]) -> dict:
    out = corpus / ("graphify-out" if tool == "graphify" else "dijkstral-out")
    g = json.loads((out / "graph.json").read_text())
    analysis = json.loads((out / ".graphify_analysis.json").read_text())
    nodes = {n["id"]: n for n in g["nodes"]}
    code = set(nodes) & code_ids
    sem = set(nodes) - code
    links = g.get("links", g.get("edges", []))
    sem_links = [l for l in links if l["source"] in sem or l["target"] in sem]
    doc_links = [
        l
        for l in links
        if (l["source"] in sem and l["target"] in code)
        or (l["target"] in sem and l["source"] in code)
    ]
    docs = sorted(
        str(p.relative_to(corpus))
        for p in corpus.rglob("*")
        if p.is_file()
        and p.suffix.lower() in DOC_EXT
        and not any(part.endswith("-out") for part in p.relative_to(corpus).parts)
    )

    def covered(doc: str) -> bool:
        stem = Path(doc).stem
        for i in sem:
            sf = nodes[i].get("source_file") or ""
            if sf.lstrip("./") == doc:
                return True
            if "converted/" in sf and Path(sf).name.startswith(stem + "_"):
                return True
        return False

    return {
        "nodes": len(nodes),
        "semantic_nodes": len(sem),
        "semantic_links": len(sem_links),
        "doc_to_code_links": len(doc_links),
        "docs": len(docs),
        "docs_covered": sum(covered(d) for d in docs),
        "hyperedges": len(g.get("hyperedges", [])),
        "tokens_in": analysis.get("tokens", {}).get("input", 0),
        "tokens_out": analysis.get("tokens", {}).get("output", 0),
    }


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--dijkstral", type=Path, default=CLI / "target/release/dijkstral")
    ap.add_argument("--graphify", type=Path, default=GRAPHIFY)
    ap.add_argument("--tools", default="graphify,dijkstral")
    ap.add_argument("--corpora", default="mixed,ripgrep")
    ap.add_argument(
        "--max-requests", type=int, default=8, help="Codex call cap for the run"
    )
    ap.add_argument(
        "--dry-run", action="store_true", help="static replies, no Codex call"
    )
    ap.add_argument(
        "--work", type=Path, help="scratch directory (default: a new temp dir)"
    )
    args = ap.parse_args()

    work = args.work or Path(
        subprocess.check_output(["mktemp", "-d"], text=True).strip()
    )
    work.mkdir(parents=True, exist_ok=True)
    port = free_port()
    corpora = {
        k: v for k, v in make_corpora(work).items() if k in args.corpora.split(",")
    }
    shim = Shim(work, port, args)
    rows = []
    try:
        for name, src in corpora.items():
            for tool in args.tools.split(","):
                corpus = work / tool / name
                shutil.copytree(src, corpus)
                binary = args.graphify if tool == "graphify" else args.dijkstral
                env = scrubbed_env(work / f"home-{tool}", port)
                code_ids = code_only_ids(tool, binary, src, work / "baseline", env)
                before = shim.requests()
                wall, log = run_tool(tool, binary, corpus, env)
                calls = shim.requests() - before
                m = measure(tool, corpus, code_ids)
                before = shim.requests()
                rerun_wall, rerun_log = run_tool(tool, binary, corpus, env)
                rerun_calls = shim.requests() - before
                row = {
                    "corpus": name,
                    "tool": tool,
                    "wall_s": round(wall, 2),
                    "calls": calls,
                    "rerun_wall_s": round(rerun_wall, 2),
                    "rerun_calls": rerun_calls,
                    **m,
                }
                rows.append(row)
                (work / f"{tool}-{name}.log").write_text(
                    log + "\n--- rerun ---\n" + rerun_log
                )
                print(json.dumps(row), file=sys.stderr)
    finally:
        shim.stop()
    meta = {
        "date": time.strftime("%Y-%m-%d"),
        "model": MODEL if not args.dry_run else "static",
        "reasoning_effort": EFFORT,
        "transport": "llm_shim.py codex responder, both tools --backend openai",
        "graphify": "graphifyy[leiden,mcp,openai,pdf,office]==0.9.86",
        "dijkstral": subprocess.run(
            [str(args.dijkstral), "--version"],
            capture_output=True,
            text=True,
            check=False,
        ).stdout.strip(),
        "codex_calls": sum(r["calls"] + r["rerun_calls"] for r in rows),
        "work": str(work),
    }
    results = HERE / "results"
    results.mkdir(exist_ok=True)
    path = results / f"semantic-{int(time.time())}.json"
    path.write_text(json.dumps({"meta": meta, "rows": rows}, indent=2))
    print(
        "| Corpus | Tool | Wall (s) | Calls | Tokens in / out | Semantic nodes | Semantic links "
        "| Docs covered | Doc→code links | Hyperedges | Rerun (s) | Rerun calls |"
    )
    print("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for r in rows:
        print(
            f"| {r['corpus']} | {r['tool']} | {r['wall_s']} | {r['calls']} | "
            f"{r['tokens_in']:,} / {r['tokens_out']:,} | {r['semantic_nodes']} | "
            f"{r['semantic_links']} | {r['docs_covered']}/{r['docs']} | {r['doc_to_code_links']} | "
            f"{r['hyperedges']} | {r['rerun_wall_s']} | {r['rerun_calls']} |"
        )
    print(f"\n{json.dumps(meta)}\nwrote {path}")


if __name__ == "__main__":
    main()

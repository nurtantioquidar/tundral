#!/usr/bin/env python3
"""Generate the key-fact question files and their provenance from tools/sources/<repo>.json.

    python3 -I tools/build.py [repo ...]        # default: every tools/sources/*.json

Writes questions/<repo>.jsonl (the shared task contract) and questions/provenance/<repo>.json.
Each source entry carries, besides the task fields, the files every fact must occur in (`in`),
the SCIP lookups behind callers/impact facts (`scip`: symbol name or full SCIP symbol, its
definition file, an optional path prefix restricting references) and the source ranges read
(`read`). SCIP lookups are expanded here from the accuracy indexes (tools/scip_refs.py), so the
provenance records the SCIP symbol, its definition, every reference occurrence and the enclosing
definitions of those references. Stdlib only; no model calls.
"""

from __future__ import annotations

import json
import sys
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
QDIR = HERE.parent
BENCH_SRC = QDIR.parent.parent  # products/dijkstral/bench
sys.path.insert(0, str(HERE))

import scip_refs

TASK_KEYS = ("id", "kind", "question", "reference", "facts", "min_facts")


def commits() -> dict[str, str]:
    data = tomllib.loads((BENCH_SRC / "corpus.toml").read_text())
    return {r["name"]: r["commit"] for r in data["repo"]}


def build(repo: str, commit: str) -> None:
    src = json.loads((HERE / "sources" / f"{repo}.json").read_text())
    # Read-only sets (locate, architecture, flow facts) need no SCIP index.
    scip = any(q.get("scip") for q in src)
    occ = scip_refs.occurrences(repo) if scip else {}
    oracle = scip_refs.oracle(repo) if scip else {}
    lines = []
    prov_q = {}
    for q in src:
        missing = [k for k in TASK_KEYS if k not in q]
        if missing:
            sys.exit(f"{q.get('id')}: missing {missing}")
        task = {
            "id": q["id"],
            "suite": "keyfacts",
            "corpus_id": repo,
            "corpus": f"~/.cache/dijkstral-bench/corpus/{repo}",
            "semantic": False,
            "question": q["question"],
            "question_date": None,
            "kind": q["kind"],
            "reference": q["reference"],
            "facts": [{"fact": f["fact"], "aliases": f["aliases"]} for f in q["facts"]],
            "min_facts": q["min_facts"],
        }
        lines.append(json.dumps(task, ensure_ascii=False))
        lookups = []
        for s in q.get("scip", []):
            r = scip_refs.lookup(
                repo, occ, s["name"], s["def_file"], s.get("within"), exact=True
            )
            if not r["definitions"]:
                sys.exit(
                    f"{q['id']}: no SCIP definition of {s['name']} in {s['def_file']}"
                )
            r["def_file"] = s["def_file"]
            r["within"] = s.get("within")
            lookups.append(r)
        prov_q[q["id"]] = {
            "kind": q["kind"],
            "facts": [
                {
                    "fact": f["fact"],
                    "type": f.get("type") or ("symbol" if f["in"] else "file"),
                    "in": f["in"],
                }
                for f in q["facts"]
            ],
            "scip": lookups,
            "read": q.get("read", []),
        }
    (QDIR / f"{repo}.jsonl").write_text("\n".join(lines) + "\n")
    parts = sorted((scip_refs.BENCH / "scip" / repo / "parts").glob("*.scip"))
    prov = {
        "repo": repo,
        "commit": commit,
        "method": (
            "callers/impact facts: SCIP definitions and reference occurrences from the accuracy "
            "indexes, attributed to enclosing definitions by the accuracy oracle; every fact "
            "re-checked against the source at the pinned commit (check.py). locate/architecture/"
            "flow facts: the source ranges listed under `read`."
        ),
        "scip_parts": [
            f"$DIJKSTRAL_BENCH_HOME/scip/{repo}/parts/{p.name}" for p in parts
        ],
        "indexers": oracle.get("indexers", []),
        "oracle_commit": oracle.get("commit"),
        "questions": prov_q,
    }
    (QDIR / "provenance").mkdir(exist_ok=True)
    (QDIR / "provenance" / f"{repo}.json").write_text(
        json.dumps(prov, indent=1, ensure_ascii=False) + "\n"
    )
    print(f"{repo}: {len(lines)} questions")


def main() -> None:
    pins = commits()
    repos = sys.argv[1:] or sorted(p.stem for p in (HERE / "sources").glob("*.json"))
    for repo in repos:
        build(repo, pins[repo])


if __name__ == "__main__":
    main()

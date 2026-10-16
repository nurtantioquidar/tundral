#!/usr/bin/env python3
"""Look up definitions and references in the accuracy SCIP indexes (stdlib only).

    python3 tools/scip_refs.py <repo> defs <name> [--file SUBSTR]
    python3 tools/scip_refs.py <repo> refs <symbol-or-name> [--file SUBSTR] [--in SUBSTR]
    python3 tools/scip_refs.py <repo> callers <symbol-or-name> [--file SUBSTR] [--in SUBSTR]

Reads $DIJKSTRAL_BENCH_HOME/scip/<repo>/parts/*.scip (written by bench/accuracy/index.sh) with a
small protobuf decoder, so it needs neither the `scip` CLI nor a protobuf package. Occurrences are
cached per repo as $DIJKSTRAL_BENCH_HOME/agent-questions/<repo>.occ.json. `callers` attributes
each reference to its enclosing function from the accuracy oracle's chains
($DIJKSTRAL_BENCH_HOME/accuracy/oracle/<repo>.json, built from the same SCIP indexes by
bench/accuracy/oracle.py). Neither tool's graph is read here.

`refs` prints every non-definition occurrence as file:line; `callers` prints the enclosing
definitions (function, method or type) whose bodies reference the symbol, with the family
(calls / references / mentions / inherits) the oracle assigned.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections import defaultdict
from pathlib import Path

BENCH = Path(
    os.environ.get("DIJKSTRAL_BENCH_HOME", Path.home() / ".cache/dijkstral-bench")
)


# --------------------------------------------------------------------------- protobuf


def varint(buf: bytes, i: int) -> tuple[int, int]:
    shift = res = 0
    while True:
        b = buf[i]
        i += 1
        res |= (b & 0x7F) << shift
        if b < 0x80:
            return res, i
        shift += 7


def fields(buf: bytes, i: int = 0, end: int | None = None):
    """Yield (field number, wire type, value) where value is int or (start, end) for wire 2."""
    end = len(buf) if end is None else end
    while i < end:
        key, i = varint(buf, i)
        fn, wt = key >> 3, key & 7
        if wt == 0:
            v, i = varint(buf, i)
            yield fn, wt, v
        elif wt == 2:
            ln, i = varint(buf, i)
            yield fn, wt, (i, i + ln)
            i += ln
        elif wt == 1:
            i += 8
        elif wt == 5:
            i += 4
        else:
            raise ValueError(f"wire type {wt}")


def packed(buf: bytes, a: int, b: int) -> list[int]:
    out = []
    while a < b:
        v, a = varint(buf, a)
        out.append(v)
    return out


def decode(path: Path) -> tuple[str, list[tuple[str, list[tuple[str, int, int]]]]]:
    """Return (project_root, [(relative_path, [(symbol, line0, roles)])])."""
    buf = path.read_bytes()
    root = ""
    docs = []
    for fn, wt, v in fields(buf):
        if fn == 1 and wt == 2:  # metadata
            for f2, w2, v2 in fields(buf, *v):
                if f2 == 3 and w2 == 2:
                    root = buf[v2[0] : v2[1]].decode()
        elif fn == 2 and wt == 2:  # document
            rel = ""
            occs = []
            for f2, w2, v2 in fields(buf, *v):
                if f2 == 1 and w2 == 2:
                    rel = buf[v2[0] : v2[1]].decode()
                elif f2 == 2 and w2 == 2:
                    rng: list[int] = []
                    sym = ""
                    roles = 0
                    for f3, w3, v3 in fields(buf, *v2):
                        if f3 == 1:
                            rng = packed(buf, *v3) if w3 == 2 else [v3]
                        elif f3 == 2 and w3 == 2:
                            sym = buf[v3[0] : v3[1]].decode()
                        elif f3 == 3 and w3 == 0:
                            roles = v3
                    if sym and rng:
                        occs.append((sym, rng[0], roles))
            docs.append((rel, occs))
    return root, docs


# --------------------------------------------------------------------------- cache


def source_base(repo: str) -> Path:
    src = BENCH / "scip" / repo / "src"
    return src if src.is_dir() else BENCH / "corpus" / repo


def occurrences(repo: str) -> dict[str, list[list]]:
    """{repo-relative file: [[symbol, line1, is_def], ...]}"""
    cache = BENCH / "agent-questions" / f"{repo}.occ.json"
    if cache.exists():
        return json.loads(cache.read_text())
    base = source_base(repo).resolve()
    out: dict[str, list[list]] = {}
    for part in sorted((BENCH / "scip" / repo / "parts").glob("*.scip")):
        root, docs = decode(part)
        rootp = Path(root.removeprefix("file://")).resolve()
        for rel, occs in docs:
            r = os.path.normpath(os.path.join(os.path.relpath(rootp, base), rel))
            if r.startswith("..") or r in out:
                continue
            out[r] = [
                [s, ln + 1, 1 if roles & 1 else 0]
                for s, ln, roles in occs
                if not s.startswith("local ")
            ]
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(out))
    return out


def oracle(repo: str) -> dict:
    return json.loads((BENCH / "accuracy" / "oracle" / f"{repo}.json").read_text())


def bare(sym: str) -> str:
    """Last descriptor name of a SCIP symbol: `pkg/Foo#bar().` -> `bar`."""
    s = sym.rstrip(".#!/():")
    i = max(s.rfind("#"), s.rfind("/"), s.rfind("."), s.rfind(" "))
    name = s[i + 1 :]
    if "]" in name:  # rust-analyzer: `impl#[Type][Trait]method().`
        name = name[name.rindex("]") + 1 :]
    if "(" in name:
        name = name[: name.index("(")]
    return name.strip("`")


def resolve(
    occ: dict, query: str, file_sub: str | None, exact: bool = False
) -> list[tuple[str, str, int]]:
    """Definitions matching a full symbol or a bare name: [(symbol, file, line)].

    `file_sub` filters definition files by substring, or by equality when `exact`."""
    hits = []
    for f, os_ in occ.items():
        if file_sub and (f != file_sub if exact else file_sub not in f):
            continue
        for s, ln, d in os_:
            if d and (s == query or bare(s) == query):
                hits.append((s, f, ln))
    return sorted(set(hits))


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("repo")
    ap.add_argument("cmd", choices=["defs", "refs", "callers"])
    ap.add_argument("queries", nargs="+")
    ap.add_argument("--file", help="definition file substring")
    ap.add_argument(
        "--in", dest="within", help="only references in files containing this"
    )
    a = ap.parse_args()
    occ = occurrences(a.repo)
    for q in a.queries:
        if len(a.queries) > 1:
            print(f"=== {q}")
        run(a, occ, q)


_ORACLE: dict[str, dict] = {}


def lookup(
    repo: str,
    occ: dict,
    query: str,
    def_file: str | None,
    within: str | None,
    exact: bool = False,
) -> dict:
    """Definitions, reference occurrences and referencing definitions of one symbol name."""
    defs = resolve(occ, query, def_file, exact)
    syms = {s for s, _, _ in defs}
    if not defs and " " in query:
        # a full symbol defined outside the indexed scope: references only
        syms = {query}
    refs = {}
    for f, os_ in sorted(occ.items()):
        if within and within not in f:
            continue
        lines = sorted({ln for s, ln, d in os_ if s in syms and not d})
        if lines:
            refs[f] = lines
    if repo not in _ORACLE:
        _ORACLE[repo] = oracle(repo)
    o = _ORACLE[repo]
    odefs = o["defs"]
    idx = {i for i, d in enumerate(odefs) if d[0] in syms}
    seen: dict[tuple, set] = defaultdict(set)
    for fam in ("calls", "references", "mentions", "inherits"):
        for rec in o["edges"][fam]:
            chain, t = rec[0], rec[1]
            if t in idx and chain:
                c = odefs[chain[0]]
                if within and within not in c[1]:
                    continue
                names = "<".join(odefs[k][3] for k in chain)
                seen[(c[1], c[2], names, c[0])].add(fam)
    users = [
        {"file": f, "line": ln, "chain": names, "symbol": sym, "families": sorted(fams)}
        for (f, ln, names, sym), fams in sorted(seen.items())
    ]
    return {
        "name": query,
        "definitions": [{"symbol": s, "file": f, "line": ln} for s, f, ln in defs],
        "reference_occurrences": refs,
        "referencing_definitions": users,
    }


def run(a: argparse.Namespace, occ: dict, query: str) -> None:
    r = lookup(a.repo, occ, query, a.file, a.within)
    if not r["definitions"]:
        print(f"no definition of {query}", file=sys.stderr)
        return
    for d in r["definitions"]:
        print(f"# def {d['file']}:{d['line']}\t{d['symbol']}")
    if a.cmd == "defs":
        return
    if a.cmd == "refs":
        for f, lines in r["reference_occurrences"].items():
            print(f"{f}: {','.join(map(str, lines))}")
        return
    for u in r["referencing_definitions"]:
        print(f"{u['file']}:{u['line']}\t{u['chain']}\t{','.join(u['families'])}")


if __name__ == "__main__":
    main()

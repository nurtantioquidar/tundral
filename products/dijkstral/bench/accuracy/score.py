#!/usr/bin/env python3
"""Score Graphify and dijkstral graphs against the SCIP oracle (see oracle.py).

    uv run --no-project bench/accuracy/score.py [--repos fastapi,ripgrep] [--rebuild-oracle]

Per repo, tool and relation family, precision and recall over unordered node pairs, restricted
to the scope the oracle can judge: both endpoints are nodes that some SCIP definition maps to
(imports: both files are indexed). Tool edges outside that scope (stubs, externals, files the
indexer did not cover, nodes no definition maps to) are counted separately, never as errors.

Families: calls = `calls`; references = `references`; inherits = `inherits` + `implements`;
imports = `imports` + `imports_from` + `re_exports`, compared as file pairs.

Defaults: Graphify = $DIJKSTRAL_BENCH_HOME/reference/<repo>.graph.json (frozen `extract
--code-only`), dijkstral = $DIJKSTRAL_BENCH_HOME/accuracy/dijkstral/<repo>/dijkstral-out/graph.json
(graphs.sh). Writes results/accuracy-<date>.{json,md} next to this script. Stdlib only.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
from collections import Counter, defaultdict
from pathlib import Path

from oracle import BENCH, load_or_build, map_defs

HERE = Path(__file__).resolve().parent
FAMILIES = {
    "calls": {"calls"},
    "references": {"references"},
    "inherits": {"inherits", "implements"},
    "imports": {"imports", "imports_from", "re_exports"},
}
DEFAULT_REPOS = [
    "fastapi",
    "graphify",
    "erpnext",
    "excalidraw",
    "ripgrep",
    "monet",
    "kubernetes",
]


def tool_graphs(repo: str) -> dict[str, Path]:
    return {
        "graphify": BENCH / "reference" / f"{repo}.graph.json",
        "dijkstral": BENCH
        / "accuracy"
        / "dijkstral"
        / repo
        / "dijkstral-out"
        / "graph.json",
    }


def pair(a: str, b: str) -> frozenset:
    return frozenset((a, b))


def pct(a: int, b: int) -> float | None:
    return round(100 * a / b, 1) if b else None


class ToolView:
    """One tool's graph seen through the oracle mapping."""

    def __init__(self, oracle: dict, graph: dict):
        self.nodes = {n["id"]: n for n in graph["nodes"]}
        self.links = graph.get("links", graph.get("edges", []))
        self.map, self.map_stats = map_defs(oracle, graph["nodes"])
        self.known = set(self.map.values())
        self.docs = set(oracle["docs"])
        self.pairs: dict[frozenset, list[dict]] = defaultdict(list)
        for e in self.links:
            self.pairs[pair(e["source"], e["target"])].append(e)

    def first_mapped(self, chain: list[int]) -> str | None:
        for d in chain:
            nid = self.map.get(d)
            if nid is not None:
                return nid
        return None

    def file_of(self, nid: str) -> str:
        return (self.nodes.get(nid) or {}).get("source_file") or ""

    def scope(self, nid: str) -> str:
        if nid in self.known:
            return "known"
        sf = self.file_of(nid)
        if not sf:
            return "stub_or_external"
        if sf not in self.docs:
            return "file_not_indexed"
        return "unmapped_node"


def score_family(fam: str, oracle: dict, tv: ToolView, samples: int) -> dict:
    rels = FAMILIES[fam]
    # ---- oracle side, in this tool's node space
    oracle_pairs: dict[frozenset, tuple] = {}
    unmappable = Counter()
    if fam == "imports":
        for a, b in oracle["edges"]["imports"]:
            oracle_pairs.setdefault(pair(a, b), (a, b, ""))
    else:
        for rec in oracle["edges"][fam]:
            chain, t = rec[0], rec[1]
            src, dst = tv.first_mapped(chain), tv.map.get(t)
            if src is None or dst is None:
                unmappable["source" if src is None else "target"] += 1
                continue
            oracle_pairs.setdefault(pair(src, dst), (src, dst, rec[2]))
    mention_pairs = set()
    for chain, t in oracle["edges"]["mentions"]:
        src, dst = tv.first_mapped(chain), tv.map.get(t)
        if src is not None and dst is not None:
            mention_pairs.add(pair(src, dst))
    other_pairs = set()
    for other in ("calls", "references", "inherits"):
        if other == fam:
            continue
        for rec in oracle["edges"][other]:
            src, dst = tv.first_mapped(rec[0]), tv.map.get(rec[1])
            if src is not None and dst is not None:
                other_pairs.add(pair(src, dst))

    # ---- tool side
    judged: dict[frozenset, dict] = {}
    unjudged = Counter()
    for e in tv.links:
        if e["relation"] not in rels:
            continue
        if fam == "imports":
            fa, fb = tv.file_of(e["source"]), tv.file_of(e["target"])
            if not fa or not fb:
                unjudged["stub_or_external"] += 1
                continue
            if fa not in tv.docs or fb not in tv.docs:
                unjudged["file_not_indexed"] += 1
                continue
            if fa == fb:
                continue
            judged.setdefault(pair(fa, fb), e)
            continue
        sa, sb = tv.scope(e["source"]), tv.scope(e["target"])
        if sa != "known" or sb != "known":
            unjudged[sb if sa == "known" else sa] += 1
            continue
        judged.setdefault(pair(e["source"], e["target"]), e)

    tp = judged.keys() & oracle_pairs.keys()
    fp = judged.keys() - oracle_pairs.keys()
    fn = oracle_pairs.keys() - judged.keys()
    tiers: dict[str, dict] = {}
    for tier in ("EXTRACTED", "INFERRED"):
        j = {p for p, e in judged.items() if e.get("confidence", "EXTRACTED") == tier}
        t = j & tp
        tiers[tier] = {"judged": len(j), "tp": len(t), "precision": pct(len(t), len(j))}
    hidden = Counter()
    if fam != "imports":
        for p in fn:
            for e in tv.pairs.get(p, []):
                if e["relation"] not in rels:
                    hidden[e["relation"]] += 1
                    break
    by_context: dict[str, dict[str, int]] = defaultdict(lambda: {"oracle": 0, "tp": 0})
    if fam in ("references", "inherits"):
        for p, (_s, _t, ctx) in oracle_pairs.items():
            by_context[ctx]["oracle"] += 1
            by_context[ctx]["tp"] += p in tp
    fp_kind = Counter()
    for p in fp:
        fp_kind[
            "oracle_mention"
            if p in mention_pairs
            else "oracle_other_relation"
            if p in other_pairs
            else "none"
        ] += 1

    def show(p: frozenset, e: dict | None) -> str:
        if fam == "imports":
            a, b = sorted(p)
            return f"{a} <-> {b}"
        if e is not None:
            s, t = e["source"], e["target"]
            return f"{s} -{e['relation']}/{e.get('confidence', '')}-> {t}  @ {e.get('source_file')}:{e.get('source_location')}"
        s, t, info = oracle_pairs[p]
        n = tv.nodes.get(s, {})
        return f"{s} -> {t}  @ {n.get('source_file')}:{info if isinstance(info, int) else n.get('source_location')} {'' if isinstance(info, int) else info}"

    return {
        "oracle": len(oracle_pairs),
        "oracle_unmappable": dict(unmappable),
        "judged": len(judged),
        "tp": len(tp),
        "precision": pct(len(tp), len(judged)),
        "recall": pct(len(tp), len(oracle_pairs)),
        "tiers": tiers,
        "hidden_by_collapse": dict(hidden),
        "unjudged": dict(unjudged),
        "fp_kind": dict(fp_kind),
        "recall_by_context": {
            k: {**v, "recall": pct(v["tp"], v["oracle"])} for k, v in by_context.items()
        },
        "_oracle": set(oracle_pairs),
        "_tp": tp,
        "_fp": fp,
        "_fn": fn,
        "_judged": judged,
        "samples": {
            "fp": [show(p, judged[p]) for p in sorted(fp, key=sorted)[:samples]],
            "fn": [show(p, None) for p in sorted(fn, key=sorted)[:samples]],
        },
    }


def disagreements(
    fam: str, res: dict[str, dict], views: dict[str, ToolView], limit: int
) -> dict:
    """Pairs one tool gets right and the other wrong, by node id (ids mostly agree)."""
    g, d = res["graphify"], res["dijkstral"]
    out = {}
    for name, a, b in (("graphify_only_tp", g, d), ("dijkstral_only_tp", d, g)):
        pairs = sorted(a["_tp"] - b["_tp"], key=sorted)
        out[name] = {
            "count": len(pairs),
            "examples": [" <-> ".join(sorted(p)) for p in pairs[:limit]],
        }
    for name, a, b, tool in (
        ("graphify_only_fp", g, d, "graphify"),
        ("dijkstral_only_fp", d, g, "dijkstral"),
    ):
        pairs = sorted(a["_fp"] - b["_judged"].keys(), key=sorted)
        ex = []
        for p in pairs[:limit]:
            e = a["_judged"][p]
            if fam == "imports":
                ex.append(" <-> ".join(sorted(p)))
            else:
                ex.append(
                    f"{e['source']} -{e['relation']}/{e.get('confidence')}-> {e['target']} "
                    f"@ {e.get('source_file')}:{e.get('source_location')}"
                )
        out[name] = {"count": len(pairs), "examples": ex}
    del views
    return out


def tier(r: dict, t: str) -> str:
    return fmt(r["tiers"][t]["precision"]) + f" ({r['tiers'][t]['judged']:,})"


def hidden(r: dict) -> int:
    return sum(r["hidden_by_collapse"].values())


def fmt(x: float | None) -> str:
    return "—" if x is None else f"{x:.1f}"


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--repos", default=",".join(DEFAULT_REPOS))
    ap.add_argument("--rebuild-oracle", action="store_true")
    ap.add_argument("--samples", type=int, default=8)
    ap.add_argument("--out", default=str(HERE / "results"))
    args = ap.parse_args()

    today = dt.date.today().isoformat()
    summary: dict = {"date": today, "repos": {}}
    rows = []
    map_rows = []
    collapse_rows = []
    for repo in [r for r in args.repos.split(",") if r]:
        if not (BENCH / "scip" / repo / "parts").is_dir():
            print(f"skip {repo}: not indexed")
            continue
        oracle = load_or_build(repo, args.rebuild_oracle)
        rs: dict = {
            "docs": len(oracle["docs"]),
            "indexers": oracle["indexers"],
            "tools": {},
            "families": {},
        }
        views = {}
        for tool, path in tool_graphs(repo).items():
            if not path.exists():
                print(f"skip {repo}/{tool}: no graph at {path}")
                continue
            views[tool] = ToolView(oracle, json.loads(path.read_text()))
            st = views[tool].map_stats
            rs["tools"][tool] = {"graph": str(path), "mapping": st}
            for group in ("func", "type"):
                s = st.get(group, {"defs": 0, "exact": 0, "shifted": 0})
                map_rows.append(
                    f"| {repo} | {tool} | {group} | {s['defs']:,} | {fmt(pct(s['exact'], s['defs']))} | "
                    f"{fmt(pct(s['exact'] + s['shifted'], s['defs']))} |"
                )
        fam_pairs: dict[str, dict[str, set]] = defaultdict(dict)
        for fam in FAMILIES:
            res = {
                tool: score_family(fam, oracle, tv, args.samples)
                for tool, tv in views.items()
            }
            for tool, r in res.items():
                fam_pairs[tool][fam] = r["_oracle"]
            if len(res) == 2:
                rs["families"].setdefault(fam, {})["disagreements"] = disagreements(
                    fam, res, views, args.samples
                )
            for tool, r in res.items():
                rs["families"].setdefault(fam, {})[tool] = {
                    k: v for k, v in r.items() if not k.startswith("_")
                }
            g, d = res.get("graphify"), res.get("dijkstral")
            if g and d:
                rows.append(
                    f"| {repo} | {fam} | {g['oracle']:,} / {d['oracle']:,} | "
                    f"{fmt(g['precision'])} | {fmt(g['recall'])} | {fmt(d['precision'])} | {fmt(d['recall'])} | "
                    f"{tier(g, 'EXTRACTED')} | {tier(g, 'INFERRED')} | {tier(d, 'EXTRACTED')} | {tier(d, 'INFERRED')} | "
                    f"{hidden(g):,} / {hidden(d):,} | {sum(g['unjudged'].values()):,} / {sum(d['unjudged'].values()):,} |"
                )
            print(
                f"{repo} {fam}: "
                + "  ".join(
                    f"{t} P {fmt(r['precision'])} R {fmt(r['recall'])}"
                    for t, r in res.items()
                )
            )
        # Oracle pairs that carry two or more node-level families (calls + references, …): a graph
        # that keeps one link per pair can show at most one of them, whichever tool built it.
        for tool, fams in fam_pairs.items():
            seen: Counter = Counter()
            for fam in ("calls", "references", "inherits"):
                seen.update(fams.get(fam, set()))
            multi = sum(1 for c in seen.values() if c > 1)
            rs["tools"][tool]["oracle_multi_family_pairs"] = {
                "pairs": len(seen),
                "multi": multi,
            }
            collapse_rows.append(
                f"| {repo} | {tool} | {len(seen):,} | {multi:,} | {fmt(pct(multi, len(seen)))} |"
            )
        summary["repos"][repo] = rs

    md = [
        f"# dijkstral vs Graphify: accuracy against SCIP ({today})",
        "",
        (
            "Precision / recall in % over unordered node pairs within the oracle's scope. "
            "`oracle` = oracle pairs in Graphify / dijkstral node space. Tier columns: precision of "
            "the tool's EXTRACTED and INFERRED edges (judged edge count). `collapsed` = oracle pairs "
            "the tool holds under another relation (one link per pair). `unjudged` = tool edges "
            "outside scope."
        ),
        "",
        "| repo | family | oracle G / D | G P | G R | D P | D R | G EXTR P | G INF P | D EXTR P | D INF P | collapsed G / D | unjudged G / D |",
        "|---|---|---|---|---|---|---|---|---|---|---|---|---|",
        *rows,
        "",
        "## Oracle pairs with more than one relation",
        "",
        (
            "Node pairs the oracle relates by two or more of calls / references / inherits. A graph "
            "with one link per pair (graph.json §8.2) cannot show all of them."
        ),
        "",
        "| repo | tool node space | oracle pairs | multi-relation pairs | % |",
        "|---|---|---|---|---|",
        *collapse_rows,
        "",
        "## Definition → node mapping",
        "",
        "| repo | tool | kind | SCIP defs | exact line % | with ±3-line shift % |",
        "|---|---|---|---|---|---|",
        *map_rows,
        "",
    ]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / f"accuracy-{today}.json").write_text(
        json.dumps(summary, indent=1, default=sorted)
    )
    (out / f"accuracy-{today}.md").write_text("\n".join(md))
    print("\n".join(md))
    print(f"wrote {out / f'accuracy-{today}.md'} and .json")


if __name__ == "__main__":
    main()

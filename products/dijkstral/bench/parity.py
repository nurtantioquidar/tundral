#!/usr/bin/env python3
"""Compare two graph.json files (dijkstral vs Graphify) as sets.

    uv run --no-project bench/parity.py OURS.json THEIRS.json [--ext .py,.pyi] [--samples 8]
    uv run --no-project bench/parity.py OURS.json THEIRS.json --json summary.json

Nodes are matched by id and links by (source, target, relation). `community` and
`community_name` are left out of the field comparison and compared on their own: the share of
shared nodes with the same community id, NMI and ARI of the two partitions over the shared nodes
(both independent of how communities are numbered), and whether the shared nodes and links come
in the same array order (clustered `graph.json` arrays are sorted by a key that includes
`community`, docs/graphify-format.md §2.2). Graphify's markdown `document` nodes (present in
`update` graphs) are dropped from both sides. `--ext` restricts both graphs to nodes and links
whose `source_file` has one of the extensions, plus the source-less nodes (stubs, externals)
those links touch.

Exit status: 0 always; read the numbers. Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import math
import os
from collections import Counter, defaultdict
from pathlib import Path

IGNORED_NODE_KEYS = {"community", "community_name"}


def load(path: str) -> tuple[dict[str, dict], dict[tuple[str, str, str], dict]]:
    g = json.loads(Path(path).read_text())
    nodes = {n["id"]: n for n in g["nodes"] if n.get("file_type") != "document"}
    links = {}
    for e in g.get("links", g.get("edges", [])):
        if e["source"] in nodes and e["target"] in nodes:
            links[(e["source"], e["target"], e["relation"])] = e
    return nodes, links


def ext_of(path: str) -> str:
    base = os.path.basename(path or "")
    return os.path.splitext(base)[1] or (base if base else "")


def restrict(nodes: dict, links: dict, exts: set[str]) -> tuple[dict, dict]:
    keep_links = {k: e for k, e in links.items() if ext_of(e.get("source_file", "")) in exts}
    touched = {x for (s, t, _) in keep_links for x in (s, t)}
    keep_nodes = {
        i: n
        for i, n in nodes.items()
        if ext_of(n.get("source_file", "")) in exts or (not n.get("source_file") and i in touched)
    }
    return keep_nodes, keep_links


def kind(n: dict) -> str:
    if n.get("type") == "external" or n.get("external"):
        return "external"
    if n.get("file_type") == "rationale":
        return "rationale"
    if n.get("source_file", "") == "":
        return "stub"
    if (
        n.get("source_file")
        and n.get("source_location") in ("L1", None)
        and n["label"].endswith(os.path.basename(n["source_file"]))
    ):
        return "file"
    return "def"


def pct(a: int, b: int) -> str:
    return f"{100 * a / b:5.1f}%" if b else "   — "


def partition_similarity(pairs: list[tuple[int, int]]) -> tuple[float, float]:
    """NMI (arithmetic-mean normalisation) and adjusted Rand index of two labelings."""
    n = len(pairs)
    if n == 0:
        return 1.0, 1.0
    joint = Counter(pairs)
    ca = Counter(a for a, _ in pairs)
    cb = Counter(b for _, b in pairs)

    def entropy(c: Counter) -> float:
        return -sum((v / n) * math.log(v / n) for v in c.values())

    mi = sum((v / n) * math.log(v * n / (ca[a] * cb[b])) for (a, b), v in joint.items())
    ha, hb = entropy(ca), entropy(cb)
    nmi = 1.0 if ha + hb == 0 else 2 * mi / (ha + hb)

    def comb(x: int) -> float:
        return x * (x - 1) / 2

    index = sum(comb(v) for v in joint.values())
    sa, sb = sum(comb(v) for v in ca.values()), sum(comb(v) for v in cb.values())
    expected = sa * sb / comb(n) if n > 1 else 0.0
    top = (sa + sb) / 2
    ari = 1.0 if top == expected else (index - expected) / (top - expected)
    return nmi, ari


def community_agreement(on: dict, tn: dict) -> dict | None:
    pairs = [
        (on[i]["community"], tn[i]["community"])
        for i in on.keys() & tn.keys()
        if on[i].get("community") is not None and tn[i].get("community") is not None
    ]
    if not pairs:
        return None
    equal = sum(a == b for a, b in pairs)
    nmi, ari = partition_similarity(pairs)
    return {"shared": len(pairs), "equal": equal, "nmi": nmi, "ari": ari}


def same_order(ours: list, theirs: list) -> tuple[bool, int]:
    """Whether the common items appear in the same relative order; first diverging position."""
    o, t = set(ours), set(theirs)
    a = [x for x in ours if x in t]
    b = [x for x in theirs if x in o]
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return False, i
    return True, -1


def compare(ours_path: str, theirs_path: str, exts: set[str] | None, samples: int) -> dict:
    on, ol = load(ours_path)
    tn, tl = load(theirs_path)
    if exts:
        on, ol = restrict(on, ol, exts)
        tn, tl = restrict(tn, tl, exts)

    report: dict = {"ours": ours_path, "theirs": theirs_path, "exts": sorted(exts or [])}
    print(f"ours   {ours_path}: {len(on)} nodes, {len(ol)} links")
    print(f"theirs {theirs_path}: {len(tn)} nodes, {len(tl)} links\n")

    # Nodes by kind (kind taken from whichever side has the node).
    shared = on.keys() & tn.keys()
    by_kind: dict[str, Counter] = defaultdict(Counter)
    for i in on.keys() | tn.keys():
        k = kind(tn.get(i) or on[i])
        by_kind[k]["both" if i in shared else ("ours" if i in on else "theirs")] += 1
    print("nodes      both   only-ours only-theirs  recall precision")
    report["nodes"] = {}
    for k in ("file", "def", "stub", "external", "rationale"):
        c = by_kind.get(k, Counter())
        both, o, t = c["both"], c["ours"], c["theirs"]
        report["nodes"][k] = {"both": both, "only_ours": o, "only_theirs": t}
        print(f"{k:9} {both:6} {o:11} {t:11}  {pct(both, both + t)}  {pct(both, both + o)}")
    total = len(shared)
    print(
        f"{'all':9} {total:6} {len(on) - total:11} {len(tn) - total:11}  {pct(total, len(tn))}  {pct(total, len(on))}"
    )
    report["nodes"]["all"] = {"both": total, "only_ours": len(on) - total, "only_theirs": len(tn) - total}

    # Field agreement on shared nodes.
    field_diff: Counter = Counter()
    field_examples: dict[str, list] = defaultdict(list)
    for i in shared:
        a, b = on[i], tn[i]
        for key in (a.keys() | b.keys()) - IGNORED_NODE_KEYS:
            if a.get(key) != b.get(key):
                field_diff[key] += 1
                if len(field_examples[key]) < samples:
                    field_examples[key].append((i, a.get(key), b.get(key)))
    if field_diff:
        print("\nnode field mismatches (shared nodes):")
        for key, n in field_diff.most_common():
            print(f"  {key:22} {n:6}   e.g. {field_examples[key][:3]}")
    report["node_field_mismatches"] = dict(field_diff)

    # Links by relation.
    rels = sorted({r for (_, _, r) in ol.keys() | tl.keys()})
    print("\nlinks by relation      both   only-ours only-theirs  recall precision")
    report["links"] = {}
    both_all = 0
    for r in rels:
        o = {k for k in ol if k[2] == r}
        t = {k for k in tl if k[2] == r}
        b = o & t
        both_all += len(b)
        report["links"][r] = {"both": len(b), "only_ours": len(o - t), "only_theirs": len(t - o)}
        print(f"  {r:18} {len(b):6} {len(o - t):11} {len(t - o):11}  {pct(len(b), len(t))}  {pct(len(b), len(o))}")
    print(
        f"  {'all':18} {both_all:6} {len(ol) - both_all:11} {len(tl) - both_all:11}  {pct(both_all, len(tl))}  {pct(both_all, len(ol))}"
    )
    report["links"]["all"] = {"both": both_all, "only_ours": len(ol) - both_all, "only_theirs": len(tl) - both_all}

    link_diff: Counter = Counter()
    link_examples: dict[str, list] = defaultdict(list)
    for k in ol.keys() & tl.keys():
        a, b = ol[k], tl[k]
        for key in a.keys() | b.keys():
            if a.get(key) != b.get(key):
                link_diff[key] += 1
                if len(link_examples[key]) < samples:
                    link_examples[key].append((k, a.get(key), b.get(key)))
    if link_diff:
        print("\nlink field mismatches (shared links):")
        for key, n in link_diff.most_common():
            print(f"  {key:22} {n:6}   e.g. {link_examples[key][:2]}")
    report["link_field_mismatches"] = dict(link_diff)

    # Communities and array order (clustered graphs only).
    agreement = community_agreement(on, tn)
    report["communities"] = agreement
    if agreement:
        print(
            f"\ncommunities (shared nodes): same id {agreement['equal']}/{agreement['shared']}"
            f" {pct(agreement['equal'], agreement['shared'])}  NMI {agreement['nmi']:.4f}  ARI {agreement['ari']:.4f}"
        )
        ours_raw = json.loads(Path(ours_path).read_text())
        theirs_raw = json.loads(Path(theirs_path).read_text())
        node_ok, node_at = same_order(
            [n["id"] for n in ours_raw["nodes"] if n["id"] in on],
            [n["id"] for n in theirs_raw["nodes"] if n["id"] in tn],
        )
        link_key = lambda e: (e["source"], e["target"], e["relation"])  # noqa: E731
        link_ok, link_at = same_order(
            [link_key(e) for e in ours_raw.get("links", []) if link_key(e) in ol],
            [link_key(e) for e in theirs_raw.get("links", []) if link_key(e) in tl],
        )
        print(
            "array order (shared items): nodes "
            + ("same" if node_ok else f"differ from position {node_at}")
            + ", links "
            + ("same" if link_ok else f"differ from position {link_at}")
        )
        report["array_order"] = {"nodes_same": node_ok, "links_same": link_ok}
    else:
        print("\ncommunities: not compared (a graph has no `community`)")

    if samples:
        print(f"\nsamples (up to {samples} per bucket):")
        for name, ids in (
            ("nodes only in theirs", tn.keys() - on.keys()),
            ("nodes only in ours", on.keys() - tn.keys()),
        ):
            pick = sorted(ids)[:samples]
            print(f"  {name}:")
            for i in pick:
                n = tn.get(i) or on[i]
                print(f"    {i}  [{n.get('label')}] {n.get('source_file')}:{n.get('source_location')}")
        for name, keys in (
            ("links only in theirs", tl.keys() - ol.keys()),
            ("links only in ours", ol.keys() - tl.keys()),
        ):
            per_rel: dict[str, list] = defaultdict(list)
            for k in sorted(keys):
                if len(per_rel[k[2]]) < samples:
                    per_rel[k[2]].append(k)
            print(f"  {name}:")
            for r, ks in sorted(per_rel.items()):
                for s, t, _ in ks:
                    e = tl.get((s, t, r)) or ol[(s, t, r)]
                    print(
                        f"    {r:13} {s} -> {t}  {e.get('source_file')}:{e.get('source_location')} {e.get('context') or ''}"
                    )
    return report


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("ours")
    p.add_argument("theirs")
    p.add_argument("--ext", default="", help="comma-separated extensions to restrict to, e.g. .py or .ts,.tsx")
    p.add_argument("--samples", type=int, default=6)
    p.add_argument("--json", help="also write the summary as JSON here")
    args = p.parse_args()
    exts = {e.strip() for e in args.ext.split(",") if e.strip()} or None
    report = compare(args.ours, args.theirs, exts, args.samples)
    if args.json:
        Path(args.json).write_text(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()

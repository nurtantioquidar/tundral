#!/usr/bin/env python3
"""Regenerate the cross-repo goldens (docs/graphify-multirepo.md section 1) by running Graphify.

Writes inputs/*.json (crafted graphs), goldens/* and cases.json next to this script. Each case runs
in a fresh scratch workspace with HOME set to <workspace>/home, so `global` never touches the real
~/.graphify. Graphify is only run, never read.

    GRAPHIFY=~/.cache/dijkstral-bench/tools/graphify-0.9.86/bin/graphify python3 -I regen.py
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = os.path.dirname(HERE)
GRAPHIFY = os.environ.get(
    "GRAPHIFY",
    os.path.expanduser("~/.cache/dijkstral-bench/tools/graphify-0.9.86/bin/graphify"),
)


def G(nodes, links, **kw):
    g = {"directed": False, "multigraph": False, "graph": {"schema_version": 1},
         "nodes": nodes, "links": links, "hyperedges": []}
    g.update(kw)
    return g


# Crafted inputs, written to inputs/<name>.json.
INPUTS = {
    "alpha": G([
        {"id": "a1", "label": "A1", "community": 3, "file_type": "code", "source_file": "a.py", "source_location": "L1"},
        {"id": "a2", "label": "Café—", "community": 7, "file_type": "code", "source_file": "a.py", "source_location": "L2"},
        {"id": "x", "label": "x", "community": 0, "external": True, "file_type": "concept", "source_file": "", "type": "external"},
        {"id": "nofile", "label": "nofile", "community": 0, "file_type": "concept", "source_file": ""},
        {"id": "typeext", "label": "typeext", "community": 0, "type": "external", "source_file": "t.py"},
        {"id": "extfalse", "label": "extfalse", "community": 0, "external": False, "source_file": "t.py"},
        {"id": "nocomm", "label": "nocomm", "source_file": "a.py"},
        {"id": "strcomm", "label": "strcomm", "community": "9", "source_file": "a.py"},
    ], [
        {"source": "a1", "target": "a2", "relation": "calls", "confidence": "EXTRACTED", "weight": 1.0},
        {"source": "a2", "target": "x", "relation": "imports", "confidence": "EXTRACTED"},
        {"source": "x", "target": "nofile", "relation": "weird"},
    ], graph={"schema_version": 1, "graphify_version": "0.9.80", "name": "A"},
        hyperedges=[{"id": "h1", "nodes": ["a1", "a2", "x"], "relation": "group", "label": "H1"}],
        built_at_commit="abc123"),
    "beta": G([
        {"id": "a1", "label": "B-A1", "community": 0, "source_file": "b.py"},
        {"id": "b2", "label": "B2", "community": 4, "source_file": "b.py"},
        {"id": "x", "label": "X-from-B", "community": 5, "external": True, "source_file": "", "extra": 1},
    ], [
        {"source": "a1", "target": "b2", "relation": "calls", "note": "first"},
        {"source": "b2", "target": "a1", "relation": "uses", "note": "second", "zz": 1},
        {"source": "x", "target": "a1", "relation": "rev"},
    ], graph={"schema_version": 1, "graphify_version": "0.9.86"},
        hyperedges=[{"id": "h1", "nodes": ["a1", "b2"], "relation": "group", "label": "HB"}],
        built_at_commit="def456"),
    "nocomm": G([{"id": "n", "label": "n"}], []),
    "ints": G([{"id": "a", "community": 2}, {"id": "b", "community": 0}], []),
    "bigext": G([{"id": "a", "community": 0}, {"id": "e", "community": 50, "external": True}], []),
    "neg": G([{"id": "a", "community": -5}, {"id": "b", "community": -1}], []),
    "flt": G([{"id": "a", "community": 2.0}, {"id": "b", "community": 1.5}, {"id": "c", "community": True}], []),
    "ghost": G([{"id": "a", "community": 0}], [{"source": "a", "target": "ghost", "relation": "r"}, {"source": "e", "target": "a", "relation": "q"}]),
    "rawedges": {"nodes": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}], "edges": [{"source": "a", "target": "b", "relation": "calls"}], "hyperedges": [], "input_tokens": 0, "output_tokens": 0},
    "directed": G([{"id": "a", "community": 0}, {"id": "b", "community": 1}], [{"source": "a", "target": "b", "relation": "r1"}, {"source": "b", "target": "a", "relation": "r2"}], directed=True),
    "multi": G([{"id": "a", "community": 0}, {"id": "b", "community": 1}], [{"source": "a", "target": "b", "relation": "r1", "key": 0}, {"source": "a", "target": "b", "relation": "r2", "key": 1}], multigraph=True),
    "upd": G([{"id": "a", "community": 0}, {"id": "b", "community": 1}], [{"source": "a", "target": "b", "relation": "r1", "only_first": 1, "shared": "first"}, {"source": "b", "target": "a", "relation": "r2", "shared": "second"}]),
    "dord": G([{"id": "a", "community": 0}, {"id": "b", "community": 0}], [{"source": "b", "target": "a", "relation": "r2"}, {"source": "a", "target": "b", "relation": "r1"}], directed=True),
    "mord": G([{"id": "a", "community": 0}, {"id": "b", "community": 0}], [{"source": "a", "target": "b", "relation": "r1"}, {"source": "b", "target": "a", "relation": "r2"}], multigraph=True),
    "hyper": G([{"id": "a", "community": 0}], [], hyperedges=[{"id": "h", "nodes": ["a", "zz", "e"]}, {"label": "noid", "nodes": ["a"]}, {"id": "h3"}, "notadict"]),
    "hk": G([{"id": "a", "community": 0}], [], hyperedges=[{"nodes": ["a"], "label": "L", "id": "h"}]),
    "hboth": G([{"id": "a", "community": 0}], [], graph={"schema_version": 1, "hyperedges": [{"id": "g"}]}, hyperedges=[{"id": "t"}]),
    "hgempty": G([{"id": "a", "community": 0}], [], graph={"schema_version": 1, "hyperedges": []}),
    "hgempty2": G([{"id": "a", "community": 0}], [], graph={"schema_version": 1, "hyperedges": []}, hyperedges=[{"id": "t"}]),
    "hnull": G([{"id": "a", "community": 0}], [], hyperedges=None),
    "nograph": {"nodes": [{"id": "a", "community": 0}], "links": []},
    "nog1": {"nodes": [{"id": "a"}], "links": []},
    "nog2": {"nodes": [{"id": "b"}], "links": []},
    "schema2": G([{"id": "a", "community": 0}], [], graph={"schema_version": 2, "foo": "bar", "hyperedges": [{"id": "old"}]}),
    "premerged": G([{"id": "t::a", "community": 4, "repo": "t", "local_id": "a", "local_community": 1}], []),
    "extnode": G([{"id": "e", "community": 0, "external": True, "label": "E"}, {"id": "a", "community": 3}], [{"source": "a", "target": "e", "relation": "imports"}]),
    "extstr": G([{"id": "e", "community": 0, "external": "yes"}, {"id": "f", "community": 0, "external": 1}], []),
    "extnocomm": G([{"id": "e", "external": True}, {"id": "a", "community": 1}], [{"source": "a", "target": "e", "relation": "r"}]),
    "numid": G([{"id": 7, "community": 0}, {"id": 8, "community": 0}], [{"source": 7, "target": 8, "relation": "r"}]),
    "selfloop": G([{"id": "a", "community": 0}], [{"source": "a", "target": "a", "relation": "self"}]),
    "dupnode": G([{"id": "a", "community": 0, "label": "first", "x": 1}, {"id": "a", "community": 2, "label": "second"}], []),
    "noid": G([{"id": "a"}, {"label": "noid"}, {"id": "c"}], []),
    "empty": G([], []),
    "ord": G([{"id": "a", "community": 0}, {"id": "b", "community": 0}, {"id": "c", "community": 0}, {"id": "d", "community": 0}],
             [{"source": "c", "target": "a", "relation": "r1"}, {"source": "b", "target": "a", "relation": "r2"}, {"source": "d", "target": "b", "relation": "r3"}, {"source": "b", "target": "c", "relation": "r4"}]),
    "ext1": G([{"id": "x", "external": True, "community": 0}, {"id": "a", "community": 0}], [{"source": "a", "target": "x", "relation": "r0"}]),
    "ext2": G([{"id": "p", "community": 0}, {"id": "q", "community": 0}, {"id": "x", "external": True, "community": 0}],
              [{"source": "x", "target": "q", "relation": "r_xq"}, {"source": "p", "target": "x", "relation": "r_px"}]),
    "ext2d": G([{"id": "p", "community": 0}, {"id": "q", "community": 0}, {"id": "x", "external": True, "community": 0}],
               [{"source": "x", "target": "q", "relation": "r_xq"}, {"source": "p", "target": "x", "relation": "r_px"}], directed=True),
    "one": G([{"id": "a", "community": 0}, {"id": "b", "community": 1}, {"id": "c", "community": 1}, {"id": "x", "external": True, "community": 0}], []),
    "two": G([{"id": "a", "community": 0}, {"id": "d", "community": 2}, {"id": "x", "external": True, "community": 3}], []),
    "prev": G([{"id": "one::a", "community": 40}, {"id": "one::b", "community": 41}, {"id": "two::a", "community": "s"},
               {"id": "x", "community": 42}, {"id": "unknown::z", "community": 99}, {"id": "two::d"}], []),
    "prev2": G([{"id": "one::a", "community": 5}], []),
    "small": G([{"id": "n", "label": "N", "community": 0}], []),
    # Global graph inputs.
    "r1": G([
        {"id": "f1", "label": "one.py", "source_file": "one.py", "community": 0},
        {"id": "s1", "label": "Foo", "norm_label": "foo", "source_file": "", "community": 1},
        {"id": "s3", "label": "Baz", "norm_label": "baz", "source_file": "", "community": 1},
        {"id": "nosf", "label": "NoSF", "community": 2},
        {"id": "same", "label": "Same", "source_file": "one.py", "community": 0},
        {"id": "e", "label": "e", "external": True, "source_file": "", "community": 3},
    ], [
        {"source": "f1", "target": "s1", "relation": "references"},
        {"source": "f1", "target": "s3", "relation": "references"},
        {"source": "f1", "target": "f1", "relation": "self"},
        {"source": "s1", "target": "f1", "relation": "back"},
        {"source": "f1", "target": "e", "relation": "imports"},
        {"source": "f1", "target": "nosf", "relation": "uses"},
    ], hyperedges=[{"id": "h", "nodes": ["f1", "s1"], "relation": "g"}]),
    "r2": G([
        {"id": "f2", "label": "two.py", "source_file": "two.py", "community": 0},
        {"id": "s2", "label": "Foo", "norm_label": "foo", "source_file": "", "community": 5},
        {"id": "s3", "label": "Other", "norm_label": "other", "source_file": "", "community": 5},
        {"id": "nosf", "label": "NoSF", "community": 6},
        {"id": "same", "label": "Same", "source_file": "two.py", "community": 0},
        {"id": "e", "label": "E2", "external": True, "source_file": "", "community": 7, "extra": 1},
        {"id": "low", "label": "foo", "norm_label": "foo", "source_file": "", "community": 5},
    ], [
        {"source": "f2", "target": "s2", "relation": "references"},
        {"source": "f2", "target": "s3", "relation": "references"},
        {"source": "f2", "target": "e", "relation": "imports"},
        {"source": "f2", "target": "nosf", "relation": "uses"},
        {"source": "f2", "target": "low", "relation": "uses"},
    ]),
    "r1b": G([
        {"id": "f1", "label": "one.py", "source_file": "one.py", "community": 0},
        {"id": "s1", "label": "Foo", "norm_label": "foo", "source_file": "", "community": 1},
        {"id": "new", "label": "New", "source_file": "one.py", "community": 2},
    ], [
        {"source": "f1", "target": "s1", "relation": "references"},
        {"source": "new", "target": "f1", "relation": "calls"},
    ]),
    "flat": G([{"id": "a", "label": "A", "source_file": "a.py"}], []),
    "base": G([
        {"id": "f", "label": "f.py", "source_file": "f.py"},
        {"id": "same", "label": "Same", "source_file": "f.py"},
        {"id": "stub", "label": "Stub", "source_file": ""},
        {"id": "stub2", "label": "Stub", "source_file": ""},
        {"id": "nullsf", "label": "NullSF", "source_file": None},
        {"id": "e", "label": "E", "external": True, "source_file": ""},
        {"id": "uni", "label": "Café", "source_file": "f.py"},
    ], [{"source": "f", "target": "stub", "relation": "r"}, {"source": "f", "target": "stub2", "relation": "r2"}, {"source": "f", "target": "nullsf", "relation": "r3"}]),
    "probe": G([
        {"id": "g", "label": "g.py", "source_file": "g.py"},
        {"id": "s_same", "label": "Same", "source_file": ""},
        {"id": "s_e", "label": "E", "source_file": ""},
        {"id": "s_stub", "label": "Stub", "source_file": ""},
        {"id": "s_null", "label": "NullSF", "source_file": ""},
        {"id": "d1", "label": "Dup", "source_file": ""},
        {"id": "d2", "label": "Dup", "source_file": ""},
        {"id": "nonstub", "label": "Stub", "source_file": "g.py"},
        {"id": "nolabel", "source_file": ""},
        {"id": "loc", "label": "Loc", "source_file": "", "source_location": "L3"},
        {"id": "ext_f", "label": "Ext", "external": False, "source_file": ""},
    ], [{"source": "g", "target": x, "relation": "r_" + x} for x in ["s_same", "s_e", "s_stub", "s_null", "d1", "d2", "nonstub", "nolabel", "loc", "ext_f"]]),
    "probe2": G([
        {"id": "h", "label": "h.py", "source_file": "h.py"},
        {"id": "n1", "source_file": ""},
        {"id": "dd", "label": "Dup", "source_file": ""},
        {"id": "ext", "label": "Ext", "source_file": ""},
    ], [{"source": "h", "target": "n1", "relation": "a"}, {"source": "h", "target": "dd", "relation": "b"}, {"source": "h", "target": "ext", "relation": "c"}]),
    "dirt": G([
        {"id": "b", "label": "Beta", "source_file": "b.py", "file_type": "code", "source_location": "L1", "community": 0},
        {"id": "a", "label": "Alpha", "source_file": "a.py", "file_type": "code", "source_location": "L1", "community": 0},
        {"id": "s1", "label": "S", "source_file": "", "community": 0},
        {"id": "s2", "label": "T", "source_file": "", "community": 0},
    ], [
        {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED", "source_file": "a.py", "source_location": "L2"},
        {"source": "s1", "target": "s2", "relation": "rel"},
    ]),
    "stubs": G([
        {"id": "x", "label": "S", "source_file": ""},
        {"id": "y", "label": "S", "source_file": ""},
        {"id": "z", "label": "T", "source_file": ""},
        {"id": "f", "label": "F", "source_file": "f.py"},
    ], [
        {"source": "x", "target": "y", "relation": "loop"},
        {"source": "y", "target": "z", "relation": "st", "extra": 1},
        {"source": "f", "target": "x", "relation": "ref"},
    ]),
    "cafe": G([{"id": "a", "label": "é", "source_file": "a.py"}], []),
}

FIXTURE_GRAPHS = {"python": "graphs-0.9.86/python.clustered.graphify.json", "go": "graphs-0.9.86/go.clustered.graphify.json",
                  "rust": "graphs-0.9.86/rust.clustered.graphify.json"}


def src(name):
    """A source file for a workspace path, relative to tests/fixtures."""
    if name in FIXTURE_GRAPHS:
        return FIXTURE_GRAPHS[name]
    return f"multirepo/inputs/{name}.json"


def merge_case(name, inputs, extra=(), exit=0, files=None, paths=None):
    """merge-graphs over <input>/graphify-out/graph.json (repeats share one path)."""
    files = dict(files or {})
    args = ["merge-graphs"]
    for i, inp in enumerate(inputs):
        p = paths[i] if paths else f"{inp}/graphify-out/graph.json"
        files[p] = src(inp)
        args.append(p)
    args += ["--out", "out.json", *extra]
    outputs = {"out.json": f"{name}.json"} if exit == 0 else {}
    return {"name": name, "files": files, "steps": [{"args": args, "exit": exit, "outputs": outputs}]}


CASES = [
    merge_case("merge_py_go", ["python", "go"], paths=["pyrepo/graphify-out/graph.json", "gorepo/graphify-out/graph.json"]),
    merge_case("merge_py_go_rs", ["python", "go", "rust"], paths=["pyrepo/graphify-out/graph.json", "gorepo/graphify-out/graph.json", "rsrepo/graphify-out/graph.json"]),
    merge_case("merge_py_twice", ["python", "python"], paths=["pyrepo/graphify-out/graph.json", "pyrepo/graphify-out/graph.json"]),
    merge_case("merge_alpha_beta", ["alpha", "beta"], paths=["alpha/graphify-out/graph.json", "beta/graph.json"]),
    merge_case("merge_beta_alpha", ["beta", "alpha"], paths=["beta/graph.json", "alpha/graphify-out/graph.json"]),
    merge_case("merge_offsets_nocomm", ["nocomm", "ints", "ints"]),
    merge_case("merge_offsets_bigext", ["ints", "bigext", "ints"]),
    merge_case("merge_offsets_negative", ["neg", "ints"]),
    merge_case("merge_offsets_float_bool", ["flt", "ints", "ints"]),
    merge_case("merge_ghost_endpoints", ["ints", "ghost"]),
    merge_case("merge_raw_edges", ["rawedges", "ints"]),
    merge_case("merge_directed", ["directed", "ints"]),
    merge_case("merge_directed_order", ["dord", "one"]),
    merge_case("merge_multigraph", ["multi", "ints"]),
    merge_case("merge_multigraph_order", ["mord", "one"]),
    merge_case("merge_edge_update", ["upd", "ints"]),
    merge_case("merge_hyperedges", ["hyper", "extnode"]),
    merge_case("merge_hyperedge_keys", ["hk", "ints"]),
    merge_case("merge_hyperedges_graph_first", ["hboth", "ints"]),
    merge_case("merge_hyperedges_graph_empty", ["hgempty", "ints"]),
    merge_case("merge_hyperedges_graph_empty_top", ["hgempty2", "ext1"]),
    merge_case("merge_hyperedges_null", ["hnull", "ints"]),
    merge_case("merge_no_graph", ["nograph", "ints"]),
    merge_case("merge_no_graph_at_all", ["nog1", "nog2"]),
    merge_case("merge_graph_attrs", ["schema2", "ints"]),
    merge_case("merge_graph_attrs_late", ["ints", "schema2"]),
    merge_case("merge_premerged_first", ["premerged", "ints"]),
    merge_case("merge_premerged_second", ["ints", "premerged"]),
    merge_case("merge_shared_external", ["extnode", "extnode"]),
    merge_case("merge_external_truthy", ["extstr", "extstr"]),
    merge_case("merge_external_no_community", ["extnocomm", "ints"]),
    merge_case("merge_external_late", ["ints", "extnocomm"]),
    merge_case("merge_numeric_ids", ["numid", "ints"]),
    merge_case("merge_self_loop", ["selfloop", "ints"]),
    merge_case("merge_duplicate_node", ["dupnode", "ints"]),
    merge_case("merge_missing_ids", ["noid", "ext1"]),
    merge_case("merge_empty_first", ["empty", "ints"]),
    merge_case("merge_empty_last", ["ints", "empty"]),
    merge_case("merge_link_order", ["ord", "ints"]),
    merge_case("merge_external_adjacency", ["ext1", "ext2"]),
    merge_case("merge_external_adjacency_directed", ["ext1", "ext2d"]),
    merge_case("merge_previous", ["one", "two"], extra=["--previous", "prev.json"], files={"prev.json": src("prev")}),
    merge_case("merge_previous_partial", ["one", "two"], extra=["--previous", "prev.json"], files={"prev.json": src("prev2")}),
    merge_case("merge_previous_missing", ["one", "two"], extra=["--previous", "nope.json"], exit=1),
    {"name": "merge_missing_input", "files": {"one/graphify-out/graph.json": src("one")},
     "steps": [{"args": ["merge-graphs", "one/graphify-out/graph.json", "nope/graph.json", "--out", "out.json"],
                "exit": 1, "outputs": {}}]},
    merge_case("merge_tags_nested", ["small", "small"], paths=["one/two/graph.json", "three/four/merged.json"]),
    merge_case("merge_tags_default_repo", ["small", "small"], paths=["graph.json", "proj/graphify-out/graph.json"]),
    merge_case("merge_tags_qualified", ["small", "small", "small"], paths=["X/a/proj/graphify-out/graph.json", "Y/a/proj/graphify-out/graph.json", "Y/b/proj/graphify-out/graph.json"]),
    merge_case("merge_tags_mixed", ["small", "small", "small", "small"], paths=["a_proj/graphify-out/graph.json", "X/a/proj/graphify-out/graph.json", "x/proj/graphify-out/graph.json", "proj/graphify-out/graph.json"]),
    merge_case("merge_tags_suffix", ["small", "small", "small"], paths=["proj/graphify-out/graph.json"] * 3),
    merge_case("merge_tags_suffix_taken", ["small", "small", "small"], paths=["proj/graphify-out/graph.json", "proj-2/graphify-out/graph.json", "proj/graphify-out/graph.json"]),
    {"name": "merge_tags_dotdot", "files": {"sub/graphify-out/graph.json": src("small"), "z/o/graph.json": src("ints")},
     "steps": [{"cwd": "sub/graphify-out", "args": ["merge-graphs", "../graphify-out/graph.json", "../../z/o/graph.json", "--out", "../../out.json"],
                "exit": 0, "outputs": {"out.json": "merge_tags_dotdot.json"}}]},
    {"name": "global_py_go", "files": {"pyrepo/graphify-out/graph.json": src("python"), "gorepo/graphify-out/graph.json": src("go")},
     "steps": [
         {"args": ["global", "add", "pyrepo/graphify-out/graph.json"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_py_go.1.graph.json", "@global/global-manifest.json": "global_py_go.1.manifest.json"}},
         {"args": ["global", "add", "gorepo/graphify-out/graph.json", "--as", "golang"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_py_go.2.graph.json", "@global/global-manifest.json": "global_py_go.2.manifest.json"}},
         {"args": ["global", "add", "gorepo/graphify-out/graph.json", "--as", "gox"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_py_go.3.graph.json", "@global/global-manifest.json": "global_py_go.3.manifest.json"}},
         {"args": ["global", "remove", "pyrepo"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_py_go.4.graph.json", "@global/global-manifest.json": "global_py_go.4.manifest.json"}},
         {"args": ["merge-graphs", "@global/global-graph.json", "pyrepo/graphify-out/graph.json", "--out", "out.json"], "exit": 0,
          "outputs": {"out.json": "global_py_go.5.merged.json"}},
     ]},
    {"name": "global_replace", "files": {"r1/graphify-out/graph.json": src("r1"), "r2/graphify-out/graph.json": src("r2"),
                                         "r1b/graphify-out/graph.json": src("r1b"), "nest/flat/graph.json": src("flat")},
     "steps": [
         {"args": ["global", "remove", "r1"], "exit": 1, "outputs": {}},
         {"args": ["global", "add", "r1/graphify-out/graph.json"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_replace.1.graph.json"}},
         {"args": ["global", "add", "r2/graphify-out/graph.json"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_replace.2.graph.json", "@global/global-manifest.json": "global_replace.2.manifest.json"}},
         {"args": ["global", "add", "r2/graphify-out/graph.json"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_replace.2.graph.json", "@global/global-manifest.json": "global_replace.2.manifest.json"}},
         {"args": ["global", "add", "r1b/graphify-out/graph.json", "--as", "r1"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_replace.3.graph.json", "@global/global-manifest.json": "global_replace.3.manifest.json"}},
         {"args": ["global", "add", "nest/flat/graph.json"], "exit": 0,
          "outputs": {"@global/global-manifest.json": "global_replace.4.manifest.json"}},
         {"args": ["global", "remove", "r1"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_replace.5.graph.json", "@global/global-manifest.json": "global_replace.5.manifest.json"}},
         {"args": ["global", "remove", "r2"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_replace.6.graph.json"}},
         {"args": ["global", "add", "missing/graph.json"], "exit": 1, "outputs": {}},
     ]},
    {"name": "global_stubs", "files": {"base/graphify-out/graph.json": src("base"), "probe/graphify-out/graph.json": src("probe"),
                                       "probe2/graphify-out/graph.json": src("probe2")},
     "steps": [
         {"args": ["global", "add", "base/graphify-out/graph.json"], "exit": 0, "outputs": {}},
         {"args": ["global", "add", "probe/graphify-out/graph.json"], "exit": 0, "outputs": {}},
         {"args": ["global", "add", "probe2/graphify-out/graph.json"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_stubs.graph.json", "@global/global-manifest.json": "global_stubs.manifest.json"}},
     ]},
    {"name": "global_direction", "files": {"dirt/graphify-out/graph.json": src("dirt"), "stubs/graphify-out/graph.json": src("stubs"),
                                           "café/graphify-out/graph.json": src("cafe")},
     "steps": [
         {"args": ["global", "add", "dirt/graphify-out/graph.json"], "exit": 0, "outputs": {}},
         {"args": ["global", "add", "stubs/graphify-out/graph.json"], "exit": 0, "outputs": {}},
         {"args": ["global", "add", "café/graphify-out/graph.json"], "exit": 0,
          "outputs": {"@global/global-graph.json": "global_direction.graph.json", "@global/global-manifest.json": "global_direction.manifest.json"}},
         {"args": ["merge-graphs", "@global/global-graph.json", "dirt/graphify-out/graph.json", "--out", "out.json"], "exit": 0,
          "outputs": {"out.json": "global_direction.merged.json"}},
     ]},
]


def normalize_manifest(text, ws):
    """Drop what differs per run: the time and the workspace directory."""
    out = []
    for line in text.split("\n"):
        if line.lstrip().startswith('"added_at": '):
            line = line[: line.index('"added_at": ')] + '"added_at": "<added_at>",'
        out.append(line)
    return "\n".join(out).replace(json.dumps(ws)[1:-1], "<ws>")


def run_case(case, goldens):
    ws = tempfile.mkdtemp(prefix="multirepo-")
    try:
        ws = os.path.realpath(ws)
        home = os.path.join(ws, "home")
        os.makedirs(home)
        gdir = os.path.join(home, ".graphify")
        for dest, source in case["files"].items():
            d = os.path.join(ws, dest)
            os.makedirs(os.path.dirname(d), exist_ok=True)
            shutil.copyfile(os.path.join(FIXTURES, source), d)
        env = dict(os.environ, HOME=home, GRAPHIFY_QUERY_LOG_DISABLE="1")
        for step in case["steps"]:
            args = [a.replace("@global", gdir) for a in step["args"]]
            cwd = os.path.join(ws, step.get("cwd", ""))
            p = subprocess.run([GRAPHIFY, *args], cwd=cwd, env=env, capture_output=True)
            if p.returncode != step["exit"]:
                sys.exit(f"{case['name']}: {args} exited {p.returncode}\n{p.stdout.decode()}{p.stderr.decode()}")
            for path, golden in step["outputs"].items():
                text = open(os.path.join(ws, path.replace("@global", gdir)), encoding="utf-8").read()
                if golden.endswith(".manifest.json"):
                    text = normalize_manifest(text, ws)
                if golden in goldens and goldens[golden] != text:
                    sys.exit(f"{case['name']}: {golden} differs between steps")
                goldens[golden] = text
    finally:
        shutil.rmtree(ws)


def main():
    os.makedirs(os.path.join(HERE, "inputs"), exist_ok=True)
    os.makedirs(os.path.join(HERE, "goldens"), exist_ok=True)
    for name, g in INPUTS.items():
        with open(os.path.join(HERE, "inputs", f"{name}.json"), "w") as f:
            f.write(json.dumps(g, indent=1) + "\n")
    goldens = {}
    for case in CASES:
        run_case(case, goldens)
    for name, text in goldens.items():
        with open(os.path.join(HERE, "goldens", name), "w", encoding="utf-8") as f:
            f.write(text)
    with open(os.path.join(HERE, "cases.json"), "w") as f:
        f.write(json.dumps({
            "tool": "graphify 0.9.86",
            "how": "Each case copies `files` (workspace path -> source under tests/fixtures) into a fresh "
                   "workspace and runs `steps` in order from <workspace>/<cwd>, HOME=<workspace>/home, "
                   "GRAPHIFY_QUERY_LOG_DISABLE=1. `@global` is $HOME/.graphify for Graphify and "
                   "$DIJKSTRAL_HOME (<workspace>/home/.dijkstral) for dijkstral. After a step, each "
                   "`outputs` file must equal goldens/<name> byte for byte; manifests are compared after "
                   "replacing added_at with <added_at> and the workspace path with <ws>. Stdout is not "
                   "compared: messages are each tool's own.",
            "cases": CASES,
        }, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()

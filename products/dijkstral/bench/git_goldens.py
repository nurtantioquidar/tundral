#!/usr/bin/env python3
"""Regenerate the git-integration goldens in cli/tests/fixtures/git/ from the pinned Graphify.

    python3 -I bench/git_goldens.py [--graphify PATH] [--out DIR]

Clean-room: this only runs Graphify's binary (`merge-driver`, `hook status`, `hook uninstall`)
on scratch copies and scratch `git init` repositories, and records what it writes. It never runs
`graphify hook install`. Specification: cli/docs/graphify-git.md.

Merge cases: each case derives `base.json`, `current.json` and `other.json` from a fixture graph,
copies them to a scratch directory as O, A and B, runs `graphify merge-driver O A B` and stores A
afterwards as `graphify.json` (only when the driver succeeded). `cases.json` indexes them.

Hook cases: hand-written hook files, `.gitattributes` lines and `merge.graphify.*` keys in a
scratch repository, with the stdout and exit code of `hook status` and `hook uninstall`, and what
uninstall left behind.
"""

import argparse
import copy
import json
import os
import shutil
import subprocess
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = os.path.join(HERE, "..", "cli", "tests", "fixtures")
DEFAULT_GRAPHIFY = os.path.expanduser("~/.cache/dijkstral-bench/tools/graphify-0.9.86/bin/graphify")
ENV = dict(os.environ, GRAPHIFY_QUERY_LOG_DISABLE="1", PYTHONHASHSEED="0")


def load(rel):
    with open(os.path.join(FIXTURES, rel)) as f:
        return json.load(f)


def dumps(g):
    return json.dumps(g, indent=1, ensure_ascii=False) + "\n"


def node_by_id(g, i):
    return next(n for n in g["nodes"] if n["id"] == i)


def link_between(g, s, t):
    return next(l for l in g["links"] if l["source"] == s and l["target"] == t)


def merge_cases():
    """(name, base, current, other, note); a value of None means an empty file."""
    sol = load("solidity.clustered.graphify.json")
    oc = load("ocaml.clustered.graphify.json")
    hy = load("semantic/hyper_probe.extract.graphify.json")
    raw = load("solidity.graphify.json")
    out = []

    out.append(("identical", sol, sol, sol, "all three equal"))

    cur = copy.deepcopy(sol)
    cur["nodes"].append({"id": "scripts_deploy_deployer_verify", "label": ".verify()",
                         "_origin": "ast", "community": 0, "file_type": "code",
                         "norm_label": ".verify()", "source_file": "scripts/deploy.sol",
                         "source_location": "L30"})
    cur["links"].append({"source": "scripts_deploy_deployer", "target":
                         "scripts_deploy_deployer_verify", "relation": "method", "_origin": "ast",
                         "confidence": "EXTRACTED", "confidence_score": 1.0,
                         "source_file": "scripts/deploy.sol", "source_location": "L30",
                         "weight": 1.0})
    oth = copy.deepcopy(sol)
    oth["nodes"].append({"id": "os", "label": "os", "community": 2, "file_type": "code",
                         "norm_label": "os", "source_file": "", "source_location": ""})
    oth["links"].append({"source": "scripts_deploy", "target": "os", "relation": "imports_from",
                         "_origin": "ast", "confidence": "EXTRACTED", "confidence_score": 1.0,
                         "source_file": "scripts/deploy.sol", "source_location": "L2",
                         "weight": 1.0})
    out.append(("disjoint", sol, cur, oth, "each side adds a different node and link"))

    cur = copy.deepcopy(oc)
    oth = copy.deepcopy(oc)
    node_by_id(cur, "tools_release_main")["label"] = "main (ours)"
    node_by_id(cur, "tools_versions_bump")["label"] = "bump (ours)"
    node_by_id(oth, "tools_release_main")["community"] = 7
    node_by_id(oth, "tools_versions_parse")["label"] = "parse (theirs)"
    node_by_id(oth, "tools_versions_bump")["label"] = "bump (theirs)"
    node_by_id(oth, "tools_versions_bump")["extra"] = "theirs only"
    out.append(("attr-conflict", oc, cur, oth,
                "both sides edit node attributes, one key in conflict"))

    cur = copy.deepcopy(oc)
    node_by_id(cur, "tools_versions_current")["label"] = "current (ours)"
    link_between(cur, "tools_release_main", "tools_release_release")["weight"] = 3.0
    out.append(("ours-only", oc, cur, oc, "only the current side changed"))

    cur = copy.deepcopy(oc)
    gone = "tools_versions_parse"
    cur["nodes"] = [n for n in cur["nodes"] if n["id"] != gone]
    cur["links"] = [l for l in cur["links"] if gone not in (l["source"], l["target"])]
    oth = copy.deepcopy(oc)
    node_by_id(oth, "tools_release_main")["label"] = "main (theirs)"
    out.append(("deletion", oc, cur, oth, "the current side deletes a node and its links"))

    cur = copy.deepcopy(oc)
    oth = copy.deepcopy(oc)
    link_between(cur, "tools_versions_current", "tools_versions_parse")["weight"] = 2.0
    flipped = link_between(oth, "tools_versions_current", "tools_versions_parse")
    flipped["source"], flipped["target"] = flipped["target"], flipped["source"]
    flipped["relation"] = "uses"
    oth["links"].append({"source": "tools_versions_bump", "target": "tools_release_release",
                         "relation": "references", "confidence": "INFERRED",
                         "confidence_score": 0.85, "source_file": "tools/versions.ml",
                         "weight": 1.0})
    out.append(("reversed-edge", oc, cur, oth,
                "the other side flips a link the current side reweighted"))

    cur = copy.deepcopy(hy)
    oth = copy.deepcopy(hy)
    for g in (cur,):
        g["hyperedges"] = [h for h in g["hyperedges"] if h["id"] != "h_one"]
        g["graph"]["hyperedges"] = copy.deepcopy(g["hyperedges"])
    extra = {"id": "h_new", "label": "new group", "nodes": ["readme_a", "src_app_util"],
             "relation": "form", "source_file": "README.md"}
    for g in (oth,):
        g["hyperedges"].append(extra)
        g["hyperedges"][0]["label"] = "code members (theirs)"
        g["graph"]["hyperedges"] = copy.deepcopy(g["hyperedges"])
    out.append(("hyperedges", hy, cur, oth,
                "current drops a hyperedge; other edits one and adds one"))

    cur = copy.deepcopy(raw)
    cur["nodes"].append({"id": "sys", "label": "sys", "file_type": "code", "source_file": "",
                         "source_location": ""})
    cur["edges"].append({"source": "scripts_deploy", "target": "sys", "relation": "imports_from",
                         "confidence": "EXTRACTED", "source_file": "scripts/deploy.sol",
                         "source_location": "L3", "weight": 1.0})
    out.append(("raw-shape", raw, cur, raw, "--no-cluster graphs (edges, no header)"))

    out.append(("empty-base", None, cur, sol, "no common ancestor: git passes an empty base"))

    oth = copy.deepcopy(sol)
    oth["directed"] = True
    out.append(("directed-mismatch", sol, sol, oth, "one side is directed"))

    out.append(("invalid-other", sol, sol, "{not json", "the other side is not JSON"))
    return out


def run_merge(graphify, out_dir):
    root = os.path.join(out_dir, "merge")
    shutil.rmtree(root, ignore_errors=True)
    index = []
    for name, base, cur, oth, note in merge_cases():
        case = os.path.join(root, name)
        os.makedirs(case)
        for fname, g in (("base.json", base), ("current.json", cur), ("other.json", oth)):
            with open(os.path.join(case, fname), "w") as f:
                f.write("" if g is None else g if isinstance(g, str) else dumps(g))
        with tempfile.TemporaryDirectory() as tmp:
            for src, dst in (("base.json", "O"), ("current.json", "A"), ("other.json", "B")):
                shutil.copyfile(os.path.join(case, src), os.path.join(tmp, dst))
            r = subprocess.run([graphify, "merge-driver", "O", "A", "B"], cwd=tmp,
                               capture_output=True, env=ENV)
            err = r.stderr.decode().strip().splitlines()
            entry = {"case": name, "note": note, "exit": r.returncode,
                     "stdout": r.stdout.decode(),
                     "stderr_last_line": err[-1] if err else ""}
            if r.returncode == 0:
                shutil.copyfile(os.path.join(tmp, "A"), os.path.join(case, "graphify.json"))
            else:
                with open(os.path.join(tmp, "A"), "rb") as a, \
                        open(os.path.join(case, "current.json"), "rb") as c:
                    entry["current_untouched"] = a.read() == c.read()
        index.append(entry)
    return index


def git(repo, *args):
    subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True)


def hook_cases():
    """(name, setup) where setup writes files into a fresh repository."""
    shell = "#!/bin/sh\necho user hook\n"
    block = "# graphify-hook-start\necho stand-in body\n# graphify-hook-end\n"
    checkout = "# graphify-checkout-hook-start\necho stand-in body\n# graphify-checkout-hook-end\n"

    def files(**kw):
        def setup(repo):
            for rel, text in kw.items():
                path = os.path.join(repo, rel.replace("__", "/"))
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "w") as f:
                    f.write(text)
        return setup

    def config(*pairs, then=None):
        def setup(repo):
            for k, v in pairs:
                git(repo, "config", k, v)
            if then:
                then(repo)
        return setup

    attrs = "graphify-out/graph.json merge=graphify\n"
    return [
        ("fresh", files()),
        ("foreign-hook", files(**{".git__hooks__post-commit": shell})),
        ("marker-only", files(**{".git__hooks__post-commit": shell + block})),
        ("marker-sole-content", files(**{".git__hooks__post-commit": "#!/bin/sh\n" + block,
                                         ".git__hooks__post-checkout": "#!/bin/sh\n" + checkout})),
        ("marker-between", files(**{".git__hooks__post-commit":
                                    "#!/bin/sh\necho before\n" + block + "echo after\n"})),
        ("config-only", config(("merge.graphify.driver", "x %O %A %B"))),
        ("attributes-only", files(**{".gitattributes": "*.png binary\n" + attrs})),
        ("merge-driver-both", config(("merge.graphify.name", "n"),
                                     ("merge.graphify.driver", "x %O %A %B"),
                                     then=files(**{".gitattributes": attrs}))),
        ("attributes-sole-line", config(("merge.graphify.driver", "x"),
                                        then=files(**{".gitattributes": "  " + attrs}))),
        ("hooks-path", config(("core.hooksPath", ".githooks"),
                              then=files(**{".githooks__post-commit": "#!/bin/sh\n" + block}))),
    ]


def snapshot(repo):
    out = {}
    for rel in (".git/hooks/post-commit", ".git/hooks/post-checkout", ".githooks/post-commit",
                ".gitattributes"):
        p = os.path.join(repo, rel)
        out[rel] = open(p).read() if os.path.exists(p) else None
    r = subprocess.run(["git", "-C", repo, "config", "--get-regexp", r"^merge\."],
                       capture_output=True)
    out["merge config"] = r.stdout.decode()
    return out


def run_hooks(graphify):
    index = []
    with tempfile.TemporaryDirectory() as tmp:
        outside = os.path.join(tmp, "not-a-repo")
        os.makedirs(outside)
        r = subprocess.run([graphify, "hook", "status"], cwd=outside, capture_output=True,
                           env=ENV)
        index.append({"case": "outside-git", "command": "status", "exit": r.returncode,
                      "stdout": r.stdout.decode()})
        r = subprocess.run([graphify, "hook", "uninstall"], cwd=outside, capture_output=True,
                           env=ENV)
        err = r.stderr.decode().strip().splitlines()
        last = err[-1] if err else ""
        last = last.replace(os.path.realpath(outside), "<dir>").replace(outside, "<dir>")
        index.append({"case": "outside-git", "command": "uninstall", "exit": r.returncode,
                      "stderr_last_line": last})
        for name, setup in hook_cases():
            repo = os.path.join(tmp, name)
            os.makedirs(repo)
            git(repo, "init", "-q")
            setup(repo)
            for command in ("status", "uninstall"):
                r = subprocess.run([graphify, "hook", command], cwd=repo, capture_output=True,
                                   env=ENV)
                index.append({"case": name, "command": command, "exit": r.returncode,
                              "stdout": r.stdout.decode().replace(os.path.realpath(repo), "<repo>")
                              .replace(repo, "<repo>"),
                              "after": snapshot(repo) if command == "uninstall" else None})
    return index


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--graphify", default=DEFAULT_GRAPHIFY)
    ap.add_argument("--out", default=os.path.join(FIXTURES, "git"))
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    merge = run_merge(a.graphify, a.out)
    hooks = run_hooks(a.graphify)
    doc = {
        "tool": "graphify 0.9.86",
        "how": "bench/git_goldens.py: merge-driver O A B in a scratch directory; hook "
               "status/uninstall in scratch git repositories with hand-written hook files "
               "(graphify hook install was never run). Env GRAPHIFY_QUERY_LOG_DISABLE=1 "
               "PYTHONHASHSEED=0.",
        "merge": merge,
        "hooks": hooks,
    }
    with open(os.path.join(a.out, "cases.json"), "w") as f:
        f.write(json.dumps(doc, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()

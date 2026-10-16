#!/usr/bin/env python3
"""Validate the key-fact question sets against the pinned corpus (stdlib only, no model calls).

    python3 -I bench/agent/questions/check.py [--corpus-root DIR]

For every questions/<repo>.jsonl:

- schema: field names and types of the shared task contract, unique ids `<repo>-kNN`, kind,
  2-5 facts, 1 <= min_facts <= len(facts), no fact or alias repeated;
- provenance: provenance/<repo>.json exists, records the pinned commit, and lists exactly the
  question's facts, each typed `file` or `symbol`;
- directory facts contain files at the pinned commit, and their aliases are the package name
  in the directory's Cargo.toml (optionally followed by " crate");
- file facts exist at the pinned commit (read from the corpus repository's object store with
  `git show <commit>:<path>`, never from the working tree), and a file alias resolves to exactly
  one path in the repository (it equals or ends with `/<alias>`);
- symbol facts and their aliases occur as whole words in every file provenance lists for them;
- every `read` range lies inside its file, and every SCIP lookup has a definition.

Prints counts per repo and kind; exits non-zero on any failure.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path

QDIR = Path(__file__).resolve().parent
BENCH_SRC = QDIR.parent.parent
KINDS = ("locate", "callers", "impact", "architecture", "flow")
KEYS = {
    "id": str,
    "suite": str,
    "corpus_id": str,
    "corpus": str,
    "semantic": bool,
    "question": str,
    "question_date": type(None),
    "kind": str,
    "reference": str,
    "facts": list,
    "min_facts": int,
}
GRAPH_WORDS = re.compile(
    r"\b(node|nodes|edge|edges|community|communities|god node|graph|graphify|dijkstral)\b", re.I
)


# Words that are domain vocabulary in a corpus repo rather than graph wording.
DOMAIN_WORDS = {"kubernetes": ("node", "nodes")}


class Repo:
    def __init__(self, root: Path, commit: str):
        self.root = root
        self.commit = commit
        self._files: list[str] | None = None
        self._text: dict[str, str | None] = {}

    def git(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["git", f"--git-dir={self.root / '.git'}", *args],
            capture_output=True,
            check=False,
        )

    def files(self) -> list[str]:
        if self._files is None:
            r = self.git("ls-tree", "-r", "--name-only", self.commit)
            if r.returncode:
                sys.exit(f"cannot list {self.root} at {self.commit}: {r.stderr.decode()}")
            self._files = r.stdout.decode().splitlines()
        return self._files

    def text(self, path: str) -> str | None:
        if path not in self._text:
            r = self.git("show", f"{self.commit}:{path}")
            self._text[path] = (
                r.stdout.decode("utf-8", "replace") if r.returncode == 0 else None
            )
        return self._text[path]


def word_in(word: str, text: str) -> bool:
    return re.search(r"(?<![\w])" + re.escape(word) + r"(?![\w])", text) is not None


def check_repo(path: Path, pins: dict[str, str], corpus_root: Path, counts: Counter) -> list[str]:
    errs: list[str] = []
    repo_name = path.stem
    if repo_name not in pins:
        return [f"{path.name}: {repo_name} is not in corpus.toml"]
    repo = Repo(corpus_root / repo_name, pins[repo_name])
    prov_path = QDIR / "provenance" / f"{repo_name}.json"
    if not prov_path.exists():
        return [f"{path.name}: missing {prov_path.relative_to(QDIR)}"]
    prov = json.loads(prov_path.read_text())
    if prov.get("commit") != repo.commit:
        errs.append(f"provenance/{prov_path.name}: commit {prov.get('commit')} != pinned {repo.commit}")
    seen_ids: set[str] = set()
    all_files = repo.files()
    for n, line in enumerate(path.read_text().splitlines(), 1):
        where = f"{path.name}:{n}"
        try:
            t = json.loads(line)
        except json.JSONDecodeError as e:
            errs.append(f"{where}: invalid JSON ({e})")
            continue
        if list(t) != list(KEYS):
            errs.append(f"{where}: keys {list(t)} != {list(KEYS)}")
        for k, ty in KEYS.items():
            if k in t and not isinstance(t[k], ty):
                errs.append(f"{where}: {k} is not {ty.__name__}")
        qid = t.get("id", "")
        where = f"{path.name}:{qid or n}"
        if not re.fullmatch(rf"{re.escape(repo_name)}-k\d\d", qid):
            errs.append(f"{where}: id must be {repo_name}-kNN")
        if qid in seen_ids:
            errs.append(f"{where}: duplicate id")
        seen_ids.add(qid)
        if t.get("suite") != "keyfacts" or t.get("corpus_id") != repo_name:
            errs.append(f"{where}: suite/corpus_id")
        if t.get("corpus") != f"~/.cache/dijkstral-bench/corpus/{repo_name}":
            errs.append(f"{where}: corpus path")
        if t.get("semantic") is not False:
            errs.append(f"{where}: semantic must be false")
        if t.get("kind") not in KINDS:
            errs.append(f"{where}: kind {t.get('kind')}")
        for m in GRAPH_WORDS.finditer(t.get("question", "")):
            if m.group(0).lower() in DOMAIN_WORDS.get(repo_name, ()):
                continue  # e.g. a Kubernetes Node is the domain term, not graph wording
            errs.append(f"{where}: question uses graph/tool wording {m.group(0)!r}")
        facts = t.get("facts", [])
        if not 2 <= len(facts) <= 5:
            errs.append(f"{where}: {len(facts)} facts (want 2-5)")
        if not 1 <= t.get("min_facts", 0) <= len(facts):
            errs.append(f"{where}: min_facts {t.get('min_facts')} out of range")
        strings: list[str] = []
        for f in facts:
            if set(f) != {"fact", "aliases"} or not isinstance(f.get("aliases"), list):
                errs.append(f"{where}: fact entry {f}")
                continue
            strings += [f["fact"], *f["aliases"]]
        dup = [s for s, c in Counter(strings).items() if c > 1]
        if dup:
            errs.append(f"{where}: repeated fact/alias {dup}")
        pq = prov.get("questions", {}).get(qid)
        if pq is None:
            errs.append(f"{where}: no provenance")
            continue
        if [p["fact"] for p in pq["facts"]] != [f.get("fact") for f in facts]:
            errs.append(f"{where}: provenance facts differ from the task's")
            continue
        for f, p in zip(facts, pq["facts"]):
            fact = f["fact"]
            if p["type"] == "file":
                if fact not in all_files:
                    errs.append(f"{where}: file {fact} not at {repo.commit[:12]}")
                for a in f["aliases"]:
                    hits = [x for x in all_files if x == a or x.endswith("/" + a)]
                    if hits != [fact]:
                        errs.append(f"{where}: alias {a!r} resolves to {hits[:4]}")
            elif p["type"] == "dir":
                if not any(x.startswith(fact.rstrip("/") + "/") for x in all_files):
                    errs.append(f"{where}: directory {fact} not at {repo.commit[:12]}")
                manifest = repo.text(fact.rstrip("/") + "/Cargo.toml") or ""
                pkg = re.search(r'^name\s*=\s*"([^"]+)"', manifest, re.M)
                names = {pkg.group(1), pkg.group(1) + " crate"} if pkg else set()
                base = fact.rstrip("/").rsplit("/", 1)[-1]
                dirs = {x.rsplit("/", 1)[0] for x in all_files if "/" in x}
                same_base = {d for d in dirs if d == base or d.endswith("/" + base)}
                if same_base == {fact.rstrip("/")}:
                    names.add(base)  # a directory (Go package) name unique in the repo
                for a in f["aliases"]:
                    if a not in names:
                        errs.append(
                            f"{where}: alias {a!r} is neither the package name of {fact} "
                            "nor its unique directory name"
                        )
            elif p["type"] == "symbol":
                if not p["in"]:
                    errs.append(f"{where}: symbol {fact} lists no files")
                for file in p["in"]:
                    text = repo.text(file)
                    if text is None:
                        errs.append(f"{where}: {file} (for {fact}) not at {repo.commit[:12]}")
                        continue
                    for s in (fact, *f["aliases"]):
                        if not word_in(s, text):
                            errs.append(f"{where}: {s!r} does not occur in {file}")
            else:
                errs.append(f"{where}: fact type {p['type']}")
        for r in pq.get("read", []):
            text = repo.text(r["file"])
            if text is None:
                errs.append(f"{where}: read {r['file']} not at {repo.commit[:12]}")
                continue
            a, _, b = r["lines"].partition("-")
            nlines = text.count("\n") + (0 if text.endswith("\n") else 1)
            if not (1 <= int(a) <= int(b or a) <= nlines):
                errs.append(f"{where}: read {r['file']}:{r['lines']} outside 1-{nlines}")
        for s in pq.get("scip", []):
            if not s.get("definitions"):
                errs.append(f"{where}: SCIP lookup {s.get('name')} has no definition")
        counts[(repo_name, t.get("kind"))] += 1
    return errs


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--corpus-root",
        default=os.path.join(
            os.environ.get("DIJKSTRAL_BENCH_HOME", os.path.expanduser("~/.cache/dijkstral-bench")),
            "corpus",
        ),
    )
    a = ap.parse_args()
    pins = {
        r["name"]: r["commit"]
        for r in tomllib.loads((BENCH_SRC / "corpus.toml").read_text())["repo"]
    }
    files = sorted(QDIR.glob("*.jsonl"))
    if not files:
        sys.exit("no question files")
    counts: Counter = Counter()
    errs: list[str] = []
    for f in files:
        errs += check_repo(f, pins, Path(a.corpus_root), counts)
    repos = sorted({r for r, _ in counts})
    print(f"{'repo':<12}" + "".join(f"{k:>14}" for k in KINDS) + f"{'total':>8}")
    for r in repos:
        row = [counts[(r, k)] for k in KINDS]
        print(f"{r:<12}" + "".join(f"{v:>14}" for v in row) + f"{sum(row):>8}")
    total = [sum(counts[(r, k)] for r in repos) for k in KINDS]
    print(f"{'all':<12}" + "".join(f"{v:>14}" for v in total) + f"{sum(total):>8}")
    if errs:
        print(f"\n{len(errs)} problem(s):")
        for e in errs:
            print("  " + e)
        sys.exit(1)
    print("\nall checks passed")


if __name__ == "__main__":
    main()

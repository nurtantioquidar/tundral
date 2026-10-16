"""Key-fact scoring for the agent-outcome benchmark (`keyfacts` suite). Stdlib only.

The memory suites are scored by `memory.py` (`locomo_score`, `longmemeval_score`, `f1`).

A fact (or one of its aliases) is a repository path or an identifier; it counts as named when it
occurs in the answer, case-sensitively, on boundaries (questions/README.md):

- Identifier: not preceded or followed by `[A-Za-z0-9_]`, so `get_openapi` is not credited by
  `get_openapi_path` nor `Preempt` by `DefaultPreemption`. A qualifier before it (`Foo.updatePod`,
  `mod::search_path`) and call parentheses after it (`updatePod()`) are fine; a trailing `()` on
  the fact itself is ignored.
- Path (contains `/` or `\\`, or ends in a source-file extension): `\\` and `/` are the same
  separator; the path must not be preceded by a path character (`[A-Za-z0-9_.\\-/]`, so a short
  path is not credited as the tail of a longer one, e.g. `utils.py` inside
  `fastapi/dependencies/utils.py`) except for a leading `./`, and not followed by one, except for a
  sentence-ending `.` (`fastapi/routing.py.` and `fastapi/routing.py:120` both count). A directory
  fact (no file extension) may be followed by `/`: `crates/searcher/src/lib.rs` names
  `crates/searcher`.
- Backticks and quotes in the answer are ignored. `pass` needs `min_facts` matches (all facts when
  it is missing or 0).
"""

from __future__ import annotations

import re

_QUOTES = str.maketrans(
    {"`": " ", "‘": " ", "’": " ", "“": " ", "”": " ", '"': " ", "'": " "}
)
_EXT = [
    "py",
    "pyi",
    "rs",
    "go",
    "ts",
    "tsx",
    "js",
    "jsx",
    "mjs",
    "java",
    "kt",
    "kts",
    "scala",
    "c",
    "h",
    "cc",
    "cpp",
    "hpp",
    "cs",
    "rb",
    "php",
    "swift",
    "ex",
    "exs",
    "erl",
    "ml",
    "jl",
    "lua",
    "dart",
    "m",
    "mm",
    "zig",
    "sh",
    "toml",
    "yaml",
    "yml",
    "json",
    "md",
    "rst",
    "txt",
    "proto",
    "mod",
    "sum",
    "cfg",
    "ini",
    "lock",
    "html",
    "css",
    "vue",
    "svelte",
]
_PATH_EXT = re.compile(r"\.(" + "|".join(_EXT) + r")$")
_IDENT = "A-Za-z0-9_"
_PATHCH = r"A-Za-z0-9_.\-/"


def _answer_text(answer: str) -> str:
    return answer.translate(_QUOTES).replace("\\", "/")


def is_path(fact: str) -> bool:
    f = fact.strip()
    return " " not in f and ("/" in f or "\\" in f or bool(_PATH_EXT.search(f)))


def fact_pattern(fact: str) -> re.Pattern[str] | None:
    f = fact.strip()
    if is_path(f):
        f = f.replace("\\", "/")
        while f.startswith("./"):
            f = f[2:]
        if not f:
            return None
        directory = not _PATH_EXT.search(f)
        f = f.rstrip("/")
        after = rf"(?![{_IDENT}\-])" if directory else rf"(?![{_IDENT}\-/])"
        return re.compile(
            rf"(?:(?<![{_PATHCH}])|(?<=(?<![{_PATHCH}])\./)){re.escape(f)}{after}(?!\.[{_IDENT}])"
        )
    f = f.removesuffix("()")
    if not f:
        return None
    return re.compile(rf"(?<![{_IDENT}]){re.escape(f)}(?![{_IDENT}])")


def fact_matches(fact: dict, answer: str) -> bool:
    """Whether `fact["fact"]` or any of `fact["aliases"]` is named in `answer`."""
    text = _answer_text(answer)
    for cand in [fact.get("fact", ""), *(fact.get("aliases") or [])]:
        pat = fact_pattern(str(cand))
        if pat is not None and pat.search(text):
            return True
    return False


def score_facts(facts: list[dict], answer: str, min_facts: int | None = None) -> dict:
    """{matched, total, recall, pass, missing}. `pass` needs `min_facts` matches (default: all)."""
    hits = [fact_matches(f, answer) for f in facts]
    total = len(facts)
    matched = sum(hits)
    need = min(min_facts, total) if min_facts else total
    return {
        "matched": matched,
        "total": total,
        "recall": (matched / total) if total else None,
        "pass": (matched >= need) if total else None,
        "missing": [
            f.get("fact") for f, hit in zip(facts, hits, strict=True) if not hit
        ],
    }

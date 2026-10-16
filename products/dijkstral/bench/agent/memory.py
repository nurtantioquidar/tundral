#!/usr/bin/env python3
"""Stage-7 chat-memory suites: LOCOMO and LongMemEval-S as Markdown corpora and agent tasks.

    uv run --no-project bench/agent/memory.py fetch            # download and verify both datasets
    uv run --no-project bench/agent/memory.py build            # corpora + task files, default sizes
    uv run --no-project bench/agent/memory.py build --suite locomo --locomo-per-conv 12 --seed 7

fetch: downloads the pinned dataset files into $DIJKSTRAL_BENCH_HOME/datasets/ (default
~/.cache/dijkstral-bench) and checks their sha256 and size.

build: samples the questions (seeded, stratified), writes one corpus folder per conversation
history under $DIJKSTRAL_BENCH_HOME/memory-corpus/<suite>/<corpus_id>/ (README.md plus one
session_<nn>.md per session), a manifest.json next to the corpus folders, and the task files
bench/agent/tasks/<suite>.jsonl. Output is deterministic: the same dataset, sizes and seed give
byte-identical files. A corpus file is rewritten only when its bytes change, so graphs built over
it stay cached. The corpora carry the conversation only: no dialog ids, evidence, answer-session
markers, summaries or question text.

Scoring helpers for the runner (importable): `f1(prediction, reference)` (LOCOMO's normalisation
and token F1), `locomo_score(prediction, reference, kind)` (LOCOMO's per-category rule),
`declines(prediction)` and `longmemeval_score(prediction, reference, abstention)` (abstention
handling), and `longmemeval_judge_prompt(...)` (LongMemEval's official LLM-judge prompt, for a
runner that grades with a model). Stdlib only; no model calls.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import re
import string
import sys
import urllib.request
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path

HERE = Path(__file__).resolve().parent
TASKS_DIR = HERE / "tasks"
BENCH_HOME = Path(
    os.environ.get("DIJKSTRAL_BENCH_HOME", Path.home() / ".cache/dijkstral-bench")
)
DEFAULT_SEED = 7
DEFAULT_LOCOMO_PER_CONV = 12
DEFAULT_LONGMEMEVAL_N = 60
# Graphify's semantic chunking (cli/docs/graphify-semantic.md §4.2): a document costs its
# cl100k tokens + 40, packed greedily into chunks of at most 60,000.
TOKEN_BUDGET = 60000
FILE_OVERHEAD_TOKENS = 40


# ---------------------------------------------------------------------------------------------
# Datasets


@dataclass(frozen=True)
class Dataset:
    suite: str
    filename: str
    url: str
    sha256: str
    size: int
    licence: str


LOCOMO_COMMIT = "3eb6f2c585f5e1699204e3c3bdf7adc5c28cb376"
LONGMEMEVAL_REVISION = "98d7416c24c778c2fee6e6f3006e7a073259d48f"
DATASETS = {
    "locomo": Dataset(
        suite="locomo",
        filename="locomo10.json",
        url=(
            "https://raw.githubusercontent.com/snap-research/locomo/"
            f"{LOCOMO_COMMIT}/data/locomo10.json"
        ),
        sha256="79fa87e90f04081343b8c8debecb80a9a6842b76a7aa537dc9fdf651ea698ff4",
        size=2805274,
        licence="CC BY-NC 4.0 (LICENSE.txt of snap-research/locomo)",
    ),
    "longmemeval": Dataset(
        suite="longmemeval",
        filename="longmemeval_s_cleaned.json",
        url=(
            "https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/"
            f"{LONGMEMEVAL_REVISION}/longmemeval_s_cleaned.json"
        ),
        sha256="d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442",
        size=277383467,
        licence="MIT (dataset card of xiaowu0162/longmemeval-cleaned)",
    ),
}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def fetch(suites: list[str], datasets_dir: Path) -> int:
    datasets_dir.mkdir(parents=True, exist_ok=True)
    status = 0
    for suite in suites:
        ds = DATASETS[suite]
        dest = datasets_dir / ds.filename
        if not dest.exists():
            tmp = dest.with_suffix(dest.suffix + ".part")
            print(f"fetching {ds.url}", file=sys.stderr)
            req = urllib.request.Request(
                ds.url, headers={"User-Agent": "dijkstral-bench"}
            )
            with (
                urllib.request.urlopen(req, timeout=600) as resp,
                tmp.open("wb") as out,
            ):
                while block := resp.read(1 << 20):
                    out.write(block)
            tmp.replace(dest)
        digest, size = sha256_file(dest), dest.stat().st_size
        ok = digest == ds.sha256 and size == ds.size
        print(
            f"{suite}\t{dest}\tsha256={digest}\tsize={size}\t{'ok' if ok else 'MISMATCH'}"
        )
        if not ok:
            status = 1
    return status


def load_dataset(suite: str, datasets_dir: Path) -> list[dict]:
    ds = DATASETS[suite]
    path = datasets_dir / ds.filename
    if not path.exists():
        raise SystemExit(f"{path} missing: run `memory.py fetch --suite {suite}` first")
    if sha256_file(path) != ds.sha256:
        raise SystemExit(f"{path}: sha256 differs from the pinned {ds.sha256}")
    with path.open(encoding="utf-8") as f:
        return json.load(f)


# ---------------------------------------------------------------------------------------------
# Stratified sampling


def equal_allocation(
    available: dict[str, int], n: int, order: list[str]
) -> dict[str, int]:
    """Spread n slots over strata as evenly as availability allows.

    Round-robin in `order`: each pass gives one slot to every stratum that still has an unused
    item, so the first strata in `order` receive the remainder.
    """
    alloc = {k: 0 for k in order}
    remaining = min(n, sum(available.get(k, 0) for k in order))
    while remaining:
        for k in order:
            if remaining and alloc[k] < available.get(k, 0):
                alloc[k] += 1
                remaining -= 1
    return alloc


def proportional_allocation(sizes: dict[str, int], n: int) -> dict[str, int]:
    """Largest-remainder (Hamilton) allocation of n slots in proportion to stratum sizes.

    Ties on the remainder go to the larger stratum, then to the stratum key in sort order.
    """
    total = sum(sizes.values())
    n = min(n, total)
    quotas = {k: n * v / total for k, v in sizes.items()}
    alloc = {k: int(q) for k, q in quotas.items()}
    left = n - sum(alloc.values())
    for k in sorted(sizes, key=lambda k: (-(quotas[k] - alloc[k]), -sizes[k], k))[
        :left
    ]:
        alloc[k] += 1
    return alloc


def sample_stratum(items: list[int], k: int, seed: int, key: str) -> list[int]:
    """Seeded sample of k items (returned sorted). Each stratum has its own RNG stream, seeded
    by a string (hashed with SHA-512 by `random`, so independent of PYTHONHASHSEED), so the
    pick in one stratum does not depend on the size of any other."""
    rng = random.Random(f"{seed}:{key}")
    return sorted(rng.sample(sorted(items), k))


# ---------------------------------------------------------------------------------------------
# Corpus rendering


def session_filename(n: int, count: int) -> str:
    """`session_<n>.md`, zero-padded to the session count's width (at least 2) so that path
    order, which is the semantic pass's dispatch order, is chronological."""
    return f"session_{n:0{max(2, len(str(count)))}d}.md"


def render_turn(speaker: str, text: str) -> str:
    text = text.strip("\n")
    return f"**{speaker}:** {text}" if text else f"**{speaker}:**"


def render_session(n: int, date: str, turns: list[tuple[str, str]]) -> str:
    out = [f"# Session {n}", "", f"Date: {date}", ""]
    for speaker, text in turns:
        out += [render_turn(speaker, text), ""]
    return "\n".join(out)


LOCOMO_CATEGORIES = {
    # Numbering from the released evaluation code (task_eval/evaluation.py: 1 is scored as
    # multi-hop with sub-answers, 5 as adversarial; gpt_utils.py adds a date hint to 2) and the
    # data (3: "would X likely…" world-knowledge questions); the README does not list it
    # (snap-research/locomo issue #6).
    1: "multi-hop",
    2: "temporal",
    3: "open-domain",
    4: "single-hop",
    5: "adversarial",
}
LOCOMO_ADVERSARIAL_REFERENCE = "Not mentioned in the conversation"


def locomo_turn_text(turn: dict) -> str:
    text = turn.get("text", "")
    caption = turn.get("blip_caption")
    if caption:
        text = (
            f"{text} [shared an image: {caption}]"
            if text
            else f"[shared an image: {caption}]"
        )
    return text


def locomo_sessions(conv: dict) -> list[tuple[int, str, list[dict]]]:
    nums = sorted(
        int(k.split("_")[1])
        for k, v in conv.items()
        if re.fullmatch(r"session_\d+", k) and isinstance(v, list) and v
    )
    return [(n, conv[f"session_{n}_date_time"], conv[f"session_{n}"]) for n in nums]


def locomo_corpus(sample: dict) -> dict[str, str]:
    conv = sample["conversation"]
    a, b = conv["speaker_a"], conv["speaker_b"]
    sessions = locomo_sessions(conv)
    files = {}
    for n, date, turns in sessions:
        files[session_filename(n, len(sessions))] = render_session(
            n, date, [(t["speaker"], locomo_turn_text(t)) for t in turns]
        )
    files["README.md"] = "\n".join(
        [
            f"# Conversation between {a} and {b}",
            "",
            f"Participants: {a} and {b}.",
            "",
            (
                f"{len(sessions)} chat sessions between two people, from {sessions[0][1]} to "
                f"{sessions[-1][1]}. One file per session, in order; each gives the session's "
                "date and time, then the turns in order, labelled by speaker. Shared images "
                "appear as a text caption in brackets."
            ),
            "",
        ]
    )
    return files


def longmemeval_corpus(item: dict) -> dict[str, str]:
    """Only the haystack: the session order and dates of the dataset, the turns' role and
    content. Session ids (evidence sessions are named `answer_*`), `answer_session_ids` and the
    turns' `has_answer` flags are never written."""
    dates, sessions = item["haystack_dates"], item["haystack_sessions"]
    files = {}
    for i, (date, turns) in enumerate(zip(dates, sessions, strict=True), start=1):
        files[session_filename(i, len(sessions))] = render_session(
            i, date, [(t["role"].capitalize(), t["content"]) for t in turns]
        )
    files["README.md"] = "\n".join(
        [
            "# Chat history between a user and an AI assistant",
            "",
            "Participants: User and Assistant.",
            "",
            (
                f"{len(sessions)} chat sessions, from {dates[0]} to {dates[-1]}. One file per "
                "session, in the order of the history; each gives the session's date and time, "
                "then the turns in order, labelled by speaker."
            ),
            "",
        ]
    )
    return files


def longmemeval_corpus_id(question_id: str) -> str:
    """Opaque: the question id carries `_abs` for abstention questions and the dataset order
    groups question types, so neither may appear in a path the agent sees."""
    return "longmemeval-" + hashlib.sha256(question_id.encode()).hexdigest()[:8]


def write_corpus(root: Path, files: dict[str, str]) -> int:
    """Write files, touching only those whose bytes differ; remove stale session files.
    Returns the number of files written."""
    root.mkdir(parents=True, exist_ok=True)
    written = 0
    for name, text in sorted(files.items()):
        path, data = root / name, text.encode("utf-8")
        if not path.exists() or path.read_bytes() != data:
            path.write_bytes(data)
            written += 1
    for stale in root.glob("session_*.md"):
        if stale.name not in files:
            stale.unlink()
    return written


def approx_tokens(text: str) -> int:
    """About 4 characters per cl100k token for English chat (stdlib estimate)."""
    return round(len(text) / 4)


def corpus_stats(files: dict[str, str]) -> dict:
    tokens = {name: approx_tokens(text) for name, text in files.items()}
    chunks, running = 0, None
    for name in sorted(files):
        cost = tokens[name] + FILE_OVERHEAD_TOKENS
        if cost > TOKEN_BUDGET:
            chunks += -(-len(files[name]) // 20000)
            running = None
        elif running is None or running + cost > TOKEN_BUDGET:
            chunks += 1
            running = cost
        else:
            running += cost
    return {
        "files": len(files),
        "sessions": sum(1 for n in files if n.startswith("session_")),
        "chars": sum(len(t) for t in files.values()),
        "bytes": sum(len(t.encode("utf-8")) for t in files.values()),
        "approx_tokens": sum(tokens.values()),
        "approx_chunks": chunks,
    }


def display_path(path: Path) -> str:
    home = Path.home()
    try:
        return "~/" + str(path.relative_to(home))
    except ValueError:
        return str(path)


# ---------------------------------------------------------------------------------------------
# Task construction


def task_line(**fields) -> dict:
    order = [
        "id",
        "suite",
        "corpus_id",
        "corpus",
        "semantic",
        "question",
        "question_date",
        "kind",
        "reference",
        "facts",
        "min_facts",
        "adversarial",
    ]
    out = {k: fields.pop(k) for k in order}
    out.update(sorted(fields.items()))
    return out


def build_locomo(
    data: list[dict], per_conv: int, seed: int, corpus_root: Path
) -> tuple[list[dict], dict[str, dict[str, str]]]:
    tasks, corpora = [], {}
    cats = sorted(LOCOMO_CATEGORIES)
    for ci, sample in enumerate(data, start=1):
        corpus_id = f"locomo-c{ci:02d}"
        corpora[corpus_id] = locomo_corpus(sample)
        by_cat: dict[int, list[int]] = defaultdict(list)
        for qi, qa in enumerate(sample["qa"]):
            by_cat[qa["category"]].append(qi)
        # Rotate which categories get the remainder slots so suite totals stay even.
        shift = ((ci - 1) * (per_conv % len(cats))) % len(cats)
        order = [str(c) for c in cats[shift:] + cats[:shift]]
        alloc = equal_allocation(
            {str(c): len(v) for c, v in by_cat.items()}, per_conv, order
        )
        picked = sorted(
            qi
            for c in cats
            for qi in sample_stratum(by_cat[c], alloc[str(c)], seed, f"locomo:{ci}:{c}")
        )
        for qi in picked:
            qa = sample["qa"][qi]
            kind = LOCOMO_CATEGORIES[qa["category"]]
            adversarial = kind == "adversarial"
            extra = {"source_id": f"{sample['sample_id']}/qa/{qi}"}
            if adversarial:
                # The dataset's `adversarial_answer` is the distractor LOCOMO offers next to
                # "Not mentioned in the conversation"; the correct response is to decline.
                extra["distractor"] = str(qa["adversarial_answer"])
            tasks.append(
                task_line(
                    id=f"{corpus_id}-q{qi:03d}",
                    suite="locomo",
                    corpus_id=corpus_id,
                    corpus=display_path(corpus_root / corpus_id),
                    semantic=True,
                    question=qa["question"],
                    question_date=None,
                    kind=kind,
                    reference=LOCOMO_ADVERSARIAL_REFERENCE
                    if adversarial
                    else str(qa["answer"]),
                    facts=[],
                    min_facts=0,
                    adversarial=adversarial,
                    **extra,
                )
            )
    return tasks, corpora


def build_longmemeval(
    data: list[dict], n: int, seed: int, corpus_root: Path
) -> tuple[list[dict], dict[str, dict[str, str]]]:
    strata: dict[str, list[int]] = defaultdict(list)
    for i, item in enumerate(data):
        abs_ = item["question_id"].endswith("_abs")
        strata[f"{item['question_type']}{'/abs' if abs_ else ''}"].append(i)
    alloc = proportional_allocation({k: len(v) for k, v in strata.items()}, n)
    picked = sorted(
        i
        for key in sorted(strata)
        for i in sample_stratum(strata[key], alloc[key], seed, f"longmemeval:{key}")
    )
    tasks, corpora = [], {}
    for i in picked:
        item = data[i]
        corpus_id = longmemeval_corpus_id(item["question_id"])
        if corpus_id in corpora:
            raise SystemExit(
                f"corpus id collision: {corpus_id} ({item['question_id']})"
            )
        corpora[corpus_id] = longmemeval_corpus(item)
        tasks.append(
            task_line(
                id=corpus_id,
                suite="longmemeval",
                corpus_id=corpus_id,
                corpus=display_path(corpus_root / corpus_id),
                semantic=True,
                question=item["question"],
                question_date=item["question_date"],
                kind=item["question_type"],
                reference=str(item["answer"]),
                facts=[],
                min_facts=0,
                adversarial=item["question_id"].endswith("_abs"),
                source_id=item["question_id"],
            )
        )
    return tasks, corpora


def write_tasks(path: Path, tasks: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = "".join(json.dumps(t, ensure_ascii=False) + "\n" for t in tasks)
    if not path.exists() or path.read_text(encoding="utf-8") != text:
        path.write_text(text, encoding="utf-8")


def build(args: argparse.Namespace, suites: list[str]) -> int:
    for suite in suites:
        data = load_dataset(suite, args.datasets_dir)
        corpus_root = args.corpus_root / suite
        if suite == "locomo":
            tasks, corpora = build_locomo(
                data, args.locomo_per_conv, args.seed, corpus_root
            )
            sizes = {"per_conv": args.locomo_per_conv}
        else:
            tasks, corpora = build_longmemeval(
                data, args.longmemeval_n, args.seed, corpus_root
            )
            sizes = {"questions": args.longmemeval_n}
        written = sum(
            write_corpus(corpus_root / cid, files) for cid, files in corpora.items()
        )
        stats = {cid: corpus_stats(files) for cid, files in sorted(corpora.items())}
        stale = sorted(
            p.name
            for p in corpus_root.iterdir()
            if p.is_dir() and p.name not in corpora
        )
        ds = DATASETS[suite]
        manifest = {
            "suite": suite,
            "dataset": {
                "url": ds.url,
                "sha256": ds.sha256,
                "size": ds.size,
                "licence": ds.licence,
            },
            "seed": args.seed,
            "sizes": sizes,
            "tasks": len(tasks),
            "kinds": dict(sorted(Counter(t["kind"] for t in tasks).items())),
            "adversarial": sum(t["adversarial"] for t in tasks),
            "corpora": stats,
        }
        (corpus_root / "manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        write_tasks(args.tasks_dir / f"{suite}.jsonl", tasks)
        total = Counter()
        for s in stats.values():
            total.update(s)
        print(
            f"{suite}: {len(tasks)} tasks, {len(corpora)} corpora, {total['sessions']} sessions, "
            f"{total['chars']:,} chars, ~{total['approx_tokens']:,} tokens, "
            f"~{total['approx_chunks']} semantic chunks; {written} files written"
        )
        for kind, count in manifest["kinds"].items():
            adv = sum(1 for t in tasks if t["kind"] == kind and t["adversarial"])
            print(
                f"  {kind}: {count}"
                + (f" ({adv} abstention)" if adv and suite != "locomo" else "")
            )
        if stale:
            print(f"  stale corpus folders not in this build: {', '.join(stale)}")
    return 0


# ---------------------------------------------------------------------------------------------
# Scoring


def _porter_cons(w: str, i: int) -> bool:
    c = w[i]
    if c in "aeiou":
        return False
    if c == "y":
        return i == 0 or not _porter_cons(w, i - 1)
    return True


def _porter_m(stem: str) -> int:
    m, prev_vowel = 0, False
    for i in range(len(stem)):
        cons = _porter_cons(stem, i)
        if cons and prev_vowel:
            m += 1
        prev_vowel = not cons
    return m


def _porter_has_vowel(stem: str) -> bool:
    return any(not _porter_cons(stem, i) for i in range(len(stem)))


def _porter_double(w: str) -> bool:
    return len(w) >= 2 and w[-1] == w[-2] and _porter_cons(w, len(w) - 1)


def _porter_cvc(w: str) -> bool:
    return (
        len(w) >= 3
        and _porter_cons(w, len(w) - 3)
        and not _porter_cons(w, len(w) - 2)
        and _porter_cons(w, len(w) - 1)
        and w[-1] not in "wxy"
    )


def _porter_rules(w: str, rules: list[tuple[str, str]], min_m: int) -> str:
    for suffix, repl in sorted(rules, key=lambda r: -len(r[0])):
        if w.endswith(suffix):
            stem = w[: -len(suffix)]
            return stem + repl if _porter_m(stem) > min_m - 1 else w
    return w


_STEP2 = [
    ("ational", "ate"), ("tional", "tion"), ("enci", "ence"), ("anci", "ance"), ("izer", "ize"),
    ("abli", "able"), ("alli", "al"), ("entli", "ent"), ("eli", "e"), ("ousli", "ous"),
    ("ization", "ize"), ("ation", "ate"), ("ator", "ate"), ("alism", "al"), ("iveness", "ive"),
    ("fulness", "ful"), ("ousness", "ous"), ("aliti", "al"), ("iviti", "ive"), ("biliti", "ble"),
]  # fmt: skip
_STEP3 = [
    ("icate", "ic"), ("ative", ""), ("alize", "al"), ("iciti", "ic"), ("ical", "ic"),
    ("ful", ""), ("ness", ""),
]  # fmt: skip
_STEP4 = [
    "al", "ance", "ence", "er", "ic", "able", "ible", "ant", "ement", "ment", "ent", "ion", "ou",
    "ism", "ate", "iti", "ous", "ive", "ize",
]  # fmt: skip


def porter_stem(word: str) -> str:
    """The original Porter (1980) stemmer. LOCOMO stems with NLTK's PorterStemmer, whose default
    mode adds a few extensions (irregular forms such as dying → die); answers rarely hit them."""
    w = word
    if len(w) <= 2:
        return w
    # Step 1a
    if w.endswith(("sses", "ies")):
        w = w[:-2]
    elif w.endswith("ss"):
        pass
    elif w.endswith("s"):
        w = w[:-1]
    # Step 1b
    again = False
    if w.endswith("eed"):
        if _porter_m(w[:-3]) > 0:
            w = w[:-1]
    elif w.endswith("ed") and _porter_has_vowel(w[:-2]):
        w, again = w[:-2], True
    elif w.endswith("ing") and _porter_has_vowel(w[:-3]):
        w, again = w[:-3], True
    if again:
        if w.endswith(("at", "bl", "iz")):
            w += "e"
        elif _porter_double(w) and w[-1] not in "lsz":
            w = w[:-1]
        elif _porter_m(w) == 1 and _porter_cvc(w):
            w += "e"
    # Step 1c
    if w.endswith("y") and _porter_has_vowel(w[:-1]):
        w = w[:-1] + "i"
    w = _porter_rules(w, _STEP2, 1)
    w = _porter_rules(w, _STEP3, 1)
    # Step 4
    for suffix in sorted(_STEP4, key=len, reverse=True):
        if w.endswith(suffix):
            stem = w[: -len(suffix)]
            if _porter_m(stem) > 1 and (suffix != "ion" or stem.endswith(("s", "t"))):
                w = stem
            break
    # Step 5
    if w.endswith("e"):
        stem = w[:-1]
        if _porter_m(stem) > 1 or (_porter_m(stem) == 1 and not _porter_cvc(stem)):
            w = stem
    if _porter_m(w) > 1 and _porter_double(w) and w.endswith("l"):
        w = w[:-1]
    return w


_PUNCT = set(string.punctuation)
_ARTICLES = re.compile(r"\b(a|an|the|and)\b")


def normalize_answer(s: object) -> str:
    """LOCOMO's normalize_answer: drop commas, lowercase, remove punctuation, remove the
    words a/an/the/and, collapse whitespace."""
    s = str(s).replace(",", "").lower()
    s = "".join(ch for ch in s if ch not in _PUNCT)
    s = _ARTICLES.sub(" ", s)
    return " ".join(s.split())


def f1(prediction: object, reference: object) -> float:
    """LOCOMO's token F1 (task_eval/evaluation.py f1_score): normalise both, Porter-stem each
    token, F1 over the token multisets; 0 when nothing overlaps."""
    pred = [porter_stem(w) for w in normalize_answer(prediction).split()]
    ref = [porter_stem(w) for w in normalize_answer(reference).split()]
    same = sum((Counter(pred) & Counter(ref)).values())
    if same == 0:
        return 0.0
    precision, recall = same / len(pred), same / len(ref)
    return 2 * precision * recall / (precision + recall)


def multi_answer_f1(prediction: object, reference: object) -> float:
    """LOCOMO's multi-hop rule: split both on commas; mean over reference parts of the best F1
    against any prediction part."""
    preds = [p.strip() for p in str(prediction).split(",")]
    refs = [r.strip() for r in str(reference).split(",")]
    return sum(max(f1(p, r) for p in preds) for r in refs) / len(refs)


_DECLINE_PATTERNS = (
    r"\bnot mentioned\b",
    r"\bno information\b",
    r"\bnever (?:mentioned|said|told|shared|discussed)\b",
    r"\b(?:did|do|does|have|has) not (?:mention|say|specify|discuss|share|state|talk about)",
    r"\b(?:didn't|don't|doesn't|haven't|hasn't) (?:mention|say|specify|discuss|share|state|talk about)",
    r"\bno (?:mention|record|indication|evidence|details?)\b",
    r"\bnot (?:been )?(?:mentioned|specified|stated|provided|discussed|recorded|available|found|given|included)\b",
    r"\b(?:cannot|can't|could not|couldn't|unable to) (?:find|determine|tell|answer|confirm|locate|see|identify)",
    r"\b(?:not|isn't|is not) (?:in|found in|covered (?:by|in)) (?:the|your|our) (?:conversations?|history|chats?|notes|records|sessions?)",
    r"\b(?:insufficient|not enough|incomplete) information\b",
    r"\bunanswerable\b",
    r"\bnot answerable\b",
    r"\bunknown\b",
)
_DECLINE = re.compile("|".join(_DECLINE_PATTERNS), re.IGNORECASE)


def declines(prediction: object) -> bool:
    """True when the answer says the information is not available. A superset of LOCOMO's own
    adversarial check ("no information available" / "not mentioned", case-insensitive): our
    agent answers open-ended rather than choosing between two options, so it may decline in
    other words ("you never mentioned…", "I can't find…", "not in the conversation").
    Curly apostrophes count as straight ones."""
    return bool(_DECLINE.search(str(prediction).replace("’", "'")))


def locomo_score(
    prediction: object, reference: object, kind: str, strict: bool = False
) -> float:
    """LOCOMO's per-category QA score (task_eval/evaluation.py eval_question_answering):

    - multi-hop: `multi_answer_f1`;
    - temporal, single-hop: `f1`;
    - open-domain: `f1` against the reference up to its first ';';
    - adversarial: 1 if the answer declines, else 0 — `declines()`, or with strict=True
      LOCOMO's exact two-phrase check.
    """
    if kind == "adversarial":
        text = str(prediction).lower()
        if strict:
            return float("no information available" in text or "not mentioned" in text)
        return float(declines(prediction))
    if kind == "multi-hop":
        return multi_answer_f1(prediction, reference)
    if kind == "open-domain":
        reference = str(reference).split(";")[0].strip()
    return f1(prediction, reference)


def longmemeval_score(prediction: object, reference: object, abstention: bool) -> float:
    """Deterministic LongMemEval score. Abstention (`_abs`) questions: 1 if the answer declines
    (`declines()`), else 0 — the official judge accepts an answer that identifies the question
    as unanswerable. Other questions: LOCOMO token `f1` against the reference, a proxy; the
    official metric is the LLM judge of `longmemeval_judge_prompt`, and preference questions
    have a rubric rather than an answer as reference, so they need the judge."""
    if abstention:
        return float(declines(prediction))
    return f1(prediction, reference)


def longmemeval_judge_prompt(
    kind: str, question: str, reference: str, response: str, abstention: bool
) -> str:
    """LongMemEval's official answer-check prompt (src/evaluation/evaluate_qa.py at
    xiaowu0162/LongMemEval 9e0b455, MIT). The judge's reply counts as correct when it contains
    "yes" (case-insensitive)."""
    tail = "\n\nQuestion: {}\n\nCorrect Answer: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only."
    base = "I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response is equivalent to the correct answer or contains all the intermediate steps to get the correct answer, you should also answer yes. If the response only contains a subset of the information required by the answer, answer no."
    if abstention:
        template = "I will give you an unanswerable question, an explanation, and a response from a model. Please answer yes if the model correctly identifies the question as unanswerable. The model could say that the information is incomplete, or some other information is given but the asked information is not.\n\nQuestion: {}\n\nExplanation: {}\n\nModel Response: {}\n\nDoes the model correctly identify the question as unanswerable? Answer yes or no only."
    elif kind in ("single-session-user", "single-session-assistant", "multi-session"):
        template = base + " " + tail
    elif kind == "temporal-reasoning":
        template = (
            base
            + " In addition, do not penalize off-by-one errors for the number of days. If the question asks for the number of days/weeks/months, etc., and the model makes off-by-one errors (e.g., predicting 19 days when the answer is 18), the model's response is still correct. "
            + tail
        )
    elif kind == "knowledge-update":
        template = (
            "I will give you a question, a correct answer, and a response from a model. Please answer yes if the response contains the correct answer. Otherwise, answer no. If the response contains some previous information along with an updated answer, the response should be considered as correct as long as the updated answer is the required answer."
            + tail
        )
    elif kind == "single-session-preference":
        template = "I will give you a question, a rubric for desired personalized response, and a response from a model. Please answer yes if the response satisfies the desired response. Otherwise, answer no. The model does not need to reflect all the points in the rubric. The response is correct as long as it recalls and utilizes the user's personal information correctly.\n\nQuestion: {}\n\nRubric: {}\n\nModel Response: {}\n\nIs the model response correct? Answer yes or no only."
    else:
        raise ValueError(f"unknown LongMemEval question type {kind!r}")
    return template.format(question, reference, response)


# ---------------------------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("fetch", "build"):
        p = sub.add_parser(name)
        p.add_argument(
            "--suite", choices=["locomo", "longmemeval", "all"], default="all"
        )
        p.add_argument("--datasets-dir", type=Path, default=BENCH_HOME / "datasets")
        if name == "build":
            p.add_argument(
                "--corpus-root", type=Path, default=BENCH_HOME / "memory-corpus"
            )
            p.add_argument("--tasks-dir", type=Path, default=TASKS_DIR)
            p.add_argument("--seed", type=int, default=DEFAULT_SEED)
            p.add_argument(
                "--locomo-per-conv",
                type=int,
                default=DEFAULT_LOCOMO_PER_CONV,
                help="questions per LOCOMO conversation, spread evenly over categories",
            )
            p.add_argument(
                "--longmemeval-n",
                type=int,
                default=DEFAULT_LONGMEMEVAL_N,
                help="LongMemEval questions, proportional to question type and abstention",
            )
    args = ap.parse_args(argv)
    suites = list(DATASETS) if args.suite == "all" else [args.suite]
    if args.cmd == "fetch":
        return fetch(suites, args.datasets_dir)
    return build(args, suites)


if __name__ == "__main__":
    sys.exit(main())

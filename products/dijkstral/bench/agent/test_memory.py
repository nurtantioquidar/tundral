"""Tests for memory.py (stdlib unittest, no network, no model).

    uv run --no-project python -m unittest discover -s bench/agent -p 'test_*.py'

Synthetic datasets cover conversion, sampling and leakage; when the real datasets are present in
$DIJKSTRAL_BENCH_HOME/datasets the leakage and determinism checks also run on them.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import sys
import tempfile
import unittest
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import memory


def locomo_sample(sample_id: str, cats: dict[int, int]) -> dict:
    qa, n = [], 0
    for cat, count in cats.items():
        for _ in range(count):
            q = {"question": f"Q{n}?", "evidence": [f"D1:{n % 3 + 1}"], "category": cat}
            if cat == 5:
                q["adversarial_answer"] = f"distractor {n}"
            else:
                q["answer"] = f"answer {n}"
            qa.append(q)
            n += 1
    return {
        "sample_id": sample_id,
        "qa": qa,
        "conversation": {
            "speaker_a": "Ann",
            "speaker_b": "Bo",
            "session_1_date_time": "1:00 pm on 1 May, 2023",
            "session_1": [
                {"speaker": "Ann", "dia_id": "D1:1", "text": "Hi Bo!"},
                {
                    "speaker": "Bo",
                    "dia_id": "D1:2",
                    "text": "Look at this.",
                    "img_url": ["https://example.invalid/x.jpg"],
                    "blip_caption": "a photo of a red kite",
                    "query": "kite",
                },
                {"speaker": "Ann", "dia_id": "D1:3", "text": "Line one\nline two"},
            ],
            "session_2_date_time": "2:00 pm on 3 May, 2023",
            "session_2": [{"speaker": "Bo", "dia_id": "D2:1", "text": "Back again."}],
            # A dated session without turns (as in conv-26) is not a session.
            "session_3_date_time": "4:00 pm on 9 May, 2023",
        },
        "observation": {"session_1_observation": "OBSERVATION-TEXT"},
        "session_summary": {"session_1_summary": "SUMMARY-TEXT"},
        "event_summary": {},
    }


def longmemeval_item(qid: str, qtype: str) -> dict:
    return {
        "question_id": qid,
        "question_type": qtype,
        "question": f"What about {qid}?",
        "question_date": "2023/05/30 (Tue) 23:40",
        "answer": 3 if qtype == "multi-session" else f"gold {qid}",
        "answer_session_ids": [f"answer_{qid}_1"],
        "haystack_dates": ["2023/05/20 (Sat) 02:21", "2023/05/21 (Sun) 09:00"],
        "haystack_session_ids": [f"sharegpt_{qid}_0", f"answer_{qid}_1"],
        "haystack_sessions": [
            [
                {"role": "user", "content": "Tell me a joke."},
                {"role": "assistant", "content": "# Joke\n\nWhy did the chicken…"},
            ],
            [
                {
                    "role": "user",
                    "content": "I adopted a cat named Luna.",
                    "has_answer": True,
                },
                {"role": "assistant", "content": "Congratulations on Luna!"},
            ],
        ],
    }


def locomo_dataset() -> list[dict]:
    return [
        locomo_sample("conv-1", {1: 4, 2: 5, 3: 3, 4: 9, 5: 6}),
        locomo_sample("conv-2", {1: 3, 2: 4, 4: 8, 5: 5}),  # no open-domain questions
    ]


def longmemeval_dataset() -> list[dict]:
    types = {
        "single-session-user": 14,
        "multi-session": 24,
        "temporal-reasoning": 25,
        "knowledge-update": 15,
        "single-session-preference": 6,
        "single-session-assistant": 11,
    }
    items = []
    for qtype, count in types.items():
        for i in range(count):
            items.append(longmemeval_item(f"{qtype}-{i:03d}", qtype))
        for i in range(
            2
            if qtype not in ("single-session-preference", "single-session-assistant")
            else 0
        ):
            items.append(longmemeval_item(f"{qtype}-{i:03d}_abs", qtype))
    return items


def read_tree(root: Path) -> dict[str, bytes]:
    return {
        str(p.relative_to(root)): p.read_bytes()
        for p in sorted(root.rglob("*"))
        if p.is_file()
    }


class ConversionTest(unittest.TestCase):
    def test_locomo_session_rendering(self) -> None:
        files = memory.locomo_corpus(locomo_sample("conv-1", {4: 1}))
        self.assertEqual(sorted(files), ["README.md", "session_01.md", "session_02.md"])
        self.assertEqual(
            files["session_01.md"],
            "# Session 1\n\nDate: 1:00 pm on 1 May, 2023\n\n"
            "**Ann:** Hi Bo!\n\n"
            "**Bo:** Look at this. [shared an image: a photo of a red kite]\n\n"
            "**Ann:** Line one\nline two\n",
        )
        self.assertIn("Participants: Ann and Bo.", files["README.md"])
        self.assertIn("2 chat sessions", files["README.md"])

    def test_session_filenames_sort_chronologically(self) -> None:
        names = [memory.session_filename(n, 12) for n in range(1, 13)]
        self.assertEqual(names, sorted(names))
        self.assertEqual(memory.session_filename(7, 120), "session_007.md")

    def test_longmemeval_rendering(self) -> None:
        files = memory.longmemeval_corpus(
            longmemeval_item("abc", "single-session-user")
        )
        self.assertEqual(
            files["session_02.md"],
            "# Session 2\n\nDate: 2023/05/21 (Sun) 09:00\n\n"
            "**User:** I adopted a cat named Luna.\n\n**Assistant:** Congratulations on Luna!\n",
        )
        self.assertIn("Participants: User and Assistant.", files["README.md"])

    def test_build_is_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            outs = []
            for run in ("a", "b"):
                root = Path(tmp) / run
                for suite, data in (
                    ("locomo", locomo_dataset()),
                    ("longmemeval", longmemeval_dataset()),
                ):
                    builder = (
                        memory.build_locomo
                        if suite == "locomo"
                        else memory.build_longmemeval
                    )
                    size = 12 if suite == "locomo" else 20
                    tasks, corpora = builder(data, size, 7, Path("/corpus") / suite)
                    for cid, files in corpora.items():
                        memory.write_corpus(root / "corpus" / suite / cid, files)
                    memory.write_tasks(root / "tasks" / f"{suite}.jsonl", tasks)
                outs.append(read_tree(root))
            self.assertEqual(outs[0], outs[1])
            self.assertTrue(outs[0])

    def test_write_corpus_skips_unchanged_and_prunes_stale_sessions(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            files = {"README.md": "r\n", "session_01.md": "a\n", "session_02.md": "b\n"}
            self.assertEqual(memory.write_corpus(root, files), 3)
            self.assertEqual(memory.write_corpus(root, files), 0)
            del files["session_02.md"]
            files["session_01.md"] = "a2\n"
            self.assertEqual(memory.write_corpus(root, files), 1)
            self.assertEqual(
                sorted(p.name for p in root.iterdir()), ["README.md", "session_01.md"]
            )


class SamplingTest(unittest.TestCase):
    def test_equal_allocation(self) -> None:
        avail = {"1": 4, "2": 5, "3": 3, "4": 9, "5": 6}
        self.assertEqual(
            memory.equal_allocation(avail, 12, ["1", "2", "3", "4", "5"]),
            {"1": 3, "2": 3, "3": 2, "4": 2, "5": 2},
        )
        # A missing category gives its slots to the others.
        self.assertEqual(
            memory.equal_allocation(
                {"1": 3, "2": 4, "4": 8, "5": 5}, 12, ["1", "2", "3", "4", "5"]
            ),
            {"1": 3, "2": 3, "3": 0, "4": 3, "5": 3},
        )
        # Availability caps the total.
        self.assertEqual(
            sum(memory.equal_allocation({"1": 1, "2": 1}, 12, ["1", "2"]).values()), 2
        )

    def test_proportional_allocation(self) -> None:
        sizes = {
            "a": 72,
            "a/abs": 6,
            "b": 121,
            "b/abs": 12,
            "c": 56,
            "d": 30,
            "e": 64,
            "e/abs": 6,
            "f": 127,
            "f/abs": 6,
        }
        alloc = memory.proportional_allocation(sizes, 60)
        self.assertEqual(sum(alloc.values()), 60)
        for k, v in sizes.items():
            self.assertLessEqual(abs(alloc[k] - 60 * v / 500), 1, k)

    def test_locomo_sampling_is_seeded_and_stratified(self) -> None:
        data = locomo_dataset()
        a, _ = memory.build_locomo(data, 12, 7, Path("/c"))
        b, _ = memory.build_locomo(data, 12, 7, Path("/c"))
        c, _ = memory.build_locomo(data, 12, 8, Path("/c"))
        self.assertEqual(a, b)
        self.assertNotEqual([t["id"] for t in a], [t["id"] for t in c])
        self.assertEqual(len(a), 24)
        kinds = Counter((t["corpus_id"], t["kind"]) for t in a)
        self.assertEqual(kinds[("locomo-c02", "open-domain")], 0)
        for (cid, _), n in kinds.items():
            self.assertIn(n, (2, 3), cid)
        self.assertEqual(len({t["id"] for t in a}), 24)

    def test_locomo_adversarial_tasks(self) -> None:
        tasks, _ = memory.build_locomo(locomo_dataset(), 12, 7, Path("/c"))
        adv = [t for t in tasks if t["adversarial"]]
        self.assertTrue(adv)
        for t in adv:
            self.assertEqual(t["kind"], "adversarial")
            self.assertEqual(t["reference"], memory.LOCOMO_ADVERSARIAL_REFERENCE)
            self.assertTrue(t["distractor"].startswith("distractor"))
        for t in tasks:
            self.assertIsNone(t["question_date"])
            self.assertEqual(
                list(t)[:12],
                [
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
                ],
            )

    def test_longmemeval_sampling(self) -> None:
        data = longmemeval_dataset()
        a, corpora = memory.build_longmemeval(data, 20, 7, Path("/c"))
        b, _ = memory.build_longmemeval(data, 20, 7, Path("/c"))
        self.assertEqual(a, b)
        self.assertEqual(len(a), 20)
        self.assertEqual(len(corpora), 20)
        self.assertEqual({t["kind"] for t in a}, {i["question_type"] for i in data})
        self.assertTrue(any(t["adversarial"] for t in a))
        for t in a:
            self.assertEqual(t["adversarial"], t["source_id"].endswith("_abs"))
            self.assertEqual(t["question_date"], "2023/05/30 (Tue) 23:40")
            self.assertIsInstance(t["reference"], str)


class LeakTest(unittest.TestCase):
    FORBIDDEN = (
        "has_answer",
        "answer_",
        "_abs",
        "evidence",
        "D1:",
        "D2:",
        "OBSERVATION-TEXT",
        "SUMMARY-TEXT",
        "sharegpt_",
        "example.invalid",
    )

    def check(
        self, tasks: list[dict], corpora: dict[str, dict[str, str]], forbidden=FORBIDDEN
    ) -> None:
        for cid, files in corpora.items():
            self.assertNotIn("_abs", cid)
            for name, text in files.items():
                for needle in forbidden:
                    self.assertNotIn(needle, name)
                    self.assertNotIn(needle, text, f"{cid}/{name}")
        by_corpus = {t["corpus_id"]: t for t in tasks}
        for t in tasks:
            self.assertNotIn("_abs", t["id"] + t["corpus_id"] + t["corpus"])
            text = "".join(corpora[t["corpus_id"]].values())
            self.assertNotIn(t["question"], text)
        self.assertTrue(by_corpus)

    def test_synthetic_corpora_carry_no_labels(self) -> None:
        tasks, corpora = memory.build_longmemeval(
            longmemeval_dataset(), 30, 7, Path("/c")
        )
        self.check(tasks, corpora)
        tasks, corpora = memory.build_locomo(locomo_dataset(), 12, 7, Path("/c"))
        self.check(tasks, corpora)


REAL = memory.BENCH_HOME / "datasets"


@unittest.skipUnless(
    all((REAL / d.filename).exists() for d in memory.DATASETS.values()),
    "datasets not fetched",
)
class RealDatasetTest(unittest.TestCase):
    def test_real_build_has_no_labels_and_is_deterministic(self) -> None:
        lme = memory.load_dataset("longmemeval", REAL)
        loc = memory.load_dataset("locomo", REAL)
        with tempfile.TemporaryDirectory() as tmp:
            args = argparse.Namespace(
                datasets_dir=REAL,
                corpus_root=Path(tmp) / "corpus",
                tasks_dir=Path(tmp) / "tasks",
                seed=memory.DEFAULT_SEED,
                locomo_per_conv=memory.DEFAULT_LOCOMO_PER_CONV,
                longmemeval_n=memory.DEFAULT_LONGMEMEVAL_N,
            )
            with contextlib.redirect_stdout(io.StringIO()):
                memory.build(args, ["locomo", "longmemeval"])
                first = read_tree(Path(tmp))
                memory.build(args, ["locomo", "longmemeval"])
            self.assertEqual(first, read_tree(Path(tmp)))
            # The committed task files are what this build produces (paths aside).
            for suite in ("locomo", "longmemeval"):
                fresh = [
                    json.loads(l)
                    for l in (Path(tmp) / "tasks" / f"{suite}.jsonl")
                    .read_text()
                    .splitlines()
                ]
                committed = [
                    json.loads(l)
                    for l in (memory.TASKS_DIR / f"{suite}.jsonl")
                    .read_text()
                    .splitlines()
                ]
                for t in fresh + committed:
                    t.pop("corpus")
                self.assertEqual(fresh, committed, suite)
        tasks, corpora = memory.build_longmemeval(
            lme, memory.DEFAULT_LONGMEMEVAL_N, memory.DEFAULT_SEED, Path("/c")
        )
        picked = {t["source_id"] for t in tasks}
        session_ids = {
            sid
            for item in lme
            if item["question_id"] in picked
            for sid in item["haystack_session_ids"]
        }
        for files in corpora.values():
            text = "".join(files.values())
            self.assertNotIn("has_answer", text)
            self.assertNotIn("answer_session", text)
            for sid in session_ids:
                self.assertNotIn(sid, text)
        for t in tasks:
            self.assertNotIn(t["source_id"], "".join(corpora[t["corpus_id"]].values()))
        tasks, corpora = memory.build_locomo(loc, 12, memory.DEFAULT_SEED, Path("/c"))
        for files in corpora.values():
            text = "".join(files.values())
            self.assertNotRegex(text, r"\bD\d+:\d+\b")
            self.assertNotIn("http", text)


class ScoringTest(unittest.TestCase):
    def test_normalize(self) -> None:
        self.assertEqual(memory.normalize_answer("The Cat, and a Dog!"), "cat dog")
        self.assertEqual(memory.normalize_answer(2022), "2022")

    def test_porter(self) -> None:
        pairs = {
            "caresses": "caress", "ponies": "poni", "cats": "cat", "feed": "feed", "agreed": "agre",
            "plastered": "plaster", "motoring": "motor", "sing": "sing", "conflated": "conflat",
            "troubled": "troubl", "sized": "size", "hopping": "hop", "falling": "fall",
            "filing": "file", "happy": "happi", "relational": "relat", "running": "run",
            "generalization": "gener", "painting": "paint", "paintings": "paint",
            "adoption": "adopt", "agencies": "agenc", "controll": "control", "rate": "rate",
            "hopeful": "hope", "goodness": "good", "electrical": "electr", "is": "is",
        }  # fmt: skip
        for word, stem in pairs.items():
            self.assertEqual(memory.porter_stem(word), stem, word)

    def test_f1_known_pairs(self) -> None:
        self.assertEqual(memory.f1("7 May 2023", "7 May 2023"), 1.0)
        self.assertEqual(memory.f1("The painting.", "paintings"), 1.0)
        self.assertEqual(
            memory.f1("Adoption agencies", "researching adoption agencies"), 0.8
        )
        self.assertEqual(memory.f1("blue", "red"), 0.0)
        self.assertEqual(memory.f1("", "red"), 0.0)
        self.assertAlmostEqual(
            memory.f1("she went to the park and the beach", "the beach"),
            2 * (1 / 5) * 1 / (1 / 5 + 1),
        )
        self.assertEqual(memory.f1(2022, "2022"), 1.0)

    def test_locomo_score(self) -> None:
        self.assertEqual(
            memory.locomo_score(
                "pottery, camping", "camping, pottery, painting", "multi-hop"
            ),
            2 / 3,
        )
        self.assertEqual(
            memory.locomo_score(
                "Likely no", "Likely no; because she...", "open-domain"
            ),
            1.0,
        )
        self.assertEqual(
            memory.locomo_score(
                "It is not mentioned in the conversation.", "x", "adversarial"
            ),
            1.0,
        )
        self.assertEqual(
            memory.locomo_score(
                "You never told me that.", "x", "adversarial", strict=True
            ),
            0.0,
        )
        self.assertEqual(
            memory.locomo_score("LGBTQ+ individuals", "x", "adversarial"), 0.0
        )
        self.assertEqual(
            memory.locomo_score("mental health", "mental health", "single-hop"), 1.0
        )

    def test_declines_and_longmemeval(self) -> None:
        for text in (
            "You did not mention this information.",
            "I can't find anything about a hamster in the history.",
            "That isn't in the conversation.",
            "There is no mention of your uncle's party.",
            "No information available.",
            "You never mentioned visiting Korea.",
            "You didn’t mention a hamster.",
            "That isn't in your chat history.",
        ):
            self.assertTrue(memory.declines(text), text)
        for text in (
            "Business Administration",
            "You mentioned your cat Luna.",
            "7 days",
        ):
            self.assertFalse(memory.declines(text), text)
        self.assertEqual(
            memory.longmemeval_score("You did not mention a hamster.", "ref", True), 1.0
        )
        self.assertEqual(memory.longmemeval_score("Its name is Max.", "ref", True), 0.0)
        self.assertEqual(
            memory.longmemeval_score(
                "Business Administration", "Business Administration", False
            ),
            1.0,
        )

    def test_judge_prompt(self) -> None:
        p = memory.longmemeval_judge_prompt("temporal-reasoning", "Q", "A", "R", False)
        self.assertIn("off-by-one", p)
        self.assertTrue(p.endswith("Answer yes or no only."))
        self.assertIn(
            "unanswerable",
            memory.longmemeval_judge_prompt("multi-session", "Q", "A", "R", True),
        )
        self.assertIn(
            "Rubric: A",
            memory.longmemeval_judge_prompt(
                "single-session-preference", "Q", "A", "R", False
            ),
        )
        with self.assertRaises(ValueError):
            memory.longmemeval_judge_prompt("other", "Q", "A", "R", False)


if __name__ == "__main__":
    unittest.main()

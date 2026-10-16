# Stage 7: agent-outcome benchmark

An agent (Codex) answers each task's question three ways: with no graph, with Graphify's graph and
with dijkstral's graph. Tasks are JSONL files in `tasks/`, one suite per file. Generated corpora
and every run output live under `$DIJKSTRAL_BENCH_HOME` (default `~/.cache/dijkstral-bench`),
never in the repository.

The runner (`run.py`: prepare, run, judge, report), its Codex flags and scoring rules are
documented in [../README.md](../README.md#stage-7-agent-outcome).

## Task contract

One JSON object per line:

| Field | Meaning |
|---|---|
| `id` | unique task id |
| `suite` | `locomo`, `longmemeval`, … |
| `corpus_id`, `corpus` | the folder the graph arms build their graph from (`~` is the home directory) |
| `semantic` | `true`: build the graph with the semantic pass (documents go to the model) |
| `question` | what the agent is asked |
| `question_date` | the date the question is asked (LongMemEval), else `null` |
| `kind` | category or question type |
| `reference` | gold answer text |
| `facts`, `min_facts` | key-fact scoring; empty and 0 for the memory suites |
| `adversarial` | `true` when the correct response is to say the information is not there |
| `source_id` | the dataset's own question id; for tracing only, **never shown to the agent** (LongMemEval ids end in `_abs` for abstention questions) |
| `distractor` | LOCOMO adversarial only: the wrong answer the dataset offers next to "Not mentioned in the conversation" |

## Memory suites: LOCOMO and LongMemEval-S

Each conversation history becomes a folder of Markdown notes; the graph arms build their graph over
it with the semantic pass (Markdown is a document, `cli/docs/graphify-semantic.md` §2), then the
agent answers the dataset's questions.

```bash
cd products/dijkstral
uv run --no-project bench/agent/memory.py fetch     # datasets → $DIJKSTRAL_BENCH_HOME/datasets/
uv run --no-project bench/agent/memory.py build     # corpora + tasks/locomo.jsonl, tasks/longmemeval.jsonl
uv run --no-project bench/agent/memory.py build --suite longmemeval --longmemeval-n 20 --seed 7
uv run --no-project python -m unittest discover -s bench/agent -p 'test_*.py'
```

`build` flags: `--suite locomo|longmemeval|all`, `--seed` (default 7), `--locomo-per-conv`
(default 12), `--longmemeval-n` (default 60), `--corpus-root`, `--tasks-dir`. The committed task
files are the defaults' output; the test suite checks that a fresh build reproduces them.

### Sources

| Suite | File | Source (pinned) | sha256 | Bytes | Licence as stated |
|---|---|---|---|---:|---|
| LOCOMO | `locomo10.json` | `snap-research/locomo` at `3eb6f2c`, `data/locomo10.json` (raw.githubusercontent.com) | `79fa87e9…a698ff4` | 2,805,274 | CC BY-NC 4.0 (`LICENSE.txt`) |
| LongMemEval-S | `longmemeval_s_cleaned.json` | Hugging Face `xiaowu0162/longmemeval-cleaned` at `98d7416`, the location the LongMemEval README names as the official release (the 2025-09 cleaned haystacks) | `d6f21ea9…678c3a442` | 277,383,467 | MIT (dataset card; the GitHub repository is MIT too) |

Full URLs and hashes are in `memory.py` (`DATASETS`); `fetch` and `build` refuse a file whose
hash differs. The committed task files hold only the datasets' questions and answers; LOCOMO's are
under CC BY-NC 4.0 (attribution: Maharana et al., "Evaluating Very Long-Term Conversational Memory
of LLM Agents", ACL 2024), so this benchmark stays non-commercial.

### Subsets

- **LOCOMO**: all 10 conversations (`locomo-c01` … `locomo-c10`, in file order), 12 questions
  each = 120. Equal allocation over the five categories as availability allows: one slot per
  category per round-robin pass, the order rotated by conversation so the remainder slots spread
  evenly; conv-30 (`locomo-c02`) has no open-domain question, so its slots go to the other four.
  Within a category, a seeded sample (`random.Random("<seed>:locomo:<conv>:<category>")`).
  Defaults give multi-hop 25, temporal 25, open-domain 21, single-hop 24, adversarial 25.
- **LongMemEval-S**: 60 of the 500 questions, proportional (largest remainder) to the strata
  question type × abstention, seeded sample per stratum. Defaults give temporal-reasoning 16
  (1 abstention), multi-session 15 (1), knowledge-update 10 (1), single-session-user 9 (1),
  single-session-assistant 7, single-session-preference 3; 4 abstention in all (30/500 in the
  dataset). Each question has its own haystack and so its own corpus.

**Category numbering (LOCOMO).** The README and paper do not map the integer `category` to a name
(issue #6 on the repository). The released evaluation code does: `task_eval/evaluation.py` scores
1 as multi-hop (comma-separated sub-answers), 5 as adversarial, and 2–4 with plain F1;
`gpt_utils.py` adds a date hint to 2. So 1 multi-hop (282 questions), 2 temporal (321),
3 open-domain (96, "would X likely…" questions needing world knowledge), 4 single-hop (841),
5 adversarial (446).

**Adversarial and abstention.** LOCOMO category 5 has no `answer`, only `adversarial_answer`, a
plausible but unsupported answer; LOCOMO asks the model to choose between it and "Not mentioned in
the conversation". Our tasks use `reference: "Not mentioned in the conversation"`,
`adversarial: true` and keep the dataset's answer as `distractor`. (Two category-5 questions in
conv-26 also carry an `answer`; neither is in the default sample.) LongMemEval `_abs` questions
keep the dataset's explanation as `reference` and get `adversarial: true`.

### Corpora

`$DIJKSTRAL_BENCH_HOME/memory-corpus/<suite>/<corpus_id>/`: `README.md` naming the participants
and the date range, and one `session_<nn>.md` per session (zero-padded so path order, the semantic
pass's dispatch order, is chronological):

```markdown
# Session 1

Date: 1:56 pm on 8 May, 2023

**Caroline:** Hey Mel! Good to see you! How have you been?

**Melanie:** … By the way, take a look at this. [shared an image: a photo of a painting of a sunset over a lake]
```

- Turns are in dataset order with the text verbatim (assistant replies keep their own Markdown).
  LOCOMO images become their BLIP caption in brackets; URLs and search queries are dropped.
- Nothing that labels the answer reaches the corpus: no LOCOMO dialog ids (`D1:3`, which the
  evidence lists use), observations or summaries; no LongMemEval session ids (evidence sessions
  are named `answer_*`), `answer_session_ids` or `has_answer` flags. LongMemEval corpus ids are
  `longmemeval-<first 8 hex of sha256(question_id)>` so that neither `_abs` nor the dataset order
  (grouped by type) shows in a path.
- LongMemEval sessions keep the haystack's order, which is by day but not always by time within a
  day (211 of the 500 haystacks), and 76 questions are dated before their last session; both are
  the dataset's.
- `manifest.json` beside the corpus folders records dataset, seed, sizes and per-corpus counts.
  Files are rewritten only when their bytes change; a folder left over from another seed is
  reported, not deleted.

Size, which drives the semantic pass's cost (cl100k tokens measured with tiktoken; `build` prints a
chars/4 estimate, about 5 % high on LOCOMO and 18 % high on LongMemEval):

| Suite | Corpora | Sessions | Bytes | Tokens | Per corpus | Chunks at the default 60k budget |
|---|---:|---:|---:|---:|---|---:|
| LOCOMO | 10 | 272 (19–32 each) | 0.91 M | 216,471 | 13.3k–25.2k | 10 (one per corpus) |
| LongMemEval-S | 60 | 2,880 (41–56 each) | 30.0 M | 6,355,236 | 102.7k–107.6k | 120 (two per corpus) |

Each tool's semantic pass reads every corpus once, so a full run sends about 0.22 M (LOCOMO) and
6.4 M (LongMemEval) document tokens per tool before prompt overhead; `--longmemeval-n` scales the
second linearly.

### Scoring (`memory.py`, importable)

- `f1(prediction, reference)`: LOCOMO's token F1. Normalisation as in `normalize_answer` of
  `task_eval/evaluation.py`: drop commas, lowercase, strip punctuation, remove the words
  a/an/the/and, collapse whitespace; then Porter-stem every token and take F1 over the token
  multisets (0 when nothing overlaps). The stemmer is the original Porter algorithm; LOCOMO uses
  NLTK's, whose default mode adds a few irregular-form extensions that short answers rarely hit.
- `locomo_score(prediction, reference, kind)`: LOCOMO's per-category rule: multi-hop splits both
  sides on commas and averages, over reference parts, the best F1 against any prediction part;
  open-domain compares with the reference up to its first `;`; temporal and single-hop use `f1`;
  adversarial scores 1 if the answer declines (`declines()`), else 0. `strict=True` uses LOCOMO's
  exact adversarial check instead ("no information available" or "not mentioned" in the answer),
  which assumes its two-option prompt.
- `declines(prediction)`: the answer says the information is not available ("not mentioned",
  "you never mentioned…", "you did not mention…", "I can't find…", "not in the conversation",
  "no record of…", "unanswerable", "unknown", …; case-insensitive, curly apostrophes allowed).
- `longmemeval_score(prediction, reference, abstention)`: an abstention (`_abs`) question counts
  correct (1) exactly when the answer declines, as the official judge accepts any answer that
  identifies the question as unanswerable; other questions get `f1`, a deterministic proxy only.
- `longmemeval_judge_prompt(kind, question, reference, response, abstention)`: the official
  answer-check prompt per question type (`src/evaluation/evaluate_qa.py`, LongMemEval `9e0b455`,
  MIT); a reply containing "yes" means correct. LongMemEval's headline metric is this judge, and
  preference questions have a rubric, not an answer, as reference, so a runner that reports
  LongMemEval accuracy should grade with it.

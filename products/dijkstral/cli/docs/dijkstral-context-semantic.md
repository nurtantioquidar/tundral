# `dijkstral context`: meaning-based retrieval (research prototype)

> The prototype code (`crates/embed`, the `context::render_with` hook) was not merged; it lives on
> branch `feature/dijkstral-context-embeddings-prototype`. This note records the measurements.

A measured exploration, not a shipped feature. `dijkstral context` (`dijkstral-context.md`)
answers a question that names no symbol from the code's text: identifier words, stems, idf,
abbreviations, comments and docstrings, and a graph-neighbourhood bonus. Its remaining misses
looked like vocabulary gaps ("where is the operationId generated" → `generate_unique_id`), so this
prototype tests whether embeddings close them. Nothing here changes `dijkstral`: the `dijkstral`
binary, `query` and every Graphify-compatible output are byte for byte as before (all 60 benchmark
answers compared), and the binary still links only the system libraries.

## 1. Prototype

- `crates/embed`, binary `dijkstral-embed` (never part of `dijkstral`):
  - `index --model DIR --out DIR [--encoder static|bert] [--text name,sig,doc,path,bodyN] [--i8]`
    embeds every code definition outside tests, configuration and data files
    (`context::semantic_candidates`), one text per node: its `enclosing › label` split into words,
    the doc comment above it or the Python docstring below it, its signature line and its path.
  - `context "<q>" --index DIR [--fusion F] [--after-named N] [--named-source S]` is `context`
    with the code-text search fused with the index (`context::render_with`, a `Retriever`).
  - `search "<q>" --index DIR` lists the nearest definitions.
- Encoders: **A** Model2Vec static embeddings (`static_model.rs`: token rows mean-pooled and
  normalised, the matrix read from `model.safetensors`, F16 or F32) with a hand-written BERT
  WordPiece tokenizer (`wordpiece.rs`); no new crate. **B** a BERT-family transformer through
  candle (`bert.rs`, CLS or mean pooling), behind the `bert` feature (`accelerate` adds Apple's
  Accelerate framework); off by default, so the workspace build and tests never compile candle.
- The index (`store.rs`): `meta.json`, `ids.txt` (graph node ids) and unit vectors as f32 or int8
  with a scale per vector, memory-mapped; search is a brute-force dot product over every vector,
  ids resolved against the graph only for the best 100.
- Fusion (`context::fuse`) of the code-text ranking with the 100 nearest definitions, for the
  whole question and for each end of a flow question: `semantic` (meaning only), `rrf:K`
  (reciprocal rank fusion), `mix:W` ((1 − W) × code-text score / best + W × similarity rescaled
  over the 100), `interleave`, `fill:N` (N code-text anchors, then meaning's) and `lexonly` (the
  code-text ranking; a control). `--after-named N` also adds N matches after the symbols a
  question names (`context` otherwise answers such questions with those symbols only), taken from
  meaning (`semantic`), from the code text (`lexical`, the control) or both (`fused`).
- Measured with `../bench/agent/context_eval.py` (the offline key-fact coverage of one answer per
  question, `bench/agent/score.py`): tuned on fastapi and ripgrep, Kubernetes held out and read
  only as aggregates. 20 held-out questions: one fact moves the held-out recall by about 0.015.

## 2. Results

Recall = mean share of a question's key facts in one answer; pass = questions with enough facts;
chars = median answer length. "+N sem/lex" = N matches added after named symbols, from meaning or
from the code text. 2026-10-17, Apple M-series, 10 cores; corpora and graphs of the stage 7
benchmark (`~/.cache/dijkstral-bench/agent/`).

**Baselines** (lexical `context`, `dijkstral` binary):

| | tuning | held-out | all | chars |
|---|---|---|---|---|
| old (`bef355c`, comments not counted) | 0.75 (26/40) | 0.49 (9/20) | 0.66 | 2464 |
| new (`d1e37cf`, comments and docstrings count) | 0.77 (27/40) | 0.61 (13/20) | 0.72 | 2514 |

**On the new baseline**, A = `potion-code-16M-v2`, text `name,sig,doc,path`:

| variant | tuning | held-out | all | chars |
|---|---|---|---|---|
| A semantic only | 0.63 (19) | 0.59 (11) | 0.62 | 1932 |
| A rrf:60 | 0.72 (24) | 0.63 (12) | 0.69 | 2142 |
| A rrf:5 | 0.77 (27) | 0.59 (11) | 0.71 | 2304 |
| A mix:0.2 | 0.77 (27) | 0.62 (12) | 0.72 | 2449 |
| A mix:0.4 | 0.77 (27) | 0.64 (12) | 0.72 | 2261 |
| A interleave | 0.73 (27) | 0.57 (11) | 0.68 | 2312 |
| A fill:2 / fill:3 | 0.74 / 0.74 | 0.57 / 0.59 | 0.68 / 0.69 | 2236 / 2431 |
| A mix:0.4 +1 / +2 / +3 sem | 0.77 / 0.79 / **0.81** (29) | 0.64 / 0.65 / 0.65 (12) | 0.73 / 0.74 / 0.75 | 2536 / 2750 / 2861 |
| A lexonly +3 sem | 0.81 (29) | 0.63 (13) | 0.75 | 2993 |
| A mix:0.4 +3 fused | 0.80 (29) | 0.66 (12) | 0.76 | 3190 |
| A int8 index, mix:0.4 +3 sem | 0.81 (29) | 0.65 (12) | 0.75 | 2861 |
| **control, no embeddings:** lexonly +2 lex | 0.79 (28) | 0.64 (13) | 0.74 | 2994 |
| control: lexonly +3 lex | 0.79 (28) | 0.64 (13) | 0.74 | 3415 |
| B `bge-small-en-v1.5` semantic only | 0.68 (23) | 0.56 (10) | 0.64 | 1868 |
| B rrf:60 | 0.75 (26) | 0.59 (12) | 0.69 | 2198 |
| B mix:0.4 | 0.76 (27) | 0.61 (12) | 0.71 | 2135 |
| B mix:0.4 +3 sem | 0.81 (30) | 0.61 (12) | 0.74 | 2860 |
| B lexonly +3 sem | 0.81 (30) | 0.61 (13) | 0.75 | 2941 |

**Embedded text and model** (A unless named; semantic only / mix:0.4 / lexonly +3 sem / mix:0.4 +3
sem, tuning–held-out):

| text, model | semantic | mix:0.4 | lexonly +3 sem | mix:0.4 +3 sem |
|---|---|---|---|---|
| name, sig | 0.56–0.46 | 0.74–0.57 | 0.80–0.63 | 0.77–0.58 |
| name, sig, doc, path | 0.63–0.59 | 0.77–0.64 | 0.81–0.63 | 0.81–0.65 |
| name, sig, doc, path, 8 body lines | 0.61–0.48 | 0.74–0.59 | 0.81–0.61 | 0.78–0.59 |
| name, sig, doc, path; `potion-retrieval-32M` | 0.62–0.46 | 0.76–0.61 | 0.80–0.63 | 0.79–0.62 |

The nearest-definition lists themselves (tuning, identifier facts of the 30 non-`callers`
questions found in the top 10 / 30 / 100): A 0.31 / 0.46 / 0.70; A name+sig 0.28 / 0.47 / 0.68;
A with body 0.33 / 0.56 / 0.67; `potion-retrieval-32M` 0.27 / 0.40 / 0.63; B 0.32 / 0.51 / 0.69.
`generate_unique_id` (the motivating miss) is in none of them: its text is three words and a
signature with nothing about OpenAPI.

**On the old baseline** (the same prototype over `bef355c`'s code-text search), A:

| variant | tuning | held-out | all | chars |
|---|---|---|---|---|
| lexical | 0.75 | 0.49 | 0.66 | 2464 |
| A semantic only | 0.63 | 0.59 | 0.62 | 1973 |
| A rrf:60 | 0.69 | 0.59 | 0.66 | 2243 |
| A mix:0.4 | 0.73 | 0.59 | 0.69 | 2377 |
| A mix:0.4 +3 sem | 0.77 | 0.60 | 0.72 | 2910 |
| control: lexonly +3 lex | 0.78 | 0.52 | 0.69 | 3304 |

**Costs** (Kubernetes: 256,451 graph nodes, 186,274 embedded):

| | A `potion-code-16M-v2` | B `bge-small-en-v1.5` |
|---|---|---|
| model | 32 MB (63,457 × 256, f16), MIT, `minishlab/potion-code-16M-v2@e9d2a44` | 129 MB (33M parameters, f32), MIT, `BAAI/bge-small-en-v1.5@5c38ec7` |
| index, Kubernetes | 193 MB f32, 57 MB int8 (8 MB of it ids) | 283 MB f32 (384 dimensions; ~80 MB int8) |
| index, fastapi / ripgrep | 0.8 MB / 3.5 MB | 1.2 MB / 5.0 MB |
| index build, Kubernetes | 2.4–4.1 s (1.2–1.6 s reading texts), 0.8 GB peak | 38 min (Accelerate, all cores, about 100 texts/s; other runs shared the machine), 7 GB peak |
| index build, ripgrep | 0.1 s | 33 s |
| nearest 100, Kubernetes, warm | 8 ms f32, 5 ms int8 (+ about 70 ms opening the model) | 27–31 ms (+ about 70 ms) |
| `context` mix:0.4 +3, Kubernetes, warm (3 own questions, not the benchmark's) | 931 ms f32, 906 ms int8 | 892 ms |
| lexical `context`, same questions | 820 ms | 820 ms |
| binary | `dijkstral-embed` 1.7 MB, no new crate; in `dijkstral` about 0.1 MB and no new link | +1.2 MB of candle (2.9 MB binary), or 2.8 MB linking Accelerate.framework |

`dijkstral` itself: 87.95 MB → 87.97 MB (the `render_with` hook), links unchanged.

## 3. Findings

1. **Comments closed most of the gap embeddings were closing.** On the old baseline, fusing
   meaning raised held-out recall 0.49 → 0.59, mostly through doc comments (Go's `// Foo does …`)
   that the code-text search then skipped. Counting comments and docstrings lexically (`d1e37cf`)
   gets 0.61 alone, and on top of it the best fusion of the no-symbol ranking is level on tuning
   (0.77) and +0.03 held out (0.64, two facts), within the spread of 20 questions.
2. **The rest of the gain is more anchors, not meaning.** Adding matches after the symbols a
   question names lifts tuning 0.77 → 0.81 with meaning and to 0.79 with the code text alone (the
   control), held out 0.61 → 0.63–0.65 either way. Embeddings over that control: +0.02 tuning,
   ±0.01 held out.
3. **Meaning alone is worse than the code text** (A 0.63 / 0.59, B 0.68 / 0.56): top-4 precision
   is what `context` needs, and the right definition is in the nearest 10 only a third of the
   time. Rank-based fusion that trusts both lists equally (`rrf:60`, `interleave`, `fill`) loses
   tuning recall; a score mix that keeps the code-text order unless meaning is clearly better
   (`mix:0.4`) is the only fusion that does not.
4. **Docstrings are what the embeddings run on.** Without them (name and signature only) meaning
   alone falls to 0.56 / 0.46 and every fusion lands below the new baseline held out (0.57–0.63);
   body lines dilute them (0.61 / 0.48). Since the code-text search now uses the same comments, the
   embedding mostly re-reads the evidence the baseline already has.
5. **A transformer buys nothing here.** `bge-small` is no better than the 32 MB static code model
   (held out 0.61 at best against 0.65), costs 500× the indexing time and 4× the model.
   `potion-code-16M-v2` beats the general `potion-retrieval-32M` at half its size. An int8 index
   scores the same as f32 at 30% of the size.

## 4. Recommendation

- **Do not ship embeddings now**, on or off by default: at about +0.02 coverage within the
  spread, they do not pay for a 32 MB model, a 57 MB index per Kubernetes-sized repository, an
  index to keep fresh and an opt-in flow. Default behaviour stays lexical.
- **Measure the cheap part instead**: matches after named symbols from the code text alone (the
  control, +2: tuning 0.79, held out 0.64) is a lexical change worth an agent run, since it also
  lengthens answers (median 2514 → 2994 chars) and coverage only lowers cost when the extra text
  is the answer.
- **If meaning is revisited** (e.g. a corpus where comments are scarce or in another language than
  the questions), ship option A only: `potion-code-16M-v2` (MIT), int8 index, fused `mix:0.4`
  plus 2–3 matches after named symbols, in the `dijkstral` binary (pure Rust, no new crate or
  link). Code extraction never calls the network, so the model is obtained by an explicit,
  separate step: `dijkstral model fetch potion-code` downloads the pinned revision from Hugging
  Face into `~/.dijkstral/models/` and checks its SHA-256 (or `DIJKSTRAL_EMBED_MODEL=<dir>` points
  at a copy the user provides); `extract --embed` then writes `dijkstral-out/embed/` beside
  `graph.dij` (rebuilt with it, its ids checked on open), and `context` fuses it when present.
  Without the model or the index nothing changes.
- Not B: a transformer through candle or ort means a framework or a 1–2 MB runtime, a 129 MB+
  model and tens of minutes of indexing per large repository, for no measured gain.

## 5. Not evaluated

- Agent token cost: only offline coverage was measured; no agent run used the prototype.
- Code-specific transformers (`nomic-ai/CodeRankEmbed`, `jinaai/jina-embeddings-v2-base-code`,
  137–161M parameters): about five times `bge-small`, too slow to index Kubernetes on the CPU here;
  ONNX Runtime (`ort`) was not tried (candle needs no native library).
- Query instructions or prefixes for A, approximate nearest-neighbour search (not needed at 5 ms),
  per-language comment conventions beyond Python, Rust and Go, and repositories other than the
  three benchmark corpora.

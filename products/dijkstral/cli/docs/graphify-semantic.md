# Graphify semantic pass and markdown structure: black-box specification

This document specifies what Graphify does with everything that is not code: which files it sends
to a model, how it packs and sends them, what it expects back, how the reply becomes graph nodes,
edges and hyperedges, how results are cached, how communities are named by a model, and the
LLM-free markdown structure that `graphify update` adds. The reference is PyPI `graphifyy` 0.9.86,
the release of `graphify-format.md` and `graphify-query.md`.

Method. Everything was observed by running Graphify, never by reading its source, prompts or skill
files. Model traffic went through `bench/llm_shim.py`, a localhost OpenAI-compatible server
(`--backend openai`, `OPENAI_BASE_URL`, dummy key) that either answers with a fixed or crafted reply
(`--responder static`, free, used for every probe of chunking, retries, repair and merging),
replays recorded replies (`replay`), or forwards to `codex exec` on the user's ChatGPT login
(`codex`, model `gpt-6.1-sol`, reasoning effort `medium`, passed explicitly on every call). The
shim stores only the structure of each request (roles, part types, sizes, hashes, where corpus
files, paths and known strings sit, the JSON keys and quoted identifiers it names) and never its
prose. Six `codex exec` runs were made in total (one smoke test, then extract, label, extract,
label and deep extract); nothing used a paid API key. Graphify always ran with a scrubbed
environment (`env -i`, `HOME` in a scratch directory) so no real key could be picked up.

Status labels, as in the other specs: **Rule** (observed consistently; reproduce), **Quirk**
(observed defect or accident; reproduce only for byte parity), **Open** (not resolved; collected in
§12).

## Contents

1. Installs, harness and goldens
2. Which files reach the semantic pass
3. Markdown structure without a model (`update`)
4. Dispatch: order, chunking, request shape, concurrency
5. The reply schema
6. Validation, repair, retries
7. From reply to graph
8. Outputs that change when semantic data is present
9. Cache and incrementality
10. Community labelling with a model
11. `add <url>` and `--google-workspace`
12. Quirks and open questions

---

## 1. Installs, harness and goldens

- The benchmark install (`bench/setup.sh`: `graphifyy[leiden,mcp]`) has **no** `openai`, `pypdf`,
  `python-docx`/`openpyxl`, `watchdog` or `faster-whisper`. With it, `extract --backend openai`
  fails every chunk (`the 'openai' package is required for this backend`) and exits 1; PDFs are
  dispatched with a warning `PDF text extraction skipped: 'pypdf' is not installed`; `.docx` and
  `.xlsx` are not counted or sent at all.
- Semantic probes used a second venv, `$DIJKSTRAL_BENCH_HOME/tools/graphify-0.9.86-semantic`, with
  `graphifyy[leiden,mcp,openai,pdf,office]==0.9.86` plus `watchdog` (for `watch`). The `video`
  extra (`faster-whisper`, `yt-dlp`) was not installed.
- With no key and no `--backend`, `extract` stops before any request:
  `error: no LLM API key found (N doc/paper/image file(s) need semantic extraction). Set
  GEMINI_API_KEY or GOOGLE_API_KEY (gemini), MOONSHOT_API_KEY (kimi), ANTHROPIC_API_KEY (claude),
  OPENAI_API_KEY (openai), DEEPSEEK_API_KEY (deepseek), or pass --backend. …` A corpus without
  docs needs no key.
- How to run Graphify and replay the goldens: `bench/semantic/README.md`.

Goldens in `tests/fixtures/semantic/`:

| File | Produced by |
|---|---|
| `markdown/` | LLM-free markdown probe corpus (code in Python, TS, Rust; markdown, mdx, rst, txt, yaml, html) |
| `markdown.update.graphify.json` | `graphify update` (clustered), re-indented like the other goldens |
| `markdown.update_raw.graphify.json` | `graphify update --no-cluster` |
| `markdown.graphify_report.md` | the `update` report (title line names the scratch dir `mdc`) |
| `mixed/` | semantic corpus: Python code, `README.md` with front matter, `docs/guide.md`, `docs/glossary.txt`, `config/queue.yaml`, `papers/write_through.pdf`, `images/architecture.png`, `office/runbook.docx`, `office/capacity.xlsx` (binaries from `bench/semantic/make_binaries.py`) |
| `mixed.transcript/req-000{1,2,3}.json` | redacted requests and full Codex replies: 1 extract, 2 label, 3 `--mode deep` extract |
| `mixed.extract.graphify.json`, `mixed.extract.graphify_analysis.json` | `extract --backend openai` through Codex |
| `mixed.extract_raw.graphify.json` | the same reply replayed into `extract --no-cluster` |
| `mixed.deep.graphify.json` | `extract --mode deep` through Codex |
| `mixed.label.graphify.json`, `…_labels.json`, `…_report.md` | `label --backend openai` after the extract |
| `mixed.update.graphify.json`, `mixed.update.graphify_report.md` | `update` after the extract (markdown structure merged with semantic nodes) |
| `mixed.cache_keys.json` | cache directories, per-file cache keys, manifest hashes, converted file names |
| `merge_probe{1,2}.reply.json` | crafted replies (static responder) exercising validation and merging |
| `merge_probe{1,2}.extract[_raw].graphify.json` | Graphify's clustered and raw graphs for those replies on `mixed/` |

Replaying `mixed.transcript` reproduced `mixed.extract`, `mixed.deep` and the label run byte for
byte (graph, analysis, labels), so every semantic golden is a deterministic function of the
recorded replies.

---

## 2. Which files reach the semantic pass

### 2.1 Classification (`extract` without `--code-only`)

`extract` prints `found C code, D docs, P papers, I images` and then `semantic extraction on N
files`.

| Class | Extensions (case-insensitive) | Sent as |
|---|---|---|
| doc | `.md .mdx .rst .txt .html .yaml .yml` (also `CMakeLists.txt`) | raw file text (HTML and RST are not converted) |
| doc (office) | `.docx .xlsx` (only with the `office` extra) | converted to markdown first (§2.2), the converted file is sent |
| paper | `.pdf` | extracted text (pypdf), §4.4 |
| image | `.png .jpg` probed (`.jpeg .gif .webp` are listed as image types by the `update` report) | a separate `image_url` content part with the raw file bytes |
| image (vector) | `.svg` | its XML text, inline like a document |

- **Rule.** `.markdown`, `.htm`, `.csv`, `.ipynb` and the "not classified" list of
  `graphify-format.md` §14.5 are not sent. The ignore rules, default skipped directories and the
  sensitive-file filter of §14.5 apply unchanged.
- **Rule.** Audio and video (`.mp3 .wav .ogg .m4a .mp4 .mov .webm .m4v .mkv .avi`) are silently
  skipped without the `video` extra: not counted, not sent, no warning (`update`'s report still
  treats them as known types, §17.5 of the format spec). **Open:** transcription with the extra.
- **Rule.** Google Workspace shortcuts (`.gdoc .gsheet .gslides`) are silently skipped (§11).
- **Rule.** The image MIME type comes from the extension (`.jpg` → `image/jpeg` even for PNG
  bytes); the bytes are base64-encoded unchanged (`data:image/png;base64,` + 12,496 chars for a
  9,371-byte PNG).
- **Rule.** Code files never go to the model; they are extracted locally as in `--code-only`.
- **Rule.** `extract` without `--code-only` does **not** build the markdown page/heading structure
  of §3: markdown files are only sent to the model.

### 2.2 Office conversion

- **Rule.** Each `.docx`/`.xlsx` is converted to `graphify-out/converted/<stem>_<h>.md`, where `h`
  is the first 8 hex digits of `sha256(relative path)` (`office/capacity.xlsx` →
  `capacity_e7fd8f6a.md`). The converted file starts with `<!-- converted from <basename> -->`;
  `.xlsx` sheets become `## Sheet: <name>` and a markdown table; `.docx` paragraphs become plain
  lines (heading styles are not marked).
- **Rule.** The converted path is what is dispatched, cached and expected back as `source_file`.
  A reply node that names the original (`office/capacity.xlsx`) is dropped as out of scope (§7.1).
- **Quirk.** `update` later indexes these converted files as markdown pages although
  `graphify-out` is a skipped directory (`mixed.update.graphify.json`:
  `graphify_out_converted_capacity_e7fd8f6a`, heading `Sheet: Capacity`), and counts them in the
  report's file and word totals.

---

## 3. Markdown structure without a model (`update`)

`graphify update` (no model) adds structural nodes for `.md` and `.mdx` files (extension
case-insensitive: `UPPER.MD` counts). `.rst .txt .yaml .yml .html` produce nothing there;
`.markdown` is "not classified". Goldens: `markdown.update*.graphify.json`; every rule below was
also probed separately (heading forms, front matter variants, 25 link forms, a 400-name sweep of
backtick references).

### 3.1 Page nodes

- **Rule.** One page node per file: id = the file id of `graphify-format.md` §4.2 (after §4.5
  collision renaming: `tools/util.md` next to `tools/util.py` is `tools_util_md_tools_util`), label
  = the §5.1 file label (markdown and code share one namespace), `file_type: "document"`,
  `node_kind: "page"`, `source_location: "L1"` even for a 0-byte file (code files get `null`),
  `_origin: "ast"`.
- **Rule.** Raw key order: `id, label, file_type, node_kind, source_file, source_location,
  [frontmatter], _origin`.
- **Rule.** A file with no recognised heading still gets its page, and is listed in the warning
  `N code file(s) yielded no symbols … : bom.md, noheads.md` (markdown is counted as "code" there).
  A 0-byte file is not listed.

### 3.2 Front matter

- **Rule.** Only when line 1 is exactly `---` and a later line is exactly `---`. Between them, each
  non-indented `key: value` line with a non-empty value becomes `frontmatter[key] = value`:
  - the value is stripped, one pair of surrounding `"` or `'` removed, then HTML-escaped with
    quotes (`<b>&"'</b>` → `&lt;b&gt;&amp;&quot;&#x27;&lt;/b&gt;`); every value is a string
    (`2`, `true`, `null`, `{a: 1}`, `[a, b]`, `>` kept as text);
  - keys are kept verbatim (spaces and case);
  - a repeated key keeps the last value;
  - empty values (`key:`), indented lines (list items, nested maps) and `#` comment lines are
    dropped.
- **Rule.** No front matter, or one with no usable pair, means no `frontmatter` key at all. An
  unclosed block, a block not on line 1 and TOML `+++` blocks are not front matter.
- **Rule.** Lines inside the block are not scanned for headings (`# a comment` is no heading), but
  line numbers count from the top of the file.

### 3.3 Heading nodes

- **Rule.** A heading is a line matching `^#{1,6} +(.+)` at column 0 (no leading spaces).
  `#NoSpace`, `####### seven`, a bare `#`, setext underlines and a heading behind a UTF-8 BOM are
  not headings. CRLF files work.
- **Rule.** Label = the text after the hashes, stripped, verbatim: closing hashes stay
  (`Closing hashes ##`), inline markup stays (`Install \`pip\``, `Deep [link](http://x.y) heading`).
- **Rule.** Id = `<file id before collision renaming>_<slug(label)>` (§4.1 slug: Unicode letters
  kept, `Café — Ünïcode 🚀` → `h_café_ünïcode`). If that id already exists in the file, the
  heading's line number is appended: `same_same`, `same_same_3`, `same_same_5`; `Setup` and
  `setup` → `h_setup`, `h_setup_27`. Heading ids use the **un-renamed** file id
  (`tools_util_util_notes` under page `tools_util_md_tools_util`).
- **Rule.** Fields: `file_type: "document"`, `node_kind: "heading"`, `source_location: "L<line>"`,
  `_origin: "ast"`.
- **Rule.** Lines inside a backtick fence (```` ``` ````) are skipped. **Quirk:** tilde fences
  (`~~~`) and HTML comments are not skipped: `## Tilde fenced heading` and `# Comment heading`
  become headings.
- **Rule (nesting).** `contains` edges, `EXTRACTED`, `weight 1.0`, `source_location` = the child's
  line: each heading's parent is the nearest preceding heading with a smaller level, else the page.
  A file starting at `##` hangs it under the page; a later `#` also goes under the page and an
  `###` after it under that `#`.

### 3.4 Backtick references to code

- **Rule.** Every inline code span outside backtick fences (single or double backticks; also in
  lists, tables, headings, tilde fences, indented code blocks and HTML comments) is a candidate.
  The span text is stripped; one trailing `()` is ignored for matching but kept in `context`.
- **Rule (shape).** The stripped text must be dotted identifiers: `name` or `a.b.c`
  (`[A-Za-z_][A-Za-z0-9_]*` segments). `run(1)`, `Gamma<T>`, `@run`, `run;`, `run.`, `.solo`,
  `Alpha::go`, `Alpha#go`, `pkg/b.Beta`, `a.py`, `run job` match nothing.
- **Rule (candidates).** Only `_callable` nodes (format spec §6: Python functions, methods and
  classes; TS/JS functions, methods, classes, interfaces, type aliases, enums; Java types and
  methods). Rust, Go, shell, JSON nodes, TS consts, enum members, externals, stubs and file nodes
  are never targets. A candidate's name is its label without a leading `.` and trailing `()`;
  comparison is case-sensitive (`alpha`, `RUN` miss).
- **Rule (uniqueness).** The last segment must name exactly one candidate in the whole graph;
  otherwise nothing is emitted (`shared` in two files, `go` as function and method, and
  `Alpha.go` because `go` is ambiguous). Same-name collisions that merge nodes (format spec §4.5)
  count once.
- **Rule (qualifiers).** For `q1.q2.….name`, every qualifier must equal (case-sensitively) a
  directory name or the stem of the target's `source_file`, or the label of its enclosing class:
  `Beta.other`, `b.Beta.other`, `pkg.Beta`, `c.area`, `web.Gamma`, `a.Twin.twin` and
  `Beta.pkg.other` (any order) resolve; `Alpha.other`, `B.other`, `PKG.Beta`, `lph.Beta`,
  `eta.other`, `b_beta.other`, `zzz.run` do not.
- **Rule (denylist).** These texts never resolve even when unique (exact case; probed lists, so
  more names may exist, §12): `abs all any bool bytes callable console delattr dict dir enumerate
  filter float getattr hasattr hash id int isinstance iter len list map max min next open print
  range repr reversed round set setattr sorted str sum super tuple type vars zip parseInt
  parseFloat isNaN isFinite encodeURIComponent decodeURIComponent Array Bool Boolean Color
  Comparable Data Date Double Equatable Error Float Font Hashable Identifiable Int JSON Map Math
  Number Object Promise RegExp Request Response Sendable Set String Symbol TypeError URL UUID View
  Character Int64 UInt NSObject Codable Decodable Encodable URLRequest URLSession DispatchQueue
  Headers FormData Blob File WeakMap WeakSet BigInt Intl Reflect Proxy CaseIterable NSError
  AnyObject SyntaxError RangeError ReferenceError EvalError URIError`. Probed and **not**
  denylisted, among others: `Any Path Config Mock Foundation Widget Item Node patch main json get
  test self object format input exit __init__ _private x ab HTTP Str Bytes ValueError KeyError
  AggregateError Text Image Button Element Float32`.
- **Rule (edge).** `source` = the nearest preceding heading, or the page for text before the first
  heading; `target` = the code node; `relation: "references"`, `confidence: "INFERRED"`,
  `confidence_score: 0.95`, `context` = the stripped span text (`run()`, `Alpha.solo`, ` run ` →
  `run`), `source_file` = the markdown file, `source_location` = the span's line, `weight: 1.0`,
  `_origin: "ast"`. Raw key order: `source, target, relation, context, confidence,
  confidence_score, source_file, source_location, weight, _origin`.
- **Rule.** One edge per (source section, target), at the first occurrence.

### 3.5 Links between documents

- **Rule.** Link targets come from inline links `[t](dest)` (also inside headings, inline code and
  tilde fences, but not backtick fences), reference definitions `[r]: dest` (the edge is at the
  definition line), and wiki links `[[name]]` / `[[name|alias]]` / `[[sub/name]]` /
  `[[name.md]]`. Images `![…](…)`, autolinks `<x.md>`, HTML `href`, `http(s):`, `mailto:` and
  absolute `/x.md` are ignored.
- **Rule (resolution).** `dest` is cleaned (surrounding spaces, `<…>`, a `"title"`, `#fragment`,
  `?query`, percent-encoding), resolved against the linking file's directory and normalised
  (`./sub/../t.md`). If that path exists it is used, else `path + ".md"` is tried (extensionless
  `other` → `other.md`); paths outside the root never resolve. The target id is
  `slug(resolved path without extension)` and must be a markdown page id (before collision
  renaming): `t10.markdown` reaches page `t10` from `t10.md` when both files exist (**Quirk**),
  `a.py` or `x.rst` never produce an edge, and `DOCS/T1.md` resolves on a case-insensitive file
  system (**Open** on others).
- **Rule (edge).** `source` = the **page** (never a heading), `relation: "references"`,
  `confidence: "EXTRACTED"`, no `context`, no `confidence_score` in raw output (1.0 when clustered),
  `source_location` = the line, `weight: 1.0`. Raw key order: `source, target, relation,
  confidence, source_file, source_location, weight, _origin`. One edge per (page, target) at the
  first occurrence; self-links produce nothing.
- **Quirk.** When two pages collide (`t21.md` + `t21.mdx` → `t21_md_t21`, `t21_mdx_t21`), a link
  resolved before the rename keeps the stale id `t21` in raw output and is dropped by the build.

### 3.6 Order and build

- **Rule (raw order).** Nodes: all code files as in format spec §2.4, then markdown files in sorted
  path order (byte order: `README.md`, `UPPER.MD`, `bom.md`, …), each as page, then headings in
  line order. Edges: per file, `contains` and link edges in line order (a heading's `contains`
  before the links on its line), then all backtick references in file order.
- **Rule (report).** `update`'s `Corpus Check` counts code files and markdown pages as files, but
  its word total also includes the other documents (`.rst .txt .yaml .html`: 418 words for the
  `markdown` fixture, 404 without them), PDF text, converted office text and raster images read
  as UTF-8 with invalid bytes dropped (`mixed`: 823 words).
- **Rule (papers).** A `.md` file with at least three paper signals is a paper and gets no page
  (Graphify's `worked/mixed-corpus/raw/attention_notes.md`). Signals, each counted once:
  `arxiv`, an arXiv id `1706.03762`, `doi`, `abstract`, a citation `[1]` (not `argv[0]`),
  `\cite{`, `journal`, `preprint`, `proceedings`; `et al.`, `References`, `ISBN`, a second DOI and
  others do not count. **Open:** exact patterns, and whether other document types are reclassified
  for the semantic pass too (dijkstral assumes so).
- **Quirk.** `update` discovers files differently from `extract --code-only`: it descends into
  `.claude/` (code and documents), and a symlinked file is indexed under its own path, its id and
  heading ids derived from the resolved absolute path (`private_tmp_…_aaa_zzz`), while a target
  sorted before its symlink (`AGENTS.md` ← `CLAUDE.md`, `a.py` ← `b.py`) is dropped. dijkstral's
  walker does neither (monet: 21 `.claude/skills` pages and 10 `CLAUDE.md` pages differ).
- **Rule.** The clustered build treats these nodes and edges like any other (§8.2 fold of the
  format spec; `docs_guide → readme` and `readme → docs_guide` keep one link).
- **Rule.** On a graph that already has semantic nodes, `update` keeps them (with
  `_origin: "semantic"`) and adds the structure; when a heading or page id equals a semantic id
  (`readme`, `docs_guide`), the structural node's attributes win and the semantic edges attach to
  it (`mixed.update.graphify.json`).

---

## 4. Dispatch: order, chunking, request shape, concurrency

### 4.1 Order

- **Rule.** Files are dispatched in this order: documents (including converted office files) and
  vector images sorted by relative path, then raster images, then papers. In one chunk the text
  part lists documents, then SVG text, then paper text; raster images follow as separate parts.
  (**Open:** whether converted files are sorted in or appended after the other documents; their
  `graphify-out/converted/` paths sort after `docs/` in every probe.)

### 4.2 Chunking (`--token-budget`, default 60000)

- **Rule (estimate).** A text file costs `len(cl100k_base.encode(text)) + 40` (fitted by bisection
  on two- and three-file corpora: `'word '*200` costs 241, three of them fit at exactly 723; a
  Japanese text whose cl100k and o200k counts differ pinned cl100k; path length and extension do
  not matter). A PDF costs its extracted text's tokens + 40 (177 for the fixture paper). Every
  raster image costs 1600, whatever its size (10×10, 560×220 and 2000×2000 all fit at 1600).
  **Open:** the estimate for backends other than `openai`.
- **Rule (packing).** Greedy and sequential in dispatch order: a file joins the current chunk while
  the running sum stays `<= budget`, otherwise it starts the next chunk. Budget 250 on the
  fixture gives `[README, queue.yaml] [glossary, guide, capacity] [runbook] [image] [paper]`.
- **Rule (oversize).** A single file whose estimate exceeds the budget is split into pieces of
  20,000 characters (25,000 chars → 20,000 + 5,000), each sent as its own chunk, independent of
  the budget value. Under the budget it is sent whole.

### 4.3 Request

- **Rule.** `POST {OPENAI_BASE_URL}/chat/completions`, never streaming, body
  `{"model", "stream": false, "max_completion_tokens": 16384, "temperature": 0, "messages"}`; no
  `response_format`, no `tools`. `model` is `--model`, else `OPENAI_MODEL`.
- **Rule.** Two messages: `system` (3,301 characters, 30 lines in normal mode; 3,589 / 35 with
  `--mode deep`; identical for every chunk and model) and `user`. The user content is an array: one
  `text` part, then one `image_url` part per raster image (`{"url": "data:<mime>;base64,…",
  "detail": "auto"}`).
- **Rule (text part).** Per document: a 24-char opening frame, the relative path, 10 chars, the
  sha256 hex of the file's full content (front matter included), 3 chars, the content verbatim, a
  46-char closing frame (sizes from the fixture; the frame text itself is not recorded). A PDF has
  its path, 77 chars of framing and the pypdf text without a hash. A chunk with raster images ends
  the text with a reference to each image path (`images/architecture.png` closes the fixture's
  text). An oversize piece uses a shorter frame (128 chars around 20,000).
- **Rule (system part).** It describes the reply schema of §5: it names the keys `nodes edges
  hyperedges id label file_type source_file source_location source_url captured_at author
  contributor rationale source target relation confidence confidence_score weight input_tokens
  output_tokens` and the placeholders `node_id`, `snake_case_id`, `stem_entity`.

### 4.4 Concurrency, retries and timeouts

- **Rule.** `--max-concurrency N` (default 4) chunks in flight; with 1 they go strictly in
  dispatch order (`#1 0.00–1.50 s, #2 1.52–3.03 s, …` with a 1.5 s reply delay).
- **Rule.** HTTP errors (500 probed) are retried by the OpenAI client itself (two retries, about 0.5 s and
  0.8 s apart) before Graphify sees a failure.
- **Rule.** `--api-timeout S` (default 600) is the client's per-request timeout.

---

## 5. The reply schema

A reply is one JSON object (Codex's replies in `mixed.transcript` are complete examples):

```json
{"nodes": [{"id": "docs_glossary_job", "label": "Job: unit of work pushed onto JobQueue",
            "file_type": "concept", "source_file": "docs/glossary.txt", "source_location": null,
            "source_url": null, "captured_at": null, "author": null, "contributor": null,
            "rationale": null}],
 "edges": [{"source": "docs_glossary", "target": "docs_glossary_job", "relation": "references",
            "confidence": "EXTRACTED", "confidence_score": 1.0, "source_file": "docs/glossary.txt",
            "source_location": null, "weight": 1.0}],
 "hyperedges": [{"id": "job_processing_flow", "label": "…", "nodes": ["a", "b", "c"],
                 "relation": "participate_in", "confidence": "EXTRACTED",
                 "confidence_score": 1.0, "source_file": "docs/guide.md"}],
 "input_tokens": 0, "output_tokens": 0}
```

- **Rule (required).** Nodes: `id`, `label`, `source_file`; edges: `source`, `target`,
  `relation`, `confidence`, `source_file` (§6 warns about each missing one). `file_type` is one of
  `code document paper image concept rationale`; `confidence` one of `EXTRACTED INFERRED
  AMBIGUOUS`.
- **Rule.** Ids are expected as `<file stem>_<entity>` snake case (§7.2 rewrites them to the full
  file id). `source_location` is free text in practice (`"Running jobs"`, `"References [1]"`).
- **Observed relations** in Codex's replies: `references`, `conceptually_related_to`,
  `shares_data_with`, `cites`, `calls`, `participate_in` (hyperedges); any string is accepted
  (§7.3).
- **Rule.** `input_tokens`/`output_tokens` in the reply are ignored; Graphify uses the API
  `usage` (§8.3).

---

## 6. Validation, repair, retries

- **Rule (repair).** A reply wrapped in a code fence, or with prose before and after the object,
  is accepted (the first JSON object is used; two concatenated objects → the first). Single
  quotes, trailing commas or a bare array are not repaired.
- **Rule (invalid).** Unparseable text logs `LLM returned invalid JSON, skipping chunk (first 200
  chars: …)` and counts as hollow.
- **Rule (hollow).** A reply that yields no nodes and no edges (logged as `content=no nodes/edges,
  output_tokens=0`; replies with non-zero output tokens were not probed) is retried on
  the same chunk up to 3 attempts in total, after 2 s and 8 s; then the chunk is given up
  (`chunk of N still hollow after 3 attempt(s)`), its files get partial cache entries (§9) and are
  retried next run. A non-empty reply that omits some files is **not** retried: `WARNING: k/N
  dispatched file(s) produced no nodes … a re-run will retry them`.
- **Rule (truncation).** `finish_reason: "length"` on a multi-file chunk bisects it (`chunk of 4
  truncated at depth 0, splitting into halves of 2 and 2`) recursively down to single files; a
  truncated single-file chunk keeps whatever parsed (`partial result kept (not cached as
  complete)`).
- **Rule (schema warnings).** One summary line, `Extraction warning (16 issues): 12x missing
  required field 'source_file' …; 1x missing required field 'label' …; 1x other schema issue (e.g.
  Edge 16 has invalid confidence 'maybe' …)`. Warnings never drop an item by themselves.
- **Rule (shrink guard).** If the new graph has fewer nodes than the existing `graph.json`, or a
  re-extracted file shrinks (`unverified semantic shrink detected for 'README.md' (5 -> 1
  nodes)`), `extract` refuses to write and exits 1 unless `--allow-partial` (or `--force` for the
  node-count guard of `update`) is given.

---

## 7. From reply to graph

Goldens: `merge_probe1`/`merge_probe2` (crafted replies through the static shim) and the Codex
runs. "Raw" is `extract --no-cluster`, "clustered" the default build.

### 7.1 Dropped items

- **Rule.** A node whose `source_file` is not one of the files dispatched in this run (a code
  file, the original `.docx`/`.xlsx`, `README.md` when only another file was sent) is dropped with
  `dropped N out-of-scope node(s) attributed to file(s) not dispatched for extraction: …`.
  `"./README.md"` counts as in scope and is kept verbatim.
- **Rule.** A node without `id` is dropped. A node without `label` or `source_file` is kept
  without the key.
- **Rule.** An edge whose `source` or `target` is neither a reply node nor an existing graph node
  is dropped (`ghost_node`, a label used as id, an edge to a dropped out-of-scope node). Edges to
  AST nodes are kept (`readme_job_service → src_app_util_slugify`).
- **Rule.** The same id twice in one reply keeps the **first** occurrence (`note: node 'dup_node'
  was extracted twice from 'README.md' under different labels — keeping 'first', dropping
  'second'`). **Quirk:** raw output shows the second.

### 7.2 Ids and attributes (clustered build)

- **Rule (re-prefix).** With `F` = the file id of `source_file` and `s` = `slug(stem)`: an id equal
  to `s` becomes `F`; an id starting with `s_` (compared after lower-casing) becomes `F` +
  `slug(rest)` (`glossary_x` → `docs_glossary_x`, `Glossary_Z` → `docs_glossary_z`, `queue_cfg` →
  `config_queue_cfg`, `capacity_e7fd8f6a_sheet` → `graphify_out_converted_capacity_e7fd8f6a_sheet`).
  Ids already starting with `F`, or not starting with the stem (`x_glossary`, `glossaryx`, another
  file's stem, `Docs Guide/Operator Guide!`), are kept verbatim (no slugging). Edges and hyperedge
  members follow the rename. Raw output keeps the reply's ids.
- **Rule (rename collisions).** A renamed id that meets an existing semantic id merges into one
  node whose attributes are the later one's (`glossary_y` → `docs_glossary_y` keeps "stem prefixed
  duplicate of y"); an edge between them becomes a self-loop and is kept.
- **Rule (AST collisions).** A semantic node whose id equals an AST node id (`src_app_util`,
  `src_app_service_run_job`) leaves the AST node's attributes in the clustered graph; edges to that
  id stay. **Quirk:** raw output shows the semantic attributes at the AST node's position.
- **Rule (no label merging).** Semantic concepts are never merged with code nodes or with each
  other by label: `readme_run_job` ("run_job") and `src_app_service_run_job` ("run_job()") stay two
  nodes with no edge between them unless the model wrote one. Across chunks, only identical ids
  meet.
- **Rule (`file_type`).** `code document paper image concept rationale` are kept; anything else
  (`video`, `audio`, `Document`, `widget`) and a missing value become `concept` (raw keeps the
  reply's value).
- **Rule (evidence).** A node with `file_type: "code"` whose label does not occur in its source
  file gets `verification: "unverified"` (`[graphify] 1 semantic node(s) had no evidence in the
  source and were flagged verification=unverified`); other types are not checked. **Open:** the
  exact match rule (case, partial words).
- **Rule.** Other node keys pass through (`x_custom`, `confidence`, `rationale`, `author`, …),
  except `community`, which is removed. Clustered key order is the usual `id, label`, then sorted.
- **Rule.** Fresh semantic nodes and edges have no `_origin`; nodes and edges read back from the
  cache or kept by `update` have `_origin: "semantic"` (**Quirk**, §9.4).

### 7.3 Edges

- **Rule.** A missing `source_file` is filled from the source node's `source_file`
  (`queue_cfg → paper` gets `config/queue.yaml`). `source_location` and `weight` are not added when
  absent; given values are kept (`weight: 0.5`).
- **Rule.** `confidence_score` in the clustered graph: kept when numeric; missing or non-numeric
  (`"high"`) → `EXTRACTED` 1.0, `INFERRED` 0.55, `AMBIGUOUS` 0.2, invalid or missing confidence 1.0
  (format spec §2.5). The invalid `confidence` value itself is kept (`"maybe"`), and an edge with
  no `relation` or `confidence` is kept without the key.
- **Rule.** Any relation string is kept (`mentions`, `shares_data_with`). Self-loops are kept.
  Extra keys (`context`, `x_edge`) pass through.
- **Rule.** The one-edge-per-pair fold of format spec §8.2 applies. Between two semantic
  relations of one pair the first in reply order survived (`semantically_similar_to` INFERRED 0.7
  over a later `implements` EXTRACTED 1.0), and a reverse edge folds into the earlier one
  (`readme_job_service → docs_guide_operator_guide` disappeared next to the reverse edge).
  **Open:** the rank of semantic relations against code relations.
- **Rule.** Leiden weights `AMBIGUOUS` links ×0.5 (format spec §17.2); semantic replies are the
  only source of `AMBIGUOUS` links.

### 7.4 Hyperedges

- **Rule.** Raw output: the reply's hyperedges verbatim, in reply order, unknown members kept.
- **Rule.** Clustered output: members that are not graph nodes are removed (`h_ghost` keeps 2 of
  3), ids follow §7.2, hyperedges without `id` move after those with one, and the list is written
  **twice**: as the top-level `hyperedges` and as `graph.hyperedges`.

---

## 8. Outputs that change when semantic data is present

### 8.1 `graph.json`

- **Rule.** Clustered: `graph` becomes `{"hyperedges": [...], "schema_version": 1,
  "graphify_version": "0.9.86"}` (key `hyperedges` first) whenever there are hyperedges; the
  top-level `hyperedges` holds the same list; `built_at_commit` as in format spec §2.1.
- **Rule.** Raw (`--no-cluster`): keys `nodes, edges, hyperedges, input_tokens, output_tokens`
  with the summed API usage of this run (`16843`, `3275` for the fixture; through the shim these are
  Codex's counts, which include about 12k tokens of Codex's own instructions).

### 8.2 Analysis and report

- **Rule.** `.graphify_analysis.json` `tokens` = this run's summed usage
  (`{"input": 16843, "output": 3275}`); 0/0 for a fully cached run.
- **Rule.** `extract` prints `tokens: 16,843 in / 3,275 out, est. cost (~openai): $0.0120` and
  writes `.graphify_semantic_marker` = `{"output_tokens": N}`.
- **Rule.** `GRAPH_REPORT.md` (from `update`, `label`, `cluster-only`): `Token cost:` shows only the
  writing command's own usage (`15,160 input · 29 output` after `label`, `0 · 0` after `update`).
  Two sections appear when relevant, both after `## Import Cycles`:
  - `## Hyperedges (group relationships)` — `- **<label>** — <member ids, comma-separated>
    [<CONFIDENCE> <score with 2 decimals>]`, sorted by label (by id gives the same order in the
    fixture; **Open**).
  - `## Ambiguous Edges - Review These` (after the communities) — ``- `<source label>` → `<target
    label>`  [AMBIGUOUS]`` then `  <source_file> · relation: <relation>`.

---

## 9. Cache and incrementality

### 9.1 Layout

- **Rule.** `graphify-out/cache/semantic/p<12 hex>/<key>.json` (`--mode deep`:
  `cache/semantic-deep/p<12 hex>/`). The directory name changes with the prompt (normal
  `p5e80268fecd6`, deep `pd68e17f4cee0`) and not with the model (`OPENAI_MODEL`, `--model`) or the
  backend URL; it is not the md5/sha1/sha256 prefix of the system prompt (**Open:** recipe).
- **Rule (key).** `sha256(body + b"\0" + lower(relative path))`, where `body` is the file's bytes
  and, for markdown with front matter, the bytes after the closing `---` (starting at the newline
  that follows it). Verified for every file of the fixture, including the PDF, the PNG and the
  converted office files (`mixed.cache_keys.json`). Editing only the front matter is a cache hit.
- **Rule (entry).** `{"nodes": [...], "edges": [...], "hyperedges": [...]}` with the reply items
  whose `source_file` is that file; `"partial": true` is added for files of a hollow or truncated
  chunk. **Quirk:** items whose `source_file` spells the same file differently (`./README.md`)
  overwrite the entry written for the canonical spelling.
- **Rule.** `cache/stat-index.json` maps each file to `size`, `mtime_ns`, `indexed_at_ns`,
  `word_count` and `hashes` (`{lower(path): key}`); `manifest.json` maps each file to `mtime`,
  `seen`, `ast_hash` and `semantic_hash` (both the md5 of the file).

### 9.2 Reruns

- **Rule.** An unchanged corpus sends nothing (`0 code, 0 docs, 0 papers, 0 images changed; 11
  unchanged`). A changed body re-sends only that file (`semantic extraction on 1 files`); a
  front-matter-only change logs `semantic cache: 1 hit / 0 miss`.
- **Rule.** The new results replace that file's previous semantic nodes (`Replaced 5 node(s) from
  re-extracted source file(s).`) subject to the shrink guard (§6).
- **Rule.** `--force` (or `GRAPHIFY_FORCE=1`) skips the manifest gate and cache reads and re-sends
  every file.
- **Rule.** Every later write over a graph with semantic data (rerun, `update`, `label`) first
  copies the previous `.graphify_semantic_marker`, `.graphify_analysis.json`, `manifest.json` and
  `graph.json` to `graphify-out/<YYYY-MM-DD>/` (`backed up semantic graph (4 files) ->
  2026-10-15/`).

### 9.3 `update`, `watch`, `check-update`

- **Rule.** `update` never calls a model. It keeps semantic nodes and edges, adds the markdown
  structure (§3) and ends with `Code graph updated. For doc/paper/image changes run /graphify
  --update in your AI assistant.`. A changed non-markdown document alone does not change the graph
  (`No code-graph topology changes detected; outputs left untouched.`).
- **Rule.** `watch` (needs `watchdog`) rebuilds on code changes and, when a non-code file changes,
  writes `graphify-out/needs_update` containing `1`.
- **Rule.** `check-update <path>` prints nothing and exits 0 without the flag; with it,
  `[graphify check-update] Pending non-code changes in <abs path>.` and `[graphify check-update]
  Run \`/graphify --update\` to apply semantic re-extraction.`, exit 0.

### 9.4 Determinism

- **Rule.** Replaying the same replies gives byte-identical `graph.json` and analysis.
- **Quirk.** A rerun served from the cache differs from the fresh run only by
  `_origin: "semantic"` on every semantic node and edge (also after `update` and `label`).

---

## 10. Community labelling with a model

- **Rule.** `label <path>` and `cluster-only <path>` without `--no-label` re-cluster, then name
  communities through the configured backend (`--backend`, `--model`; `--max-concurrency` default 4,
  `--batch-size` default 100 communities per call; `--missing-only` keeps existing names).
- **Rule (request).** One `user` message, no system message, `temperature: 0`,
  `max_completion_tokens = 256 + 48 · (communities in the batch)` (496 for 5, 352 for 2, 304 for 1).
  Its text has a short instruction and one line per community with a selection of member labels
  (trailing `()` removed, `, `-separated; **Open:** which members, how many, their order and any
  length cut: 7 of 9 labels appeared for the fixture's largest community).
- **Rule (reply).** One JSON object mapping community id strings to names:
  `{"0":"Job Queue Caching","1":"Job Queue Implementation",...}`.
- **Rule (failure).** An unusable reply splits the batch and retries the halves down to single
  communities (5 communities → 9 requests); communities still unnamed keep the structural fallback
  name (`labeled 0 of 5 communities; 5 kept structural fallback names`).
- **Rule (outputs).** Names go to every node's `community_name`, `.graphify_labels.json`
  (`{"<cid>": "<name>"}`: one line after `label`, `indent=2` after `update`, which names
  communities by hub label) and `.graphify_labels.json.sig`
  (`{"<cid>": "<16 hex>"}`, **Open:** what is hashed), and to `GRAPH_REPORT.md` and `graph.html`.

---

## 11. `add <url>` and `--google-workspace`

- **Rule.** `add` accepts only `http`/`https` (`error: ingest: Blocked URL scheme 'file' - only
  http and https are allowed.`) and resolves the host first, refusing private and loopback
  addresses (`error: ingest: Blocked private/internal IP 127.0.0.1 (resolved from '127.0.0.1').`,
  also `localhost` → `::1`, `[::1]`), exit 1. With the clean-room rule of fetching nothing but a
  local URL, no download could be observed; `--dir` (default `./raw`), `--author` and
  `--contributor` are documented by `--help` only (**Open:** saved file name, format and how the
  graph is updated).
- **Rule.** `.gdoc`/`.gsheet`/`.gslides` shortcut files are not counted or sent, with or without
  `--google-workspace`; without a `gws` binary on `PATH` the flag prints nothing and creates no
  conversion (`graphify-out/converted/` stays empty). **Open:** behaviour with `gws` installed.

---

## 12. Quirks and open questions

### 12.1 Quirks

| # | Quirk | Where |
|---|---|---|
| S1 | Tilde fences and HTML comments are not skipped: headings and backtick references inside them count. | §3.3, §3.4 |
| S2 | Office conversions under `graphify-out/converted/` become markdown pages in `update`. | §2.2 |
| S3 | A link to `x.markdown` reaches page `x` from `x.md`; links resolve before collision renaming and dangle afterwards. | §3.5 |
| S4 | Raw output shows the last of two same-id reply nodes and the semantic attributes of an AST collision; the clustered graph keeps the first / the AST node. | §7.1, §7.2 |
| S5 | Cached and fresh runs differ by `_origin: "semantic"`. | §9.4 |
| S6 | A non-canonical `source_file` spelling overwrites the canonical file's cache entry. | §9.1 |
| S7 | Hyperedges are written twice (`hyperedges` and `graph.hyperedges`). | §7.4 |
| S8 | The report's token line counts only the writing command, not the graph's history. | §8.2 |

### 12.2 Open

1. Completeness of the backtick denylist (§3.4): only the ~420 probed names are classified.
2. Link resolution on case-sensitive file systems (§3.5).
3. Token estimate for non-`openai` backends; placement of converted office files in the order
   (§4.1–4.2).
4. The cache directory recipe `p<12 hex>` (§9.1) and the `.graphify_labels.json.sig` values (§10).
5. Evidence check details for `file_type: "code"` nodes (§7.2).
6. Fold ranks of semantic relations against AST relations (§7.3).
7. Label prompt member selection (§10).
8. `add` downloads, `--google-workspace` with `gws`, audio/video transcription with the `video`
   extra (§2.1, §11).
9. Qualified backtick references on real corpora (§3.4): fastapi resolves
   `APIRoute.get_route_handler()` among nine `get_route_handler` callables,
   `fastapi.security.base.SecurityBase` among two and `fastapi.sse` to a function in
   `tests/test_stream_status_code.py`, while the probes (`Alpha.go`, `pkg.Beta.go`, `zzz.run`,
   a scratch `fastapi.sse`) do not resolve. 68 of fastapi's 19,650 markdown links (0.35%) and 3
   of Graphify's 3,880 depend on this.
10. Paper detection patterns and the symlink rule of `update` (§3.6).

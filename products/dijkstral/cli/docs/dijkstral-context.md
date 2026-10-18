# `dijkstral context`

dijkstral's own command (Graphify has none like it): the evidence for a question in one answer.
An assistant's cost is set by its round trips — every command re-sends the conversation — so
where `query` returns a neighbourhood to explore and `explain` one symbol at a time, `context`
returns what an answer needs, read from the graph and the source tree together. `query`,
`explain`, `path` and `affected` keep Graphify's behaviour; `context` adds to them.

```text
dijkstral context "<question>" [--budget N] [--root DIR] [--graph PATH]
```

## 1. Anchors

- **Rule (named symbols).** The question's code-like names are resolved first: backticked spans;
  words with `_`, `.`, `::` or a trailing `()`; a capitalised word beside a kind word (`the Sink
  trait`, `struct Searcher`); and bare CamelCase words (`FastAPI`). `Type::method` and
  `Type.method` resolve to the method whose enclosing node (§2) is `Type`; other names resolve as
  `explain` resolves them (`graphify-query.md` §4), an unknown qualified name retrying with its
  last segment and an ambiguous one keeping its two best-connected candidates outside tests. A
  bare CamelCase word counts only when a label spells it exactly and it is not a hub (degree at
  or above `query`'s hub threshold), so `FastAPI` in "where does FastAPI decide …" is not the
  `FastAPI` class every test touches. At most 4.
- **Rule (code text).** A code question that names no symbol is answered from the code's text: each
  line of every code file (not configuration or data files: TOML, JSON, YAML, lock, INI, XML,
  Markdown, text, CSV), comment and prose lines skipped (`#`, `//`, `/*`, `*`, docstring quotes, and
  lines of six or more words without `(`, `)`, `=`, `{`, `}`, `[`, `]` or `;`, such as help text),
  is split into words — identifiers at `_` and lower-to-upper changes (`readJSONBody` → read, json,
  body), stemmed — and credited to the innermost graph node starting at or before it. Each node
  scores Σ idf(term) × (1 + ln count) over the question's subject terms (scaffolding and words that
  describe code in general removed; each term also matches its common abbreviations — `config`/`cfg`
  for configuration, `err`, `dir`, `req`, `buf`, `ctx` and about forty more — and a two-word phrase
  code spells as one word adds that word: `mmap`, `regex`, `stdin`, `timeout`), plus 3 × idf for
  each of them its own name contains (`_should_embed_body_fields` for "embedded … body"), plus a
  quarter of the scores of its three best-scoring callers and callees (`calls`, `indirect_call`,
  `uses`: the graph's view of where the answer lives), times the number of distinct terms, damped by
  √(1 + span/40) where its span runs to the next node of its file; it needs two distinct terms (or
  the only one). The top 4, tests last (test files, Rust `mod tests`, `test…` names, and examples,
  documentation code and benchmarks: `examples/`, `docs_src/`, `samples/`, `bench…/`), become
  anchors, each with its three best matching lines not already shown; anchors after the second show
  only the first 2 lines of their definition.
- **Rule (named and unnamed).** A question that names symbols may still be about code it does not
  name ("which steps run when `X` changes"): its two best code-text matches not already anchored
  follow the named symbols, ranked as above.
- **Rule.** Otherwise the anchors are `query`'s seeds for the question's terms
  (`graphify-query.md` §3.6). When every one is a document node (a fact, concept or document), up
  to 8 are kept, those whose label shares a subject word with the question: the question's terms,
  stemmed, without scaffolding words (`many`, `few`, `last`, `months`, `years`, …).

- **Rule (structure).** When the question names no symbol and asks how the code is organised
  (a word starting with `organis`, `organiz`, `divid`, `split`, `structur`, `architectur`,
  `responsib`, `layer`, `layout`, `overview`, `crate`, `package`, `module`, `component` or
  `subsystem`), the answer adds a map of the code's areas: each top-level directory, or each of
  its subdirectories when it holds three or more (`crates/*`, `pkg/*`), with its symbol count and
  the four symbols outside tests used from the most other files (then by degree: many calls inside
  one file do not make a symbol representative); areas whose path or top symbols carry the
  question's subject words come first, then larger ones; at most 12. When the first area matches
  the question, its subdirectories follow the same way (`Inside <area>/`), so a question about
  one subsystem of a large repository sees how that subsystem is divided.

- **Rule (flow).** A flow question that names no symbol — text after `from` up to ` to `,
  ` until `, ` into ` or ` through to `, or around `end up (as|in)`, `get(s) to`, `reach(es)`,
  `become(s)`, `turn(ed) into` — is split into its two ends; each end's best code-text match
  becomes an anchor, and the answer adds the shortest call chain from the start's top two
  matches to the end's: breadth first over `calls`, `indirect_call`, `uses`, `references`,
  `instantiates`, `contains` and `method` links in their stored direction, never through a file
  node, at most 6 hops, each step with its relation and the line of the link.
- **Rule.** Bare CamelCase names (above) never stop the code-text and flow searches; they are
  added after what those found.

- **Rule (speed).** The code is scanned once per question, files in parallel, for the union of
  the terms of the whole question and of a flow's two ends; a line is split into words only when
  its lower case contains one of the terms (a stem is a substring of every word it matches).
  About 0.4 s on Kubernetes with a warm page cache.

- **Rule (descriptions).** Comment lines (`#`, `//`, `/*`, `*`, …) and the lines of Python
  docstrings (between `"""` or `'''`) are the code described in plain words: their terms
  count for the enclosing node exactly as its code's terms do (they bridge "memory map" and
  `mmap`), but they are not shown as matching lines. Prose inside string literals (help text)
  still counts for nothing.

## 2. One anchor

```text
## <enclosing › label>  <file>:<line>
  <line>| <the first 4 lines of its definition, for code>
  matching lines:
  <line>| <its best lines for the question, for a code-text anchor (§1)>
  <line>| <up to 2 source lines best matching its label, for a document fact>
  called/used by (<n>[, <k> in tests]):
    <enclosing › caller> [<relation>] <file>:<line>  | <the call-site line>
  imported by: <files> (+n)
  calls: <labels> (+n)
```

- **Rule.** `enclosing ›` names the function or class a nested node sits in, found by trimming its
  id back to the nearest node of the same file (`…_get_request_handler_app` →
  `get_request_handler() › app()`); file nodes are never shown as enclosing.
- **Rule.** Callers and users are the incoming links with relation `calls`, `indirect_call`, `uses`,
  `references`, `inherits`, `extends`, `implements`, `mixes_in`, `embeds` or `instantiates`, each
  with its call-site line cut to 80 characters. Callers outside tests, examples and benchmarks are
  listed by file and line, at most 12, with a pointer to `explain --incoming --all` for the rest;
  the others are one line: their count and two names. Importers (`imports`, `imports_from`,
  `re_exports`, `dynamic_import`) are listed by file, at most 8. Callees are the outgoing `calls`,
  `indirect_call` and `uses` links, by label, at most 10.
- **Rule.** Source lines are read from `--root`, by default the directory holding the graph's
  `dijkstral-out/` or `graphify-out/` (else the current directory), cut to 160 characters; a file
  above 2 MiB or unreadable contributes no lines.
- **Rule (document facts).** A label's best lines contain most of its content words (stem
  equality: `babies` ~ `baby`, `friendly` ≁ `friend`), at least two of them or its only one.

## 3. Documents and the budget

- **Rule.** When the anchors are document nodes, the answer ends with a search for every
  occurrence of the question's subject — the graph summarises, and a "how many", "all of" or
  "the latest" question needs each one: `rg -n -i "<up to 4 subject stems>"`, a stem ending in
  `y` searched by its prefix (`bab` finds `baby` and `babies`).
- **Rule.** `--budget N` (default 1500) caps the answer at about N tokens (4 characters each):
  anchors that do not fit are left out and counted in a closing line. The first anchor is always
  shown.
- **Rule.** Exit 0 with an answer; exit 4 with `No node matches: <question>` when nothing anchors.

## 4. Tests

`crates/serve/src/context.rs` unit tests (name detection, stems, identifier words, label lines,
paths) and `crates/cli/tests/query_goldens.rs` `context_answers_with_source_lines` on the Python
fixture. Its effect on agents is measured by `bench/agent/` (`bench/README.md`, stage 7); the
share of a question's key facts its output already contains is the offline measure used while
tuning it, on fastapi and ripgrep, with Kubernetes held out (`bench/agent/context_eval.py`).
Meaning-based retrieval was measured as a separate prototype and not adopted
(`dijkstral-context-semantic.md`; code on branch `feature/dijkstral-context-embeddings-prototype`).
Reranking candidates with a hosted classifier (TypeSafe Jev) was measured the same way and not
adopted (`dijkstral-context-rerank.md`; code on branch `feature/dijkstral-context-rerank-prototype`).

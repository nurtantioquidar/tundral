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
- **Rule (code text).** A code question that names no symbol is answered from the code's text:
  each line of every code file (not configuration or data files: TOML, JSON, YAML, lock, INI, XML,
  Markdown, text, CSV), comment and prose lines skipped (`#`, `//`, `/*`, `*`, docstring quotes,
  and lines of six or more words without `(`, `)`, `=`, `{`, `}`, `[`, `]` or `;`, such as help
  text), is split into words — identifiers at `_` and lower-to-upper changes (`readJSONBody` →
  read, json, body), stemmed — and credited to the innermost graph node starting at or before
  it. Each node scores Σ idf(term) × (1 + ln count) over the question's subject terms (scaffolding
  and words that describe code in general removed), times the number of distinct terms, damped
  by √(1 + span/40) where its span runs to the next node of its file; it needs two distinct terms
  (or the only one). The top 3, tests last (test files, Rust `mod tests`, `test…` names), become
  anchors, each with its three best matching lines.
- **Rule.** Otherwise the anchors are `query`'s seeds for the question's terms
  (`graphify-query.md` §3.6). When every one is a document node (a fact, concept or document), up
  to 8 are kept, those whose label shares a subject word with the question: the question's terms,
  stemmed, without scaffolding words (`many`, `few`, `last`, `months`, `years`, …).

## 2. One anchor

```text
## <enclosing › label>  <file>:<line>
  <line>| <the first 6 lines of its definition, for code>
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
- **Rule.** Callers and users are the incoming links with relation `calls`, `indirect_call`,
  `uses`, `references`, `inherits`, `extends`, `implements`, `mixes_in`, `embeds` or
  `instantiates`; callers outside tests come first, then by file and line; at most 12, with a
  pointer to `explain --incoming --all` for the rest. Importers (`imports`, `imports_from`,
  `re_exports`, `dynamic_import`) are listed by file, at most 8. Callees are the outgoing
  `calls`, `indirect_call` and `uses` links, by label, at most 10.
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
tuning it, on fastapi and ripgrep, with Kubernetes held out.

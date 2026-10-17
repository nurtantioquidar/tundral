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

- **Rule.** The question's code-like names are resolved first: backticked spans, and words with
  `_`, `.`, `::`, a trailing `()` or an inner capital (`solve_dependencies`, `Override::matched`,
  `util.PatchPodStatus`). Each resolves as `explain` resolves names (`graphify-query.md` §4); an
  unknown qualified name retries with its last `.`/`::` segment, and an ambiguous one keeps its
  two best-connected candidates outside tests.
- **Rule.** Without a resolvable name, the anchors are `query`'s seeds for the question's terms
  (`graphify-query.md` §3.6).
- **Rule.** At most 4 anchors for code. When every anchor is a document node (a fact, concept or
  document), up to 8, keeping only those whose label shares a subject word with the question:
  the question's terms, stemmed, without scaffolding words (`many`, `few`, `last`, `months`,
  `years`, …).

## 2. One anchor

```text
## <enclosing › label>  <file>:<line>
  <line>| <the first 6 lines of its definition, for code>
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

`crates/serve/src/context.rs` unit tests (name detection, stems, label lines, paths) and
`crates/cli/tests/query_goldens.rs` `context_answers_with_source_lines` on the Python fixture.
Its effect on agents is measured by `bench/agent/` (`bench/README.md`, stage 7).

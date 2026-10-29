# Graphify query surface: black-box specification

This document specifies what Graphify's query surface does: the CLI commands `query`, `path`,
`explain`, `affected`, `god-nodes` and `benchmark`, the `graphify-mcp` server, the assistant hook
guard, and the files the assistant installers write. The reference is PyPI `graphifyy` 0.9.86
(`graphify-format.md` moved to 0.9.97; the goldens here read the frozen 0.9.86 fixture graphs in
`tests/fixtures/graphs-0.9.86/`). Everything here was derived by running the installed
`graphify` and `graphify-mcp` binaries against the fixture goldens, the frozen corpus references in
`~/.cache/dijkstral-bench/reference/`, and small synthetic graphs written for one question each.
Graphify's source, skill files, hook scripts and prompts were not read. Installer probes ran in
throwaway directories with `HOME` pointed at a scratch directory, and only paths, sizes, hashes,
headings and JSON structure of the written files were recorded, never their prose.

Every run set `GRAPHIFY_QUERY_LOG_DISABLE=1`; the kubernetes graph also needed
`GRAPHIFY_MAX_GRAPH_BYTES=8GB` (§2.1).

Status labels, as in `graphify-format.md`:

- **Rule**: observed consistently. Reproduce it.
- **Quirk**: observed, but a defect or an accident of implementation. Dijkstral reproduces it only
  where byte parity with Graphify output is the goal; parity tests list it as an expected
  difference.
- **Open**: not resolved. The open questions are collected in §15.

Where a constant was fitted from many observations rather than read off one output, the text says
"fitted" and gives the evidence.

## Contents

1. Harness, goldens and conventions
2. Common behaviour: graph loading, the in-memory graph, side effects
3. `query`
4. Name resolution for `path` and `explain`
5. `path`
6. `explain`
7. `affected`
8. `god-nodes`
9. `benchmark`
10. MCP server
11. Hook guard
12. Install footprint
13. Latency
14. Goldens
15. Quirks summary and open questions

---

## 1. Harness, goldens and conventions

- Goldens live in `tests/fixtures/query/` (§14). They were produced from a scratch directory that
  holds copies of `tests/fixtures/graphs-0.9.86/<fixture>.clustered.graphify.json`, with
  `--graph <fixture>.clustered.graphify.json` given relative to that directory, and with
  `PYTHONHASHSEED=0`.
- **Rule.** `PYTHONHASHSEED` matters for exactly one thing: the order of `EDGE` lines in BFS `query`
  output (§3.9). Node order, seeds, headers and every other command's output were identical across
  hash seeds in every probe. Compare BFS `EDGE` lines as a multiset.
- **Rule.** Unless stated otherwise, output goes to stdout, ends with exactly one `\n`, and exit
  codes are: 0 answer, 3 ambiguous name, 4 nothing matched, 1 usage or I/O error. The exceptions are
  listed per command.

## 2. Common behaviour

### 2.1 Loading the graph

- **Rule.** `--graph <path>` selects the graph; the default is `graphify-out/graph.json` relative to
  the process cwd. `benchmark` takes the path as a positional argument instead.
- **Rule.** Errors go to stderr with exit 1:
  - missing file: `error: graph file not found: <absolute real path>`
  - a path not ending in `.json`: `error: graph file must be a .json file`
  - a file larger than the cap: two lines,
    `error: graph file <real path> is <size with _ separators> bytes, exceeds <cap with _ separators>-byte cap`
    and `(set GRAPHIFY_MAX_GRAPH_BYTES=<bytes> or GRAPHIFY_MAX_GRAPH_BYTES=<N>GB to raise the limit)`.
- **Rule.** The default cap is 536 870 912 bytes (512 MiB); the 570 MB kubernetes reference exceeds
  it. `GRAPHIFY_MAX_GRAPH_BYTES` accepts an integer byte count or `<N>GB`.
- **Quirk.** An unparseable value (`1KB`, `abc`) silently falls back to the default cap.
- **Rule.** Both the clustered shape (`links`) and the raw `--no-cluster` shape (`edges`) load.
  Missing node or edge fields print as empty strings (§3.2).
- **Quirk.** `benchmark` with no path and no `graphify-out/graph.json` dies with a Python traceback
  on stderr and exit 1.

### 2.2 The graph the commands see

These rules hold for every command in this document.

- **Rule (adjacency).** A node's neighbours are visited in **`links` array order**: for each link
  `source → target`, `target` is appended to `source`'s neighbour list and `source` to `target`'s.
  Traversal is undirected unless a command says otherwise; displayed edges keep the stored
  direction.
- **Rule (degree).** A node's degree is the **number of links incident to it** (a link is counted
  once per endpoint), i.e. in-degree plus out-degree. In Graphify's own graphs every node pair has
  at most one link (`graphify-format.md` §8.2), so this equals the undirected degree. When a pair has
  links in both directions it counts twice.
- **Quirk.** Two links between the same pair (both directions) produce one `EDGE` line in `query`
  output but two connections in `explain`. Graphify never writes such graphs.

### 2.3 Side effects

- **Rule.** `query`, `path` and `explain` create `<directory of the graph file>/cache/` and write
  `last_query_stamp` there: the Unix time as a float (`1791903064.00167`), no newline. This happens
  even when nothing matched, and with `GRAPHIFY_QUERY_LOG_DISABLE=1`. `affected`, `god-nodes`,
  `benchmark` and the MCP server write nothing.
- **Quirk.** Running these commands with `--graph tests/fixtures/<x>.json` therefore litters
  `tests/fixtures/cache/`. Run them on copies.
- **Open.** What `GRAPHIFY_QUERY_LOG_DISABLE` suppresses when unset; it was always set.

---

## 3. `query`

```text
graphify query "<question>" [--dfs] [--context C]... [--budget N] [--graph PATH]
```

### 3.1 Arguments

- **Rule.** The first argument after `query` is the question, whatever it looks like:
  `query --help` searches for "--help" (and then fails to find the default graph).
  `query` with no argument prints
  `Usage: graphify query "<question>" [--dfs] [--context C] [--budget N] [--graph path]` and
  exits 1.
- **Rule.** `--budget` defaults to 2000. A non-integer gives stderr
  `error: --budget must be an integer` and exit 1.
- **Rule.** `--context C` is repeatable; values are kept in first-seen order with duplicates removed
  and are not validated (`--context bogus` simply matches no edge). A comma is not a separator:
  `--context call,import` is the single context `call,import`.
- **Quirk.** Unknown flags (`--depth 3`, `--bogus`) and a trailing `--context` with no value are
  ignored silently. The CLI has no depth control: depth is always 2.
- **Rule.** An empty or whitespace question prints `No matching nodes found.` and exits 4.

### 3.2 Output layout

```text
Graph: <graph path> (<N> nodes) | Traversal: <BFS|DFS> depth=2 | Start: <seed labels> | [Context: <c1, c2, …> (<heuristic|explicit>) | ]<K> nodes found
<blank>
[<notice paragraph>
<blank>]
NODE <label> [src=<source_file> loc=<source_location> community=<community>]
…
EDGE <source label> --<relation> [<confidence>[ context=<context>]]--> <target label>[ at=<source_file>:<source_location>]
…
[... (truncated — <K-k> more nodes cut by ~<B>-token budget. Narrow with context_filter=['call'] or use get_node for a specific symbol)]
```

- **Rule.** `<graph path>` is the path relative to the process cwd when the graph is inside the cwd
  (`graphify-out/graph.json`, `python.clustered.graphify.json`); otherwise the absolute real path
  (symlinks resolved).
- **Rule.** `<seed labels>` is Python's `repr` of the list of seed labels: `['UserService',
  '.register()']`. A label containing `'` is quoted with `"`, as `repr` does.
- **Rule.** The `Context:` field appears only when a context filter is active (§3.4).
- **Rule.** `<K>` counts every node reached, before budget truncation.
- **Rule.** A missing node field prints as empty (`community=`); a `null` community prints
  `community=None`. A missing edge `confidence` prints `[]`; a missing `context` omits
  ` context=…`; a link without a `source_location` (missing or empty) has no ` at=…`, even with a
  `source_file` (refined in stage 3 on excalidraw's `dynamic_import` links).
- **Rule (stage 3, excalidraw).** Line breaks are removed from labels in `NODE` and `EDGE` lines
  (a multi-line destructuring label `[\n  a,\n  b,\n]` prints `[  a,  b,]`); the `Start:` repr,
  `path` and `explain` keep them.
- **Rule.** Notices (exact text, `<…>` filled in):

```text
[!] TRUNCATED: showing <k> of <K> nodes (~<B>-token budget). The answer may be among the <K-k> cut nodes — raise the token budget to <need> to get all <K> (CLI: --budget <need>, MCP: token_budget=<need>) or narrow the query (e.g. context_filter=['call'], or get_node for a specific symbol).
[i] Complete answer over budget: all <K> nodes and <E> edges shown (~<total> tokens vs the requested ~<B>-token budget). Edges are never dropped once every node fits, so this is already the full answer — raising --budget further will not shrink it. Narrow with context_filter=['call'] or use get_node for a specific symbol to reduce size instead.
```

- **Rule.** No match: stdout `No matching nodes found.`, exit 4.

### 3.3 Turning the question into terms

Fitted from a vocabulary sweep (≈ 330 words, each probed as `zzzq <word>` on a graph holding a node
labelled with the word) and checked against ≈ 600 questions on fixtures and corpus graphs (§3.13).

1. **Rule.** Lower-case the question and take the runs of `[a-z0-9_]`. Every other character,
   including `-`, `.`, `/`, `:`, `,`, `#`, `'`, `(`, `)`, `?`, `!` and whitespace, separates
   tokens; `_` does not. So `user-service` and `UserService::register` give two tokens each, and
   `UserService.register()` gives `userservice`, `register`.
2. **Rule.** Drop stopwords and short tokens. A token is short when it has fewer than 3 characters
   and no digit (`re`, `fo`, `xy`, `is` are dropped; `n1`, `v2` and `2` are kept). The stopwords of
   3 or more characters are:

   ```text
   about all and any are been being but can could did does for from had has have here how into its
   may might must not off onto shall should some that the their them there these they this those
   was were what when where which who whom whose why will with without work working works would
   ```

   Words that look like stopwords but are kept as terms include `get`, `set`, `use`, `call`,
   `show`, `find`, `explain`, `function`, `class`, `file`, `code`, `also`, `like`, `you`, `your`,
   `her`, `him`, `our`, `own`, `same`, `just`, `very`, `more`, `most`, `once`, `only`.
3. **Rule.** Split each surviving token on `_` and keep **every non-empty part**, with no length or
   stopword filter: `n_1` gives `n` and `1`; `build_from_json` gives `build`, `from`, `json`.
4. **Rule.** Remove duplicate terms, keeping first occurrence. The result is the term list `T`,
   in question order; `|T|` is the number of distinct terms.
5. **Rule.** If no term survives, repeat steps 1–4 without the stopword filter (the short-token
   rule still applies): `the` searches for `the`, `how does it work` searches for `how`, `does`,
   `work`.
6. **Open.** Non-ASCII letters: `réal` matched nothing, consistent with `é` being a separator
   (leaving `r`, `al`, both dropped) but not proven.

### 3.4 Context filter

- **Rule.** `--context` sets an explicit filter (`(explicit)` in the header) and replaces the
  heuristic entirely.
- **Rule.** Without `--context`, the question's whitespace-separated words are matched,
  lower-cased, against this table (a trailing `,` or `?` is ignored; `calls.`, `(calls)` and
  `call's` do not match). Each hit adds its context, and the header lists them in the table's
  column order, not question order: `call, import, field, parameter_type, return_type,
  generic_arg` (`(heuristic)` in the header).

  | Context | Trigger words |
  |---|---|
  | `call` | call calls called caller callers invoke invokes invoked |
  | `import` | import imports imported module modules |
  | `field` | field fields member members |
  | `parameter_type` | parameter parameters param params argument arguments |
  | `return_type` | return returns returned |
  | `generic_arg` | generic generics |

  Words that do **not** trigger anything include `calling`, `callee`, `importing`, `package`,
  `dependency`, `type`, `typed`, `decorator`, `attribute`, `export`, `extends`, `implements`,
  `inherit`, `uses`, `constructor`, `contains`.
- **Rule.** Trigger words are still search terms (§3.3).
- **Rule.** With a filter, only links whose `context` is in the filter exist for traversal,
  ordering degree (§3.8) and `EDGE` lines. Links without a `context` (`contains`, `method`, …) are
  excluded. Seed scoring and the hub threshold are not affected.

### 3.5 Scoring

Every node is scored against `T`. Let `N` be the node count, `l` the lower-cased label, `core` = `l`
minus one trailing `()`, `words(l)` = the `[a-z0-9]+` runs of `l` joined by single spaces, and
`phrase` = the terms of `T` joined by single spaces.

- **Rule (idf, fitted).** `df(t)` = number of nodes whose lower-cased label contains `t` as a
  substring (source files and ids do not count), and `idf(t) = ln(1 + N / (1 + df(t)))`. Fitted
  from 13 independent transition points (graph sizes 100–2000, top-term df 1–20); the fit is exact
  to four decimals at every point, and `ln(N/df)`, smoothed sklearn idf, BM25 idf and square-root
  forms all fail.
- **Rule (per-term tiers, fitted).** For each term `t`, the label matches at most one tier:

  | Tier | Test | Weight |
  |---|---|---|
  | exact | `core == t` (`foo`, `Foo`, `foo()`) | 1.0 |
  | prefix | `l.startswith(t)` (`foobar`, `foo_bar`, `foo.py`, `foo ()`) | 0.1 |
  | substring | `t in l` (`xfoo`, `.foo()`, `bar_foo`) | 0.001 |

  and independently a **source match** when `t` occurs in the lower-cased `source_file`, weight
  0.0005. The node id is not searched by substring.
- **Rule (coverage factor, fitted).** Let `m` be the number of terms with a label match of any
  tier. The score is

  ```text
  score = (m/|T|)^2 * Σ_{exact,prefix terms} w(t)*idf(t)
        + Σ_{substring terms} 0.001*idf(t)
        + Σ_{source-matched terms} 0.0005*idf(t)
        + phrase bonus
  ```

  Evidence: with reference nodes of known weight, a prefix-plus-substring node measured
  `0.4 + 0.001*|T|^2` and a two-substring node `0.002*|T|^2` (relative to an exact single-term
  node, whose own score carries `1/|T|^2`) for `|T|` = 3, 4, 5, 6, 8, exactly; extra substring
  occurrences of one term add nothing; unmatched terms count in `|T|` whatever their idf.
- **Rule (phrase bonus).** Let `maxidf` be the largest `idf(t)` over `T`.
  - `words(l) == phrase`, or `|T| == 1` and the node **id** equals the term: `+ PE * maxidf`.
  - otherwise `words(l)` starts with `phrase`: `+ 1.0 * maxidf` (fitted, exact at three
    `idf` ratios).
  - otherwise (phrase only inside the words, `x_aaa_bbb`): no observable bonus.

  So for `foo bar`, labels `foo_bar`, `foo bar`, `foo.bar`, `foo-bar`, `foo::bar`, `foo_bar()` are
  phrase-exact and `foo_bar_x`, `foo bar x`, `foo_barx` phrase-prefix; `foobar`, `FooBar` and
  `bar_foo` get nothing. For a one-term question, `.foo()`, `_foo`, `(foo)`, `foo ()`, `foo ( )` are
  phrase-exact.
- **Rule (bounds on PE).** `PE` is large: a phrase-prefix label (`aaa x`, 1.1 per idf) falls below
  20 % of an id-exact node, so `PE > 5.5`. No probe produced an upper bound or a closer value;
  `PE = 10` reproduces every observation. **Open:** the exact value.
- **Rule.** Nodes with score 0 are not candidates. If there are none: exit 4.

### 3.6 Choosing the seeds

1. **Rule (ranking).** Sort candidates by score descending, then label length ascending, then id
   ascending. Degree plays no part here.
2. **Rule (label dedupe).** Walk the ranking and drop any node whose lower-cased label was already
   taken (`Foo` hides a lower-ranked `foo`; `SQLModel` hides the concept `sqlmodel`).
3. **Rule (score gap, fitted).** Primary seeds are the deduplicated nodes with
   `score >= 0.2 * top score`, capped at **3**. The 0.2 ratio was fitted from 13 transition points
   with margin < 1e-4.
4. **Rule (term coverage).** Then, for each term in **alphabetical order** (not question order),
   pick that term's best node and append it to the seeds unless it is already a seed (same id, or
   same lower-cased label):
   - The candidate pool is taken from the first non-empty tier of this list, testing every node
     (score 0 included):
     1. `core == t`
     2. `id == t`
     3. `l` with trailing non-alphanumerics removed `== t` (`name!`, `message!`)
     4. `l` with `()` removed and leading and trailing non-alphanumerics removed `== t` (`.area()`,
        `_nodes()`, `.__iter__()`, `<Derived>`)
     5. `l.startswith(t)`
     6. tier-4 normalised label starts with `t` (`.card_at()`, `_budget_hint()`)
     7. `t in l`
     8. `t` in the lower-cased `source_file` (`pkg/models.py` picks `service.py` for `pkg`)
   - Within the tier, the best node is the one whose `source_file` contains `t`, then the highest
     degree, then the shortest label, then the smallest id.
   - If that best node is already a seed, nothing is added for the term (a lower node of the tier
     is not tried).
   - **Rule (relation words).** Terms in this set get no coverage step, unless every term is in it:
     `call calls called caller callers invoke invokes invoked use uses used using reference
     references referenced depend depends import imports imported export exports exported extend
     extends extended implement implements implemented`. They are still scored. Not in the set:
     `callee`, `invocation`, `usage`, `importer`, `importing`, `exporting`, `extending`,
     `implementing`, `dependency`, `returns`, `inherits`, `contains`, `defines`, `method`.
5. **Rule.** The `Start:` list is the primary seeds in rank order followed by the coverage
   additions in term order. There is no overall cap: 8 one-word terms each matching one node gave 8
   seeds.

Examples (python fixture): `user service` → `['User', 'service()', 'UserService']` (two exact
terms and a prefix+substring node above the gap); `register,save` → `['.save()', '.register()',
'.test_register()']`; `name` → three substring matches; `pkg/models.py` → `['models.py',
'service.py']` (coverage of `pkg` through source files, highest degree).

### 3.7 Traversal

- **Rule.** Depth is 2 for both modes.
- **Rule (hub threshold, fitted).** `H = max(50, P99)`, where `P99` is the 99th percentile of all
  node degrees with linear interpolation (NumPy's default). Measured: a 50-degree hub is not
  expanded in small graphs; with 1.2–1.4 % of nodes at degree 70 or 80 the threshold rose to 70 or
  80; with 0.7 % at degree 140 it stayed at 50. The node's degree is over the full graph.
- **Rule (refined during stage 3).** With a context filter, `P99` is taken over the degrees in the
  **filtered** links (every node, zeros included): kubernetes `who calls infeasible` (filter
  `call`) does not expand `syscall6()` (92 links, 90 of them calls) because the call-degree P99 is
  14, so `H` = 50; over all links P99 is 156 and it would be expanded.
- **Rule.** A node with degree ≥ `H` is reached (listed) but never expanded, **except** a seed,
  which is always expanded.
- **Rule (BFS).** Layer 0 is the seeds. For layers 0 and 1, expand every frontier node (subject to
  the hub rule); every unseen neighbour joins the next layer.
- **Rule (DFS).** An explicit stack of `(node, depth)`, initialised with the seeds so that the first
  seed is popped first. Pop; skip if visited; mark visited; if `depth < 2` and the node is not a
  non-seed hub, push `(v, depth + 1)` for every unvisited neighbour `v` in adjacency order (so the
  last neighbour is popped next). A node first reached at depth 2 is never expanded even if it is
  also adjacent to a seed, so DFS reaches fewer nodes than BFS (python `slugify`: 11 against 24).
- Validated: node sets equal Graphify's on every probe, including 866-node and 659-node results on
  fastapi and DFS on ripgrep.

### 3.8 Node order

- **Rule.** Seeds first, in `Start:` order. Then every other node sorted by: BFS distance from the
  seed set (over the context-filtered adjacency, ignoring the hub rule) ascending, then degree
  counted over the context-filtered links descending, then id ascending. DFS output uses the same
  order; it does not follow visit order.
- Validated byte for byte on fixtures and on fastapi, ripgrep, excalidraw and erpnext results of up
  to 866 nodes.

### 3.9 Edge lines

- **Rule (set).** Every link whose two endpoints are both among the reached nodes (and whose
  context passes the filter) yields one `EDGE` line in its stored direction; not only the links the
  traversal used. Python `how does UserService register users`: 44 nodes and all 87 links among
  them.
- **Rule (DFS order).** For each node in DFS visit order, its adjacency is walked in `links` order
  and every not-yet-printed link to a reached node is printed.
- **Quirk (BFS order).** BFS `EDGE` order depends on Python's string hash seed and changes between
  runs. Dijkstral may use any deterministic order (the DFS rule is a good one); tests compare BFS
  edge lines as a set. Dijkstral uses the DFS rule for both modes: reached nodes in visit order,
  each node's links in `links` order.
- **Rule (self-loops).** A link from a node to itself never prints an `EDGE` line (go and
  typescript goldens), although it counts twice in degree (§2.2).

### 3.10 Budget

- **Rule.** A line costs `len(line) + 1` characters and tokens are characters / 3.
- **Rule.** Nodes are kept as the longest prefix of the ordered node list whose `NODE` lines cost
  at most `budget` tokens (`chars/3 <= budget`, compared as a fraction), but never fewer than the
  seeds. When some node is cut, **no** `EDGE` lines are printed, the `[!] TRUNCATED` notice appears
  and the truncation footer ends the output. `<need>` = `ceil(node_chars / 3)` for all nodes.
- **Rule.** When every node fits, every edge is printed. If `floor(total_chars / 3) > budget`
  (nodes plus edges) the `[i] Complete answer over budget` notice appears with `<total>` =
  `floor(total_chars / 3)`; otherwise there is no notice. `total_chars` is the length of the
  `NODE` and `EDGE` lines joined by newlines, one less than the sum of the per-line costs: the
  typescript golden `how does UserService run` has 10 803 per-line characters and reports
  `~3600`. Python, `UserService` + `.register()`:
  nodes cost 2371 characters (`need` 791), nodes and edges 9400 (`~3133 tokens`); budget 790
  truncates, 791 shows everything with the `[i]` notice, 3133 shows everything with no notice.
- **Quirk.** A negative budget truncates nothing and prints the `[i]` notice with the negative
  number.
- **Open.** Whether `<total>` is floored or rounded (all probes had a fractional part below .5).

### 3.11 Exit codes

| Code | When |
|---|---|
| 0 | at least one seed (truncated or not) |
| 4 | no candidate: `No matching nodes found.` on stdout |
| 1 | usage, graph loading, non-integer `--budget` |

- **Open.** `--help` text says query exits 3 on an ambiguous name; no probe produced it.

### 3.12 Reference algorithm

```text
T        = terms(question)                         # §3.3
if T empty: print "No matching nodes found."; exit 4
idf[t]   = ln(1 + N / (1 + #{n : t in lower(label(n))}))
for n in nodes: score[n] = §3.5
cand     = [n | score[n] > 0] sorted by (-score, len(label), id), deduped by lower(label)
if cand empty: exit 4
seeds    = [n in cand | score[n] >= 0.2 * score[cand[0]]][:3]
for t in sorted(T) unless relation-word rule skips it:
    best = best node of the first non-empty coverage tier for t      # §3.6 step 4
    if best and best not in seeds by id or lower(label): seeds.append(best)
reached  = BFS or DFS from seeds, depth 2, hub threshold max(50, P99(degree))
order    = seeds + sort(reached - seeds, key=(dist, -filtered_degree, id))
print header, budget notice, NODE lines, EDGE lines or truncation footer   # §3.2, §3.10
```

### 3.13 Validation record

The model of §3.3–§3.8 was checked against Graphify on 27 hand-written python questions, 10 rust, 5
TypeScript, 4 mixed, 20 fastapi, 15 ripgrep, 24 graphify, 10 monet and 15 excalidraw questions, and
then on random questions built from label words (one to three words, with prefixes such as `how
does` and `who uses`) on every fixture and on the ripgrep, excalidraw, monet, erpnext, fastapi and
graphify references: about 600 questions in all. After the last refinement every `Start:` list
matched. The random sets surfaced the coverage tiers, the relation-word rule and the `must`, `may`,
`whose`, `onto`, `shall`, `might` stopwords.

---

## 4. Name resolution for `path` and `explain`

- **Rule (refined by probes during stage 3).** Matching is case-insensitive. Labels and the query
  are compared as their **words**: the lower-cased text split on every character that is not
  alphanumeric (Unicode letters and digits; `_` separates), joined by single spaces. So
  `register` finds `.register()`, `SERVICE()` finds `service()`, `store` finds `{ Store }`,
  `_store_` and `Store!`, `foo bar` finds `foo_bar`, `foo-bar` and `foo.bar`, and `naïve` stays one
  word. `foo<T>` (words `foo t`) and `foo(a)` are not exact matches for `foo`, and `a()b` is
  `a b`, not `ab`. Ids are compared lower-cased against the lower-cased query with one trailing
  `()` removed (`main()` matches the id `main`; `.pkg_x()` does not match `pkg_x`).
- **Rule.** Tiers, first non-empty wins:
  1. exact: label words `==` query words, or id `==` the id form of the query;
  2. prefix: label words start with the query words, or the **id** starts with the id form (`test`
     matches every node of `tests/test_service.py` through ids `tests_…`, including `service()`);
  3. substring: label words contain the query words (`o.b` finds `Foo.Bar`).

  One node in the winning tier resolves; several are ambiguous (exit 3); none is exit 4.
- **Open.** On an extensionless file (`benchsuite/benchsuite` in ripgrep), the exact id of the file
  node (`benchsuite_benchsuite`) resolves to the first node of that file instead (`Benchmark`);
  with a suffix (`pkg/mod.py`) the exact id wins. Dijkstral resolves the exact id in both cases.
- **Rule.** `path::symbol` restricts candidates to nodes whose `source_file` ends with `path`
  (`pkg/util.py::helper`, `util.py::helper()`); the symbol then resolves as above. An unknown path
  gives the not-found message for the whole string.
- **Rule.** Ambiguity output lists candidates in `nodes` array order, in one of two forms:

```text
Ambiguous: '<q>' matches <n> nodes in different files.
  <source_file, or the node's label when source_file is empty>
    id: <id>
  …
Retry with path::symbol using one of the paths above (e.g. <path>::<label of first match>) or the full node id.
```

```text
Ambiguous: '<q>' matches <n> nodes in <source_file>.
  <source_location>
    id: <id>
  …
Retry with the full node id.
```

- **Rule (refined).** In the different-files form only the first candidate (node order) of each
  `source_file` is listed and counted: fastapi's 393 exact `get` candidates (381 of them stubs
  with an empty `source_file`) print as `matches 11 nodes`. A candidate with an empty
  `source_file` is listed by its **id**. The same-file form lists every candidate; when that file
  is empty the header says `matches <n> nodes with no source file.` and a candidate without a
  `source_location` is listed by its label.
- **Rule.** Not found: `No node matching '<q>' found.` with `<q>` echoed as typed.
- **Quirk.** `main()` is ambiguous in the python fixture between `main()` (id `main_main`) and the
  file node whose id is `main`, because the normalised query `main` equals that id.

## 5. `path`

```text
graphify path "<A>" "<B>" [--graph PATH] [--directed|--undirected]
```

- **Rule.** Missing names print
  `Usage: graphify path "<source>" "<target>" [--graph path] [--directed|--undirected]`, exit 1.
- **Quirk.** Arguments are positional: `path A --graph g.json` takes `--graph` as `B` and falls
  back to the default graph.
- **Rule (streams).** Ambiguity, not-found and same-node messages go to **stderr**; the path and the
  no-path messages go to stdout.
- **Rule.** Both names resolving to one node: stderr `'<A>' and '<B>' both resolved to the same node
  '<id>'. Use a more specific label or the exact node ID.`, exit 3.
- **Rule (direction).** Directed is the default and the JSON `directed` flag is ignored. A link
  `u → v` is walkable from `u` to `v`; **`contains` links are also walkable backwards** (from the
  contained symbol to its file). No other relation is: 25 relations were probed one by one. With
  `--undirected` every link is walkable both ways.
- **Rule (shortest path and ties, refined).** An unweighted shortest path, chosen as a
  bidirectional breadth-first search does it: keep a forward fringe from the source and a
  reverse fringe from the target, always expand the smaller one (the forward one on a tie), mark
  each newly seen node with the node it came from, and stop at the first node the other side has
  already seen. A node's successors are its forward-walkable neighbours sorted by id followed by
  its backward-walkable (`contains`) neighbours sorted by id; predecessors likewise; undirected
  neighbours are sorted by id. This reproduces all 630 probed paths (236 on the rust fixture, 234
  on python, 160 on fastapi, directed and undirected, found and not found) byte for byte,
  including the two that a "smallest id at the earliest hop" rule gets wrong (rust `main()` to
  `src_model_circle` goes through `Kind`, not `model.rs`). It does not depend on link or node
  array order.
- **Rule (display).** Each hop shows the first link from the hop's start to its end in `links`
  order (`-->`), otherwise the first link back (`<--`).
- **Rule (output).**

```text
Shortest path (<h> hops):
  <A label> --<relation> [<confidence>]--> <next label> <--<relation> [<confidence>]-- <next label> …
```

  `-->` segments follow a link forwards and `<--…--` segments backwards. `hops` is never singular.
  A link without `confidence` prints `--<relation>-->` with no brackets.
- **Quirk (exit codes).** No path exits **0**: `No directed path found between '<A>' and '<B>'.
  Re-run with --undirected to search ignoring edge direction.` (directed) or `No path found between
  '<A>' and '<B>'.` (undirected), echoing the inputs as typed.
- **Rule.** There is no hop limit on the CLI (MCP has `max_hops`, §10.6).

## 6. `explain`

```text
graphify explain "<X>" [--all] [--graph PATH]
```

- **Rule.** Usage error as for `path`
  (`Usage: graphify explain "<node>" [--all] [--graph path]`, exit 1). Ambiguity (exit 3) and
  not-found (exit 4) messages go to **stdout**, unlike `path`.
- **Rule (layout).**

```text
Node: <label>
  ID:        <id>
  Source:    <source_file> <source_location>
  Type:      <file_type>
  Community: <community>
  Degree:    <degree>

Connections (<n>):
  --> <target label> [<relation>] [<confidence>] <source_file>:<source_location>
  <-- <source label> [<relation>] [<confidence>] <source_file>:<source_location>
```

  The `Source:` line is right-stripped (`  Source:` for a concept); the other header lines are not
  (`  Type:      ` keeps its trailing spaces when empty). Missing relation or confidence print
  `[]`; the trailing location is omitted when the link has neither `source_file` nor
  `source_location`. `<degree>` and `<n>` are the incident link count (§2.2).
- **Rule (order, validated on fixtures and fastapi).** Outgoing links in `links` order, then
  incoming links in `links` order, then a stable sort by the neighbour's degree, descending.
- **Rule (top 20).** Without `--all` only the first 20 connections print, followed by:

```text
  ... and <n-20> more (--all lists all <n>)
  Grouped by file:
    <--|--> <edge source_file or "(unknown file)">: <k> connection[s]
    …
    ... and <g-20> more files
```

  Groups cover only the hidden connections, keyed by direction and the **link's** `source_file`
  (not the neighbour's), sorted by count descending, then incoming (`<--`) before outgoing
  (`-->`), then path ascending (probed on fastapi), at most 20 shown. `connection` is singular
  for 1. With 20 or fewer connections, or with `--all`, there is no footer.
- **Rule.** A node without connections prints only the header block: no blank line and no
  `Connections (0):`.
- dijkstral extension: `--incoming` keeps only the links into the node (its callers and users,
  the `<--` lines) and `--outgoing` only the links out of it; the count, the cut at 20 and the
  file summary then cover those links alone. Without either flag the output is Graphify's.
- dijkstral extension: several names (`explain A B C`) explain each in turn, separated by a blank
  line, an ambiguous or unknown name printing its usual message in its place; exit 0 when any
  name resolved, else the first failure's code. One name behaves exactly as Graphify's.


## 7. `affected`

```text
graphify affected "<X>" [--relation R]... [--depth N] [--graph PATH]
```

- **Rule (resolution, its own).** Node id exact, else label exact case-insensitively, else a
  **unique** case-insensitive label substring. No `()` stripping, no `path::symbol`, no prefix
  tier. Otherwise stdout `No unique node match for <X>` with **exit 0** (**Quirk**), whether the
  name is ambiguous or unknown.
  dijkstral extension: when these find no single node and the name is `path::symbol`, it
  resolves as `explain` does (§4), so the qualified name `explain`'s ambiguity message suggests
  works for `affected` too (Graphify prints `No unique node match` for it).
- **Rule (traversal).** Reverse BFS from the node over **incoming** links whose relation is in the
  set, up to `--depth` layers (default 2). Each node is listed once, with the relation and
  `source_file:source_location` of the link that first reached it, in BFS layer order; within a
  layer, frontier order, then incoming links in `links` order.
- **Rule (members and overrides, probed during stage 3).** The start layer is the node followed
  by its direct members: targets of its outgoing `contains` and `method` links, in `links` order
  (grandchildren are not included), all marked seen. When a frontier node is processed, the
  methods it **overrides** that are not yet seen are processed right after it: for each class
  that owns it through a `method` link, that class's ancestors (breadth-first over outgoing
  `inherits`, `extends` and `implements` links in `links` order, then visited in **reverse** of
  that order) contribute their `method` targets with the same label. With `A(B, D)` and `B(C)`,
  `A.run` is followed by `C.run`, `D.run`, `B.run`. Overridden methods are not marked seen, so one
  can be listed when it calls another (erpnext `StockEntry`: `StockController.validate` calls
  `AccountsController.validate` and is listed), and one already listed is skipped as an override.
  `mixes_in`, `uses` and `contains` do not make an ancestor, and an ancestor's `contains`
  children are not overrides. Checked on random fastapi, erpnext and ripgrep nodes.
- **Rule.** A link without `source_file` or `source_location` prints the listed node's own value
  instead (erpnext `dynamic_import` links have no location and print `:L1`).
- **Rule.** The default relation set, printed in this order: `calls, indirect_call, references,
  imports, imports_from, dynamic_import, re_exports, inherits, extends, implements, uses, mixes_in,
  embeds, defines, requires`. `--relation` (repeatable) replaces it and is printed as given.
  `contains` and `method` are never followed by default.
- **Rule (output).**

```text
Affected nodes for <label>
Relations: <r1, r2, …>
Depth: <N>
- <label> [<relation>] <source_file>:<source_location>
…
```

  or `No affected nodes found.` in place of the list. A depth of 0 or less finds nothing. A
  non-integer depth gives stderr `error: --depth must be an integer`, exit 1.

## 8. `god-nodes`

```text
graphify god-nodes [--top N] [--json] [--graph PATH]
```

- **Rule (candidates, refined).** The report's god-node candidates (`graphify-format.md` §17.4):
  symbols whose `source_file` has a suffix (`Gemfile`, `Makefile`, `x.y/z` are excluded; `.env`
  is kept), not file nodes (label a `/`-suffix of `source_file`), not methods (label starts with
  `.` and ends with `()`; `.prop` stays), not in the noise-label list, and, in a `.json` file
  (suffix compared case-insensitively), not a generic JSON key (compared stripped and
  case-insensitively: `" name"` and `"  Dependencies "` are excluded). The mixed golden drops
  `dependencies` and `devDependencies` of `package.json` this way. Rationale and document-section
  nodes stay. Functions (label ends with `()`) with at most one link are dropped (stage 3, visible
  only past the top ten: python `--top 30` omits `wrapper()`, `bump()`, `test_run()`; checked
  with `--top 500` on every fixture and `--top 5000` on fastapi).
- **Rule (order).** Degree (§2.2) descending, stable over `nodes` array order; the first `N`
  (default 10; values below 1 count as 1; no value keeps 10). Non-integer: stderr
  `error: --top must be an integer`, exit 1.
- **Rule (text).** `God nodes (most connected):` then `  <i>. <label> - <degree> edges` (never
  singular).
- **Rule (JSON).** A JSON array of `{"id", "label", "degree"}` objects in that key order, indent 2,
  trailing newline.

## 9. `benchmark`

```text
graphify benchmark [graph.json]
```

- **Rule (layout).** A leading blank line, then:

```text
graphify token reduction benchmark
<50 × U+2500>
  Corpus:          <words> words → ~<naive> tokens (naive)
  Graph:           <N> nodes, <M> edges
  Avg query cost:  ~<avg> tokens
  Reduction:       <r>x fewer tokens per query

  Per question:
    [<r_q>x] <question>
…
<blank>
```

  Numbers use `,` thousands separators; ratios have one decimal.
- **Rule.** The questions are fixed, in this order, and a question is listed only when it matched:
  `how does authentication work`, `what is the main entry point`, `how are errors handled`,
  `what connects the data layer to the api`, `what are the core abstractions`.
- **Rule (fitted).** With only a graph as input, `words = 50 × N` and `naive = floor(words × 4/3)`.
  Per-question cost is `floor(len(text) / 4)`, where `text` joins with `\n` one line per reached
  node, `NODE <label> src=<source_file> loc=<source_location>`, and one per traversal edge,
  `EDGE <u label> --<relation>--> <v label>`. The traversal is BFS to **depth 3**; its edges are the
  links from a layer to nodes first reached in the next layer (two parents of one new node both
  count; links inside a layer do not). `r_q = naive / cost`, `avg` = the mean cost over listed
  questions **floored**, `r = naive / avg` with that floored value (a 318.6 mean prints `~318`
  and its ratio uses 318).
- **Rule (seeds, fitted during stage 3).** A question's terms are `query`'s (§3.3). A node is a
  candidate when its lower-cased label contains at least one term; candidates are ranked by the
  number of terms their label contains, then by **id descending**, and the first **three** are
  the seeds. Match tier, source files and degree play no part. Fitted with seed-identifying
  synthetic graphs (isolated nodes whose costs decode the chosen subset) and checked on 150
  random graphs (750 questions) and a further 120 random graphs end to end: every output equal.
- **Rule.** No question matched: `Benchmark error: No matching nodes found for sample questions.
  Build the graph first.` with exit 0.
- **Open.** Whether `words` comes from scan metadata when present.

---

## 10. MCP server

`graphify-mcp` (equivalently `python -m graphify.serve`) serves one graph over MCP. Everything
below was observed by driving the binary over stdio with a small JSON-RPC client
(`GRAPHIFY_QUERY_LOG_DISABLE=1`, `HOME` set to a scratch directory) against the python and rust
fixture goldens copied to `<scratch>/<fixture>/graphify-out/graph.json`, and against the fastapi
corpus reference. Transcripts are in `tests/fixtures/query/mcp/` (§10.11). Tool and resource
description prose is deliberately not recorded, only its length.

### 10.1 Command line and graph discovery

```text
graphify-mcp [graph_path] [--graph PATH] [--transport {stdio,http}] [--host HOST] [--port PORT]
             [--api-key API_KEY] [--path PATH] [--json-response] [--stateless]
             [--session-timeout SESSION_TIMEOUT]
```

- **Rule.** The default transport is stdio. `graph_path` (positional) and `--graph` are aliases;
  the default is `graphify-out/graph.json` resolved against the process cwd. HTTP defaults:
  host `127.0.0.1`, port `8080`, mount path `/mcp`, `--api-key` falls back to env
  `GRAPHIFY_API_KEY`, session timeout 3600 s (0 disables). The HTTP transport was not probed.
- **Rule.** The startup graph is resolved to a real path (`/tmp/...` became `/private/tmp/...`
  on macOS) and loaded when the server starts: `initialize` takes about 260 ms longer with the
  fastapi graph than with a fixture (§10.10).
- **Rule.** A missing or unparseable startup graph does **not** stop the server. `initialize`
  still succeeds, nothing is written to stdout, and every later graph-backed call fails with
  the error text from §10.8. The traceback goes to stderr when a call fails, not at startup.
- **Rule.** The graph file is re-checked on every call. Replacing `graph.json` between two
  calls makes the next call answer from the new file. Deleting it makes the next call fail with
  `graph.json not found: <path>`. No stale copy is served.
- **Rule.** The server writes no files. In particular it does not create
  `graphify-out/cache/last_query_stamp`, which CLI `query` does. Nothing was written under
  `HOME`.
- **Rule.** On stdin EOF the server exits 0.

### 10.2 Framing and handshake

- **Rule.** The transport is newline-delimited JSON-RPC 2.0, one compact JSON object per line
  (`{"jsonrpc":"2.0","id":7,"error":{...}}`). `Content-Length:` framing gets no response.
- **Rule.** `initialize` result (key order as sent):

```json
{"capabilities":{"resources":{"listChanged":false,"subscribe":false},"tools":{"listChanged":false}},
 "protocolVersion":"<negotiated>","serverInfo":{"name":"graphify","version":"0.9.86"}}
```

- **Rule.** Version negotiation: the client's `protocolVersion` is echoed when it is one of
  `2024-11-05`, `2025-03-26`, `2025-06-18` or `2025-11-25`. An unknown version such as
  `1999-01-01` gets `2025-11-25`, the latest the server supports. No `instructions` field, and
  no `prompts` or `logging` capability.
- **Rule.** `ping` returns `{}`. `prompts/list`, `resources/templates/list` and any unknown
  method return `{"code":-32601,"message":"Method not found","data":"<method>"}`.
- **Rule.** A request sent before `initialize`, such as `tools/list`, and a `tools/call` without
  `params.name` return `{"code":-32602,"message":"Invalid request parameters","data":""}`.
- **Quirk.** A line that is not valid JSON gets no response at all; JSON-RPC expects -32700. The
  server keeps running.

### 10.3 Tool list

**Rule.** `tools/list` returns 10 tools in this order. Each tool object has exactly the keys
`description`, `inputSchema` and `name`: no `title`, `annotations` or `outputSchema`. The
description lengths are given in characters.

| # | name | desc len | required |
|---|---|---|---|
| 1 | `query_graph` | 94 | `question` |
| 2 | `get_node` | 52 | none (one of `label`/`node_id`/`id` in practice) |
| 3 | `get_neighbors` | 53 | none (same) |
| 4 | `get_community` | 45 | `community_id` |
| 5 | `god_nodes` | 79 | none |
| 6 | `graph_stats` | 85 | none |
| 7 | `shortest_path` | 143 | `source`, `target` |
| 8 | `list_prs` | 213 | none |
| 9 | `get_pr_impact` | 223 | `pr_number` |
| 10 | `triage_prs` | 242 | none |

**Rule.** The `inputSchema` objects are reproduced below exactly as sent, including key order and
the schema's own `description` strings, because they are the interface. `P` stands for the
`project_path` property, which is identical in all ten tools:

```json
"project_path": {"type": "string", "description": "Absolute path to a project directory containing graphify-out/graph.json. Optional — defaults to the graph this server was started with."}
```

```json
{"query_graph": {"properties": {"question": {"type": "string", "description": "Natural language question or keyword search"}, "mode": {"type": "string", "enum": ["bfs", "dfs"], "default": "bfs", "description": "bfs=broad context, dfs=trace a specific path"}, "depth": {"type": "integer", "default": 3, "description": "Traversal depth (1-6)"}, "token_budget": {"type": "integer", "default": 2000, "description": "Max output tokens"}, "context_filter": {"type": "array", "items": {"type": "string"}, "description": "Optional explicit edge-context filter, e.g. ['call', 'field']"}, "project_path": P}, "required": ["question"], "type": "object"},
 "get_node": {"properties": {"label": {"type": "string", "description": "Node label or ID to look up"}, "node_id": {"type": "string", "description": "Alias for label (node id)"}, "project_path": P}, "type": "object"},
 "get_neighbors": {"properties": {"label": {"type": "string"}, "node_id": {"type": "string", "description": "Alias for label (node id)"}, "relation_filter": {"type": "string", "description": "Optional: filter by relation type"}, "token_budget": {"type": "integer", "default": 2000, "description": "Max output tokens"}, "project_path": P}, "type": "object"},
 "get_community": {"properties": {"community_id": {"type": "integer", "description": "Community ID (0-indexed by size)"}, "token_budget": {"type": "integer", "default": 2000, "description": "Max output tokens"}, "project_path": P}, "required": ["community_id"], "type": "object"},
 "god_nodes": {"properties": {"top_n": {"type": "integer", "default": 10}, "exclude_hubs_percentile": {"type": "number", "description": "Suppress nodes whose degree exceeds this percentile (0-100) of the degree distribution, matching cluster()'s hub exclusion"}, "project_path": P}, "type": "object"},
 "graph_stats": {"properties": {"project_path": P}, "type": "object"},
 "shortest_path": {"properties": {"source": {"type": "string", "description": "Source concept label or keyword"}, "target": {"type": "string", "description": "Target concept label or keyword"}, "max_hops": {"type": "integer", "default": 8, "description": "Maximum hops to consider"}, "undirected": {"type": "boolean", "default": false, "description": "Ignore stored edge direction when searching"}, "project_path": P}, "required": ["source", "target"], "type": "object"},
 "list_prs": {"properties": {"base": {"type": "string", "description": "Base branch to filter PRs by (auto-detected if omitted)"}, "repo": {"type": "string", "description": "GitHub repo (owner/repo). Defaults to current repo."}, "project_path": P}, "type": "object"},
 "get_pr_impact": {"properties": {"pr_number": {"type": "integer", "description": "PR number to analyse"}, "repo": {"type": "string", "description": "GitHub repo (owner/repo). Defaults to current repo."}, "project_path": P}, "required": ["pr_number"], "type": "object"},
 "triage_prs": {"properties": {"base": {"type": "string", "description": "Base branch to filter PRs by (auto-detected if omitted)"}, "repo": {"type": "string", "description": "GitHub repo (owner/repo). Defaults to current repo."}, "project_path": P}, "type": "object"}}
```

- **Quirk.** `get_neighbors.label` is the only property without a `description`.
- **Rule.** The verbatim objects are in `mcp/python.tools_list.json`, with the description prose
  replaced by `<redacted: N chars>`.

### 10.4 Tool result envelope and argument handling

- **Rule.** Every `tools/call` result is
  `{"content":[{"text":"<text>","type":"text"}],"isError":<bool>}`: one text item, no
  `structuredContent`, no `_meta`. The text never ends with a newline. The CLI prints the same
  text followed by one `\n`.
- **Quirk.** `isError` is `false` for every graph-tool failure: unknown tool, missing argument,
  missing graph, node not found, ambiguity, Python exceptions. Only the three PR tools set
  `isError: true`. The failure is visible only in the text (§10.8).
- **Rule.** Arguments are not validated against the schema. Integers are coerced with `int()`,
  so `"2"` works and `"abc"` gives `Error executing <tool>: invalid literal for int() with base
  10: 'abc'`. Unknown extra arguments are ignored. `get_node` and `get_neighbors` also accept an
  undocumented `id` key. When `label` and `node_id` are both given, `label` wins.

### 10.5 `project_path`

- **Rule.** When `project_path` is given and non-empty, the server opens
  `<project_path>/graphify-out/graph.json`. The path is resolved against the **server's cwd**
  (not the startup graph's directory) and then turned into a real path. `~` is not expanded. An
  empty string means the startup graph.
- **Rule.** A path to the `graph.json` file itself or to the `graphify-out/` directory is not
  accepted. The suffix is appended anyway, giving
  `[Errno 20] Not a directory: '…/graph.json/graphify-out/graph.json'` or
  `graph.json not found: …/graphify-out/graphify-out/graph.json`.
- **Rule.** Graphs are cached per resolved path, and the startup graph is pinned in the cache.
  Calls with `project_path` do not change the default graph that later calls use.
- **Rule.** The `query_graph` header `Graph: <path>` shows the graph path relative to the server
  cwd when the graph is under that cwd (`graphify-out/graph.json`,
  `py/graphify-out/graph.json`). Otherwise it shows the absolute real path
  (`/private/tmp/<scratch>/py/graphify-out/graph.json`). It never uses `../`.

### 10.6 Per-tool behaviour and text formats

#### `query_graph`

**Rule.** The text is CLI `query` output (§3). For equal parameters, the header, the
budget and truncation notices and all `NODE` lines are byte-identical to `graphify query`, and
the `EDGE` lines are the same multiset. The truncation notice names both spellings
(`CLI: --budget N, MCP: token_budget=N`). The differences:

| Aspect | MCP `query_graph` | CLI `query` |
|---|---|---|
| default depth | 3 | 2 |
| depth bounds | values above 6 are clamped to 6 (header `depth=6`). Values below 1 are kept: `depth=0` and `depth=-1` print in the header and return only the seeds | no flag |
| mode | `mode` string, uppercased into the header. **Quirk:** an unknown value such as `"xyz"` gives `Traversal: XYZ` and BFS behaviour | `--dfs` |
| DFS neighbour order | NetworkX `MultiDiGraph.to_undirected()` order: links enumerated source node by source node (nodes in file order, each source's targets in first-link order); a neighbour sits where its first such link falls. Fitted in stage 3 on every DFS probe of the seven fixtures and fastapi | `links` array order (§3.7) |
| `context_filter` | list of strings. **Quirk:** a bare string is iterated per character and de-duplicated (`"call"` gives `Context: c, a, l (explicit)`) | `--context` repeated |
| no match | `No matching nodes found.` (`isError:false`) | the CLI's own message and exit 4 |
| `EDGE` line order | **Quirk:** can differ from the CLI for the same query, and for some queries (for example rust `render`, depth 1) varies with `PYTHONHASHSEED` between processes; stable within one process | also hash-seed dependent |

- **Rule.** Compare `EDGE` lines as a set. `NODE` line order was stable in every probe.
- **Rule.** `question: ""` gives `No matching nodes found.` A non-string `question` gives
  `Error executing query_graph: 'int' object has no attribute 'split'`. A missing `question`
  gives `Error executing query_graph: 'question'`.
- **Rule.** `token_budget: 0` still prints one seed node plus the truncation notice. The token
  counting is CLI `query`'s.

#### `get_node`

```text
Node: <label>
  ID: <id>
  Source: <source_file> <source_location>
  Type: <file_type>
  Community: <community>
  Degree: <undirected degree>
```

- **Rule.** Every field is `"<key>: <value>"` with one space, unlike the column-aligned CLI
  `explain` header. Missing `source_file` and `source_location` print as empty strings, so a
  concept node shows `  Source:  ` (two trailing spaces). There is no connection list; use
  `get_neighbors`.
- **Rule (stage 3).** `get_node`, `get_neighbors` and `get_community` print labels with line
  breaks removed, as `NODE` lines do (§3.2); `shortest_path` keeps them.
- **Rule.** With none of `label`, `node_id` or `id`:
  `Provide a node label or id (accepted keys: label, node_id, id).`
- **Rule.** Name resolution matches CLI `explain`/`path` (§4). These
  observations on the fixtures came through MCP:
  - Matching is case-insensitive (`USERSERVICE` finds `UserService`).
  - A trailing `()` and a leading `.` are ignored (`register` finds `.register()`; `area` matches
    the three `.area()` nodes; `main` matches `main()` uniquely in rust).
  - The node id is also matched exactly. In python, `main()` is ambiguous between `main_main`
    (label `main()`) and the file node whose **id** is `main`.
  - An exact match beats a prefix match (`service` finds `service()`, not `service.py` or
    `UserService`), and a prefix match beats a substring match (`userserv` finds `UserService`,
    not `TestUserService`).
  - `circ` is ambiguous between `Circle` and the concept `circle`.
  - Free-text prefixes of rationale labels work (`A person` finds the rationale node).
  - `path::symbol` narrows by file, and the path may be a suffix: `pkg/util.py::helper()`,
    `pkg/util.py::helper` and `util.py::helper()` all resolve to `pkg_util_helper`.
- **Rule.** Ambiguity text is identical to the CLI's exit-3 output. There are two forms:

```text
Ambiguous: '<q>' matches <n> nodes in different files.
  <source_file or, when empty, the node's label>
    id: <id>
  …
Retry with path::symbol using one of the paths above (e.g. <path>::<label of first match>) or the full node id.
```

```text
Ambiguous: '<q>' matches <n> nodes in <source_file>.
  <source_location>
    id: <id>
  …
Retry with the full node id.
```

- **Quirk.** `get_node` and `get_neighbors` lowercase the echoed query in ambiguity and
  not-found messages: `Ambiguous: 'circle' …` for input `Circle`, and `No node matching
  'nosuchthing' found.` for `NoSuchThing`. `shortest_path` and the CLI echo the input unchanged.
- **Rule.** Candidates in ambiguity lists are in `nodes` array order (python `helper()` lists
  util, models, service: positions 6, 15 and 33 of the clustered golden), as in the CLI (§4).

#### `get_neighbors`

```text
Neighbors of <label>:
  --> <target label> [<relation>] [<confidence>] at=<source_file>:<source_location>
  …
  <-- <source label> [<relation>] [<confidence>] at=<source_file>:<source_location>
```

- **Rule (stage 3).** A link without a `source_location` has no ` at=…` (as in §3.2).
- **Rule.** All outgoing edges come first, then all incoming edges. Each group is in `links`
  array order of the stored graph, so direction is the stored `source`→`target`. There is no
  `context=` here and no top-20 cap; the CLI `explain` list uses a different order and format.
- **Rule.** `relation_filter` is a case-insensitive substring test on `relation` (`meth` and
  `METHOD` both select `method`). A filter that matches nothing leaves just the header line
  `Neighbors of <label>:`.
- **Rule.** Truncation, shared with `get_community`:
  - Let `text` be the full output and `chars = token_budget * 3`. If `len(text) <= chars`, the
    output is untruncated.
  - Otherwise keep `text[:chars]` with any partial last line dropped. `k` is the kept line count
    (an empty keep still counts as 1), `n` the total line count, and `need = ceil(len(text) / 3)`.
    The output is:

```text
[!] TRUNCATED: showing <k> of <n> lines (~<budget>-token budget). Raise the token budget to <need> to get all <n> (token_budget=<need>). <hint>

<kept lines>
... (truncated — <n-k> more lines cut by ~<budget>-token budget. <hint>)
```

  The hint is `Narrow with relation_filter or use get_node for a specific symbol` for
  `get_neighbors` and `Raise token_budget or use get_node for specific members` for
  `get_community`. This was checked at budget boundaries: 358 truncates and 359 does not for a
  1077-character text.
- **Quirk.** `token_budget: 0` reports `showing 1 of N lines` but the kept line is empty.

#### `get_community`

```text
Community <id> (<m> nodes):
  <label> [<source_file>]
  …
```

- **Rule.** Members are listed in `nodes` array order of `graph.json`. `community_id` is the
  stored `community` value, coerced with `int()`. An unknown id gives
  `Community <id> not found.` A missing id gives `Error executing get_community:
  'community_id'`.

#### `god_nodes`

- **Rule.** With no `exclude_hubs_percentile`, the text equals CLI `graphify god-nodes --top N`
  minus the trailing newline:

```text
God nodes (most connected):
  1. <label> - <degree> edges
```

- **Rule.** `top_n` less than 1 is treated as 1.
- **Rule (corrected in stage 3).** `exclude_hubs_percentile p` computes NumPy's
  `interpolated_inverted_cdf` percentile of the degree of **every** node (sorted degrees `d`,
  position `p·n/100 − 1` clamped to `[0, n−1]`, linear between neighbours) and drops candidates
  whose degree is strictly greater than it. Fitted on all seven fixtures and fastapi at p = 0,
  0.5, …, 100 (1,608 probes); NumPy's default linear method misses 34 of them (mixed: p 51 keeps
  only degree ≤ 1). At p=0 only the lowest-degree candidates remain.
- **Quirk.** p > 100 gives `Error executing god_nodes: list index out of range`.

#### `graph_stats`

```text
Nodes: <n>
Edges: <m>
Communities: <distinct community values>
EXTRACTED: <pct>%
INFERRED: <pct>%
AMBIGUOUS: <pct>%
```

- **Rule.** The text ends with `%\n`; it is the only tool whose text keeps a trailing newline.
  The percentages are rounded integers (python: 98/107 gives 92%).
- **Rule (stage 3).** When `graph.json` has a top-level `built_at_commit`, one more line follows:
  `Built at commit: <sha>` (and the newline). The fixtures have none; the corpus references do.

#### `shortest_path`

- **Rule.** The output equals CLI `graphify path` for the same nodes (directed by default;
  `undirected: true` is `--undirected`):

```text
Shortest path (<h> hops):
  <a> --<rel> [<conf>]--> <b> <--<rel> [<conf>]-- <c> …
```

- **Rule.** `max_hops` (default 8): a path that is found but too long gives
  `Path exceeds max_hops=<k> (<h> hops found).` The CLI has no such limit.
- **Rule.** When there is no directed path:
  `No directed path found between '<src label>' and '<tgt label>'. Retry with undirected=true to
  search ignoring edge direction.` The CLI echoes the raw inputs and says `Re-run with
  --undirected`.
- **Rule.** Ambiguous endpoints produce the ambiguity text from `get_node`, but echo the input
  in its original case.

#### PR tools (`list_prs`, `get_pr_impact`, `triage_prs`)

- **Rule.** These shell out to `gh`. Without an authenticated `gh` they return `isError: true`
  with `Error: gh CLI not found or not authenticated. Run: gh auth login` (list, triage) or
  `PR #<n> not found or gh not authenticated.` (impact). The first call took about 430 ms. Their
  output with a working `gh` is specified in `graphify-memory.md` §5.6.

### 10.7 Resources

**Rule.** `resources/list` returns six resources in this order. There are no resource templates
and no `subscribe`.

| uri | name | mimeType | desc len |
|---|---|---|---|
| `graphify://report` | Graph Report | `text/markdown` | 20 |
| `graphify://stats` | Graph Stats | `text/plain` | 51 |
| `graphify://god-nodes` | God Nodes | `text/plain` | 27 |
| `graphify://surprises` | Surprising Connections | `text/plain` | 38 |
| `graphify://audit` | Confidence Audit | `text/plain` | 43 |
| `graphify://questions` | Suggested Questions | `text/plain` | 37 |

- **Rule.** `resources/read` returns `{"contents":[{"mimeType":…,"text":…,"uri":…}]}` for the
  startup graph.
  - `stats` is the `graph_stats` text.
  - `god-nodes` is `god_nodes` with `top_n` 10.
  - `report` is the bytes of `GRAPH_REPORT.md` next to the graph. When the file is absent it is
    `GRAPH_REPORT.md not found. Run graphify extract first.`
  - `audit` is:

```text
Total edges: <m>
EXTRACTED: <n> (<pct>%)
INFERRED: <n> (<pct>%)
AMBIGUOUS: <n> (<pct>%)
```

    followed by a trailing newline.
  - `surprises` is `Surprising cross-community connections:` followed by
    `  <a> <-> <b> [<relation>]` lines.
  - `questions` is `Suggested questions:` followed by `  - <question>` lines.
- **Rule (stage 3).** Without `.graphify_analysis.json` (the only case probed), `surprises` and
  `questions` are computed from the stored graph, **not** as the report computes them: the
  communities are the stored `community` values, named `Community <id>`, and the graph is the
  directed multigraph of `links` (NetworkX `MultiDiGraph`; a node's successors in first-link
  order). Fitted on the seven fixtures and the fastapi, ripgrep, excalidraw, erpnext, graphify and
  monet references:
  - `surprises`: every link `u → v` between hub candidates (`graphify-format.md` §17.4) that are
    not `concept` or `rationale` and lie in different files, scored as in the report
    (confidence 1/2/3, +2 across top-level directories, +1 across stored communities, +1 when
    `min(deg) ≤ 2` and `max(deg) ≥ 5`, degree counting every link); stable sort by score over
    `MultiDiGraph.edges()` order; the best **ten**, printed `  <source label> <-> <target label>
    [<relation>]`. Parallel links print twice (rust `render() <-> Circle`).
  - `questions`, at most **ten**, interleaved one per kind per round (bridge, verify, isolated,
    low cohesion):
    - bridge: of the **three** hub candidates with the highest directed betweenness (NetworkX,
      `k = 100`, seed 42 above 1,000 nodes), those whose successors lie in other stored
      communities: ``Why does `<label>` connect `Community <own>` to `Community <a>`, …?``, the
      others in Python set order of insertion in successor order.
    - verify: of the five god-node candidates (§8) with the most **incoming** links (fastapi:
      `Query()` outranks `APIRouter`), those with at least two outgoing `INFERRED` links,
      ``Are the <n> inferred relationships involving `<label>` (e.g. with `<a>` and `<b>`)
      actually correct?``, `a`, `b` the first two targets in link order.
    - isolated: symbols (§17.4) that are not rationale and not functions, with at most one link
      that is not an outgoing `contains`: ``What connects `<a>`, `<b>`, `<c>` to the rest of the
      system?`` (the first three in node order).
    - low cohesion: stored communities with cohesion below 0.15 (internal links over
      `m(m−1)/2`), in order of first appearance among the nodes: ``Should `Community <id>` be
      split into smaller, more focused modules?``.
- **Rule.** An unknown URI, or a missing graph, gives JSON-RPC error
  `{"code":0,"message":"Unknown resource: <uri>"}` or `{"code":0,"message":"graph.json not
  found: <path>"}`, with the traceback on stderr. Resources ignore `project_path`.

### 10.8 Error text summary

| Situation | Text (`isError:false` unless noted) |
|---|---|
| unknown tool name | `Unknown tool: <name>` |
| missing required arg | `Error executing <tool>: '<arg>'` (Python `KeyError` text) |
| graph file absent | `Error executing <tool>: graph.json not found: <realpath>` |
| graph unparseable | `Error executing <tool>: could not load graph.json at <realpath>` |
| node not found (`get_node`, `get_neighbors`) | `No node matching '<lowercased q>' found.` |
| bad int | `Error executing <tool>: invalid literal for int() with base 10: '<v>'` |
| `tools/call` without `name` | JSON-RPC error -32602 `Invalid request parameters` |
| PR tool without `gh` | `isError:true`, see §10.6 |

- **Rule.** `shortest_path` not found: `No node matching source '<q>' found.` or
  `No node matching target '<q>' found.` The source is checked first. The CLI prints `No node
  matching '<q>' found.` and exits 4.
- **Rule.** When both endpoints resolve to one node, `shortest_path` and the CLI print the same
  text (the CLI exits 3):
  `'<src>' and '<tgt>' both resolved to the same node '<id>'. Use a more specific label or the
  exact node ID.`

### 10.9 What dijkstral must match versus may improve

- **Rule (match).** Match the tool names and order, the `inputSchema` objects, `serverInfo`
  name and version semantics, `capabilities`, the resource URIs, every text format above, the
  `project_path` resolution, the reload-on-change behaviour and the ambiguity texts. Agents
  parse these.
- **Quirk (do not reproduce).** Do not reproduce `isError:false` on failures, the per-character
  `context_filter` iteration, the unvalidated `mode`, the silence on malformed JSON, the
  lowercased echo, or the percentile-above-100 crash. Graph parity tests should list them as
  expected differences.

### 10.10 Latency (Apple M5, 12 cold server starts each)

Each start used `client → initialize → graph_stats ×2 → query_graph(depth 2) → god_nodes`.
Times are in ms, p50 / p95.

| Graph | initialize (spawn → response) | 1st call | warm `graph_stats` | warm `query_graph` d=2 | warm `god_nodes` |
|---|---|---|---|---|---|
| python fixture (59 nodes) | 419 / 430 | 0.8 / 1.0 | 0.5 / 0.5 | 0.8 / 1.0 | 0.7 / 0.8 |
| fastapi reference | 677 / 696 | 21.9 / 26.9 | 19.5 / 21.5 | 61.1 / 66.1 | 2.2 / 2.3 |

- **Rule.** Startup cost is dominated by Python import (about 420 ms) plus the eager load of the
  startup graph (about 260 ms on fastapi). `graph_stats` is recomputed on each call (about
  20 ms at fastapi size).

### 10.11 Goldens

`tests/fixtures/query/mcp/<fixture>.<kind>.<n>.json` holds 53 files. Each is
`{server, stable_across_hash_seeds, request, response}`: the server was run with cwd containing
`graphify-out/graph.json` (the fixture's clustered golden) and no arguments. A transcript marked
`stable_across_hash_seeds: false` (rust `query_graph.2`) gives identical responses only up to
`EDGE` line order. `<fixture>.initialize.json`, `.tools_list.json` and `.resources_list.json`
hold the handshake and the lists; description prose is redacted to `<redacted: N chars>`.

---

## 11. Hook guard

Observed by piping Claude Code `PreToolUse` JSON into `graphify hook-guard <mode> [--strict]` (the
command that `graphify claude install` wires, §12.1) and `graphify hook-check` (wired by `graphify
codex install`, §12.2). Probe project: the python fixture sources with
`tests/fixtures/graphs-0.9.86/python.clustered.graphify.json` copied to `graphify-out/graph.json` (and, where noted,
`python.graphify_report.md` as `graphify-out/GRAPH_REPORT.md`). Neither subcommand appears in
`graphify --help`.

### 11.1 Invocation and I/O contract

- **Rule.** Argument shape: `hook-guard search` handles `Grep` and `Bash`; `hook-guard read` handles
  `Read` and `Glob`. A tool handled by the other mode (e.g. `Read` under `search`, `Bash` under
  `read`) produces no output. A missing or unknown mode (`hook-guard`, `hook-guard bogus`) produces
  no output.
- **Rule.** `--strict` is accepted only after the mode (`read --strict`); `--strict search` behaves
  like an unknown mode (silent). Extra trailing arguments (`search extra`, `search --bogus`) are
  ignored.
- **Rule.** Exit code is **always 0**, and **stderr is always empty**, for every case probed: allow,
  nudge, deny, stale, no graph, empty stdin, non-JSON stdin, `[]`, `null`, `{}`, non-UTF-8 bytes,
  missing `tool_input`, `tool_input` that is a string, unknown `hook_event_name`. The guard never
  blocks via exit code 2.
- **Rule.** "Allow" is expressed as **empty stdout** (zero bytes), not as an `allow` decision.
- **Rule.** Non-empty stdout is one line of compact JSON (separators `,` and `:`, no spaces), raw
  UTF-8 (not `\u`-escaped; one message contains an em dash), followed by `\n`. Exactly one top-level
  key. Three shapes:

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"<text>"}}
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"<text>"}}
```

  `hookEventName` is always the literal `"PreToolUse"`, even when the input's `hook_event_name` is
  something else (input field is not read).
- **Rule.** Texts are constant strings (no paths, no file names, no graph statistics); lengths in
  characters (Python `len`):

  | Message | Key | Length | When |
  |---|---|---|---|
  | search nudge | `additionalContext` | 190 | §11.3 |
  | read nudge | `additionalContext` | 400 | §11.4 |
  | read stale | `additionalContext` | 239 | §11.5 |
  | strict deny | `permissionDecisionReason` | 421 | §11.6 |

  Content, paraphrased only: the search nudge says a graph exists and to run `graphify query` before
  grepping; the read nudge says the same for reading source and names `graphify query`, `explain` and
  `path`, and asks for the rule to be passed on to subagents; the stale message says the graph may be
  stale for this file, suggests `graphify update`, and says reading directly is fine. Dijkstral writes
  its own wording.

### 11.2 Graph detection

- **Rule.** The guard looks **only** at `<process cwd>/graphify-out/graph.json`. The stdin `cwd` field
  is ignored (field = project, process cwd = elsewhere → silent; field = elsewhere, process cwd =
  project → nudge; no `cwd` field → still works). There is **no upward search**: process cwd =
  `<proj>/pkg` → silent.
- **Rule.** No graph file → every mode is silent (no "build a graph" nudge).
- **Rule.** The graph is **never parsed**: an empty file, `{bad`, and `{"nodes":[],"links":[]}` all
  behave exactly like a valid graph, and the 570 MB kubernetes graph gives the same latency as the
  59-node fixture (§11.8). A symlinked `graphify-out` directory works.
- **Rule.** `GRAPH_REPORT.md` presence is irrelevant.
- **Quirk.** `graphify-out/graph.json` as a **directory**: `search` is silent but `read` still nudges
  and `--strict` still denies (search tests "is a file", read tests "exists").
- **Rule.** Not a git concern: a git repo with no commits behaves the same.

### 11.3 `search` mode (Grep, Bash)

- **Rule.** `Grep`: any non-empty `tool_input.pattern` → search nudge, regardless of `path` (even
  `/etc` or another project), `glob`, `type`, `output_mode`. Empty or missing pattern → silent.
- **Rule.** `Bash`: nudge iff `tool_input.command` contains a search program in command position.
  Observed (case-insensitive: `RG x`, `Grep x` nudge):

  | Nudge | Silent |
  |---|---|
  | `rg`, `grep`, `egrep`, `fgrep`, `zgrep`, `ag`, `ack`, `fd`, `find`, `ripgrep`; `git grep` (also `git -C . grep`, `git  grep`); bare `rg`, `find`, `fd`, `grep`, `git grep`; `rg --files` | `ugrep`, `rga`, `locate`, `pgrep`, `findstr`, `fd-find`, `ast-grep`, `sg`, `semgrep`, `sk`, `fzf`, `tree`, `ls -R`, `cat`, `head`, `bat`, `sed`, `awk`, `git log --grep`, `git ls-files`, `git status`, `git diff`, `npm test`, `cargo test`, `rgx`, `rg-x`, `grepx`, `mygrep` |

  Command position includes: start of string (leading whitespace ok), after `;`, `&`, `&&`, `|`, `||`
  (with or without spaces), newline, `(`, `{ `, `$(`, backtick; after the prefixes `sudo`, `env`,
  `time`, `nice`, `nohup`, `exec`, `command`, `xargs` (also `xargs -0`), and after `VAR=val`
  assignments; with a path (`/usr/bin/grep`, `./rg`) or `.exe` suffix (`rg.exe`, `find.exe`).
  Not command position: arguments (`echo grep`, `echo find`, `echo fd x`), quoted strings
  (`echo 'rg x'`, `bash -c 'rg x'`, `"rg" x`, `'grep' x`), assignments (`x=grep`, `rg='x'`),
  redirect targets (`cat f>grep`), and after `watch`, `timeout 5`, `sudo -u x`, `!`, `if`.
- **Open.** The exact tokenizer (why `xargs -0 grep` matches but `sudo -u x rg` does not). Dijkstral
  should match the table above; treat the boundary cases as unspecified.
- **Rule.** A `graphify ...` command alone is silent; `graphify query x && rg x` nudges (the `rg`
  segment matches). Searches aimed at `graphify-out/` still nudge.
- **Rule.** `search` output never depends on staleness or `--strict`; `--strict` never denies in
  `search` mode.

### 11.4 `read` mode (Read, Glob)

- **Rule.** `Read`: nudge iff `tool_input.file_path` (a) has a "code" extension, (b) resolves inside the
  process cwd, and (c) is not under `graphify-out/`. Silent otherwise. The file need not exist.
- **Rule.** Extension = the last suffix, case-insensitive (`x.PY`, `a.d.ts`, `a.test.py`,
  `a.md.txt`, `CMakeLists.txt` nudge; `a.py.bak`, `a.tar.gz` do not). The observed nudge set:

  ```text
  py ts tsx js jsx cjs rs go java kt scala c h cc cpp hpp cs rb php swift lua vue svelte astro sh ps1
  md mdx rst txt
  ```

  Observed silent (not exhaustive): `pyi pyw mts cts mjs kts cxx hh m mm r jl ex exs erl hs ml dart
  zig nim v bash zsh fish bat pl pm groovy gradle sql proto graphql gql adoc org tex json jsonc yaml
  yml toml ini cfg conf xml html htm css scss less ipynb csv log env lock tf hcl elm clj cljs fs fsx
  f90 sol move cairo`, extensionless files (`Makefile`, `Dockerfile`, `README`, `BUILD`, `.env`,
  `.bashrc`).
- **Quirk.** `mjs`, `mts`, `cts`, `pyi`, `bash`, `zsh` are silent although Graphify indexes several of
  them; `md`/`txt`/`rst` nudge although a code-only graph has no markdown nodes. Reproduce the set
  for parity; a dijkstral-native mode may align it with the indexed languages.
- **Rule.** Relative `file_path` resolves against the process cwd (`pkg/service.py`,
  `./pkg/service.py` nudge); `..` is normalised (`<proj>/../nograph/main.py` silent); absolute paths
  outside the project (`/etc/hosts`, a sibling project) are silent; empty path silent.
- **Quirk.** `~/x.py` is not expanded: it is treated as the relative path `<cwd>/~/x.py` and nudges.
- **Rule.** `node_modules/x.js` nudges (no ignore rules are consulted); `graphify-out/GRAPH_REPORT.md`
  is silent (the output directory is excluded).
- **Rule.** `Glob`: nudge iff `tool_input.pattern`'s last suffix is in the same extension set and the
  optional `tool_input.path` is not outside the project. `**/*.py`, `*.py`, `**/*.md`, `**/*.PY`,
  `**/service.py`, `pkg/service.py` nudge; `**/*`, `*`, `src/**`, `pkg/*`, `**/*.json`, `**/*.mjs`,
  `**/test_*`, `**/*.py*`, `**/*.py,**/*.json`, empty pattern are silent; `**/*.py` with
  `path: "/etc"` is silent.
- **Quirk.** Brace patterns are not expanded: `**/*.{ts,tsx}` is silent.
- **Rule.** Glob never reports staleness and never denies (not even with `--strict`).

### 11.5 Staleness (read mode only)

- **Rule.** For `Read`, the graph is **stale for that file** when the target file's mtime is newer
  than `graph.json`'s mtime, **or** when `graphify-out/needs_update` exists (any content, any mtime).
  A stale `Read` gets the 239-char `additionalContext` instead of the read nudge, and `--strict` does
  **not** deny it.
- **Rule.** Staleness is per file: another source file being newer does not affect a `Read` of an
  unchanged file; a brand-new file (newer than the graph) is stale; a non-existent file is never
  stale. A newer `GRAPH_REPORT.md` or another file under `graphify-out/` does not count.
- **Rule.** `search` mode and `Glob` ignore staleness.
- **Open.** Equal mtimes and sub-second resolution were not probed.

### 11.6 `--strict`

- **Rule.** `read --strict` on a `Read` that would nudge (§11.4) and is not stale denies **once per
  session**: the first such call for a `session_id` returns the deny shape and creates the empty
  marker file `graphify-out/cache/hook_sessions/<sid>.denied`; every later call for that session gets
  the ordinary read nudge. A different `session_id` is denied again.
- **Rule.** `<sid>` = `str(session_id)` with every character outside `[A-Za-z0-9_-]` replaced by `_`
  (`../escape` → `___escape`, `a/b` → `a_b`, `.hidden` → `_hidden`, `é-ab_C9` → `_-ab_C9`, integer
  `5` → `5`), truncated to 64 characters. The marker cannot escape the directory.
- **Rule.** Missing or empty `session_id` → never denies (read nudge instead) and writes no marker.
- **Rule.** `--strict` changes nothing for `Glob`, `Grep`, `Bash`. The marker directory is under the
  project's `graphify-out/`, never under `HOME`.

### 11.7 `hook-check` (Codex)

- **Rule.** `graphify hook-check` prints nothing and exits 0 for any stdin (valid payload, empty,
  closed), with or without a graph. It is a deliberate no-op (the Codex install message says Codex
  Desktop rejects `additionalContext` on `PreToolUse`, so guidance lives in `AGENTS.md`).

### 11.8 Latency

Apple M5, wall clock per process from Python `subprocess.run`, 5 warm-up + 50 timed sequential runs
(`GRAPHIFY_QUERY_LOG_DISABLE=1`), python fixture graph. Raw numbers in `hook_latency.json`.

| Case | p50 ms | p95 ms |
|---|---|---|
| `search`, Bash `git status` (silent) | 72.8 | 75.4 |
| `search`, Grep (nudge) | 73.3 | 83.2 |
| `read`, Read `.py` (nudge) | 73.4 | 80.7 |
| `read`, Read `.json` (silent) | 73.7 | 86.8 |
| `search`, Grep, no graph (silent) | 75.1 | 83.7 |
| `hook-check` (Codex no-op) | 74.5 | 82.5 |
| `search` Grep / `read` Read `.go`, 570 MB kubernetes graph (20 runs) | 77.0 / 75.0 | 87.1 / 82.7 |

- **Rule.** Latency is Python process start-up (~73 ms) and is independent of the decision and of
  graph size. Dijkstral's target (< 5 ms) is a native binary that stats two paths and never opens
  the graph.

## 12. Install footprint

Each probe ran in a fresh `<scratch>/<case>/proj` (`git init`, no commits) with
`HOME=<scratch>/<case>/home`; full trees were hashed before and after every step. The prose of
written instruction files and skill files was not read; only paths, sizes, sha256 prefixes, heading
lines and JSON structure are recorded.

### 12.1 `graphify claude install` (project)

- **Rule.** Writes into the **process cwd** (not the git root; works outside git). Never touches
  `HOME`. Creates or updates exactly:

  | Path | Fresh content |
  |---|---|
  | `CLAUDE.md` | the graphify section only: 974 bytes, 10 lines, sha256 `440e73d1d953…`, first line `## graphify` |
  | `.claude/settings.json` | 591 bytes, the hook JSON below |
  | `.claude/settings.json.graphify-bak` | only when `settings.json` already existed: byte copy of the previous file |

- **Rule.** Hook JSON, fresh project (indent 2, **no trailing newline**, existing key order preserved):

```json
{
  "hooks": {
    "PreToolUse": [
      {"matcher": "Bash|Grep",
       "hooks": [{"type": "command", "command": "<graphify> hook-guard search", "timeout": 10}]},
      {"matcher": "Read|Glob",
       "hooks": [{"type": "command", "command": "<graphify> hook-guard read", "timeout": 10}]}
    ]
  }
}
```

  `<graphify>` is an absolute path: `shutil.which("graphify")`-style lookup on `PATH` (unresolved:
  a symlink dir on `PATH` is kept as is); if `graphify` is not on `PATH`, the `graphify` script next
  to the running interpreter (`<venv>/bin/graphify`). `--strict` is never written by the installer.
  Key order inside a hook: `type, command, timeout`; inside an entry: `matcher, hooks`.
- **Rule.** Merge into an existing `settings.json`: all other top-level keys, other hook events
  (`PostToolUse`, `Stop`, …) and unrelated `PreToolUse` entries are preserved in place; the two
  graphify entries are appended at the end of `PreToolUse`. Before appending, every existing
  `PreToolUse` entry whose `matcher` is exactly `Bash`, `Bash|Grep` or `Read|Glob` **and** any of whose
  hook commands contains the substring `graphify` is removed (so re-installs from a different path
  replace, not duplicate).
- **Quirk.** That removal also deletes user hooks such as `{"matcher":"Bash","command":"graphify query
  x"}` or `echo graphify`. Entries with other matchers (`Read`, `Grep`, `Glob`, `Edit`, `Grep|Bash`)
  are never touched even when they run `graphify hook-guard`.
- **Rule.** Invalid `settings.json` (`{bad`, empty file, a JSON array, `{"hooks": 5}`): stderr
  `[graphify] refusing to modify .claude/settings.json: not valid JSON (fix or move it and re-run)`,
  file left untouched, **exit 1** — but `CLAUDE.md` has already been written (stdout reports it).
- **Rule.** `CLAUDE.md` section placement:
  - missing or empty file → file = section;
  - existing file without a `## graphify` heading → section appended after the existing text,
    separated by one blank line (a missing final newline is added first);
  - existing `## graphify` heading (leading/trailing whitespace tolerated, exact lower-case text,
    level 2 only) → that section is **replaced in place** with the canonical text (user edits inside it
    are discarded); stdout says "section written".
  - section already byte-identical → stdout says "already configured … (no change)", no file write
    (hash unchanged).
  - `## Graphify`, `### graphify`, `# graphify`, `## graphify notes` are not recognised; the section is
    appended (two graphify headings can result).
- **Rule.** Idempotent: a second `claude install` changes no byte in the project or `HOME`.
- **Rule.** stdout is three report lines plus a blank line and a two-line closing note; exit 0.

### 12.2 `graphify codex install` (project)

- **Rule.** Writes into the process cwd; never touches `HOME` (no `~/.codex/config.toml` change).
  Creates or updates:

  | Path | Fresh content |
  |---|---|
  | `AGENTS.md` | graphify section: 1331 bytes, 13 lines, sha256 `f1b864527bb9…`, first line `## graphify` (different text from the CLAUDE.md section) |
  | `.codex/hooks.json` | 280 bytes |
  | `.codex/hooks.json.graphify-bak` | only when `hooks.json` already existed |

```json
{
  "hooks": {
    "PreToolUse": [
      {"matcher": "Bash",
       "hooks": [{"type": "command", "command": "<graphify> hook-check"}]}
    ]
  }
}
```

  No `timeout` key. Same `<graphify>` path rule, same indent-2/no-trailing-newline serialisation,
  same merge (other events and keys preserved, entry appended), same `## graphify` section rules,
  same invalid-JSON refusal (exit 1 after `AGENTS.md` was written), same idempotence ("already
  configured … (no change)").
- **Rule.** Round trip: `codex install` then `codex uninstall` on a pre-existing `AGENTS.md` and
  `hooks.json` restores `AGENTS.md` byte-identically; `hooks.json` is re-serialised (indent 2), so it
  is equal as JSON but not as bytes.

### 12.3 `graphify claude uninstall` / `graphify codex uninstall`

- **Rule.** Section removal: from the `## graphify` line up to (not including) the next line that
  starts with `## ` (a level-2 heading), or end of file. `#`, `###` and `####` headings do not end the
  section, so a following `# H1` section is deleted with it.
- **Quirk.** Fenced code blocks are not respected: a `## graphify` line inside a ``` fence is treated
  as the heading and the rest of the file up to the next `## ` is removed.
- **Rule.** Joining: the text before the section keeps its content; the remainder starts at the next
  heading; exactly one blank line separates them (`run make\n\n## After`); if nothing precedes, the
  remainder starts at byte 0 (leading blank lines dropped). Every `## graphify` section is removed
  when there are several.
- **Rule.** If the file is empty (whitespace) after removal it is **deleted** (stdout says so).
- **Rule.** Hook removal uses the same matcher + `graphify`-substring rule as install. An emptied
  `PreToolUse` stays as `[]`: the file becomes exactly `{\n  "hooks": {\n    "PreToolUse": []\n  }\n}`
  (41 bytes); the file and `.claude/`/`.codex/` directories are never deleted, and
  `*.graphify-bak` is left behind.
- **Rule.** No section / no file → stdout "not found … nothing to do" / "No CLAUDE.md found in
  current directory - nothing to do", exit 0.
- **Rule.** `claude uninstall` also removes the global skill directory `~/.claude/skills/graphify/`
  if present (stdout `skill removed -> …/SKILL.md`), and creates a lock file (§12.5) even when there
  is nothing to remove. `codex uninstall` touches nothing in `HOME`.

### 12.4 `graphify install --platform claude` (global skill)

- **Rule.** `graphify install` with no `--platform` behaves as `--platform claude` on macOS.
- **Rule.** Touches only `HOME`, never the project:

  | Path under `HOME` | Content |
  |---|---|
  | `.claude/skills/graphify/SKILL.md` | 42 787 bytes; YAML front matter with keys `name`, `description` |
  | `.claude/skills/graphify/references/{add-watch,exports,extraction-spec,github-and-merge,hooks,query,transcribe,update}.md` | 8 reference files (1–14 KB) |
  | `.claude/skills/graphify/.graphify_version` | `0.9.86` (6 bytes, no newline) |
  | `.claude/CLAUDE.md` | created as a 3-line, 228-byte section starting with `# graphify` (level **1**); if the file exists, `\n` + section is appended |
  | `.cache/graphify/skill-locks/<h>.lock` | empty lock file (§12.5) |

  No `settings.json` hooks are written by the global install.
- **Rule.** Idempotent: a re-run rewrites nothing (stdout "already registered (no change)" for
  `CLAUDE.md`; skill files byte-identical).
- **Rule.** `graphify uninstall` (all platforms): removes `~/.claude/skills/graphify/` (and the
  `skills/` directory once empty), then runs every platform's project uninstall in the cwd
  (`CLAUDE.md` + `.claude/settings.json`, `CODEBUDDY.md`, `GEMINI.md`, Cursor rule, Antigravity rule,
  `AGENTS.md` + `.codex/hooks.json`, git hooks, merge driver), each reporting "nothing to do" when
  absent; exit 0. `--purge` also deletes `graphify-out/`.
  dijkstral's `uninstall` does the same for Claude Code and Codex and, inside a git work tree,
  runs `hook uninstall` (`graphify-git.md` §3); the other platforms and `--purge` are deferred
  (`../../DIJKSTRAL_DESIGN.md`).
- **Quirk.** Neither `uninstall` nor `claude uninstall` removes the `# graphify` section from
  `~/.claude/CLAUDE.md`.

### 12.5 Lock files

- **Rule.** Every skill install/uninstall creates (and leaves) an empty file
  `~/.cache/graphify/skill-locks/<h>.lock`, `<h>` = first 16 hex chars of sha256 of the **real path**
  of the skill directory (e.g. sha256(`/private/tmp/…/home/.claude/skills/graphify`)). `graphify
  uninstall` creates seven (one per platform skill directory it checks). Lock files are never
  removed. Dijkstral need not reproduce them.

### 12.6 Open

- **Open.** Hook command quoting when the `graphify` path contains spaces was not probed.
- **Open.** Where the hook entries go when `settings.json` has `hooks` but no `PreToolUse` key (assumed:
  key added after existing events).
- **Open.** Detection of an existing `# graphify` section in `~/.claude/CLAUDE.md` with altered text.

---

## 13. Latency

Apple M5 (10 cores, 16 GB), macOS 27.0, 2026-10-13. Cold process per run, wall clock around
`subprocess.run`, one discarded warm-up run (page cache warm), then 20 sequential timed runs; p95 is
linear-interpolated; peak RSS from one extra run under `/usr/bin/time -l`. Light fixture probes ran
concurrently, so tails may be slightly high. Raw data: `tests/fixtures/query/latency.json`.

| Repo (nodes / links) | Command | p50 ms | p95 ms | Peak RSS |
|---|---|---|---|---|
| fastapi (9 268 / 20 101) | `query "how does APIRouter add routes"` | 414 | 434 | 115 MB |
| | `path APIRouter APIRoute` | 407 | 414 | 118 MB |
| | `explain APIRouter` | 377 | 401 | 106 MB |
| | `hook-guard search` (Grep payload) | 65 | 70 | 28 MB |
| erpnext (24 633 / 61 108) | `query "how does SalesInvoice post GL entries"` | 1 023 | 1 041 | 260 MB |
| | `path SalesInvoice AccountsController` | 1 073 | 1 139 | 286 MB |
| | `explain SalesInvoice` | 1 063 | 1 468 | 244 MB |
| | `hook-guard search` | 77 | 119 | 28 MB |
| kubernetes (256 424 / 1 401 514) | `query "how does PriorityQueue requeue unschedulable pods"` | 11 448 | 16 127 | 3.13 GB |
| | `path PriorityQueue SchedulingQueue` | 12 902 | 14 338 | 3.65 GB |
| | `explain PriorityQueue` | 10 546 | 11 334 | 3.22 GB |
| | `hook-guard search` | 72 | 81 | 28 MB |

- **Rule.** `query`, `path` and `explain` cost a Python start (~70 ms) plus a full JSON load and
  graph build on every call, so latency scales with graph size whatever the question: about 0.4 s
  at 9k nodes, 1 s at 25k and 11–13 s at 256k nodes, with 3–3.7 GB peak RSS on kubernetes.
- **Rule.** `hook-guard` never opens the graph (§11.2): 65–77 ms p50 at every size, the cost of
  starting Python. 50-run fixture measurements are in §11.8.
- **Rule.** The MCP server pays the import and graph load once (initialize p50 419 ms on a fixture,
  677 ms on fastapi), then answers a depth-2 `query_graph` in about 61 ms on fastapi (§10.10).

Dijkstral's targets (DIJKSTRAL_DESIGN.md stage 3) are measured against these numbers with the same
argv.

## 14. Goldens

`tests/fixtures/query/`:

| File | Content |
|---|---|
| `<fixture>.<command>.<n>.txt` | stdout of one CLI run, byte for byte |
| `cases.json` | index: `{"tool", "how", "cases": [{"fixture", "argv", "exit", "stdout", "stderr", "edge_lines"}]}` |
| `latency.json` | §13 and §11.8 measurements, with argv and method |
| `mcp/<fixture>.<kind>.<n>.json` | MCP transcripts (§10.11) |

- **Rule (how the CLI goldens were made).** Each `argv` was run as `graphify <argv…>` from a
  directory holding copies of `tests/fixtures/graphs-0.9.86/<fixture>.clustered.graphify.json`, so the graph path
  in `argv` and in `Graph:` headers is the bare file name. Environment:
  `GRAPHIFY_QUERY_LOG_DISABLE=1 PYTHONHASHSEED=0`. Regenerating twice gave identical bytes.
- **Rule (comparing).** `stdout` must match byte for byte, `stderr` exactly, and the exit code.
  When `edge_lines` is `"set"` (BFS `query`), compare the `EDGE` lines as a multiset and everything
  else byte for byte (§3.9). Run dijkstral on copies too, since Graphify-compatible `query`, `path`
  and `explain` write `cache/last_query_stamp` next to the graph (§2.3).
- 74 cases cover all seven fixtures: `query` (BFS, DFS, explicit and heuristic context, budget
  truncation at 0 and 200, stopword fallback, no match), `path` (directed, no directed path,
  undirected, `contains` walked backwards, ambiguous, unknown, same node, `path::symbol`, ids),
  `explain` (top 20, `--all`, ambiguous in one file and across files, unknown, concept node,
  `path::symbol`), `affected` (default, `--relation`, `--depth`, non-unique), `god-nodes` (text,
  `--top`, `--json`) and `benchmark`.

## 15. Quirks summary and open questions

### 15.1 Quirks (reproduce only for byte parity)

| Where | Quirk |
|---|---|
| §2.1 | an unparseable `GRAPHIFY_MAX_GRAPH_BYTES` silently means 512 MiB; `benchmark` without a graph raises a traceback |
| §2.2 | reciprocal links print one `EDGE` line but count twice in degree and in `explain` |
| §2.3 | `query`/`path`/`explain` write `cache/last_query_stamp` next to the graph |
| §3.1 | unknown flags and `--depth` are ignored; `query --help` is a question |
| §3.9 | BFS `EDGE` order depends on the Python hash seed |
| §3.10 | a negative budget truncates nothing |
| §4 | the normalised query is also compared with ids, so `main()` collides with the file node `main` |
| §5 | `path A --graph g` treats `--graph` as `B`; `contains` is walkable backwards in directed mode; "no path" exits 0; messages split between stdout and stderr |
| §6 | `explain` prints ambiguity and not-found on stdout, `path` on stderr |
| §7 | `affected` exits 0 when the name does not resolve |
| §10 | MCP: `isError:false` on failures, per-character `context_filter` strings, unvalidated `mode`, silence on malformed JSON, lower-cased echoes |
| §11–§12 | hook-guard extension set (`md`/`txt` nudge, `mjs`/`pyi` do not), `~` not expanded, brace globs, installer deletes user hooks that mention graphify, global `CLAUDE.md` section never removed |

### 15.2 Open questions

1. `query` score constant `PE` (phrase-exact or id-exact bonus): only `PE > 5.5` is known; 10 fits
   every probe (§3.5).
2. Phrase-substring bonus (`x_aaa_bbb` for `aaa bbb`): not observable; treated as 0.
3. Rounding of the `[i]` notice's token total, and whether `query` can ever exit 3 (§3.10, §3.11).
4. Non-ASCII tokenisation (§3.3).
5. Resolved in stage 3: `explain` groups with equal counts list incoming first (§6).
6. `benchmark`: seeds and `avg` resolved in stage 3 (§9); open: the source of `words` when scan
   metadata exists.
7. Path selection when several links join one node pair (§5). Path ties are resolved (§5).
8. MCP resources `surprises` and `questions` with an analysis file present (without one they are
   resolved, §10.7); the HTTP transport (§10). (PR tools with a working `gh`: resolved in
   `graphify-memory.md` §5.6.)
9. Hook guard: Bash tokenizer boundary cases, equal mtimes; installer: quoting of a `graphify` path
   with spaces, `hooks` without `PreToolUse` (§11, §12).
10. What `GRAPHIFY_QUERY_LOG_DISABLE` suppresses (§2.3).
11. Name resolution of an extensionless file node's exact id (§4).

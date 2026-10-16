# Graphify exporters: black-box specification

What `graphifyy` 0.9.86 writes for `graph.html`, the Markdown wiki and GraphML (stage 4a,
§1–§8), and for Cypher and the Neo4j/FalkorDB push, the Obsidian vault, SVG, the Mermaid call-flow
page and the D3 tree (stage 4b, §9–§15), and how dijkstral's `crates/export` reproduces it.
Derived 2026-10-14/15 only by running Graphify on the clustered fixture goldens
(`tests/fixtures/<lang>.clustered.graphify.json`), on the frozen corpus references
(`~/.cache/dijkstral-bench/reference/<repo>.graph.json`) and on hand-written probe graphs
(hostile labels, colliding names, mixed attribute types), then reading the outputs (for `--push`,
the protocol traffic). Graphify's source and its page templates were not read; dijkstral's pages
are its own and only the embedded data shapes are reproduced.

Status labels as in `graphify-format.md`: **Rule** (reproduce), **Quirk** (a defect; dijkstral
does not reproduce it unless noted), **Open** (unresolved).

## Contents

1. Commands, inputs and shared rules
2. `graph.html`
3. Wiki
4. GraphML
5. Goldens and tests
6. dijkstral's build and `export` command
7. Measurements
8. Quirks and open questions
9. Loading for the export-only formats
10. Cypher and `--push` (Neo4j, FalkorDB)
11. Obsidian vault
12. SVG
13. Call-flow HTML
14. Tree
15. Stage-4b measurements

---

## 1. Commands, inputs and shared rules

| Command | Writes | Notes |
|---|---|---|
| `graphify extract <dir> [--code-only]` | no `graph.html` | `--wiki` is accepted and ignored (quirk X1) |
| `graphify update <dir>` | `graphify-out/graph.html` | regenerated when missing or stale, even with no topology change |
| `graphify cluster-only <dir> [--no-viz]` | `graph.html` unless `--no-viz` | |
| `graphify export html [--graph P] [--labels P] [--node-limit N] [--no-viz]` | `graph.html` next to the graph | `--no-viz` prints `--no-viz: skipped graph.html` |
| `graphify export wiki [--graph P] [--labels P]` | `wiki/` next to the graph | |
| `graphify export graphml [--graph P]` | `graph.graphml` next to the graph | |

- **Rule (loading).** The graph is the NetworkX graph of `graph.json` with nodes and links in file
  order (the `cluster-only` load of `graphify-format.md` §17.4). *Degree* is NetworkX degree (a
  self-loop counts twice). *Adjacency* of a node lists its neighbours in the order of the links
  that introduced them; a self-loop appears once. *`G.edges()` order*: for each node `u` in node
  order, each neighbour `v` in adjacency order not yet visited as a `u` (self-loops included), with
  `u` first.
- **Rule (community names).** Names come only from a labels file: `--labels P`, else
  `.graphify_labels.json` next to the graph (`{"0": "name", …}`; its `.sig` file is not needed).
  `community_name` fields inside `graph.json` are ignored. Without a labels file every community is
  `Community N`.
- **Rule (missing community).** A node without `community` is drawn in community 0 in
  `graph.html`, gets `community` −1 in GraphML, and belongs to no community in the wiki.
- **Rule (link direction, `_src`/`_tgt`).** The global graph (`graphify-multirepo.md` §5.2)
  writes each link's true direction as `_src`/`_tgt` and its `source`/`target` in NetworkX
  iteration order, so about half of them are reversed (`global_py_go.3`: 119 of 247 links). Every
  exporter that shows a direction draws a link from `_src` to `_tgt`: `graph.html` (`from`/`to`,
  vis and WebGL), `cypher.txt` and the push, the Obsidian canvas, the call-flow page (callers,
  callees, relation summaries, internal-link counts, diagram and overview links) and dijkstral's
  tree inspector. **GraphML is the exception**: it writes `G.edges()` order (§4) and drops
  `_`-prefixed fields, so `_src`/`_tgt` change nothing there. The wiki, the Obsidian notes and
  SVG show no direction.
  - Each end is taken separately: the drawn source is `_src` when the link has that key, else its
    `source`; the drawn target is `_tgt`, else `target`. Probe `exports/probes/direction`: a link
    `b–c` with only `_src: "c"` is drawn `c→c`; a link `c–d` carrying `_src: "e", _tgt: "a"` is
    drawn `e→a` (Quirk L1). Degree, adjacency and `G.edges()` order keep the stored ends.
  - The ends are resolved per link before a simple graph merges a repeated pair, so the merged
    edge takes the last link's drawn direction: `{e→f, _src f, _tgt e}` then a plain `e→f` gives
    `e→f`; a plain `f→g` then `{f→g, _src g, _tgt f}` gives `g→f`.
  - A link without either key keeps its own direction, so graphs without `_src`/`_tgt` (every
    build's `graph.json`) are unaffected.
  - **Quirk L2** (not reproduced). A `_src`/`_tgt` value naming no node is used as it is:
    `graph.html` gets `"from": "zz"`, `7` or `null`; `cypher.txt` writes `MATCH (a {id: 'zz'})`
    (which matches nothing) and crashes on a number or `null` (`TypeError`); the canvas and the
    call-flow page leave the link out (and a `null` pair drops it from the call-flow page's edge
    count). dijkstral uses the stored end instead. The global graph always names both ends.

  dijkstral resolves the direction once while loading (`View::dir` in `view.rs`, `NxEdge::from`/
  `to` in `nx.rs`); `TRUE_SOURCE`/`TRUE_TARGET` in `view.rs` name the keys.

## 2. `graph.html`

### 2.1 Page

- **Rule.** One self-contained HTML file loading vis-network **9.1.6** from
  `https://unpkg.com/vis-network@9.1.6/standalone/umd/vis-network.min.js`; everything else is
  inline. The data is four top-level JavaScript constants, each one line:
  `RAW_NODES`, `RAW_EDGES`, `LEGEND` and `hyperedges`, written as Python `json.dumps` (`", "` and
  `": "` separators, ASCII-escaped) with `<` written as `<` (so `</script>` in a label cannot
  close the element; `>` and `&` are left as they are).

### 2.2 `RAW_NODES` (one per node, `graph.json` order)

| Key | Value |
|---|---|
| `id` | node id |
| `label` | label with every character below U+0020 removed (`"\0ctl\a\ttab"` → `"ctltab"`, line breaks dropped, not replaced); the id when the node has no `label` |
| `color` | `{"background": c, "border": c, "highlight": {"background": "#ffffff", "border": c}}` |
| `size` | `round(10 + 30 · degree / max_degree, 1)` (Python rounding: 13.75 → 13.8, 21.25 → 21.2) |
| `font` | `{"size": 12 if degree / max_degree ≥ 0.15 else 0, "color": "#ffffff"}` (`3/20` shows; `140/933` shows; `2/18` hides) |
| `title` | same as `label` |
| `community` | the node's community (0 when absent) |
| `community_name` | the labels-file name, else `Community N` |
| `source_file`, `file_type` | as in `graph.json` (`""` when absent) |
| `degree` | NetworkX degree |

`c` is Tableau 10 indexed by `community mod 10`: `#4E79A7 #F28E2B #E15759 #76B7B2 #59A14F #EDC948
#B07AA1 #FF9DA7 #9C755F #BAB0AC` (community 12 → `#E15759`, 109 → `#BAB0AC`).

### 2.3 `RAW_EDGES`, `LEGEND`, `hyperedges`

- **Rule.** `RAW_EDGES` in `G.edges()` order, each with the link's own direction (`_src` →
  `_tgt` when present, §1):
  `{"from", "to", "label": relation ("" when absent), "title": "<relation> [<CONF>]", "dashes":
  CONF != "EXTRACTED", "width": 2 if EXTRACTED else 1, "color": {"opacity": 0.7 if EXTRACTED else
  0.35}, "confidence": CONF}`; a link without `confidence` counts as `EXTRACTED`.
- **Rule.** `LEGEND` with a labels file: `{"cid", "color", "label", "count"}` per community by id.
- **Quirk X2.** Without a labels file `LEGEND` is `[]`, so the page lists no communities although
  every node is coloured by one.
- **Rule.** `hyperedges` is `graph.json`'s top-level `hyperedges` (always `[]` for code graphs).

### 2.4 Node limit

- **Rule.** Default limit 5,000 nodes (`--node-limit N`). A graph with **more** than N nodes
  (exactly N renders normally) is replaced by an aggregated view: one node per community (`id`
  `str(cid)`, `label`/`title`/`community_name` the name, empty `source_file`/`file_type`, `degree`
  the number of linked communities, `size` §2.2's formula over `member_count`, which is added) and
  one link per linked community pair
  (`label` `"<n> cross-community edges"`, `confidence` `AGGREGATED`, dashed, width 1).
- **Rule.** If the aggregated view still has more than N nodes, Graphify raises
  `ValueError: Graph has 9417 nodes - too large for HTML viz (limit: 5000)` and writes nothing
  (Kubernetes: exit 1 after 6.4 s and 2.7 GB RSS). A `graph.json` above its size cap (Kubernetes'
  571 MB) also forces the aggregated view, with a warning.

### 2.5 dijkstral's page

dijkstral writes its own page (`crates/export/src/page.html`): a canvas, a side panel with search
(substring, exact and prefix matches first, then by degree), the selected node's `file:line`,
community, kind, degree and links grouped by relation (each clickable), and a community list with
visibility toggles. All text reaches the DOM through `textContent`.

- **Data.** One `<script id="graph-data" type="application/json">` element holding
  `{"mode", "meta", "RAW_NODES", "RAW_EDGES", "LEGEND", "hyperedges"}` with Graphify's records and
  values exactly (§2.2–2.3), plus `source_location` on each node (for `file:line`). Every `<`, `>`,
  `&`, U+2028 and U+2029 in the JSON is a `\u` escape, so no string can end the element or form
  markup; strings are UTF-8 rather than ASCII-escaped.
- **Up to the limit** (`mode: "vis"`): vis-network **9.1.9** from jsDelivr with subresource
  integrity. `LEGEND` is always filled (X2 not reproduced).
- **Above the limit** (`mode: "webgl"`): instead of aggregating or refusing, the full graph is
  drawn with sigma.js **3.0.2** and graphology **0.25.4** (jsDelivr, with integrity). The same
  records are embedded column-wise to stay loadable: `nodeFields` `["id", "label", "community",
  "source_file", "file_type", "degree", "source_location"]` and `nodes` as arrays; `relations` and
  `confidences` string tables; `edges` as `[from, to, relation, confidence]` indices in `RAW_EDGES`
  order. Colour, size, label visibility and community name are derived with §2.2's formulas;
  `webgl_columns_expand_to_the_same_records` checks the expansion equals the per-item form.
  Nodes are placed by a deterministic packing (communities on a sunflower spiral, members inside
  each disc, best connected at the centre). Above 150,000 links only the selected node's links are
  drawn.
- Offline, everything but the renderer works; the page says the renderer could not be loaded.

## 3. Wiki

### 3.1 Layout

- **Rule.** `wiki/index.md`, one article per community (all of them) and one per god node; files
  have **no trailing newline**. Graphify refuses when the graph has no communities
  (`.graphify_analysis.json is missing or empty — refusing to export wiki`, although the sidecar is
  not actually needed); dijkstral errors the same way.
- **Rule.** Communities are ordered by first appearance in node order (Python dict order); this
  order breaks size ties in the index and decides file-name claims.

### 3.2 File names

- **Rule (slug).** Drop `(` `)`; `/` and `:` → `-`; `"` `#` `%` `*` `<` `>` `?` `\` `|`, space,
  every character ≤ U+0020 and U+007F → `_`; everything else is kept (`&`, `'`, `!`, `[`, `]`,
  U+00A0, U+2028…); at most 200 characters; a result of only dots (or empty) becomes `unnamed`.
  `run()` → `run`, `Community 0` → `Community_0`, `a/b\c:d` → `a-b_c-d`.
- **Rule (uniqueness).** Claimed in order: communities, then god nodes. Names are unique ignoring
  case; `index` is reserved; the n-th claim of a name becomes `<slug>_<n>` (`Foo`, `foo`, `Foo` →
  `Foo.md`, `foo_2.md`, `Foo_3.md`; `index` → `index_2.md`; a god node `Community 0` next to the
  community → `Community_0_2.md`).

### 3.3 God nodes

- **Rule.** `graphify-format.md` §17.4's god-node ranking (hub candidates that are not generic JSON
  keys, top 10 by degree, ties in node order), additionally skipping labels ending in `()` with
  degree ≤ 1 (observed with `graphify god-nodes` too; §17.4 does not mention it because no corpus
  top 10 reaches such a node).

### 3.4 Links and text

- **Rule.** Link text escapes `[` and `]` with a backslash and nothing else. Labels elsewhere
  (headings, bold names) are written raw.
- **Rule.** In god-node articles a neighbour is linked when its **label** names an article: the
  first article claiming that label wins, communities before god nodes. So an enum member `Admin`
  links to the class `Admin`'s article, and a file node `models.py` to the community named
  `models.py`.
- **Quirk W3.** The index links each god node by label the same way, so the second of two god
  nodes labelled `Formatter` (java fixture; erpnext's `make_stock_entry()`), or a god node named
  like a community, points at the other article and its own is unreachable. dijkstral links every
  index entry to its own article.

### 3.5 `index.md`

```text
# Knowledge Graph Index
<blank>
> Auto-generated by graphify. Start here — read community articles for context, then drill into god nodes for detail.
<blank>
**<nodes> nodes · <links> edges · <communities> communities**
<blank>
---
<blank>
## Communities
(sorted by size, largest first)
<blank>
- [<name>](<file>) — <size> nodes            (size descending, ties in first-appearance order)
<blank>
## God Nodes
(most connected concepts — the load-bearing abstractions)
<blank>
- [<label>](<file>) — <degree> connections
<blank>
---
<blank>
*Generated by [graphify](https://github.com/Graphify-Labs/graphify)*
```

### 3.6 Community article

```text
# <name>
<blank>
> <size> nodes
<blank>
## Key Concepts
<blank>
- **<label>** (<degree> connections) — `<source_file>`     (" — `…`" omitted without a file)
  … members by degree descending, ties in node order, first 25, then
- *... and <n> more nodes in this community*
<blank>
## Relationships
<blank>
- [<name>](<file>) (<n> shared connections)               (first 12)
  or "- No strong cross-community connections detected"
<blank>
## Source Files                                             (section omitted when empty)
<blank>
- `<path>`                                                 (sorted, distinct, non-empty, first 20)
<blank>
## Audit Trail
<blank>
- EXTRACTED: <n> (<p>%)
- INFERRED: <n> (<p>%)
- AMBIGUOUS: <n> (<p>%)
<blank>
---
<blank>
*Part of the graphify knowledge wiki. See [index](index.md) to navigate.*
```

- **Rule.** Relationships count, for each member in node order and each neighbour in adjacency
  order, every neighbour in another community; sorted by count descending, ties in first-counted
  order.
- **Rule.** The audit trail counts each link touching a member once (a link without `confidence`
  as EXTRACTED); `p = round(100·n/total)` with Python rounding, 0 when there are no links.

### 3.7 God-node article

```text
# <label>
<blank>
> God node · <degree> connections · `<source_file>`
<blank>
**Community:** [<name>](<file>)                (omitted when the name is empty)
<blank>
## Connections by Relation
<blank>
### <relation>                                 (relations sorted)
- [<label>](<file>) `<CONF>`  or  - <label> `<CONF>`   (" `CONF`" omitted when the link has none)
  … neighbours by degree descending, ties in adjacency order, first 20, then
- *…and <n> more `<relation>` connection(s) not listed (lowest-degree first to go)*
<blank>
---
<blank>
*Part of the graphify knowledge wiki. See [index](index.md) to navigate.*
```

dijkstral writes the same text with its own name in the three product lines
(`Auto-generated by dijkstral.`, `*Generated by dijkstral*`, `Part of the dijkstral knowledge
wiki`).

## 4. GraphML

What NetworkX's `write_graphml` (ElementTree writer) produces for the loaded graph:

- **Rule (document).** `<?xml version='1.0' encoding='utf-8'?>`, then `<graphml>` with the
  GraphML namespace, `xmlns:xsi` and `xsi:schemaLocation`; two-space indentation; elements without
  children written `<x … />`; a trailing newline.
- **Rule (keys).** One `<key id="dN" for="graph|node|edge" attr.name=… attr.type=…/>` per
  (scope, name, type), numbered in creation order and **written newest first**. Creation order:
  graph attributes, then node attributes (nodes in order, each node's fields in order), then edge
  attributes. Types: string → `string`, integer → `long`, float → `double`, boolean → `boolean`;
  `null`, lists and objects → `string`. One name with several types gets several keys.
- **Rule (graph data).** `graph.json`'s `graph` object, then `hyperedges` when present, written as
  `<data>` at the end of `<graph edgedefault="undirected">`. `built_at_commit` is dropped.
- **Rule (nodes).** In order; `id` as the attribute; every other field in its `graph.json` order
  except names starting with `_` (`_origin`, `_callable`, …). In a graph with communities, a node
  without `community` gets `community` −1 appended after its other fields.
- **Rule (edges).** `G.edges()` order with `source` = the endpoint met first (not the link's
  direction, and not `_src`/`_tgt` either: a global graph's GraphML has its links as stored);
  every field except `source`, `target` and `_…`, in order.
- **Rule (values).** Strings as text (`&`, `<`, `>` escaped; quotes not); characters XML 1.0
  forbids (U+0000–U+0008, U+000B, U+000C, U+000E–U+001F) removed; `\t`, `\n`, `\r` kept raw.
  Booleans `True`/`False`. Floats in Python `repr` (`1e-05`, `1e+17`). `null` and `""` → empty
  element. Lists and objects as `json.dumps(sort_keys=True)` (ASCII-escaped).
- **Quirk G1.** A raw (`--no-cluster`) graph makes `export graphml` crash
  (`ValueError: not enough values to unpack`); dijkstral writes it, without `community`.

## 5. Goldens and tests

`tests/fixtures/exports/<lang>/` holds, for each clustered fixture golden: `graph.graphml` and
`wiki/` from `graphify export graphml|wiki --graph <lang>.clustered.graphify.json`, and
`html_payload.json`, the four constants extracted from `graphify export html`'s page (the
template itself is not stored). Graphify's output was byte-identical across reruns.

`crates/export/tests/goldens.rs`: GraphML byte-identical for all 7 fixtures; wiki byte-identical
after the product-name lines, with `ALLOWED` listing W3 (java index); every `RAW_NODES`/`RAW_EDGES`
record equal in order (plus `source_location`), `LEGEND` filled. `escaping.rs` renders hostile
labels, ids, relations, community names and titles in both views. `crates/cli/tests/export_build.rs`
covers the build integration (§6).

Global graphs (2026-10-16): `tests/fixtures/exports/global_direction/` and `global_py_go/` hold
Graphify's `cypher.txt`, `graph.graphml`, `html_payload.json`, `wiki/`, the Obsidian vault, the
call-flow page data (`callflow/page.json`) and the tree hierarchy (`tree/tree.json`) for the
multirepo goldens `global_direction.graph.json` (3 links, 2 stored reversed) and
`global_py_go.3.graph.json` (247 links, 119 reversed). `goldens.rs`, `cypher.rs` (also the push
plan), `obsidian.rs`, `callflow.rs` and `tree.rs` in `crates/export/tests/` each have a test over
them: all byte- or record-identical (wiki: W3 in `global_py_go`'s index; call-flow: open
heuristics F4, F7, F11 listed in the test). Before the §1 direction rule,
dijkstral's `cypher.txt` differed in 2 and 119 statements, `RAW_EDGES` in 2 and 119 records and
the canvas in 1 of 2 and 99 of 200 links. `exports/probes/direction.{graph.json,cypher.txt}` is
the per-field and merge probe.

Beyond the fixtures (`dijkstral export … --no-label` against Graphify on the same reference graph,
2026-10-14): GraphML byte-identical and every HTML record equal for monet, graphify, fastapi,
ripgrep, excalidraw and erpnext; wiki byte-identical except W3 (one line of erpnext's index).
The probe graphs (hostile labels, attribute types, bare nodes, name collisions, slugs, brackets)
match except W3.

## 6. dijkstral's build and `export` command

- `extract` and `update` write `dijkstral-out/graph.html` by default (Graphify's `update` and
  `cluster-only` do; its `extract` does not). `--no-viz` skips it; `--wiki` also writes
  `dijkstral-out/wiki/` (not with `--no-cluster`); `--node-limit N` sets the WebGL threshold. The
  build passes its in-memory graph in `graph.json` order with the report's community names
  (`GRAPH_REPORT.md`'s hub labels), so the page equals `dijkstral export html` on the written
  `graph.json`.
- An unchanged build is skipped only when the requested exporters' outputs exist
  (`graph.html`, `wiki/index.md`); asking for `--wiki` the first time rebuilds.
- `dijkstral export html|wiki|graphml [--graph PATH]` reads any Graphify-format `graph.json`
  (default `dijkstral-out/graph.json`, else `graphify-out/graph.json`) and writes next to it.
  Names: `--labels P`, else `.graphify_labels.json` next to the graph, else (unlike Graphify)
  each community's hub label as in `GRAPH_REPORT.md`; `--no-label` gives `Community N`, which is
  what the goldens use. `export html` takes `--node-limit` and accepts `--no-viz`.
- Not reproduced: Graphify's `cluster-only` rewriting `graph.html` (dijkstral's `cluster-only`
  writes no page), the aggregated community view and its refusal above the limit, and X2/W3.

## 7. Measurements (2026-10-14, release build, Apple Silicon)

| Graph | nodes / links | dijkstral `html` phase | page | view | Graphify `export html` |
|---|---|---|---|---|---|
| erpnext (build) | 24,669 / 63,087 | 50 ms | 6.2 MB | WebGL, all links | 1.6 s, aggregated view (2,402 communities), 1.7 MB |
| kubernetes (build) | 256,312 / 1,402,195 | 544 ms | 69 MB | WebGL, selected node's links | fails after 6.4 s (9,417 communities > 5,000), no page |

`dijkstral export html` from Kubernetes' `graph.json` (JSON load included) takes 10 s. Headless
Chrome (Playwright, SwiftShader WebGL) opened the python fixture page (vis-network), the python
fixture in WebGL mode, monet (vis-network, 3,454 nodes), erpnext and kubernetes (WebGL; 11 s to
load 256k nodes) with no console errors (only SwiftShader's `GPU stall due to ReadPixels`
performance warnings); search, selection and the detail panel worked in each.

## 8. Quirks and open questions
9. Loading for the export-only formats
10. Cypher and `--push` (Neo4j, FalkorDB)
11. Obsidian vault
12. SVG
13. Call-flow HTML
14. Tree
15. Stage-4b measurements

| Id | Kind | Summary |
|---|---|---|
| X1 | Quirk | `extract --wiki` is accepted and ignored; `extract` writes no `graph.html` |
| X2 | Quirk | `LEGEND` is empty without a labels file |
| W3 | Quirk | index god-node links resolve by label, so duplicate labels point at the wrong article |
| G1 | Quirk | `export graphml` crashes on a raw (`--no-cluster`) graph |
| L1 | Quirk (reproduced) | `_src` and `_tgt` are taken separately and need not name the link's ends: one alone gives a self-loop, other nodes redirect the drawn link (§1) |
| L2 | Quirk | a `_src`/`_tgt` naming no node (unknown id, number, `null`) is drawn to that value, crashes `cypher.txt` (number, `null`) or drops the link (canvas, call-flow); dijkstral keeps the stored end |
| O6 | Open | `graph.html` and the wiki (`View`) keep every link of a pair repeated in a simple graph, so links and degrees differ; Graphify's NetworkX load merges them (probe `direction`: 7 links against dijkstral's 9). GraphML and the §9 formats merge as Graphify does. Builds and the global graph never repeat a pair |
| O1 | Open | `graph.html` when every node has degree 0 (max degree 0): dijkstral divides by 1 |
| O2 | Open | whether the slug's `unnamed` rule is "only dots" or a strip of leading/trailing dots |
| O3 | Open | truncation and the `_n` suffix: dijkstral truncates to 200 characters before suffixing |
| O4 | Open | a god node without a community: dijkstral omits the `**Community:**` line |
| O5 | Open | control characters other than C0 (U+007F–U+009F) in `graph.html` labels: kept |

## 9. Loading for the export-only formats

`cypher.txt`, the push, the Obsidian vault, SVG, the call-flow page and the tree read the graph
through `crates/export/src/nx.rs`, which reproduces NetworkX's `node_link_graph` as Graphify
calls it, with every attribute kept raw (the stage-4a exporters read `View`). Observed through
`cypher.txt`, whose statement order exposes the graph's edge order.

- **Rule (kind).** `directed` defaults to false and `multigraph` to **true**. Graphify's own
  `graph.json` says `false` to both (a simple undirected graph); a file without the flags, such as
  an extraction fragment with `nodes` and `edges`, loads as an undirected multigraph and keeps
  every link (the python fragment: 119 statements, against 107 for its clustered graph).
  `edges` is read when `links` is absent.
- **Rule (nodes).** File order; a repeated id merges its fields into the first occurrence. A link
  endpoint that is not a node becomes a node without attributes (label = id), appended in link
  order, source before target.
- **Rule (simple graphs).** A repeated pair (unordered, or ordered when `directed`) keeps the
  position of its first link, merges the fields of every link (later keys win, missing keys keep
  earlier values) and takes the **last** link's direction: `p→q {first, INFERRED}` then
  `q→p {second}` give `q→p {second, INFERRED}`.
- **Rule (multigraphs).** Every link is an edge; a pair's parallel edges are listed together at
  the position of the pair's first link.
- **Rule (`G.edges()` order).** Undirected: for each node in order, each incident edge whose other
  end has not been visited (§1), parallel edges in link order. Directed: for each node, its
  outgoing edges grouped by successor in first-link order.
- **Rule (attributes).** A node's fields minus `id`, a link's minus `source` and `target`, in
  file order (`_src`/`_tgt` stay attributes; the drawn direction they give is resolved per link
  as in §1 and kept beside the stored ends).

## 10. Cypher and `--push` (`export neo4j`, `export falkordb`)

### 10.1 Commands

| Command | Writes or prints |
|---|---|
| `graphify export neo4j [--graph P]` | `cypher.txt` next to the graph; `cypher.txt written - import with: cypher-shell < <dir>/cypher.txt` |
| `graphify export falkordb [--graph P]` | the same file, byte for byte; `cypher.txt written (<dir>/cypher.txt) - statements are OpenCypher. FalkorDB's GRAPH.QUERY runs one statement at a time (no bulk script import), so load a graph with: graphify export falkordb --push falkordb://localhost:6379` |
| `graphify export neo4j --push URI [--user U] [--password P]` | nothing on disk; `Pushed to Neo4j: <nodes> nodes, <edges> edges` |
| `graphify export falkordb --push URI [--user U] [--password P]` | `Pushed to FalkorDB: …` |

`<dir>` is the graph's directory as given (`.` for a bare file name). Neo4j's push requires a
password (`--password`, else `NEO4J_PASSWORD`; `error: --password required for --push`); the
user defaults to `neo4j`. FalkorDB's password (`FALKORDB_PASSWORD`) is optional; it accepts
`falkordb://`, `redis://` and a bare `host:port`.

### 10.2 `cypher.txt`

The file is these lines joined by `\n` (so no trailing newline, and with no edges it ends with
the empty line):

```text
// Neo4j Cypher import - generated by /graphify
<empty>
MERGE (n:<Label> {id: '<id>', label: '<label>'});                       one per node, node order
<empty>
MATCH (a {id: '<source>'}), (b {id: '<target>'}) MERGE (a)-[:<TYPE> {confidence: '<conf>'}]->(b);
                                                                         one per edge, G.edges() order
```

- **Rule (direction).** Each edge is written with its own direction (§9; `_src` → `_tgt` when
  present, §1), not in walk order: `pkg_init → pkg_models_admin` is listed under
  `pkg_models_admin`; `global_direction`'s `Alpha → Beta`, stored `dirt::b`/`dirt::a`, is
  `MATCH (a {id: 'dirt::a'}), (b {id: 'dirt::b'})`.
- **Rule (label).** `file_type` (`unknown` when absent, null or empty), Python-`capitalize`d,
  keeping only `[A-Za-z0-9_]`; `Entity` unless the result starts with an ASCII letter. `code` →
  `Code`, `CONCEPT` → `Concept`, `code file` → `Codefile`, `c++` → `C`, `ß` → `Ss`, `ünï` → `n`,
  `élan` → `lan`, `x_y` → `X_y`, `""` → `Unknown`, `"  "`/`9abc`/`_lead`/`12` → `Entity`.
- **Rule (type).** `relation` upper-cased (Unicode: `ß` → `SS`, `ﬀ` → `FF`), keeping only
  `[A-Z0-9_]`; `RELATES_TO` unless the result starts with a letter. `imports_from` →
  `IMPORTS_FROM`, `has-part.x` → `HASPARTX`, `ünï` → `N`, `a—b` → `AB`, `calls\n` → `CALLS`;
  missing, `""`, `9`, `1starts_digit`, `_x`, `__` → `RELATES_TO`.
- **Rule (strings).** `id`, `label` (the id when absent) and `confidence` (`EXTRACTED` when
  absent; `""` stays `''`): every character below U+0020 except tab removed, then `\` → `\\` and
  `'` → `\'`. Double quotes, U+2028, non-ASCII and `}` are kept as they are, so a label
  `}) DETACH DELETE n //` stays a string literal. The id is escaped the same way, so `ctl\nid\x01`
  is matched as `ctlid`.
- **Rule (properties).** Only `id` and `label` on nodes and `confidence` on relationships; every
  other attribute is dropped. `MERGE` everywhere; matches are by `id` alone, without a label.
- **Quirk C1.** A non-string `label`, `file_type`, `relation` or `confidence` (a number, `null`)
  makes the export crash (`TypeError`/`AttributeError`) and writes nothing. dijkstral writes the
  value's JSON text (`null` label → the id).
- **Quirk C2.** The header names Neo4j and `/graphify` for `export falkordb` too.

dijkstral writes the same bytes after the first line, which reads
`// Neo4j Cypher import - generated by dijkstral`.

### 10.3 `--push`

Observed through a logging TCP proxy in front of Neo4j 5.26 Community (Bolt 5.8 negotiated by the
`neo4j` 6.4 driver) and FalkorDB (Redis 8.10, `falkordb` 1.8 client), decoding the messages.

- **Rule (statements).** One auto-commit statement per item, in this order:
  1. `CREATE INDEX IF NOT EXISTS FOR (n:<L>) ON (n.id)` for each distinct push label, sorted
     (FalkorDB: `CREATE INDEX FOR (n:<L>) ON (n.id)`); failures (an existing index, an invalid
     label) are ignored.
  2. `MERGE (n:<L> {id: $id}) SET n += $props` per node in node order.
  3. `MATCH (a {id: $src}), (b {id: $tgt}) MERGE (a)-[r:<T>]->(b) SET r += $props` per edge in
     `G.edges()` order, with the edge's own direction (as `cypher.txt`; for `_src`/`_tgt`
     see Open C8).
  FalkorDB receives the parameters as a `CYPHER id="…" props={`k`:v,…}` prefix to
  `GRAPH.QUERY graphify <query> --compact`; the graph key is always `graphify`.
- **Rule (push label).** As §10.2's but with no `Entity` fallback for a leading digit or
  underscore, and `Entity` (not `Unknown`) for an absent or empty `file_type`: `_lead` → `_lead`,
  `""` → `Entity`.
- **Rule (push type).** `relation` upper-cased, spaces and hyphens to `_`, then every character
  outside `[A-Z0-9_]` to `_`; `RELATED_TO` (not `RELATES_TO`) when empty or absent:
  `has-part.x` → `HAS_PART_X`, `ünï` → `_N_`, `calls\n` → `CALLS_`, `Mixed Case` → `MIXED_CASE`,
  `__` → `__`.
- **Rule (properties).** Every attribute not starting with `_` whose value is a string, integer,
  float or boolean, in file order (nodes: then `id`); `null`, lists and objects are dropped.
  Values are sent unescaped as parameters (labels keep newlines and control characters).
- **Quirk C3.** A label or type starting with a digit (`9abc`, `1starts_digit`) is sent bare and
  rejected by both servers (`Invalid input '9abc'`); the push stops with a traceback, after the
  earlier statements were committed.
- **Quirk C4.** FalkorDB: the client writes strings with `\` and `"` escaped but control
  characters raw; a NUL or CR in any value fails the statement (`Failed to parse the value of
  parameter 'props'`) and the push stops. A property key containing a backtick fails the same way.
- **Quirk C5.** An integer beyond 64 bits stops the Neo4j push (`OverflowError`); a `null`
  `relation` stops both (`AttributeError`).
- **Quirk C6.** The edge `MATCH` has no label, so every statement scans all nodes: pushing is
  quadratic in practice (erpnext, 24,669 nodes and 63,087 links: see §10.6).

### 10.4 dijkstral

- `dijkstral export neo4j|falkordb [--graph P]` writes `cypher.txt` next to the graph with the
  messages above (FalkorDB's naming `dijkstral export falkordb --push …`); `dijkstral export
  cypher [--graph P]` writes the same file and prints `wrote <dir>/cypher.txt`.
- `--push URI [--user U] [--password P]` (passwords also from `NEO4J_PASSWORD` /
  `FALKORDB_PASSWORD`; Neo4j requires one, as Graphify does) loads the same nodes, labels,
  relationship types and properties as Graphify's push (§10.3), so the database ends in the same
  state, with these differences:
  - **Batched.** `UNWIND $rows AS row MERGE (n:`L` {id: row.id}) SET n += row.props` per label
    and `UNWIND $rows AS row MATCH (a:`SL` {id: row.src}) MATCH (b:`TL` {id: row.tgt})
    MERGE (a)-[r:`T`]->(b) SET r += row.props` per (source label, type, target label), 5,000 rows
    per Neo4j statement and about 1 MB of query text per FalkorDB statement, after
    `CALL db.awaitIndexes(300)` (Neo4j). Endpoints are matched by label and id, so the `id`
    indexes are used (C6 not reproduced); a node elsewhere in the database with the same id but
    another label is not linked.
  - **Names quoted.** Labels and types are always backtick-quoted; they contain only
    `[A-Za-z0-9_]`, so a leading digit loads (C3 not reproduced).
  - **Values.** FalkorDB strings are written with `"`, `\` and every control character escaped
    (`\u000a`), so any string round-trips (C4 not reproduced); a property key holding a backtick
    or NUL is skipped. Integers beyond 64 bits are skipped; a non-string `relation` or label is
    used as JSON text (C5 not reproduced).
  - **FalkorDB graph key** `dijkstral` by default, `--name NAME` to choose (`--name graphify`
    replaces Graphify's graph).
- **Protocols.** dijkstral's own clients over plain TCP (`crates/export/src/bolt.rs`,
  `push.rs`): Bolt 5.0–5.8 or 4.2–4.4 with `HELLO`/`LOGON` basic authentication (`none` without a
  password), auto-commit `RUN` + `PULL`, `RESET` after an ignored failure, `GOODBYE`; Redis RESP2
  with `AUTH [user] password` and `GRAPH.QUERY … --compact`. `bolt://` and `neo4j://` (connected
  directly, no routing table), `falkordb://`, `redis://` and `host:port`. **TLS is not available
  in this build**: `bolt+s`, `neo4j+s`, `+ssc` and `rediss` are refused with that message.
  Nothing is sent anywhere but the URI given.

### 10.5 Tests and verification

`crates/export/tests/cypher.rs`: `cypher.txt` byte-identical (after the header) with Graphify's
for the 7 fixtures (`tests/fixtures/exports/<lang>/cypher.txt`) and 8 probe graphs
(`tests/fixtures/exports/probes/`: hostile strings, label/type naming, the four graph kinds,
no edges, no nodes); hostile strings parse back as the same literals in the same statement shape;
the push plan keeps names to `[A-Za-z0-9_]`. `tests/push.rs` drives `--push` against in-process
fake Bolt and Redis servers (handshake, `HELLO`/`LOGON`, statements, rows, `RESET` after a failed
index, refusing TLS, reporting an authentication failure).

By hand (2026-10-14, Neo4j 5.26.x Community and FalkorDB `latest` in local containers): all 15
clustered fixtures and the python extraction fragment pushed by both tools leave identical
databases (labels, every property and its order, relationship types and properties) in both
servers; `cypher.txt` is byte-identical for those 16 graphs and for erpnext and kubernetes. The
hostile probe, which Graphify cannot push (C3, C4), loads with dijkstral, and dijkstral's
hostile `cypher.txt` runs through `cypher-shell` leaving 13 nodes and 12 relationships with the
injection attempts stored as text.

### 10.6 Measurements

See §15.

### 10.7 Quirks

| Id | Kind | Summary |
|---|---|---|
| C1 | Quirk | non-string `label`/`file_type`/`relation`/`confidence` crashes the file export |
| C2 | Quirk | `export falkordb` writes the Neo4j header |
| C3 | Quirk | push sends labels/types starting with a digit bare; the server rejects them mid-push |
| C4 | Quirk | FalkorDB push fails on NUL/CR in values and backticks in keys |
| C5 | Quirk | push fails on integers beyond 64 bits and a `null` relation |
| C6 | Quirk | push matches edge endpoints without a label: one full scan per edge |
| C7 | Open | Bolt over TLS and Neo4j routing (`neo4j://` clusters) are untested in dijkstral; TLS is refused |
| C8 | Open | the push of a global graph was not observed: the reference install has neither the `neo4j` nor the `falkordb` client (`ImportError`, 2026-10-16). dijkstral pushes `_src` → `_tgt`, as `cypher.txt` (observed) does |

## 11. Obsidian vault (`export obsidian`)

Derived 2026-10-14 by running `graphify export obsidian` on the seven clustered fixtures, the
ripgrep reference graph (4,472 files), erpnext (27,038 files), Graphify's own raw extraction
fragment (`python.graphify.json`) and six hostile probe graphs (names, collisions, YAML, tie
orders, labels files, a clustered multigraph), then reading the vaults. Graphify's source was not
read.

### 11.1 Command and files

- **Rule.** `graphify export obsidian [--graph P] [--labels P] [--dir D]` loads the graph as in
  §9 (`nx.rs`; a raw fragment is a multigraph), names communities from `--labels` or
  `.graphify_labels.json` next to the graph (else `Community N`) and prints
  `Obsidian vault: <n> notes in <D>/`, `Canvas: <D>/graph.canvas`,
  `Open <D>/ as a vault in Obsidian.` (`<n>` counts the node and community notes written).
- **Quirk B1.** Without `--dir` the vault goes to `graphify-out/obsidian` under the **current
  directory**, wherever the graph is. `--dir` is relative to the current directory.
- **Rule.** Files: `<note>.md` per node, `_COMMUNITY_<note>.md` per community,
  `.obsidian/graph.json`, `graph.canvas` and `.graphify_obsidian_manifest.json`, all directly in
  the vault. No file ends with a newline except a community note without cross-community links
  (it ends with the closing fence and `\n`). A `.graphify_analysis.json` next to the graph
  changes nothing.
- **Rule (ownership).** The manifest is `{"files": [...]}` (`indent=2`, ASCII-escaped, sorted by
  code point): every note and `.obsidian/graph.json` the run wrote or found current — not
  `graph.canvas`, not itself. A target that already exists and is not in the previous manifest is
  left untouched, left out of the manifest and reported on stderr
  (`[graphify] WARNING: skipped N pre-existing file(s) graphify did not create, to avoid
  overwriting your notes: a.md, b.md. …`), including files written earlier in the same run under
  a name the file system folds together. Files of the previous manifest not written now are
  deleted (`[graphify] pruned N note(s) for nodes no longer in the graph`). `graph.canvas` is
  always overwritten, even when the user created it.
- **Quirk B2.** A label whose note name contains a character the file system rejects (U+FFFE on
  APFS) aborts the export with a traceback, leaving a partial vault.

### 11.2 Note names

- **Rule (node notes)**, from `label` (else the id): characters U+0000–U+001F and U+007F become
  spaces; `\ / : * ? " < > | [ ] # ^` are dropped (`%`, `&`, `` ` ``, `{`, `!`, `~`, `$` and
  non-ASCII are kept); surrounding whitespace is trimmed (Python `strip`); a name with no word
  character (`\w`) is `unnamed`; otherwise leading dots are stripped and replaced by `dot-`
  (`.promote()` → `dot-promote()`, `..x` → `dot-x`, `. x` → `dot- x`, `../../etc/passwd` →
  `dot-etcpasswd`; trailing dots stay: `x.` → `x..md`); a name over 200 UTF-8 bytes keeps its
  first 191 bytes (whole characters) plus `_` and the first 8 hex digits of the SHA-1 of the full
  name.
- **Rule (uniqueness).** Claimed in node order, unique under Python `lower()`: a taken name
  becomes `<name>_<k>` with the smallest free `k` (`Foo`, `foo`, `FOO`, `foo`, `foo_1` →
  `Foo`, `foo_1`, `FOO_2`, `foo_3`, `foo_1_1`; a label `unnamed` after 14 unnamed nodes →
  `unnamed_14`).
- **Rule (community notes).** The same rules on the community name, claimed separately in order
  of first appearance in node order (python: community 1 first), then prefixed `_COMMUNITY_`.
- **Quirk B3.** The two claim sets are independent, so a node labelled `_COMMUNITY_Community 0`
  takes community 0's file and the community note is skipped as "pre-existing". Names equal only
  under the file system's folding (`straße`/`STRASSE` on APFS) likewise lose the second note.

### 11.3 Node note

```text
---
source_file: "<source_file>"           (always; "" when absent)
type: "<file_type>"                    (always; "" when absent)
community: "<name>"                    ("Community None" without a community)
location: "<str(source_location)>"     (only when truthy; 12 → "12")
tags:
  - graphify/<file_type or "document">
  - graphify/<dominant confidence>
  - community/<community tag>
---
<blank>
# <label>
<blank>
## Connections                         (section omitted for an isolated node)
- [[<note>]] - `<relation>` [<confidence>]
<blank>
#graphify/<file_type or "document"> #graphify/<dominant> #community/<community tag>
```

- **Rule (quoting).** Quoted values escape `\` `"`, NUL as `\0`, tab/LF/CR as `\t \n \r`,
  U+2028/U+2029 as `\L \P`, other C0 and DEL as `\xHH` (lowercase); everything else (C1,
  U+00A0, U+FEFF, U+FFFE, astral) is raw.
- **Rule (connections).** One line per neighbour, neighbours sorted by label (code point) with
  ties in adjacency order; a self-loop is listed once. In a multigraph the line shows the first
  parallel link. `relation` defaults to `""`, `confidence` to `EXTRACTED`.
- **Rule (dominant confidence).** The most common `confidence` over the node's links (each
  parallel link counts), ties to the first met in adjacency order; `EXTRACTED` without links.
- **Rule (community tag).** Spaces → `_`, then everything but `\w`, `/` and `-` dropped; empty →
  `unnamed` (`My: "Comm" #1/x` → `My_Comm_1/x`, `../evil` → `/evil`, `a b` → `ab`).
- **Quirk B4.** The file-type and confidence tags are written raw: `graphify/code file`,
  `graphify/a: b` (a YAML mapping), a newline in the file type breaks the frontmatter, `""`
  confidence gives `graphify/`.
- **Quirk B5.** The heading, the relation code span, the confidence brackets and the community
  note's member and heading text are raw: a newline in a label or relation splits the line, a
  backtick in a relation ends its code span.
- **Quirk B6.** "Community None" is written for nodes without a community and their tag is
  `community/Community_None`, though no such community note exists.

### 11.4 Community note

```text
---
type: community
members: <n>
---
<blank>
# <name>
<blank>
**Members:** <n> nodes                    ("nodes" also for 1)
<blank>
## Members
- [[<note>]] - <file_type> - <source_file>   (members by label; empty parts omitted)
<blank>
## Live Query (requires Dataview plugin)
<blank>
```dataview
TABLE source_file, type FROM #community/<community tag>
SORT file.name ASC
```
<blank>
## Connections to other communities      (both sections only with cross-community links)
- <k> edge(s) to [[_COMMUNITY_<note>]]
<blank>
## Top bridge nodes
- [[<note>]] - degree <d>, connects to <k> community|communities
```

- **Rule (connections).** Links counted in `G.edges()` order (each parallel link counts) between
  two nodes of different communities, nodes without a community ignored; sorted by count
  descending, ties in the order the pair was first met in that walk (python community 0:
  `1 edge to 5`, `8`, `6`). All are listed.
- **Rule (bridges).** Members with a neighbour in another community, by number of other
  communities descending, then degree descending, then node order; the first 5. Degree is
  NetworkX degree.

### 11.5 `.obsidian/graph.json`

- **Rule.** `{"colorGroups": [...]}` (`indent=2`): empty without a labels file; with one, one
  group per id in the labels file in numeric order, present in the graph or not:
  `{"query": "tag:#community/<community tag>", "color": {"a": 1, "rgb": <int>}}`, the colour
  `graph.html`'s Tableau 10 entry for `id mod 10` as an integer (id 1 → 15896107).

### 11.6 `graph.canvas`

- **Rule (groups).** One group per community in id order (`id` `g<cid>`, `label` the name,
  `color` `"1"`–`"6"` cycling by position). A graph with no communities at all gets a single
  group 0 holding every node (Quirk B7: its notes still say `Community None` and no community
  note exists); nodes without a community in a clustered graph are not drawn.
- **Rule (layout).** Groups on a grid of ⌈√groups⌉ per row; a column is as wide as its widest
  group and a row as tall as its tallest, 80 apart. A group of m members has ⌈√m⌉ columns and
  ⌈m/cols⌉ rows, `width` max(600, 220·cols), `height` max(400, 100·rows + 120). Members by label
  at `(gx + 20 + 200·col, gy + 80 + 80·row)`, 180 × 60, `type` `file`, `id` `n_<node id>`,
  `file` `<note>.md`.
- **Rule (edges).** Links between drawn nodes in `G.edges()` order (parallel links each), stably
  sorted by `weight` descending (missing = 1), the first **200**; `id` `e_<source>_<target>`,
  `fromNode`, `toNode` in the link's direction (`_src` → `_tgt` when present, §1; "drawn" is
  decided on those ends), duplicates possible, `label` `"<relation> [<confidence>]"` stripped.
- **Rule.** `json.dumps(indent=2)`, ASCII-escaped, no trailing newline; an empty graph gives
  `{"nodes": [], "edges": []}`.

### 11.7 dijkstral

`crates/export/src/obsidian.rs`, `dijkstral export obsidian [--graph P] [--labels P | --no-label]
[--dir D]`.

- Byte-identical to Graphify on all seven fixtures (goldens in
  `tests/fixtures/exports/<lang>/obsidian/`, compared file for file by
  `crates/export/tests/obsidian.rs`), on ripgrep and erpnext (`--no-label`), on the raw fragment
  and on the multigraph and tie-order probes.
- Default vault: `obsidian/` next to the graph (`dijkstral-out/obsidian`); `--dir` as Graphify
  (B1 not reproduced). Names: as `export wiki` (§6): labels file, else hub labels, `--no-label`
  for `Community N`; with names, `colorGroups` lists every named community.
- Ownership and pruning as Graphify, using the same manifest file name so either tool respects
  the other's vault. Manifest entries other than plain `*.md` names and `.obsidian/graph.json`
  are ignored, so a doctored manifest cannot delete outside the vault. Unchanged notes are not
  rewritten.
- Deliberate differences, for hostile input only: B3's node/community collision gives the
  community note the next free `_k` name; tags use the community-tag rule (B4: `code_file`,
  `a_b`); control characters other than tab become spaces in headings, list text, relations and
  confidences, and a relation holding a backtick uses a double-backtick span (B5). A note name
  the file system rejects still stops the export part-way (B2), with an error naming the file
  instead of a traceback.
- Not reproduced: B1. Kept: B6, B7, B3's file-system folding case, wikilink syntax inside label
  and file-type text (shown as text, as Graphify).

Measured (release, Apple Silicon, machine shared with other builds): erpnext 27,035 notes,
116 MB, 8.6 s fresh / 13.2 s rerun, 281 MB RSS (Graphify 49.8 s, 242 MB, identical output);
kubernetes 265,841 notes, 1.2 GB, 157 s fresh and rerun, 3.6 GB RSS (file-system bound: 50,000
small files take ~11 s single-threaded, ~8 s with 8 threads, so writes stay sequential and
deterministic).

| Id | Kind | Summary |
|---|---|---|
| B1 | Quirk | default vault is `./graphify-out/obsidian` regardless of `--graph` |
| B2 | Quirk | a note name the file system rejects aborts mid-vault |
| B3 | Quirk | node and community notes claim names independently; FS case folding beyond `lower()` loses notes |
| B4 | Quirk | file-type and confidence tags are raw (spaces, `:`, newlines) |
| B5 | Quirk | headings, relations, confidences and member text are raw |
| B6 | Quirk | `Community None` / `community/Community_None` for nodes without a community |
| B7 | Quirk | an unclustered graph's canvas draws one `Community 0` group with no matching note |
| BO1 | Open | `\w` is approximated by Rust `is_alphanumeric() \|\| '_'` (differs from Python for some combining marks) |
| BO2 | Open | directed graphs: neighbours are every incident link (no directed `graph.json` observed) |
| BO3 | Open | Graphify passes a `cohesion` value to the exporter; no output showed it |

## 12. SVG (`export svg`)

Observed 2026-10-14 by running `graphify export svg` (0.9.86 with its `svg` extra: matplotlib
3.11.2, Pillow, NumPy; plus SciPy, see S1) on the seven clustered fixtures, the monet and erpnext
reference graphs and the hostile probe graphs, then reading the SVG. Graphify draws with
matplotlib, so the file is matplotlib's (glyph outlines as `<use>` paths, a `<dc:date>` timestamp,
a random clip-path id) and cannot be reproduced byte for byte; only what the drawing shows is
specified.

### 12.1 Command and file

| Command | Writes |
|---|---|
| `graphify export svg [--graph P] [--labels P]` | `graph.svg` next to the graph; prints `graph.svg written - embeds in Obsidian, Notion, GitHub READMEs` |

- **Rule (loading).** The NetworkX graph of §9 (phantom nodes for missing endpoints included).
  Community names as §1: `--labels P`, else `.graphify_labels.json` next to the graph.
- **Rule (canvas).** A 20 × 14 in figure at 72 pt/in, saved with a tight bounding box:
  `width="1432.8pt" height="1000.8pt"` for every graph size (fixtures, monet's 3,454 nodes),
  background `#1a1a2e`, no axes, no title.
- **Rule (layout).** A force-directed (spring-family) layout, deterministic across runs (reruns
  differ only in `<dc:date>` and the clip-path id). Positions are stretched to fill the canvas on
  each axis independently.
- **Rule (links).** Every link of `G.edges()`, in that order, drawn under the nodes as a straight
  line, stroke `#aaaaaa`, width 0.8 pt. A link whose `confidence` is present and not `EXTRACTED`
  (`INFERRED`, `AMBIGUOUS`, any other string) is dashed (`2.96,1.28`) at opacity 0.3; the rest,
  including links without `confidence`, are solid at opacity 0.6. A self-loop is a zero-length
  line (invisible). Lines have no arrowheads, so `_src`/`_tgt` (§1) change nothing. (On
  2026-10-16 the reference install had lost the `svg` extra: `export svg` exits 1 with
  `ImportError: matplotlib not installed` on every graph, the global ones included, before
  reading it; that is the environment, not the graph.)
- **Rule (nodes).** One filled circle per node in node order, colour Tableau 10 by
  `community mod 10` in lower case (`#4e79a7 #f28e2b #e15759 #76b7b2 #59a14f #edc948 #b07aa1
  #ff9da7 #9c755f #bab0ac`; a node without `community` takes `#4e79a7`), fill and stroke opacity
  0.9. Marker area `300 + 1200 · degree / max_degree` pt², so radius `√area / 2` (degree 1 of 18 →
  9.574 pt, the maximum → 19.365 pt). Degree is NetworkX degree.
- **Rule (labels).** Every node's label (else its id), white DejaVu Sans 7 pt, centred
  horizontally and vertically on the node, drawn above all nodes in node order, never truncated. A
  `\n` starts a new line; other control characters are dropped as missing glyphs (`\r` draws a
  box); glyphs missing from DejaVu Sans (CJK) are dropped with a warning.
- **Rule (legend).** Only with community names from a labels file: a box in the top-left corner
  (fill `#2a2a4e` at opacity 0.7, border `#cccccc`), one row per labels-file entry **sorted by
  community id**, a colour patch (`community mod 10`) and `name (members)` in white 8 pt. Members
  counts nodes whose `community` is that id (a node without `community`, though drawn in
  community 0's colour, is not counted).

### 12.2 Quirks and open questions

| Id | Kind | Summary |
|---|---|---|
| S1 | Quirk | the `svg` extra omits SciPy, which NetworkX's spring layout needs at 500+ nodes: erpnext fails with `ModuleNotFoundError: No module named 'scipy'` and writes nothing (graphs up to the fixtures' size work) |
| S2 | Quirk | no legend without a labels file, although every node is coloured by community (as X2) |
| S3 | Quirk | the legend lists every labels-file entry, including names of communities with no members (`Seven (0)`), and omits communities the file does not name |
| S1b | Quirk | with SciPy installed, monet (3,454 nodes) takes 24.8 s and 329 MB and gives an 9.0 MB SVG; erpnext (24,669 nodes) was still in the layout after 16 min and was stopped |
| S4 | Quirk | the canvas never grows: monet's 3,454 nodes and 3,454 labels share 1,432.8 × 1,000.8 pt, and an outer node's label is clipped at the edge |
| S5 | Quirk | non-`EXTRACTED` confidence is drawn dashed even when the string is not a confidence (`"it's"`) |
| S6 | Open | the exact layout: not `networkx.spring_layout(seed=42)` for the `k`/iteration values tried (best fit 78 pt RMS on the python fixture), so positions are not specified |

### 12.3 dijkstral's `export svg`

`dijkstral export svg [--graph P] [--labels P] [--no-label]` writes `graph.svg` next to the graph
(names as `export html`: labels file, else hub names; `--no-label` gives `Community N`). It keeps
every rule above except as listed, and is written with plain SVG elements (`<line>`, `<circle>`,
`<text>`), so labels stay selectable text.

- **Layout.** Up to 3,000 nodes: Fruchterman–Reingold (seed 42 SplitMix64 start in the unit
  square, k = √(1/n), 100 iterations, linear cooling from 0.1, a weak pull to the centre so
  separate components stay in frame), stretched to the canvas like Graphify's. Above: communities
  packed on a sunflower spiral, largest at the centre, members on a spiral inside each disc, best
  connected first (the same packing as `graph.html`'s WebGL view), at a fixed scale of about one
  node per 13 pt, the canvas growing to fit with the aspect ratio kept. Identical input gives
  identical bytes (fixed seed, iteration count and summation order; single-threaded, so
  `--threads` does not apply).
- **Canvas.** 1,440 × 1,008 user units (Graphify's figure before trimming), scaled by
  √(n / 800) above 800 nodes (S4 not reproduced); the side margins leave room for half the widest
  label.
- **Bounds.** At most 100,000 links are drawn (those whose endpoints' degrees sum highest, ties
  in `G.edges()` order; drawn in that order); above 3,000 nodes only the 2,000 best-connected
  labels are drawn; the legend lists at most the 40 largest communities and counts the rest.
- **Legend.** Always drawn when the graph has communities (S2 not reproduced): every community
  with members, by id, `name (members)` (S3 not reproduced).
- **Text.** Labels and names are escaped (`&`, `<`, `>`); characters XML 1.0 forbids are dropped
  and a tab becomes a space; `\n` gives one `<tspan>` per line. An SVG `<title>` gives the node
  and link counts.
- **Tests.** `crates/export/tests/svg.rs` compares each fixture with
  `tests/fixtures/exports/<lang>/svg_facts.json` (Graphify's per-node colour, radius and label in
  drawing order, solid/dashed link counts and legend presence, extracted from its SVG): all equal.
  It also checks determinism (force and packed), hostile labels and names (`</text><script>`,
  `]]>`, C0 controls, U+FFFE/U+FFFF, comment and PI openers) for well-formed output, empty and
  one-node graphs, and the large-graph caps.

## 13. Call-flow HTML (`export callflow-html`)

Derived 2026-10-14/15 by running `graphify export callflow-html` (0.9.86) on the 15 clustered
fixtures, on corpus references (monet, fastapi, ripgrep, excalidraw, graphify, erpnext, sinatra,
guzzle, okhttp, kubernetes) and on probe graphs (one-word communities for keyword scans over
~263k words, hostile labels, ordering fuzz), then reading the page's text. Graphify's page
markup, CSS and script were not copied; only its data (headings, prose, Mermaid text, table
cells) is reproduced.

### 13.1 Command and files

- **Rule.** `graphify export callflow-html [GRAPH|DIR] [--graph P] [--labels P] [--report P]
  [--sections P] [--output HTML] [--lang auto|zh-CN|en] [--max-sections N] [--diagram-scale N]`
  writes `<project>-callflow.html`, `<project>` being the name of the graph's directory
  (`--output` overrides). One self-contained page loading `mermaid@11` (unpinned) from
  jsDelivr.
- **Rule.** `--max-sections N`: 0 means the default 15; any value below 2 (1, negatives) means 4;
  otherwise N sections in total *including* "Other". A non-integer exits 1 with a traceback.
- **Quirk F1.** A graph above 512 MiB is refused ("exceeds 536_870_912-byte cap") unless
  `GRAPHIFY_MAX_GRAPH_BYTES` is raised (Kubernetes). dijkstral has no cap.
- **Open F2.** `--sections` file format: `{…}`, `[…]` and `{"sections": […]}` with ids, titles
  and community lists all fail with "no sections defined". dijkstral rejects `--sections` with a
  clear error. `--report` and `--labels` change nothing observed; dijkstral accepts and ignores
  them.
- **Rule.** `Commit:` in the subtitle is the graph's top-level `built_at_commit`, first 7
  characters, else `unknown`.

### 13.2 Sections

- **Rule (communities).** Every node belongs to the community named by Python's `str` of its
  `community`, a missing or `null` one being `unknown` (members in node order; probe: `unknown`
  for 9 nodes without or with `null`, `0`, `x`, `7.5`, `True`); the global graph's nodes from
  unclustered inputs form `unknown`. Sorted by size descending, ties by that name
  (`"103" < "104" < "99"`). The subtitle and the stats count these names; the layers table shows
  them.
- **Open F12.** The page reads every link of the file, not NetworkX's merged graph: a pair
  repeated in a simple graph counts and draws twice (direction probe: `calls f→e` and
  `uses e→f`), and a link whose `_src` and `_tgt` are both `null` is dropped from the edge count.
  dijkstral reads the §9 graph (one edge per pair). Links follow `_src` → `_tgt` (§1).
- **Rule (category).** Each community is scored for nine categories, in priority order:
  `extract-pipeline` Extraction Pipeline, `build-graph` Graph Build, `analysis-clustering`
  Analysis & Clustering, `outputs-docs` Outputs & Docs, `cli-skills` CLI & Skill Installers,
  `ingest-cache-update` Ingestion & Updates, `serve-api` Serving API, `security-global`
  Security & Global Graph, `tests-fixtures` Tests & Fixtures. Score = occurrences of the
  category's keywords among the lower-cased `[a-z0-9]+` words of each member's `label`,
  `source_file`, `file_type`, `kind`, `type`, `category` and `node_type` (not `id`,
  `norm_label`, `source_location` or relations), plus one per keyword among the community's top
  three name tokens (§13.3). The best score ≥ 2 wins, ties by priority; else the section is
  generated. Same-category communities merge into one section. Keyword lists (from a scan of
  ~263k words): see `CATEGORIES` in `callflow.rs` (e.g. `path` counts for Analysis and
  Security, `html` for Outputs and Ingestion, `tree` for Extraction and Outputs).
- **Open F3.** 14 of 2,959 corpus communities are classified differently (fastapi 7, ripgrep 0,
  excalidraw 2/3/5/8, …): all are large (100+ members); the scoring above gives a different
  winner. Fixtures and monet classify identically.
- **Rule (generated name).** The community's three most frequent name tokens (ties: first seen;
  tokens from each member's label then `source_file`): split on whitespace, `/`, `_`, `-`; keep
  `[a-z0-9]` of the lower-cased part; at least 3 characters; not one of `and class core css file
  for from function html index init java jsx lib main method src that the this tsx with`.
  Slug: tokens joined by `-`; title: Python `str.title()` of each (`neo4j` → `Neo4J`).
  A repeated slug gets `-` and the first 6 hex digits of SHA-1 of the title.
- **Rule (order).** Category sections in priority order, then generated sections; past the limit
  the rest merge into `other` "Other" (communities listed in section order).
- **Rule (zh-CN).** Titles 提取管线, 图谱构建, 分析聚类, 输出文档, CLI 与技能安装, 摄取与增量更新,
  服务 API, 安全与全局图, 测试与样例, 其他. `--lang auto` picks zh-CN for a CJK label
  (**Open F5**: exact trigger).

### 13.3 Page text

- Title `<project> — Complete Call Flow & Architecture Documentation`; subtitle
  `Generated from graphify knowledge graph: N nodes, E edges, C communities. Commit: X`
  (dijkstral writes its own name).
- Section intro: `<title> groups implementation around t1, t2, t3, t4, mostly in f1, f2, f3.
  This section covers N nodes and M internal edges; …` — four top name tokens, three most
  common files (ties: first seen), paths shortened to their last three components everywhere on
  the page; M counts edges with both ends in the section.
- Key Files: up to 8 files by count, ties by path (byte order); `N nodes` (`1 nodes` kept).
- Design note: `This section comes from graphify community clustering. Relationship summary:
  r x n, … .` — the four most common relations of the section's high-confidence internal edges
  (confidence not AMBIGUOUS and score ≥ 0.8 — **Open F6**, threshold bracketed by 0.6/0.85),
  counted in member order (each member's out-links in link order); else `No high-confidence
  call edges detected`. Relations are shown with `imports_from` → imports, `rationale_for` →
  explains, `indirect_call` → indirect call, otherwise `_` → space, `"` → `'`, `|` → space.
- Stats: Nodes, Edges, Hyperedges, Communities, Documented Sections; EXTRACTED / INFERRED /
  AMBIGUOUS counts. Footer `Generated: <UTC minute> · graphify callflow-html` (the only
  run-dependent text).

### 13.4 Overview

- Node `SLUGID("Title<br/><small>n nodes</small>")` + `class … module;` per section, id
  `mermaid_id(slug)` upper-cased (§13.6).
- Edges: per ordered section pair (`_src` → `_tgt`, §1), the number of links between them,
  labelled `rel xN`, or `rel` alone for a single link; the 12 busiest pairs. Without any cross-section link, consecutive sections joined by `-.->`.
- **Open F7.** The label relation and tie order: the most common relation explains most pairs,
  with `inherits`/`method` preferred in ties; 33 % of fixture overviews match exactly.
- **Open F8.** "Core Flow" (up to 7 titles joined by ` -> `): a walk over section links; without
  links the first 7 sections in order. dijkstral follows the strongest unvisited successor from
  the section that sends most (27 % exact).

### 13.5 Section diagram

- `flowchart LR`, `%% Section: <title> (N nodes, M edges)`, subgraphs, nodes, edges, then
  `class <id> <kind>;` in declaration order.
- **Rule (edges).** Internal links in member order (out-links in link order); keep `calls`,
  `uses`, `method`, `imports`, `imports_from` with high confidence when any exist, else all;
  sort by confidence (EXTRACTED, INFERRED, AMBIGUOUS), relation (method; calls/uses;
  imports), score descending, enumeration; at most 24, all drawn `-->|rel|`.
- **Rule (nodes).** All members up to 18; larger sections show 18 (**Open F4**: which 18 —
  dijkstral takes endpoints of the best edges, then by degree).
- **Rule (groups).** Nodes of one file (≥ 2) form `subgraph <id>["short/path"]`, file-less
  nodes `"External / generated"`; none when everything is one group; groups by size, ties by
  name; single nodes after the groups.
- **Open F4 (order).** Linked nodes: up to six sources by (out − in, out, id) then the rest in
  edge order — 99 % of 280 fuzz graphs, fails on some cycles; unlinked nodes: non-rationale
  first, then label (byte order; `global_direction` has `é` before `F`, against it). 69 % of
  fixture section diagrams match exactly.
- **Rule (node).** `id("label<br/><small>short/path</small>")`, file nodes and file-less nodes
  without the `<small>`. Label without a leading `.`; longer than 40 characters → first 39
  (trailing space dropped) + `...`; `"` → `'`; `{ } # ; `` ` `` `|` removed; whitespace
  collapsed.

### 13.6 Ids and escaping

- **Rule.** `mermaid_id(t)`: runs of characters outside `[A-Za-z0-9]` → one `_` (none at the
  ends), first 48 characters, `node_` before a leading digit, then `_` + first 8 hex of
  SHA-1(t). Nodes: t = node id; subgraphs: t = `<slug>_<short path or "External / generated">`.
- **Quirk F9.** Graphify writes labels into Mermaid with `htmlLabels` on and no escaping of
  `<`, `>` or `&`, so a label such as `<b>x</b>` renders as markup inside the diagram (Mermaid's
  sanitiser removes scripts). dijkstral writes `<` and `>` as Mermaid entity codes (`#lt;`,
  `#gt;`); the page runs Mermaid 11.4.1 (pinned, SRI) with `securityLevel: "strict"`, and every
  other string reaches the page through HTML escaping.
- **Quirk F10.** Edge labels are written unquoted (`a -->|calls| b`), so a relation holding a
  parenthesis or bracket, or an empty one (`-->||`), makes that diagram fail to parse. dijkstral
  writes plain-word labels the same way and quotes any other (`-->|"rel) x"|`, `-->|" "|`);
  headless Chrome renders every diagram of the hostile probe page.

### 13.7 Call details

- **Rule.** First 30 section members (section order); columns #, Node (`label` + short path),
  Type, Caller, Callees, Description.
- **Rule (type).** rationale → Concept; path containing `test` → Test; label containing `api`,
  `route`, `endpoint` → API; `client`, `command`, `cli`, `click`, `typer` → Entry; `props`,
  `store`, `component`, `hook` → UI; `async`, `await`, `stream`, `sse` → Async; a `.tsx`/`.jsx`
  file → UI; `_callable_class`, or a non-callable non-concept label starting upper-case and not
  ending `()` → Class; a file node with extension py, ts, js, go, rs, rb, php, kt, swift, toml,
  mod → Module; else Function. Diagram classes: api, entry, ui, async, klass, module, test,
  concept, function (98.5 % of fixture rows; **Open F10** the rest).
- **Rule (callers/callees).** In-/out-links with relation `calls`, `imports`, `imports_from`,
  `uses`, `method`, `indirect_call`; distinct nodes sorted by lower-cased label (leading `.`
  dropped), the first 3 (label shortened as in diagrams, short path when the node has a file),
  then `+N more`; none → `External entry / no inbound edge` / `No direct outbound edge`.
- **Rule (description).** Concept → `Design note for <label>.`; a `.py`/`.ts`/`.tsx` file node →
  `Module file in <path>.`; label keywords in priority order (`analy`; `graph`, `dependency`;
  `prompt`; `config`; `cache`, `hash`; `clone`, `git`, `ingest`; `chat`, `rag`, `retrieve`,
  `useragent`; `page`, `sidebar`, `wiki`; `export`, `html`, `markdown`; `scan`; `test`) → fixed
  sentences; else `<label> node in <path or project>.` (100 % of fixture rows).
- **Open F11.** Go module labels shown as `com google uuid` in caller lists.

### 13.8 dijkstral's command

`dijkstral export callflow-html [GRAPH|DIR] [--graph P] [--output HTML] [--lang auto|zh-CN|en]
[--max-sections N] [--diagram-scale N] [--labels P] [--report P]` writes
`<project>-callflow.html` next to the graph (`<project>` as for `graph.html`). The page is
dijkstral's own (`crates/export/src/callflow.html`): sticky section navigation, per-diagram zoom
buttons, light/dark colours; offline, the Mermaid source stays readable with a note.
`--diagram-scale` scales the diagrams' font size.

Tests: `crates/export/tests/callflow.rs` compares every component with
`tests/fixtures/exports/<lang>/callflow/page.json` (Graphify's extracted data, generation time
normalised) for 15 fixtures. Exact on every fixture: title, subtitle, navigation, headings,
layers, overview nodes, intros, key files, stats, node cells, descriptions. Match rates (floors
in the test): call types 98.5 %, callers 99.9 %, callees 99.4 %, design notes 98.8 %, section
diagrams 69.9 %, overview edges 33 %, core flow 27 %. Plus hostile-label inertness (both
languages), determinism and language detection.

Measurements (release, Apple Silicon): erpnext 0.9 s / 102 MB RSS, 409 KB page (Graphify 4.4 s /
280 MB, 459 KB); Kubernetes 5.7 s / 1.66 GB, 307 KB (Graphify refuses by default; with the cap
raised 52.3 s / 2.97 GB). Headless Chrome (Playwright, installed Chrome): python fixture (en and
zh-CN), a hostile probe, erpnext and Kubernetes rendered every diagram (9/9, 9/9, 2/2, 16/16,
16/16) with no console errors.

## 14. Tree (`tree`)

Derived 2026-10-14/15 by running `graphify tree` on the 15 clustered fixtures (default,
`--max-children 0|1|3`, `--label`, `--root`), on eight corpus references (monet, erpnext, fastapi,
graphify, ripgrep, excalidraw, okhttp, dart-http) and on probe graphs (path forms, file-node
labels, ordering and case ties, truncation, roots, hostile labels), reading only the data the page
embeds. Graphify's page markup, CSS and script were not stored or copied.

### 14.1 Command and output

| Flag | Graphify | dijkstral |
|---|---|---|
| `--graph P` | default `graphify-out/graph.json`; missing → `error: graph.json not found at …` | default `dijkstral-out/graph.json`, else `graphify-out/graph.json` |
| `--output HTML` | default `GRAPH_TREE.html` **next to the graph** (not `graphify-out/` as `--help` says); parent directories created | same; an existing directory gets `GRAPH_TREE.html` inside it |
| `--root PATH` | see §14.3 | same |
| `--max-children N` | default 200; see §14.4 | same (`usize`; negative values rejected by the parser) |
| `--top-k-edges N` | default 12; accepted and ignored (quirk T1) | outbound links per symbol in the inspector |
| `--label NAME` | the root's name and page title | same |

- **Rule (page data).** One self-contained HTML page loading D3 v7 (`https://d3js.org/d3.v7.min.js`,
  unpinned, no integrity). The hierarchy is one JavaScript constant on one line,
  `const initialJsonData = <json>;`, written as Python `json.dumps(…, separators=(",", ":"))`
  (ASCII-escaped, `\uXXXX` lowercase, surrogate pairs) with `<` written `<`. Every entry is
  `{"name", "total_count", "children"}` in that key order; leaves have `"children":[]`.
- **Quirk T2.** A `graph.json` above 536,870,912 bytes is refused:
  `error: graph file … is 570_715_182 bytes, exceeds 536_870_912-byte cap`, exit 2 (Kubernetes).
  dijkstral has no cap.
- **Quirk T3.** `--output DIR` for an existing directory crashes with a traceback.
- **Quirk T4.** `--max-children -1` keeps all but the last symbol of every file and reports
  `(+len+1 more)`, doubling every count.

### 14.2 Hierarchy

- **Rule (members).** Every node with a non-empty string `source_file` belongs to that file, in
  node order. Nodes without one (externals, concepts) are not shown.
- **Rule (file nodes).** A node is the file itself, and not listed, when its `file_type` is
  exactly `code`, its label is non-empty and the label equals `source_file` or is a trailing part
  of it after a `/` (`x.py`, `hub/x.py` for `apps/hub/x.py`; `b//c.py` for `a/b//c.py`). Labels
  `Y.py`, ` y.py`, `./a/y.py`, `zz/y.py`, or `y.py` with `file_type` `rationale`, `concept`,
  `document`, `""` or none, stay as symbols. The label is `label`, else the id.
- **Rule (files).** One file entry per distinct raw `source_file` string, so `a/y.py`, `a/./y.py`
  and `a//y.py` are three entries named `y.py`. A file whose only node is its file node is a leaf
  (`total_count` 1, no children).
- **Rule (paths).** Paths are split as `pathlib.PurePosixPath` does: `/` separates, empty and `.`
  components vanish, `..` stays, `\` is an ordinary character, a leading `//` is its own anchor.
  Files are placed in order of their raw path (code point order); directories are created on
  first use and shared by name; a file and a directory with the same name are separate entries.
- **Rule (counts).** `total_count` of a leaf is 1; of an entry with children, the sum of its
  children's; of a `(+N more)` entry, N.
- **Rule (empty).** No member at all gives `{"name":"(empty graph)","total_count":0,
  "children":[]}`, ignoring `--label` and `--root`.

### 14.3 Root

- **Rule.** The root is `--root` as given, else `os.path.commonpath` of every distinct
  `source_file` (empty when absolute and relative paths are mixed, or nothing is shared). Each file
  is placed by its path relative to the root (`PurePosixPath.relative_to`: same anchor, root
  components a prefix); a file equal to the root keeps its full path; a file outside the root is
  placed at the top by its last component (`--root pkg`: `main.py`, `tests/x.py` → `x.py`).
- **Rule (name).** `--label`, else the root's last component, else the root text as given
  (`.`, `./`, `/`), else `/` when there is no root. `--root a/..` is named `..` (no normalisation).
- **Rule (no match).** With `--root`, when no file is under it: `error: --root R matched 0 of N
  source files (source_file paths are stored repo-relative, e.g. 'P')`, exit 1, nothing written;
  N counts distinct source files and P is the first in code point order.

### 14.4 Order and truncation

- **Rule (order).** Children are sorted stably by (has no children, `name.lower()`): directories
  and files with members first, then leaves, each by Python's lowercase in code point order
  (`ß` after `z`, `İ` after `i`; Kelvin sign ties `k`), ties in placement order (files: raw path
  order; symbols: node order).
- **Rule (truncation).** Only files are cut. When a file has more than `--max-children` symbols,
  the kept ones are the first N by (`name` starts with `_`, `name.lower()`) — public names first —
  then a leaf `(+H more)` with `total_count` H is added and the whole list is sorted as above
  (`(` sorts between `#` and `.`). Directories are never cut.
- **Quirk T1.** `--top-k-edges` changes nothing in the page; Graphify's page has no edge data.

### 14.5 dijkstral's page

`crates/export/src/tree.html` is dijkstral's own: a header (title, file and symbol counts, search,
expand-level, collapse-all, recenter), a zoomable D3 tree that opens collapsed below the first
level and expands on click, and a side panel. D3 **7.9.0** from jsDelivr with subresource
integrity. Data elements, each `type="application/json"`:

- `tree-data`: Graphify's hierarchy text exactly (§14.1), so `<` is escaped and U+2028/9 are `\u`
  escapes.
- `tree-inspector` (`html_safe`: `<`, `>`, `&`, U+2028/9 escaped): `title`; `kinds`, one letter per
  entry in pre-order (`d` directory, `f` file, `s` symbol, `m` more); `leaves`, one record per
  leaf in pre-order — `null` for files without symbols and `(+N more)`, else
  `[id, file_type, source_file, source_location, community, degree, outbound total,
  [[relation, target]…]]` with up to `--top-k-edges` outbound links (links whose direction, `_src`
  → `_tgt` when present (§1), starts at the symbol), best-connected targets first, ties in adjacency order; `targets`:
  `[label, source_file, source_location]`.

All text reaches the DOM through `textContent`; the title is HTML-escaped. Offline, the page
explains that D3 could not be loaded.

### 14.6 Tests and measurements

`crates/export/tests/tree.rs`: `tests/fixtures/exports/<lang>/tree/tree.json` and
`tree_max3.json` (Graphify's embedded text for the seven export fixtures) byte for byte; leaves
line up with inspector records; hostile labels, paths, ids and relations stay data; the `--root`
error; the empty graph; identical pages across threads. Beyond the goldens (2026-10-15): the
embedded text is byte-identical to Graphify's for all 15 clustered fixtures and two raw fragments
(default, `--max-children 0|1|3`, `--label`, `--root src|pkg` incl. matching errors), for every
probe graph, and for monet, erpnext, fastapi, graphify, ripgrep, excalidraw, okhttp and dart-http
(default, `--max-children 0|1|3`, `--label`).

| Graph | dijkstral `tree` | page | Graphify `tree` |
|---|---|---|---|
| erpnext (24,669 nodes) | 0.18 s, 115 MB RSS | 7.1 MB (1.7 MB hierarchy + inspector) | 0.45 s, 205 MB RSS, 1.7 MB |
| kubernetes (256,312 nodes, 571 MB) | 4.3 s, 1.49 GB RSS | 65 MB | refused (T2), exit 2 after 0.25 s |

Headless Chrome (Playwright, installed Chrome) opened the python fixture, a probe graph, erpnext
and kubernetes pages (load ≤ 0.6 s; collapsed start) with no console errors; expanding a directory,
expanding two levels, search (200 hits), revealing a hit, selecting a symbol (inspector with its
outbound links) and collapse-all worked.

### 14.7 Open

| Id | Summary |
|---|---|
| T-O1 | A non-string `label` or `source_file`: dijkstral uses the id / skips the node |
| T-O2 | A `source_file` with no components (`.`, `/`): dijkstral names the file entry `""` |
| T-O3 | `//net` anchors are matched literally; Graphify's `commonpath` collapses them to `/net` (same tree in the probe) |

## 15. Stage-4b measurements (2026-10-15, release build, Apple Silicon)

On Graphify's reference graphs (`~/.cache/dijkstral-bench/reference/`): erpnext 24,633 nodes /
61,108 links (32 MB), kubernetes 256,424 nodes / 1.4 M links (571 MB). Wall time and peak RSS of
the whole command (JSON load included); the machine was shared with other builds, so times are
indicative. Graphify refuses any graph above 512 MiB unless `GRAPHIFY_MAX_GRAPH_BYTES` is raised
(T2), so its Kubernetes column needs that override.

| Export | erpnext dijkstral | erpnext Graphify | kubernetes dijkstral | kubernetes Graphify |
|---|---|---|---|---|
| `cypher.txt` | 0.5 s, 99 MB; 18.7 MB, identical | 2.5 s, 353 MB | 3.3 s, 1.67 GB; 311 MB, identical | refused; 96 s, 1.75 GB with the cap raised |
| Neo4j `--push` (local 5.26) | 8.5 s, all 61,108 relationships | 600 s cap hit: 13,602 of 61,108 relationships (C6) | 43 s, 1.89 GB; all 1,401,514 relationships (Neo4j given 6 GB; at 1 GB the server died) | not run |
| FalkorDB `--push` | 16.7 s, all | not run (same per-statement shape) | 93 s, 1.96 GB; all | not run |
| Obsidian vault | 2.6–8.6 s, 151 MB; 27,038 files, 117 MB, identical | 49.8 s | 41 s, 1.98 GB; 265,844 files, 1.2 GB | not run |
| `graph.svg` | 0.15 s, 107 MB; 5.5 MB | fails (S1); with SciPy still running after 16 min | 2.8 s, 1.71 GB; 26 MB, 100k of 1.4 M links drawn | not run |
| call-flow page | 0.9 s, 102 MB; 409 KB page | 4.4 s, 280 MB | 5.7 s, 1.66 GB; 307 KB page | refused; 52.3 s, 2.97 GB with the cap raised |
| `GRAPH_TREE.html` | 0.18 s, 115 MB; 7.1 MB page | 0.45 s, 205 MB; 1.7 MB page | 4.3 s, 1.49 GB; 65 MB page | refused (T2) |

The loader (§9) streams `graph.json` into compact attribute lists; holding the parsed
`serde_json::Value` tree instead took 2.9 GB for Kubernetes' `cypher.txt` and 3.6 GB for its
vault.

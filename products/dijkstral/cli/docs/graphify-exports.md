# Graphify exporters: black-box specification (`graph.html`, wiki, GraphML)

What `graphifyy` 0.9.86 writes for `graph.html`, the Markdown wiki and GraphML, and how
dijkstral's `crates/export` reproduces it. Derived 2026-10-14 only by running Graphify on the
clustered fixture goldens (`tests/fixtures/<lang>.clustered.graphify.json`), on the frozen corpus
references (`~/.cache/dijkstral-bench/reference/<repo>.graph.json`) and on hand-written probe
graphs (hostile labels, colliding names, mixed attribute types), then reading the outputs.
Graphify's source and its page template were not read; dijkstral's page is its own and only the
embedded data shapes are reproduced.

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

- **Rule.** `RAW_EDGES` in `G.edges()` order, each with the link's own direction:
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
  direction); every field except `source`, `target` and `_…`, in order.
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

| Id | Kind | Summary |
|---|---|---|
| X1 | Quirk | `extract --wiki` is accepted and ignored; `extract` writes no `graph.html` |
| X2 | Quirk | `LEGEND` is empty without a labels file |
| W3 | Quirk | index god-node links resolve by label, so duplicate labels point at the wrong article |
| G1 | Quirk | `export graphml` crashes on a raw (`--no-cluster`) graph |
| O1 | Open | `graph.html` when every node has degree 0 (max degree 0): dijkstral divides by 1 |
| O2 | Open | whether the slug's `unnamed` rule is "only dots" or a strip of leading/trailing dots |
| O3 | Open | truncation and the `_n` suffix: dijkstral truncates to 200 characters before suffixing |
| O4 | Open | a god node without a community: dijkstral omits the `**Community:**` line |
| O5 | Open | control characters other than C0 (U+007F–U+009F) in `graph.html` labels: kept |

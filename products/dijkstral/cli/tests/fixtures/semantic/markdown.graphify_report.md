# Graph Report - mdc  (2026-10-15)

## Corpus Check
- 16 files · ~418 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 1 file(s) not represented in the graph (top: .markdown 1)

## Summary
- 85 nodes · 94 edges · 13 communities (7 shown, 6 thin omitted)
- Extraction: 77% EXTRACTED · 23% INFERRED · 0% AMBIGUOUS · INFERRED: 22 edges (avg confidence: 0.95)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- run
- c.ts
- Resolution
- a.py
- Alpha
- Beta
- Then three
- helper
- Windows
- lib.rs
- UPPER.MD

## God Nodes (most connected - your core abstractions)
1. `Resolution` - 23 edges
2. `run()` - 7 edges
3. `Gamma` - 5 edges
4. `Alpha` - 4 edges
5. `Beta` - 4 edges
6. `Markdown Probe ##` - 4 edges
7. `Setup` - 4 edges
8. `tsFn()` - 3 edges
9. `API` - 3 edges
10. `Then three` - 3 edges

## Surprising Connections (you probably didn't know these)
- `Install `pip`` --references--> `Gamma`  [INFERRED]
  README.md → web/c.ts
- `Padded` --references--> `run()`  [INFERRED]
  docs/refs.md → a.py
- `Unique function` --references--> `run()`  [INFERRED]
  docs/refs.md → a.py
- `Trailing parens` --references--> `Gamma`  [INFERRED]
  docs/refs.md → web/c.ts
- `Nested function` --references--> `inner_helper()`  [INFERRED]
  docs/refs.md → a.py

## Import Cycles
- None detected.

## Communities (13 total, 6 thin omitted)

### Community 0 - "run"
Cohesion: 0.16
Nodes (9): run(), Same, Starts at two, Padded, Unique function, Mdx Notes, Comment heading, Deep [link](http://example.com) heading (+1 more)

### Community 1 - "c.ts"
Cohesion: 0.16
Nodes (12): API, Guide, Qualified by file stem, Trailing parens, TS interface, Twice in one section, Color, Red (+4 more)

### Community 2 - "Resolution"
Cohesion: 0.14
Nodes (13): Ambiguous across files, Ambiguous methods, Call syntax, Denylisted names, External module, File name, Not callable, Qualified with ambiguous tail (+5 more)

### Community 3 - "a.py"
Cohesion: 0.20
Nodes (6): Data, edges(), inner_helper(), HTTP, Nested function, Uppercase class

### Community 4 - "Alpha"
Cohesion: 0.25
Nodes (6): Alpha, Install `pip`, Markdown Probe ##, Setup, Setup, Tilde fenced heading

### Community 5 - "Beta"
Cohesion: 0.29
Nodes (3): Qualified by class, Qualified by directory, Beta

### Community 6 - "Then three"
Cohesion: 0.50
Nodes (4): Same, Same, Then one, Then three

## Knowledge Gaps
- **1 isolated node(s):** `Red`
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 41 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **6 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Resolution` connect `Resolution` to `run`, `c.ts`, `a.py`, `Beta`?**
  _High betweenness centrality (0.414) - this node is a cross-community bridge._
- **Are the 6 inferred relationships involving `run()` (e.g. with `Padded` and `Unique function`) actually correct?**
  _`run()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Red` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Resolution` be split into smaller, more focused modules?**
  _Cohesion score 0.14285714285714285 - nodes in this community are weakly interconnected._
- **Why does `run()` connect `run` to `a.py`, `Alpha`?**
  _High betweenness centrality (0.269) - this node is a cross-community bridge._
- **Are the 3 inferred relationships involving `Gamma` (e.g. with `Trailing parens` and `Twice in one section`) actually correct?**
  _`Gamma` has 3 INFERRED edges - model-reasoned connections that need verification._
- **Why does `Unique function` connect `run` to `Resolution`?**
  _High betweenness centrality (0.099) - this node is a cross-community bridge._
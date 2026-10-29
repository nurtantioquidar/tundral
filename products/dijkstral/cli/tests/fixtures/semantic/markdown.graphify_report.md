# Graph Report - mdc  (2026-10-29)

## Corpus Check
- 17 files · ~418 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 1 file(s) not represented in the graph (top: .markdown 1)

## Summary
- 85 nodes · 93 edges · 13 communities (7 shown, 6 thin omitted)
- Extraction: 77% EXTRACTED · 23% INFERRED · 0% AMBIGUOUS · INFERRED: 21 edges (avg confidence: 0.95)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- run
- Resolution
- a.py
- c.ts
- guide.md
- other.md
- Beta
- helper
- Windows
- lib.rs
- UPPER.MD

## God Nodes (most connected - your core abstractions)
1. `Resolution` - 23 edges
2. `run()` - 7 edges
3. `Alpha` - 4 edges
4. `Beta` - 4 edges
5. `Gamma` - 4 edges
6. `Setup` - 4 edges
7. `tsFn()` - 3 edges
8. `Markdown Probe` - 3 edges
9. `API` - 3 edges
10. `Then three` - 3 edges

## Surprising Connections (you probably didn't know these)
- `Padded` --references--> `run()`  [INFERRED]
  docs/refs.md → a.py
- `Unique function` --references--> `run()`  [INFERRED]
  docs/refs.md → a.py
- `API` --references--> `tsFn()`  [INFERRED]
  docs/guide.md → web/c.ts
- `Mdx Notes` --references--> `run()`  [INFERRED]
  notes.mdx → a.py
- `Nested function` --references--> `inner_helper()`  [INFERRED]
  docs/refs.md → a.py

## Import Cycles
- None detected.

## Communities (13 total, 6 thin omitted)

### Community 0 - "run"
Cohesion: 0.14
Nodes (12): Alpha, run(), Padded, Unique function, Comment heading, Deep [link](http://example.com) heading, Indented three spaces, Install `pip` (+4 more)

### Community 1 - "Resolution"
Cohesion: 0.14
Nodes (13): Ambiguous across files, Ambiguous methods, Call syntax, Denylisted names, External module, File name, Not callable, Qualified with ambiguous tail (+5 more)

### Community 2 - "a.py"
Cohesion: 0.20
Nodes (6): Data, edges(), inner_helper(), HTTP, Nested function, Uppercase class

### Community 3 - "c.ts"
Cohesion: 0.22
Nodes (9): Trailing parens, TS interface, Twice in one section, Color, Red, Gamma, lowerValue, Shape (+1 more)

### Community 4 - "guide.md"
Cohesion: 0.29
Nodes (4): API, Guide, Qualified by file stem, Mdx Notes

### Community 5 - "other.md"
Cohesion: 0.29
Nodes (6): Same, Same, Same, Starts at two, Then one, Then three

### Community 6 - "Beta"
Cohesion: 0.29
Nodes (3): Qualified by class, Qualified by directory, Beta

## Knowledge Gaps
- **1 isolated node(s):** `Red`
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 41 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **6 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Resolution` connect `Resolution` to `run`, `a.py`, `c.ts`, `guide.md`, `Beta`?**
  _High betweenness centrality (0.440) - this node is a cross-community bridge._
- **Are the 6 inferred relationships involving `run()` (e.g. with `Padded` and `Unique function`) actually correct?**
  _`run()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Red` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `run` be split into smaller, more focused modules?**
  _Cohesion score 0.14166666666666666 - nodes in this community are weakly interconnected._
- **Why does `run()` connect `run` to `a.py`, `guide.md`?**
  _High betweenness centrality (0.284) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `Gamma` (e.g. with `Trailing parens` and `Twice in one section`) actually correct?**
  _`Gamma` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Should `Resolution` be split into smaller, more focused modules?**
  _Cohesion score 0.14285714285714285 - nodes in this community are weakly interconnected._
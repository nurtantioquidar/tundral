# Graph Report - co  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 32 nodes · 42 edges · 6 communities (4 shown, 2 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5

## God Nodes (most connected - your core abstractions)
1. `Admin` - 5 edges
2. `User` - 5 edges
3. `describe()` - 4 edges
4. `Person` - 4 edges
5. `Label` - 4 edges
6. `validate()` - 3 edges
7. `Internal` - 3 edges
8. `Shop` - 3 edges
9. `Box` - 3 edges
10. `Entity` - 3 edges

## Surprising Connections (you probably didn't know these)
- `Fixture` --references--> `Admin`  [EXTRACTED]
  test/runtests.jl → src/models.jl
- `Fixture` --references--> `Label`  [EXTRACTED]
  test/runtests.jl → src/models.jl

## Import Cycles
- None detected.

## Communities (6 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.31
Nodes (7): Printf, describe(), promote(), register(), @registered, Base.show(), validate()

### Community 1 - "Community 1"
Cohesion: 0.29
Nodes (5): Base, Dates, Internal, Shop, Shop

### Community 2 - "Community 2"
Cohesion: 0.60
Nodes (4): Box, Entity, name(), Person

### Community 3 - "Community 3"
Cohesion: 0.50
Nodes (3): Admin, Label, Fixture

## Knowledge Gaps
- **7 isolated node(s):** `Printf`, `@registered`, `Base`, `Dates`, `Shop` (+2 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 12 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Fixture` connect `Community 3` to `Community 4`?**
  _High betweenness centrality (0.097) - this node is a cross-community bridge._
- **What connects `Printf`, `@registered`, `Base` to the rest of the system?**
  _7 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Admin` connect `Community 3` to `Community 2`, `Community 5`?**
  _High betweenness centrality (0.078) - this node is a cross-community bridge._
- **Why does `Label` connect `Community 3` to `Community 2`, `Community 5`?**
  _High betweenness centrality (0.043) - this node is a cross-community bridge._
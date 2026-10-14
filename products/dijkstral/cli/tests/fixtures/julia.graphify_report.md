# Graph Report - upd  (2026-10-14)

## Corpus Check
- 4 files · ~211 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 32 nodes · 42 edges · 6 communities (4 shown, 2 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- service.jl
- Internal
- models.jl
- Admin
- runtests.jl
- User

## God Nodes (most connected - your core abstractions)
1. `User` - 5 edges
2. `Admin` - 5 edges
3. `Person` - 4 edges
4. `Label` - 4 edges
5. `describe()` - 4 edges
6. `Shop` - 3 edges
7. `Internal` - 3 edges
8. `Entity` - 3 edges
9. `Box` - 3 edges
10. `name()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `Fixture` --references--> `Admin`  [EXTRACTED]
  test/runtests.jl → src/models.jl
- `Fixture` --references--> `Label`  [EXTRACTED]
  test/runtests.jl → src/models.jl

## Import Cycles
- None detected.

## Communities (6 total, 2 thin omitted)

### Community 0 - "service.jl"
Cohesion: 0.31
Nodes (7): Printf, describe(), promote(), register(), @registered, Base.show(), validate()

### Community 1 - "Internal"
Cohesion: 0.29
Nodes (5): Base, Dates, Internal, Shop, Shop

### Community 2 - "models.jl"
Cohesion: 0.60
Nodes (4): Box, Entity, name(), Person

### Community 3 - "Admin"
Cohesion: 0.50
Nodes (3): Admin, Label, Fixture

## Knowledge Gaps
- **7 isolated node(s):** `Dates`, `Base`, `Shop`, `Printf`, `@registered` (+2 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 12 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Fixture` connect `Admin` to `runtests.jl`?**
  _High betweenness centrality (0.097) - this node is a cross-community bridge._
- **What connects `Dates`, `Base`, `Shop` to the rest of the system?**
  _7 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Admin` connect `Admin` to `models.jl`, `User`?**
  _High betweenness centrality (0.078) - this node is a cross-community bridge._
- **Why does `Label` connect `Admin` to `models.jl`, `User`?**
  _High betweenness centrality (0.043) - this node is a cross-community bridge._
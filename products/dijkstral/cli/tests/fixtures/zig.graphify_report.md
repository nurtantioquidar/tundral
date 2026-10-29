# Graph Report - zig  (2026-10-29)

## Corpus Check
- 6 files · ~469 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 52 nodes · 69 edges · 8 communities (7 shown, 1 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- main.zig
- main
- shapes.zig
- main_test.zig
- Tally
- Hue
- Shape

## God Nodes (most connected - your core abstractions)
1. `main()` - 8 edges
2. `Hue` - 7 edges
3. `Tally` - 6 edges
4. `helper()` - 4 edges
5. `Shape` - 4 edges
6. `add()` - 4 edges
7. `report()` - 3 edges
8. `LoadError` - 3 edges
9. `paint()` - 3 edges
10. `Probe` - 3 edges

## Surprising Connections (you probably didn't know these)
- `main()` --calls--> `paint()`  [INFERRED]
  main.zig → src/shapes.zig
- `callback()` --calls--> `helper()`  [INFERRED]
  tests/main_test.zig → main.zig
- `paint()` --calls--> `tint()`  [INFERRED]
  src/shapes.zig → src/hue.zig
- `main()` --calls--> `add()`  [EXTRACTED]
  main.zig → util.zig
- `main()` --calls--> `scale()`  [EXTRACTED]
  main.zig → util.zig

## Import Cycles
- None detected.

## Communities (8 total, 1 thin omitted)

### Community 0 - "main.zig"
Cohesion: 0.20
Nodes (8): helper(), load(), LoadError, Corrupt, Missing, report(), Snapshot, callback()

### Community 1 - "main"
Cohesion: 0.36
Nodes (6): main(), add(), Buffer(), List(), scale(), Sentinel

### Community 2 - "shapes.zig"
Cohesion: 0.29
Nodes (4): tint(), paint(), Raw, Rect

### Community 3 - "main_test.zig"
Cohesion: 0.25
Nodes (4): Fixture, Probe, off, on

### Community 4 - "Tally"
Cohesion: 0.40
Nodes (3): Tally, busy, idle

### Community 5 - "Hue"
Cohesion: 0.40
Nodes (4): Hue, blue, green, red

### Community 6 - "Shape"
Cohesion: 0.50
Nodes (3): Shape, circle, rect

## Knowledge Gaps
- **11 isolated node(s):** `idle`, `busy`, `Missing`, `Corrupt`, `red` (+6 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 24 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Hue` connect `Hue` to `shapes.zig`?**
  _High betweenness centrality (0.146) - this node is a cross-community bridge._
- **What connects `idle`, `busy`, `Missing` to the rest of the system?**
  _11 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Tally` connect `Tally` to `main.zig`?**
  _High betweenness centrality (0.119) - this node is a cross-community bridge._
- **Why does `Shape` connect `Shape` to `shapes.zig`?**
  _High betweenness centrality (0.111) - this node is a cross-community bridge._
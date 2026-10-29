# Graph Report - zig  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 52 nodes · 69 edges · 8 communities (7 shown, 1 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5
- Community 6

## God Nodes (most connected - your core abstractions)
1. `main()` - 8 edges
2. `Hue` - 7 edges
3. `Tally` - 6 edges
4. `helper()` - 4 edges
5. `add()` - 4 edges
6. `Shape` - 4 edges
7. `LoadError` - 3 edges
8. `report()` - 3 edges
9. `scale()` - 3 edges
10. `paint()` - 3 edges

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

### Community 0 - "Community 0"
Cohesion: 0.20
Nodes (8): helper(), load(), LoadError, Corrupt, Missing, report(), Snapshot, callback()

### Community 1 - "Community 1"
Cohesion: 0.36
Nodes (6): main(), add(), Buffer(), List(), scale(), Sentinel

### Community 2 - "Community 2"
Cohesion: 0.29
Nodes (4): tint(), paint(), Raw, Rect

### Community 3 - "Community 3"
Cohesion: 0.25
Nodes (4): Fixture, Probe, off, on

### Community 4 - "Community 4"
Cohesion: 0.40
Nodes (3): Tally, busy, idle

### Community 5 - "Community 5"
Cohesion: 0.40
Nodes (4): Hue, blue, green, red

### Community 6 - "Community 6"
Cohesion: 0.50
Nodes (3): Shape, circle, rect

## Knowledge Gaps
- **11 isolated node(s):** `Corrupt`, `Missing`, `off`, `on`, `busy` (+6 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 24 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Hue` connect `Community 5` to `Community 2`?**
  _High betweenness centrality (0.146) - this node is a cross-community bridge._
- **What connects `Corrupt`, `Missing`, `off` to the rest of the system?**
  _11 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Tally` connect `Community 4` to `Community 0`?**
  _High betweenness centrality (0.119) - this node is a cross-community bridge._
- **Why does `Shape` connect `Community 6` to `Community 2`?**
  _High betweenness centrality (0.111) - this node is a cross-community bridge._
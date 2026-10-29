# Graph Report - rust  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 78 nodes · 100 edges · 8 communities (6 shown, 2 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 4 edges (avg confidence: 0.78)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5
- Community 7

## God Nodes (most connected - your core abstractions)
1. `Circle` - 14 edges
2. `Shape` - 9 edges
3. `Kind` - 7 edges
4. `round_kind()` - 5 edges
5. `apply()` - 4 edges
6. `Named` - 3 edges
7. `Meters` - 3 edges
8. `total_area()` - 3 edges
9. `main()` - 3 edges
10. `fixture()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `render()` --references--> `Circle`  [INFERRED]
  src/util/mod.rs → src/model.rs
- `main()` --calls--> `round_kind()`  [INFERRED]
  src/main.rs → src/model.rs
- `render()` --references--> `Circle`  [EXTRACTED]
  src/util/fmt.rs → src/model.rs
- `widen()` --references--> `Meters`  [INFERRED]
  src/util/mod.rs → src/model.rs
- `main()` --calls--> `total_area()`  [INFERRED]
  src/main.rs → src/lib.rs

## Import Cycles
- 2-file cycle: `src/model.rs -> src/util/fmt.rs -> src/model.rs`

## Communities (8 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.14
Nodes (4): total_area(), main(), Named, Shape

### Community 1 - "Community 1"
Cohesion: 0.16
Nodes (5): Circle, .UNIT, fixture(), configure(), render()

### Community 2 - "Community 2"
Cohesion: 0.19
Nodes (7): Meters, apply(), helper(), reconfigure(), render(), run_all(), widen()

### Community 3 - "Community 3"
Cohesion: 0.21
Nodes (8): classify(), Kind, Empty, Round, Square, Platform, round_kind(), Unit

### Community 4 - "Community 4"
Cohesion: 0.22
Nodes (5): build(), GREETING, square!, MAX_SHAPES, Registry

### Community 5 - "Community 5"
Cohesion: 0.25
Nodes (4): Pair, Pair<T>, Wrapper, Wrapper<T>

## Knowledge Gaps
- **4 isolated node(s):** `Pair<T>`, `Empty`, `Square`, `shapes`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 41 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Circle` connect `Community 1` to `Community 0`, `Community 2`, `Community 3`, `Community 4`?**
  _High betweenness centrality (0.270) - this node is a cross-community bridge._
- **What connects `Pair<T>`, `Empty`, `Square` to the rest of the system?**
  _4 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.14285714285714285 - nodes in this community are weakly interconnected._
- **Why does `Shape` connect `Community 0` to `Community 1`, `Community 3`, `Community 4`?**
  _High betweenness centrality (0.251) - this node is a cross-community bridge._
- **Why does `Kind` connect `Community 3` to `Community 1`?**
  _High betweenness centrality (0.066) - this node is a cross-community bridge._
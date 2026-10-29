# Graph Report - rust  (2026-10-29)

## Corpus Check
- 7 files · ~694 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 78 nodes · 100 edges · 8 communities (6 shown, 2 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 4 edges (avg confidence: 0.76)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- main.rs
- Circle
- mod.rs
- model.rs
- lib.rs
- T
- shapes

## God Nodes (most connected - your core abstractions)
1. `Circle` - 14 edges
2. `Shape` - 9 edges
3. `Kind` - 7 edges
4. `round_kind()` - 5 edges
5. `apply()` - 4 edges
6. `total_area()` - 3 edges
7. `main()` - 3 edges
8. `Meters` - 3 edges
9. `Named` - 3 edges
10. `fixture()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `render()` --references--> `Circle`  [INFERRED]
  src/util/mod.rs → src/model.rs
- `main()` --calls--> `round_kind()`  [INFERRED]
  src/main.rs → src/model.rs
- `main()` --calls--> `total_area()`  [INFERRED]
  src/main.rs → src/lib.rs
- `render()` --references--> `Circle`  [EXTRACTED]
  src/util/fmt.rs → src/model.rs
- `widen()` --references--> `Meters`  [INFERRED]
  src/util/mod.rs → src/model.rs

## Import Cycles
- 2-file cycle: `src/model.rs -> src/util/fmt.rs -> src/model.rs`

## Communities (8 total, 2 thin omitted)

### Community 0 - "main.rs"
Cohesion: 0.14
Nodes (4): total_area(), main(), Named, Shape

### Community 1 - "Circle"
Cohesion: 0.16
Nodes (5): Circle, .UNIT, fixture(), configure(), render()

### Community 2 - "mod.rs"
Cohesion: 0.19
Nodes (7): Meters, apply(), helper(), reconfigure(), render(), run_all(), widen()

### Community 3 - "model.rs"
Cohesion: 0.21
Nodes (8): classify(), Kind, Empty, Round, Square, Platform, round_kind(), Unit

### Community 4 - "lib.rs"
Cohesion: 0.22
Nodes (5): build(), GREETING, square!, MAX_SHAPES, Registry

### Community 5 - "T"
Cohesion: 0.25
Nodes (4): Pair, Pair<T>, Wrapper, Wrapper<T>

## Knowledge Gaps
- **4 isolated node(s):** `shapes`, `Square`, `Empty`, `Pair<T>`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 41 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Circle` connect `Circle` to `main.rs`, `mod.rs`, `model.rs`, `lib.rs`?**
  _High betweenness centrality (0.270) - this node is a cross-community bridge._
- **What connects `shapes`, `Square`, `Empty` to the rest of the system?**
  _4 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `main.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.14285714285714285 - nodes in this community are weakly interconnected._
- **Why does `Shape` connect `main.rs` to `Circle`, `model.rs`, `lib.rs`?**
  _High betweenness centrality (0.251) - this node is a cross-community bridge._
- **Why does `Kind` connect `model.rs` to `Circle`?**
  _High betweenness centrality (0.066) - this node is a cross-community bridge._
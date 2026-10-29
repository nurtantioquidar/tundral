# Graph Report - fortran  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 50 nodes · 68 edges · 8 communities (7 shown, 1 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5
- Community 6
- Community 7

## God Nodes (most connected - your core abstractions)
1. `geometry` - 13 edges
2. `shapes` - 9 edges
3. `circle` - 7 edges
4. `point` - 7 edges
5. `describe()` - 6 edges
6. `main` - 6 edges
7. `solve()` - 5 edges
8. `external_draw()` - 4 edges
9. `make_circle()` - 4 edges
10. `utils` - 4 edges

## Surprising Connections (you probably didn't know these)
- `describe()` --references--> `options`  [EXTRACTED]
  src/shapes.f90 → legacy/solver.f90
- `summarize()` --references--> `circle`  [EXTRACTED]
  app/main.f90 → src/shapes.f90
- `main` --calls--> `solve()`  [EXTRACTED]
  app/main.f90 → legacy/solver.f90
- `report_circle()` --calls--> `log_value()`  [EXTRACTED]
  src/geometry.f90 → utils.f90
- `make_circle()` --calls--> `log_value()`  [EXTRACTED]
  src/shapes.f90 → utils.f90

## Import Cycles
- None detected.

## Communities (8 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.29
Nodes (8): area_fn(), circle, circle_area(), describe(), external_draw(), geometry, utils, shapes

### Community 1 - "Community 1"
Cohesion: 0.32
Nodes (6): label, legacy_types, options, solve(), step(), trace()

### Community 2 - "Community 2"
Cohesion: 0.25
Nodes (5): report_circle(), make_circle(), label, log_value(), utils

### Community 3 - "Community 3"
Cohesion: 0.33
Nodes (6): geometry, main, print_summary(), print_summary_inner(), summarize(), shapes

### Community 4 - "Community 4"
Cohesion: 0.40
Nodes (4): constants, iso_fortran_env, geometry, point3

### Community 5 - "Community 5"
Cohesion: 0.40
Nodes (5): add_points(), origin(), point, point_norm(), shift()

### Community 7 - "Community 7"
Cohesion: 1.00
Nodes (3): distance(), distance_coords(), distance_points()

## Knowledge Gaps
- **9 isolated node(s):** `geometry`, `utils`, `label`, `label`, `geometry` (+4 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 20 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `make_circle()` connect `Community 2` to `Community 0`, `Community 5`?**
  _High betweenness centrality (0.457) - this node is a cross-community bridge._
- **What connects `geometry`, `utils`, `label` to the rest of the system?**
  _9 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `circle` connect `Community 0` to `Community 2`, `Community 3`?**
  _High betweenness centrality (0.381) - this node is a cross-community bridge._
- **Why does `point` connect `Community 5` to `Community 2`, `Community 4`, `Community 7`?**
  _High betweenness centrality (0.290) - this node is a cross-community bridge._
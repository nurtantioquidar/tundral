# Graph Report - fortran  (2026-10-14)

## Corpus Check
- 6 files · ~658 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 49 nodes · 64 edges · 9 communities (6 shown, 3 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- shapes
- main
- solver.f90
- geometry
- point
- utils
- constants
- distance_points
- report_circle

## God Nodes (most connected - your core abstractions)
1. `geometry` - 13 edges
2. `shapes` - 9 edges
3. `point` - 7 edges
4. `circle` - 7 edges
5. `describe()` - 6 edges
6. `main` - 5 edges
7. `distance_points()` - 4 edges
8. `external_draw()` - 4 edges
9. `utils` - 4 edges
10. `summarize()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `describe()` --references--> `options`  [EXTRACTED]
  src/shapes.f90 → legacy/solver.f90
- `summarize()` --references--> `circle`  [EXTRACTED]
  app/main.f90 → src/shapes.f90
- `make_circle()` --references--> `point`  [EXTRACTED]
  src/shapes.f90 → src/geometry.f90

## Import Cycles
- None detected.

## Communities (9 total, 3 thin omitted)

### Community 0 - "shapes"
Cohesion: 0.27
Nodes (9): area_fn(), circle, circle_area(), describe(), external_draw(), geometry, utils, make_circle() (+1 more)

### Community 1 - "main"
Cohesion: 0.33
Nodes (6): geometry, main, print_summary(), print_summary_inner(), summarize(), shapes

### Community 2 - "solver.f90"
Cohesion: 0.38
Nodes (6): label, legacy_types, options, solve(), step(), trace()

### Community 3 - "geometry"
Cohesion: 0.40
Nodes (4): constants, iso_fortran_env, geometry, point3

### Community 4 - "point"
Cohesion: 0.40
Nodes (5): add_points(), origin(), point, point_norm(), shift()

### Community 7 - "distance_points"
Cohesion: 1.00
Nodes (3): distance(), distance_coords(), distance_points()

## Knowledge Gaps
- **9 isolated node(s):** `geometry`, `shapes`, `label`, `constants`, `iso_fortran_env` (+4 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 20 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `make_circle()` connect `shapes` to `point`?**
  _High betweenness centrality (0.332) - this node is a cross-community bridge._
- **What connects `geometry`, `shapes`, `label` to the rest of the system?**
  _9 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `point` connect `point` to `shapes`, `geometry`, `distance_points`?**
  _High betweenness centrality (0.327) - this node is a cross-community bridge._
- **Why does `circle` connect `shapes` to `main`?**
  _High betweenness centrality (0.303) - this node is a cross-community bridge._
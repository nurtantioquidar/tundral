# Graph Report - cpp  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 82 nodes · 115 edges · 8 communities (7 shown, 1 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 5 edges (avg confidence: 0.85)
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
1. `Shape` - 10 edges
2. `Registry` - 8 edges
3. `Rect` - 7 edges
4. `Point` - 6 edges
5. `Kind` - 6 edges
6. `Circle` - 6 edges
7. `main()` - 6 edges
8. `Derived` - 5 edges
9. `use_point()` - 5 edges
10. `total_area()` - 5 edges

## Surprising Connections (you probably didn't know these)
- `geo::Point::dist()` --references--> `Point`  [EXTRACTED]
  src/shape.cpp → include/geo/point.h
- `total_area()` --references--> `Registry`  [EXTRACTED]
  src/shape.cpp → include/geo/shape.hpp
- `Shape::Shape()` --references--> `Kind`  [EXTRACTED]
  src/shape.cpp → include/geo/shape.hpp
- `print_area()` --references--> `Shape`  [EXTRACTED]
  src/main.cc → include/geo/shape.hpp
- `total_area()` --references--> `Shape`  [EXTRACTED]
  src/shape.cpp → include/geo/shape.hpp

## Import Cycles
- None detected.

## Communities (8 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.15
Nodes (9): Bits, f, u, Point, x, y, Points, Vec2 (+1 more)

### Community 1 - "Community 1"
Cohesion: 0.18
Nodes (3): Registry, items_, Shape::name()

### Community 2 - "Community 2"
Cohesion: 0.26
Nodes (8): Base, clamp_to(), Counter, n, Derived, counter, legacy_add(), use_point()

### Community 3 - "Community 3"
Cohesion: 0.24
Nodes (6): Kind, Circle, Rect, Shape, Shape::Shape(), total_area()

### Community 4 - "Community 4"
Cohesion: 0.31
Nodes (4): launch_scale(), helper(), main(), print_area()

### Community 6 - "Community 6"
Cohesion: 0.29
Nodes (5): Circle, center_, r_, main(), test_circle()

### Community 7 - "Community 7"
Cohesion: 0.29
Nodes (5): Named, label, Rect, h_, w_

## Knowledge Gaps
- **14 isolated node(s):** `f`, `u`, `x`, `y`, `items_` (+9 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 34 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Registry` connect `Community 1` to `Community 3`?**
  _High betweenness centrality (0.146) - this node is a cross-community bridge._
- **What connects `f`, `u`, `x` to the rest of the system?**
  _14 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Shape` connect `Community 3` to `Community 1`, `Community 4`, `Community 6`, `Community 7`?**
  _High betweenness centrality (0.116) - this node is a cross-community bridge._
- **Why does `Rect` connect `Community 7` to `Community 1`, `Community 3`?**
  _High betweenness centrality (0.108) - this node is a cross-community bridge._
# Graph Report - cpp  (2026-10-14)

## Corpus Check
- 8 files · ~656 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 84 nodes · 118 edges · 7 communities (6 shown, 1 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 5 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- legacy.cxx
- Point
- point.h
- main.cc
- shape.hpp
- shape.cpp
- Shape

## God Nodes (most connected - your core abstractions)
1. `Shape` - 11 edges
2. `Registry` - 9 edges
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
- `Shape::Shape()` --references--> `Kind`  [EXTRACTED]
  src/shape.cpp → include/geo/shape.hpp
- `print_area()` --references--> `Shape`  [EXTRACTED]
  src/main.cc → include/geo/shape.hpp
- `total_area()` --references--> `Registry`  [EXTRACTED]
  src/shape.cpp → include/geo/shape.hpp
- `total_area()` --references--> `Shape`  [EXTRACTED]
  src/shape.cpp → include/geo/shape.hpp

## Import Cycles
- None detected.

## Communities (7 total, 1 thin omitted)

### Community 0 - "legacy.cxx"
Cohesion: 0.21
Nodes (8): Base, clamp_to(), Counter, n, Derived, counter, legacy_add(), use_point()

### Community 1 - "Point"
Cohesion: 0.17
Nodes (9): Point, x, y, Circle, center_, r_, geo::Point::dist(), main() (+1 more)

### Community 2 - "point.h"
Cohesion: 0.17
Nodes (8): Bits, f, u, Named, label, Rect, h_, w_

### Community 3 - "main.cc"
Cohesion: 0.21
Nodes (5): launch_scale(), scale_kernel(), helper(), main(), print_area()

### Community 4 - "shape.hpp"
Cohesion: 0.25
Nodes (3): Registry, items_, Registry<T>::first()

### Community 6 - "Shape"
Cohesion: 0.24
Nodes (6): Kind, Circle, Rect, Shape, Shape::Shape(), total_area()

## Knowledge Gaps
- **14 isolated node(s):** `x`, `y`, `label`, `f`, `u` (+9 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 33 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Circle` connect `Point` to `shape.hpp`, `Shape`?**
  _High betweenness centrality (0.208) - this node is a cross-community bridge._
- **What connects `x`, `y`, `label` to the rest of the system?**
  _14 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Shape` connect `Shape` to `Point`, `point.h`, `main.cc`, `shape.hpp`, `shape.cpp`?**
  _High betweenness centrality (0.194) - this node is a cross-community bridge._
- **Why does `Registry` connect `shape.hpp` to `Shape`?**
  _High betweenness centrality (0.157) - this node is a cross-community bridge._
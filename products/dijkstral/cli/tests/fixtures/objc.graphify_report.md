# Graph Report - objc  (2026-10-29)

## Corpus Check
- 9 files · ~355 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 41 nodes · 64 edges · 9 communities (5 shown, 4 thin omitted)
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 5 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Repo
- Shape.h
- Circle
- Shape
- Bridge.mm
- <Drawable>
- +shapeNamed
- -refresh

## God Nodes (most connected - your core abstractions)
1. `Shape` - 14 edges
2. `Repo` - 8 edges
3. `Circle` - 7 edges
4. `-report` - 6 edges
5. `<Drawable>` - 5 edges
6. `-refresh` - 5 edges
7. `-find` - 5 edges
8. `ShapeKind` - 4 edges
9. `+shapeNamed` - 4 edges
10. `Bridge` - 3 edges

## Surprising Connections (you probably didn't know these)
- `-run` --calls--> `-report`  [INFERRED]
  Shop/Bridge.mm → Shop/Services/Repo.h
- `-report` --calls--> `-area`  [INFERRED]
  Shop/Services/Repo.h → Shop/Models/Circle.h
- `-report` --calls--> `-draw`  [INFERRED]
  Shop/Services/Repo.h → Shop/Models/Circle.m
- `-add` --calls--> `-refresh`  [INFERRED]
  Shop/Services/Repo.h → Shop/Models/Shape.m
- `-run` --references--> `Repo`  [EXTRACTED]
  Shop/Bridge.mm → Shop/Services/Repo.h

## Import Cycles
- None detected.

## Communities (9 total, 4 thin omitted)

### Community 1 - "Shape.h"
Cohesion: 0.29
Nodes (3): ShapeKind, ShapeKindCircle, ShapeKindSquare

### Community 2 - "Circle"
Cohesion: 0.53
Nodes (4): Circle, -area, -draw, -report

### Community 3 - "Shape"
Cohesion: 0.50
Nodes (3): Shape, -area, -find

### Community 5 - "<Drawable>"
Cohesion: 0.50
Nodes (3): <Drawable>, -caption, -draw

### Community 7 - "-refresh"
Cohesion: 0.67
Nodes (3): -draw, -refresh, -add

## Knowledge Gaps
- **4 isolated node(s):** `-draw`, `-caption`, `ShapeKindCircle`, `ShapeKindSquare`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 13 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Shape` connect `Shape` to `Shape.h`, `Circle`, `<Drawable>`, `+shapeNamed`, `-refresh`?**
  _High betweenness centrality (0.402) - this node is a cross-community bridge._
- **Are the 3 inferred relationships involving `-report` (e.g. with `-run` and `-area`) actually correct?**
  _`-report` has 3 INFERRED edges - model-reasoned connections that need verification._
- **What connects `-draw`, `-caption`, `ShapeKindCircle` to the rest of the system?**
  _4 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Repo` connect `Repo` to `Circle`, `Shape`, `Bridge.mm`, `-refresh`?**
  _High betweenness centrality (0.186) - this node is a cross-community bridge._
- **Why does `Circle` connect `Circle` to `Repo`, `Shape`?**
  _High betweenness centrality (0.149) - this node is a cross-community bridge._
# Graph Report - misc  (2026-10-15)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 110 nodes · 109 edges · 20 communities (10 shown, 10 thin omitted)
- Extraction: 90% EXTRACTED · 10% INFERRED · 0% AMBIGUOUS · INFERRED: 11 edges (avg confidence: 0.55)
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
- Community 8
- Community 9
- Community 10
- Community 11
- Community 12
- Community 13

## God Nodes (most connected - your core abstractions)
1. `AccountService` - 15 edges
2. `Shapes` - 10 edges
3. `BILLING` - 9 edges
4. `TShape` - 9 edges
5. `step()` - 5 edges
6. `Particle` - 4 edges
7. `MAIN-PARA` - 4 edges
8. `Demo` - 4 edges
9. `TCircle` - 4 edges
10. `Grow()` - 4 edges

## Surprising Connections (you probably didn't know these)
- `Area()` --calls--> `Compute()`  [EXTRACTED]
  pascal/Shapes.pas → pascal/Shapes.pas  _Bridges community 1 → community 2_

## Import Cycles
- None detected.

## Communities (20 total, 10 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.14
Nodes (10): AccountService, Account, Notifier, Status, BaseService, Contact, Database, insert (+2 more)

### Community 1 - "Community 1"
Cohesion: 0.20
Nodes (5): Demo, Compute(), Log(), Register(), Shapes

### Community 2 - "Community 2"
Cohesion: 0.27
Nodes (10): IDrawable, TCircle, Area(), Grow(), TShape, Create(), Draw(), Paint() (+2 more)

### Community 3 - "Community 3"
Cohesion: 0.27
Nodes (9): BILLING, WS-NAME, WS-REC, WS-TOTAL, CALC-EXIT, CALC-PARA, CALC-SECTION, INIT-PARA (+1 more)

### Community 4 - "Community 4"
Cohesion: 0.24
Nodes (5): Particle, x, y, speed(), step()

### Community 5 - "Community 5"
Cohesion: 0.22
Nodes (4): TButton, TMainForm, TMemo, TPanel

### Community 6 - "Community 6"
Cohesion: 0.33
Nodes (5): elem "info" [CHILD], elem "mainwindow" [MAIN], elem "map" [MAP], window "infowindow", window "mainwindow"

### Community 7 - "Community 7"
Cohesion: 0.40
Nodes (3): Geometry, TBaseShape, TObject

### Community 8 - "Community 8"
Cohesion: 0.50
Nodes (3): AccountTrigger, Account, upsert

### Community 9 - "Community 9"
Cohesion: 0.50
Nodes (4): Extra, LCL, Widgets, WidgetsPkg

## Knowledge Gaps
- **15 isolated node(s):** `Account`, `BaseService`, `Contact`, `Database`, `insert` (+10 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 64 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **10 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Shapes` connect `Community 1` to `Community 2`, `Community 7`?**
  _High betweenness centrality (0.046) - this node is a cross-community bridge._
- **Are the 7 inferred relationships involving `AccountService` (e.g. with `Account` and `BaseService`) actually correct?**
  _`AccountService` has 7 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Account`, `BaseService`, `Contact` to the rest of the system?**
  _15 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.14166666666666666 - nodes in this community are weakly interconnected._
- **Why does `TShape` connect `Community 2` to `Community 1`?**
  _High betweenness centrality (0.026) - this node is a cross-community bridge._
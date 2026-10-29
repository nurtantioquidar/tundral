# Graph Report - misc  (2026-10-29)

## Corpus Check
- 25 files · ~657 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 111 nodes · 108 edges · 22 communities (10 shown, 12 thin omitted)
- Extraction: 90% EXTRACTED · 10% INFERRED · 0% AMBIGUOUS · INFERRED: 11 edges (avg confidence: 0.5)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- AccountService
- TShape
- Shapes
- BILLING
- TMainForm
- kernels.metal
- window "mainwindow"
- WidgetsPkg
- TBaseShape
- AccountTrigger
- icons.dmi
- record.cpy
- TImage
- Widgets

## God Nodes (most connected - your core abstractions)
1. `AccountService` - 15 edges
2. `Shapes` - 10 edges
3. `BILLING` - 9 edges
4. `TShape` - 9 edges
5. `MAIN-PARA` - 4 edges
6. `Particle` - 4 edges
7. `Demo` - 4 edges
8. `TMainForm` - 4 edges
9. `TCircle` - 4 edges
10. `Grow()` - 4 edges

## Surprising Connections (you probably didn't know these)
- `Compute()` --calls--> `Register()`  [EXTRACTED]
  pascal/Shapes.pas → pascal/Shapes.pas  _Bridges community 2 → community 1_

## Import Cycles
- None detected.

## Communities (22 total, 12 thin omitted)

### Community 0 - "AccountService"
Cohesion: 0.14
Nodes (10): AccountService, Account, Notifier, Status, BaseService, Contact, Database, insert (+2 more)

### Community 1 - "TShape"
Cohesion: 0.24
Nodes (11): IDrawable, Compute(), TCircle, Area(), Grow(), TShape, Create(), Draw() (+3 more)

### Community 2 - "Shapes"
Cohesion: 0.20
Nodes (4): Demo, Log(), Register(), Shapes

### Community 3 - "BILLING"
Cohesion: 0.27
Nodes (9): BILLING, WS-NAME, WS-REC, WS-TOTAL, CALC-EXIT, CALC-PARA, CALC-SECTION, INIT-PARA (+1 more)

### Community 4 - "TMainForm"
Cohesion: 0.22
Nodes (4): TButton, TMainForm, TMemo, TPanel

### Community 5 - "kernels.metal"
Cohesion: 0.32
Nodes (5): Particle, x, y, speed(), step()

### Community 6 - "window "mainwindow""
Cohesion: 0.33
Nodes (5): elem "info" [CHILD], elem "mainwindow" [MAIN], elem "map" [MAP], window "infowindow", window "mainwindow"

### Community 7 - "WidgetsPkg"
Cohesion: 0.40
Nodes (4): Extra, LCL, Widgets, WidgetsPkg

### Community 8 - "TBaseShape"
Cohesion: 0.40
Nodes (3): Geometry, TBaseShape, TObject

### Community 9 - "AccountTrigger"
Cohesion: 0.50
Nodes (3): AccountTrigger, Account, upsert

## Knowledge Gaps
- **15 isolated node(s):** `BaseService`, `Queueable`, `Database`, `Account`, `update` (+10 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 65 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **12 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Shapes` connect `Shapes` to `TBaseShape`, `TShape`?**
  _High betweenness centrality (0.045) - this node is a cross-community bridge._
- **Are the 7 inferred relationships involving `AccountService` (e.g. with `Account` and `BaseService`) actually correct?**
  _`AccountService` has 7 INFERRED edges - model-reasoned connections that need verification._
- **What connects `BaseService`, `Queueable`, `Database` to the rest of the system?**
  _15 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `AccountService` be split into smaller, more focused modules?**
  _Cohesion score 0.14166666666666666 - nodes in this community are weakly interconnected._
- **Why does `TShape` connect `TShape` to `Shapes`?**
  _High betweenness centrality (0.025) - this node is a cross-community bridge._
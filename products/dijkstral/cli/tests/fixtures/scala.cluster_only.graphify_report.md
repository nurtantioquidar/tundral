# Graph Report - co  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 94 nodes · 150 edges · 9 communities (5 shown, 4 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 1 edges (avg confidence: 0.85)
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

## God Nodes (most connected - your core abstractions)
1. `User` - 14 edges
2. `UserService` - 12 edges
3. `Model` - 7 edges
4. `InMemoryRepository` - 7 edges
5. `Address` - 5 edges
6. `Auditable` - 5 edges
7. `Repository` - 5 edges
8. `Entity` - 4 edges
9. `Status` - 4 edges
10. `Logging` - 4 edges

## Surprising Connections (you probably didn't know these)
- `User` --inherits--> `Model`  [EXTRACTED]
  src/main/scala/shop/model/User.scala → src/main/scala/shop/model/Model.scala
- `User` --references--> `Status`  [EXTRACTED]
  src/main/scala/shop/model/User.scala → src/main/scala/shop/model/Model.scala
- `UserService` --inherits--> `Auditable`  [EXTRACTED]
  src/main/scala/shop/service/UserService.scala → src/main/scala/shop/service/Audit.scala
- `InMemoryRepository` --inherits--> `Repository`  [EXTRACTED]
  src/main/scala/shop/service/Audit.scala → src/main/scala/shop/service/UserService.scala

## Import Cycles
- None detected.

## Communities (9 total, 4 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.16
Nodes (8): Active, Color, Green, Red, Entity, Inactive, Model, Status

### Community 1 - "Community 1"
Cohesion: 0.22
Nodes (3): Address, User, UserOps

### Community 2 - "Community 2"
Cohesion: 0.23
Nodes (4): Auditable, Audited, InMemoryRepository, Logging

### Community 3 - "Community 3"
Cohesion: 0.15
Nodes (3): Main, Formatter, UserServiceSuite

### Community 5 - "Community 5"
Cohesion: 0.39
Nodes (3): Formatter, slugify(), words()

## Knowledge Gaps
- **2 isolated node(s):** `Green`, `Red`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 19 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `Community 1` to `Community 0`, `Community 3`?**
  _High betweenness centrality (0.357) - this node is a cross-community bridge._
- **What connects `Green`, `Red` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `UserService` connect `Community 4` to `Community 8`, `Community 2`, `Community 6`, `Community 7`?**
  _High betweenness centrality (0.256) - this node is a cross-community bridge._
- **Why does `Address` connect `Community 1` to `Community 4`?**
  _High betweenness centrality (0.174) - this node is a cross-community bridge._
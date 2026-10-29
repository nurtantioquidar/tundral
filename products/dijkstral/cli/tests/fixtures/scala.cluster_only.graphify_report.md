# Graph Report - scala  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 91 nodes · 145 edges · 6 communities
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 1 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5

## God Nodes (most connected - your core abstractions)
1. `User` - 14 edges
2. `UserService` - 12 edges
3. `Model` - 7 edges
4. `InMemoryRepository` - 7 edges
5. `Repository` - 5 edges
6. `Address` - 5 edges
7. `Auditable` - 5 edges
8. `Entity` - 4 edges
9. `Status` - 4 edges
10. `Logging` - 4 edges

## Surprising Connections (you probably didn't know these)
- `InMemoryRepository` --inherits--> `Repository`  [EXTRACTED]
  src/main/scala/shop/service/Audit.scala → src/main/scala/shop/service/UserService.scala
- `UserService` --inherits--> `Auditable`  [EXTRACTED]
  src/main/scala/shop/service/UserService.scala → src/main/scala/shop/service/Audit.scala
- `User` --inherits--> `Model`  [EXTRACTED]
  src/main/scala/shop/model/User.scala → src/main/scala/shop/model/Model.scala
- `User` --references--> `Status`  [EXTRACTED]
  src/main/scala/shop/model/User.scala → src/main/scala/shop/model/Model.scala

## Import Cycles
- None detected.

## Communities (6 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.17
Nodes (5): helper(), Registry, Repository, Stats, UserService

### Community 1 - "Community 1"
Cohesion: 0.16
Nodes (8): Active, Color, Green, Red, Entity, Inactive, Model, Status

### Community 2 - "Community 2"
Cohesion: 0.22
Nodes (3): Address, User, UserOps

### Community 3 - "Community 3"
Cohesion: 0.15
Nodes (3): Main, Formatter, UserServiceSuite

### Community 4 - "Community 4"
Cohesion: 0.24
Nodes (4): Auditable, Audited, InMemoryRepository, Logging

### Community 5 - "Community 5"
Cohesion: 0.39
Nodes (3): Formatter, slugify(), words()

## Knowledge Gaps
- **2 isolated node(s):** `Green`, `Red`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 19 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `Community 2` to `Community 1`, `Community 3`?**
  _High betweenness centrality (0.367) - this node is a cross-community bridge._
- **What connects `Green`, `Red` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `UserService` connect `Community 0` to `Community 4`?**
  _High betweenness centrality (0.261) - this node is a cross-community bridge._
- **Why does `Address` connect `Community 2` to `Community 0`?**
  _High betweenness centrality (0.179) - this node is a cross-community bridge._
# Graph Report - co  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 74 nodes · 112 edges · 9 communities (5 shown, 4 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
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
1. `UserService` - 14 edges
2. `User` - 14 edges
3. `InMemoryRepository` - 8 edges
4. `Entity` - 5 edges
5. `Model` - 5 edges
6. `Status` - 5 edges
7. `UserServiceTest` - 5 edges
8. `Repository` - 5 edges
9. `Runnable` - 4 edges
10. `Address` - 4 edges

## Surprising Connections (you probably didn't know these)
- `UserServiceTest` --calls--> `UserService`  [INFERRED]
  src/test/kotlin/com/shop/service/UserServiceTest.kt → src/main/kotlin/com/shop/service/UserService.kt
- `UserServiceTest` --calls--> `InMemoryRepository`  [INFERRED]
  src/test/kotlin/com/shop/service/UserServiceTest.kt → src/main/kotlin/com/shop/service/Audit.kt
- `User` --inherits--> `Model`  [EXTRACTED]
  src/main/kotlin/com/shop/model/User.kt → src/main/kotlin/com/shop/model/Model.kt
- `User` --references--> `Status`  [EXTRACTED]
  src/main/kotlin/com/shop/model/User.kt → src/main/kotlin/com/shop/model/Model.kt
- `UserService` --implements--> `Auditable`  [EXTRACTED]
  src/main/kotlin/com/shop/service/UserService.kt → src/main/kotlin/com/shop/service/Audit.kt

## Import Cycles
- None detected.

## Communities (9 total, 4 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.15
Nodes (8): Entity, Fail, Model, Ok, Result, Status, ACTIVE, INACTIVE

### Community 1 - "Community 1"
Cohesion: 0.26
Nodes (4): Stats, UserService, Runnable, main()

### Community 2 - "Community 2"
Cohesion: 0.31
Nodes (3): Address, displayName(), User

### Community 3 - "Community 3"
Cohesion: 0.32
Nodes (3): Auditable, Audited, InMemoryRepository

### Community 4 - "Community 4"
Cohesion: 0.29
Nodes (3): Formatter, second(), slugify()

## Knowledge Gaps
- **2 isolated node(s):** `ACTIVE`, `INACTIVE`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 19 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `UserService` connect `Community 1` to `Community 3`, `Community 5`, `Community 6`, `Community 7`?**
  _High betweenness centrality (0.351) - this node is a cross-community bridge._
- **What connects `ACTIVE`, `INACTIVE` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `User` connect `Community 2` to `Community 0`, `Community 5`, `Community 6`?**
  _High betweenness centrality (0.307) - this node is a cross-community bridge._
- **Why does `slugify()` connect `Community 4` to `Community 1`?**
  _High betweenness centrality (0.169) - this node is a cross-community bridge._
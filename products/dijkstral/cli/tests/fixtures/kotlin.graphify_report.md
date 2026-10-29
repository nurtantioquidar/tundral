# Graph Report - kotlin  (2026-10-29)

## Corpus Check
- 8 files · ~540 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 71 nodes · 106 edges · 8 communities (4 shown, 4 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Model.kt
- InMemoryRepository
- UserService
- User
- UserServiceTest
- Strings.kt
- UserService.kt

## God Nodes (most connected - your core abstractions)
1. `User` - 14 edges
2. `UserService` - 14 edges
3. `InMemoryRepository` - 7 edges
4. `Model` - 5 edges
5. `Status` - 5 edges
6. `Repository` - 5 edges
7. `UserServiceTest` - 5 edges
8. `Entity` - 4 edges
9. `Address` - 4 edges
10. `Runnable` - 4 edges

## Surprising Connections (you probably didn't know these)
- `UserServiceTest` --calls--> `InMemoryRepository`  [INFERRED]
  src/test/kotlin/com/shop/service/UserServiceTest.kt → src/main/kotlin/com/shop/service/Audit.kt
- `UserServiceTest` --calls--> `UserService`  [INFERRED]
  src/test/kotlin/com/shop/service/UserServiceTest.kt → src/main/kotlin/com/shop/service/UserService.kt
- `main()` --calls--> `UserService`  [EXTRACTED]
  src/main/kotlin/Main.kt → src/main/kotlin/com/shop/service/UserService.kt
- `User` --inherits--> `Model`  [EXTRACTED]
  src/main/kotlin/com/shop/model/User.kt → src/main/kotlin/com/shop/model/Model.kt
- `User` --references--> `Status`  [EXTRACTED]
  src/main/kotlin/com/shop/model/User.kt → src/main/kotlin/com/shop/model/Model.kt

## Import Cycles
- None detected.

## Communities (8 total, 4 thin omitted)

### Community 0 - "Model.kt"
Cohesion: 0.15
Nodes (9): Entity, Fail, Model, Ok, Result, Status, ACTIVE, INACTIVE (+1 more)

### Community 1 - "InMemoryRepository"
Cohesion: 0.18
Nodes (5): Auditable, Audited, InMemoryRepository, Repository, main()

### Community 2 - "UserService"
Cohesion: 0.31
Nodes (3): Stats, UserService, Runnable

### Community 3 - "User"
Cohesion: 0.31
Nodes (3): Address, displayName(), User

## Knowledge Gaps
- **2 isolated node(s):** `ACTIVE`, `INACTIVE`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 23 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `UserService` connect `UserService` to `InMemoryRepository`, `UserServiceTest`, `UserService.kt`?**
  _High betweenness centrality (0.351) - this node is a cross-community bridge._
- **What connects `ACTIVE`, `INACTIVE` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `User` connect `User` to `Model.kt`, `UserServiceTest`, `UserService.kt`?**
  _High betweenness centrality (0.321) - this node is a cross-community bridge._
- **Why does `slugify()` connect `Strings.kt` to `UserService`?**
  _High betweenness centrality (0.152) - this node is a cross-community bridge._
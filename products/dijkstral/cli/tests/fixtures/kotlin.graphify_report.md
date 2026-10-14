# Graph Report - up  (2026-10-14)

## Corpus Check
- 8 files · ~540 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 74 nodes · 112 edges · 9 communities (5 shown, 4 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Model.kt
- UserService
- User
- InMemoryRepository
- Strings.kt
- UserServiceTest
- UserService.kt
- Repository

## God Nodes (most connected - your core abstractions)
1. `User` - 14 edges
2. `UserService` - 14 edges
3. `InMemoryRepository` - 8 edges
4. `Entity` - 5 edges
5. `Model` - 5 edges
6. `Status` - 5 edges
7. `Repository` - 5 edges
8. `UserServiceTest` - 5 edges
9. `Address` - 4 edges
10. `Runnable` - 4 edges

## Surprising Connections (you probably didn't know these)
- `UserServiceTest` --calls--> `InMemoryRepository`  [INFERRED]
  src/test/kotlin/com/shop/service/UserServiceTest.kt → src/main/kotlin/com/shop/service/Audit.kt
- `UserServiceTest` --calls--> `UserService`  [INFERRED]
  src/test/kotlin/com/shop/service/UserServiceTest.kt → src/main/kotlin/com/shop/service/UserService.kt
- `main()` --calls--> `InMemoryRepository`  [EXTRACTED]
  src/main/kotlin/Main.kt → src/main/kotlin/com/shop/service/Audit.kt
- `User` --inherits--> `Model`  [EXTRACTED]
  src/main/kotlin/com/shop/model/User.kt → src/main/kotlin/com/shop/model/Model.kt
- `User` --references--> `Status`  [EXTRACTED]
  src/main/kotlin/com/shop/model/User.kt → src/main/kotlin/com/shop/model/Model.kt

## Import Cycles
- None detected.

## Communities (9 total, 4 thin omitted)

### Community 0 - "Model.kt"
Cohesion: 0.15
Nodes (8): Entity, Fail, Model, Ok, Result, Status, ACTIVE, INACTIVE

### Community 1 - "UserService"
Cohesion: 0.26
Nodes (4): Stats, UserService, Runnable, main()

### Community 2 - "User"
Cohesion: 0.31
Nodes (3): Address, displayName(), User

### Community 3 - "InMemoryRepository"
Cohesion: 0.32
Nodes (3): Auditable, Audited, InMemoryRepository

### Community 4 - "Strings.kt"
Cohesion: 0.29
Nodes (3): Formatter, second(), slugify()

## Knowledge Gaps
- **2 isolated node(s):** `ACTIVE`, `INACTIVE`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 19 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `UserService` connect `UserService` to `InMemoryRepository`, `UserServiceTest`, `UserService.kt`, `Repository`?**
  _High betweenness centrality (0.351) - this node is a cross-community bridge._
- **What connects `ACTIVE`, `INACTIVE` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `User` connect `User` to `Model.kt`, `UserServiceTest`, `UserService.kt`?**
  _High betweenness centrality (0.307) - this node is a cross-community bridge._
- **Why does `slugify()` connect `Strings.kt` to `UserService`?**
  _High betweenness centrality (0.169) - this node is a cross-community bridge._
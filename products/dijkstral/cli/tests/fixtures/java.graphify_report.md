# Graph Report - java  (2026-10-13)

## Corpus Check
- 6 files · ~502 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 64 nodes · 93 edges · 7 communities (3 shown, 4 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 5 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- UserService
- User
- Model
- UserServiceTest.java
- Status
- Audited.java

## God Nodes (most connected - your core abstractions)
1. `User` - 13 edges
2. `UserService` - 11 edges
3. `Model` - 9 edges
4. `Status` - 6 edges
5. `Address` - 5 edges
6. `Formatter` - 5 edges
7. `UserServiceTest` - 5 edges
8. `Entity` - 4 edges
9. `Audited` - 3 edges
10. `Repository` - 3 edges

## Surprising Connections (you probably didn't know these)
- `User` --inherits--> `Model`  [EXTRACTED]
  src/main/java/com/shop/model/User.java → src/main/java/com/shop/model/Model.java
- `UserService` --references--> `User`  [EXTRACTED]
  src/main/java/com/shop/service/UserService.java → src/main/java/com/shop/model/User.java
- `UserServiceTest` --references--> `UserService`  [EXTRACTED]
  src/test/java/com/shop/service/UserServiceTest.java → src/main/java/com/shop/service/UserService.java
- `Model` --implements--> `Entity`  [EXTRACTED]
  src/main/java/com/shop/model/Model.java → src/main/java/com/shop/model/Entity.java

## Import Cycles
- None detected.

## Communities (7 total, 4 thin omitted)

### Community 0 - "UserService"
Cohesion: 0.17
Nodes (4): Formatter, Repository, Stats, UserService

### Community 1 - "User"
Cohesion: 0.24
Nodes (4): Address, Badge, User, Runnable

### Community 4 - "Status"
Cohesion: 0.33
Nodes (3): Status, ACTIVE, SUSPENDED

## Knowledge Gaps
- **2 isolated node(s):** `ACTIVE`, `SUSPENDED`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 23 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `User` to `UserService`, `Model`, `UserServiceTest.java`, `Status`, `User.java`?**
  _High betweenness centrality (0.413) - this node is a cross-community bridge._
- **What connects `ACTIVE`, `SUSPENDED` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Model` connect `Model` to `UserService`, `User`, `Status`?**
  _High betweenness centrality (0.384) - this node is a cross-community bridge._
- **Why does `UserService` connect `UserService` to `User`, `UserServiceTest.java`?**
  _High betweenness centrality (0.229) - this node is a cross-community bridge._
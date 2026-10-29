# Graph Report - java  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 64 nodes · 93 edges · 7 communities (3 shown, 4 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 5 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5

## God Nodes (most connected - your core abstractions)
1. `User` - 13 edges
2. `UserService` - 11 edges
3. `Model` - 9 edges
4. `Status` - 6 edges
5. `Formatter` - 5 edges
6. `Address` - 5 edges
7. `UserServiceTest` - 5 edges
8. `Entity` - 4 edges
9. `Repository` - 3 edges
10. `Formatter` - 3 edges

## Surprising Connections (you probably didn't know these)
- `UserService` --references--> `User`  [EXTRACTED]
  src/main/java/com/shop/service/UserService.java → src/main/java/com/shop/model/User.java
- `UserServiceTest` --references--> `UserService`  [EXTRACTED]
  src/test/java/com/shop/service/UserServiceTest.java → src/main/java/com/shop/service/UserService.java
- `User` --inherits--> `Model`  [EXTRACTED]
  src/main/java/com/shop/model/User.java → src/main/java/com/shop/model/Model.java
- `Model` --implements--> `Entity`  [EXTRACTED]
  src/main/java/com/shop/model/Model.java → src/main/java/com/shop/model/Entity.java

## Import Cycles
- None detected.

## Communities (7 total, 4 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.17
Nodes (4): Formatter, Repository, Stats, UserService

### Community 1 - "Community 1"
Cohesion: 0.24
Nodes (4): Address, Badge, User, Runnable

### Community 4 - "Community 4"
Cohesion: 0.33
Nodes (3): Status, ACTIVE, SUSPENDED

## Knowledge Gaps
- **2 isolated node(s):** `ACTIVE`, `SUSPENDED`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 23 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `Community 1` to `Community 0`, `Community 2`, `Community 3`, `Community 4`, `Community 6`?**
  _High betweenness centrality (0.413) - this node is a cross-community bridge._
- **What connects `ACTIVE`, `SUSPENDED` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Model` connect `Community 2` to `Community 0`, `Community 1`, `Community 4`?**
  _High betweenness centrality (0.384) - this node is a cross-community bridge._
- **Why does `UserService` connect `Community 0` to `Community 1`, `Community 3`?**
  _High betweenness centrality (0.229) - this node is a cross-community bridge._
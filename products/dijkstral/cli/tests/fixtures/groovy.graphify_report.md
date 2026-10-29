# Graph Report - groovy  (2026-10-29)

## Corpus Check
- 10 files · ~414 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 55 nodes · 70 edges · 10 communities (3 shown, 7 thin omitted)
- Extraction: 90% EXTRACTED · 10% INFERRED · 0% AMBIGUOUS · INFERRED: 7 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- UserService.groovy
- Base
- User
- UserServiceSpec.groovy
- VersionTask
- .getName
- Role
- .main
- Auditable

## God Nodes (most connected - your core abstractions)
1. `User` - 11 edges
2. `UserService` - 6 edges
3. `Role` - 5 edges
4. `VersionTask` - 4 edges
5. `Base` - 4 edges
6. `Named` - 4 edges
7. `UserServiceSpec` - 4 edges
8. `App` - 3 edges
9. `Repository` - 3 edges
10. `title()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `User` --inherits--> `Base`  [EXTRACTED]
  src/main/groovy/com/shop/model/User.groovy → src/main/groovy/com/shop/model/Base.groovy
- `User` --implements--> `Named`  [EXTRACTED]
  src/main/groovy/com/shop/model/User.groovy → src/main/groovy/com/shop/model/Base.groovy
- `User` --implements--> `Auditable`  [EXTRACTED]
  src/main/groovy/com/shop/model/User.groovy → src/main/java/com/shop/model/Auditable.java

## Import Cycles
- None detected.

## Communities (10 total, 7 thin omitted)

### Community 0 - "UserService.groovy"
Cohesion: 0.18
Nodes (3): Repository, UserService, Logger

### Community 3 - "UserServiceSpec.groovy"
Cohesion: 0.33
Nodes (3): UserServiceSpec, "lists names", "registers a user"

### Community 6 - "Role"
Cohesion: 0.40
Nodes (4): Role, ADMIN, GUEST, MEMBER

## Knowledge Gaps
- **5 isolated node(s):** `ADMIN`, `MEMBER`, `GUEST`, `"registers a user"`, `"lists names"`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 21 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **7 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `User` to `Auditable`, `Base`, `.getName`?**
  _High betweenness centrality (0.339) - this node is a cross-community bridge._
- **What connects `ADMIN`, `MEMBER`, `GUEST` to the rest of the system?**
  _5 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `UserService` connect `UserService.groovy` to `Base`?**
  _High betweenness centrality (0.187) - this node is a cross-community bridge._
- **Why does `Role` connect `Role` to `User`?**
  _High betweenness centrality (0.127) - this node is a cross-community bridge._
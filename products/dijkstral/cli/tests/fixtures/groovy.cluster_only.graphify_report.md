# Graph Report - co  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 55 nodes · 70 edges · 10 communities (3 shown, 7 thin omitted)
- Extraction: 90% EXTRACTED · 10% INFERRED · 0% AMBIGUOUS · INFERRED: 7 edges (avg confidence: 0.85)
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
1. `User` - 11 edges
2. `UserService` - 6 edges
3. `Role` - 5 edges
4. `Base` - 4 edges
5. `Named` - 4 edges
6. `VersionTask` - 4 edges
7. `UserServiceSpec` - 4 edges
8. `Repository` - 3 edges
9. `App` - 3 edges
10. `Auditable` - 3 edges

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

### Community 0 - "Community 0"
Cohesion: 0.18
Nodes (3): Repository, UserService, Logger

### Community 3 - "Community 3"
Cohesion: 0.33
Nodes (3): UserServiceSpec, "lists names", "registers a user"

### Community 6 - "Community 6"
Cohesion: 0.40
Nodes (4): Role, ADMIN, GUEST, MEMBER

## Knowledge Gaps
- **5 isolated node(s):** `"lists names"`, `"registers a user"`, `ADMIN`, `GUEST`, `MEMBER`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 21 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **7 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `Community 2` to `Community 8`, `Community 1`, `Community 5`?**
  _High betweenness centrality (0.339) - this node is a cross-community bridge._
- **What connects `"lists names"`, `"registers a user"`, `ADMIN` to the rest of the system?**
  _5 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `UserService` connect `Community 0` to `Community 1`?**
  _High betweenness centrality (0.187) - this node is a cross-community bridge._
- **Why does `Role` connect `Community 6` to `Community 2`?**
  _High betweenness centrality (0.127) - this node is a cross-community bridge._
# Graph Report - php  (2026-10-29)

## Corpus Check
- 8 files · ~459 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 50 nodes · 75 edges · 8 communities (2 shown, 6 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 4 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- UserService
- Model
- User
- UserServiceTest.php
- .make
- slugify
- Timestamps

## God Nodes (most connected - your core abstractions)
1. `User` - 14 edges
2. `UserService` - 10 edges
3. `Model` - 9 edges
4. `Repository` - 7 edges
5. `Status` - 6 edges
6. `UserServiceTest` - 4 edges
7. `Timestamps` - 3 edges
8. `slugify()` - 3 edges
9. `main()` - 2 edges
10. `helper()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `User` --inherits--> `Model`  [EXTRACTED]
  src/Models/User.php → src/Models/Model.php
- `User` --mixes_in--> `Timestamps`  [EXTRACTED]
  src/Models/User.php → src/Models/Repository.php
- `UserService` --references--> `Repository`  [EXTRACTED]
  src/Service/UserService.php → src/Models/Repository.php
- `User` --references--> `Status`  [EXTRACTED]
  src/Models/User.php → src/Models/Status.php

## Import Cycles
- None detected.

## Communities (8 total, 6 thin omitted)

### Community 0 - "UserService"
Cohesion: 0.33
Nodes (3): Repository, helper(), UserService

### Community 2 - "User"
Cohesion: 0.29
Nodes (4): Status, Active, Banned, User

## Knowledge Gaps
- **2 isolated node(s):** `Active`, `Banned`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 15 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **6 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `User` to `UserService`, `Model`, `UserServiceTest.php`, `.make`, `User.php`, `slugify`, `Timestamps`?**
  _High betweenness centrality (0.661) - this node is a cross-community bridge._
- **What connects `Active`, `Banned` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Model` connect `Model` to `UserService`, `User`?**
  _High betweenness centrality (0.239) - this node is a cross-community bridge._
- **Why does `Status` connect `User` to `UserService`?**
  _High betweenness centrality (0.143) - this node is a cross-community bridge._
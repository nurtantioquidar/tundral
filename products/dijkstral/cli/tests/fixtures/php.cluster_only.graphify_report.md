# Graph Report - php  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 50 nodes · 71 edges · 8 communities (2 shown, 6 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 1 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5
- Community 7

## God Nodes (most connected - your core abstractions)
1. `User` - 15 edges
2. `UserService` - 12 edges
3. `Model` - 9 edges
4. `Repository` - 7 edges
5. `Status` - 6 edges
6. `UserServiceTest` - 4 edges
7. `Timestamps` - 3 edges
8. `slugify()` - 3 edges
9. `main()` - 2 edges
10. `helper()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `main()` --calls--> `UserService`  [EXTRACTED]
  bin/app.php → src/Service/UserService.php
- `UserService` --references--> `Repository`  [EXTRACTED]
  src/Service/UserService.php → src/Models/Repository.php
- `User` --mixes_in--> `Timestamps`  [EXTRACTED]
  src/Models/User.php → src/Models/Repository.php
- `User` --inherits--> `Model`  [EXTRACTED]
  src/Models/User.php → src/Models/Model.php
- `User` --references--> `Status`  [EXTRACTED]
  src/Models/User.php → src/Models/Status.php

## Import Cycles
- None detected.

## Communities (8 total, 6 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.29
Nodes (3): main(), helper(), UserService

### Community 5 - "Community 5"
Cohesion: 0.40
Nodes (3): Status, Active, Banned

## Knowledge Gaps
- **2 isolated node(s):** `Active`, `Banned`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 18 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **6 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `Community 3` to `Community 0`, `Community 1`, `Community 2`, `Community 4`, `Community 5`, `Community 6`, `Community 7`?**
  _High betweenness centrality (0.692) - this node is a cross-community bridge._
- **What connects `Active`, `Banned` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Model` connect `Community 2` to `Community 1`, `Community 3`?**
  _High betweenness centrality (0.268) - this node is a cross-community bridge._
- **Why does `Status` connect `Community 5` to `Community 0`, `Community 3`?**
  _High betweenness centrality (0.158) - this node is a cross-community bridge._
# Graph Report - swift  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 88 nodes · 168 edges · 7 communities
- Extraction: 94% EXTRACTED · 6% INFERRED · 0% AMBIGUOUS · INFERRED: 10 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4
- Community 5
- Community 6

## God Nodes (most connected - your core abstractions)
1. `User` - 32 edges
2. `UserService` - 11 edges
3. `Role` - 11 edges
4. `UserRepository` - 10 edges
5. `Payment` - 9 edges
6. `Circle` - 9 edges
7. `Cache` - 6 edges
8. `ShopError` - 6 edges
9. `Shape` - 6 edges
10. `UserServiceTests` - 6 edges

## Surprising Connections (you probably didn't know these)
- `UserServiceTests` --references--> `UserService`  [EXTRACTED]
  Tests/ShopTests/UserServiceTests.swift → Sources/Shop/Services/UserService.swift
- `run()` --calls--> `log()`  [INFERRED]
  Sources/App/main.swift → Sources/Shop/Services/UserService.swift
- `User` --references--> `String`  [EXTRACTED]
  Sources/Shop/Models/User.swift → Sources/Shop/Extensions/User+Format.swift
- `User` --implements--> `Describable`  [EXTRACTED]
  Sources/Shop/Models/User.swift → Sources/Shop/Models/Entity.swift
- `User` --implements--> `Entity`  [EXTRACTED]
  Sources/Shop/Models/User.swift → Sources/Shop/Models/Entity.swift

## Import Cycles
- None detected.

## Communities (7 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.18
Nodes (5): .description, .summary, User, .displayName, log()

### Community 1 - "Community 1"
Cohesion: 0.17
Nodes (6): String, Payment, failed, paid, pending, Cache

### Community 2 - "Community 2"
Cohesion: 0.19
Nodes (7): Combine, Repository, ShopError, invalid, missing, UserRepository, UserService

### Community 3 - "Community 3"
Cohesion: 0.20
Nodes (6): run(), Box, Circle, .summary, Pair, Shape

### Community 4 - "Community 4"
Cohesion: 0.20
Nodes (4): Foundation, Array, Describable, Entity

### Community 5 - "Community 5"
Cohesion: 0.25
Nodes (6): Address, Role, admin, customer, guest, .isPrivileged

### Community 6 - "Community 6"
Cohesion: 0.29
Nodes (3): Shop, UserServiceTests, XCTest

## Knowledge Gaps
- **13 isolated node(s):** `.displayName`, `failed`, `paid`, `pending`, `Combine` (+8 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 25 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `String` connect `Community 1` to `Community 0`, `Community 2`, `Community 3`, `Community 4`, `Community 5`?**
  _High betweenness centrality (0.417) - this node is a cross-community bridge._
- **What connects `.displayName`, `failed`, `paid` to the rest of the system?**
  _13 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `User` connect `Community 0` to `Community 1`, `Community 2`, `Community 4`, `Community 5`?**
  _High betweenness centrality (0.320) - this node is a cross-community bridge._
- **Why does `Payment` connect `Community 1` to `Community 5`, `Community 6`?**
  _High betweenness centrality (0.132) - this node is a cross-community bridge._
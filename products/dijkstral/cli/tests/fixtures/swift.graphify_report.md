# Graph Report - swift  (2026-10-29)

## Corpus Check
- 7 files · ~801 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 88 nodes · 168 edges · 7 communities
- Extraction: 94% EXTRACTED · 6% INFERRED · 0% AMBIGUOUS · INFERRED: 10 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- User
- String
- UserService
- Circle
- Foundation
- Role
- UserServiceTests

## God Nodes (most connected - your core abstractions)
1. `User` - 32 edges
2. `Role` - 11 edges
3. `UserService` - 11 edges
4. `UserRepository` - 10 edges
5. `Circle` - 9 edges
6. `Payment` - 9 edges
7. `Shape` - 6 edges
8. `ShopError` - 6 edges
9. `Cache` - 6 edges
10. `UserServiceTests` - 6 edges

## Surprising Connections (you probably didn't know these)
- `UserServiceTests` --references--> `UserService`  [EXTRACTED]
  Tests/ShopTests/UserServiceTests.swift → Sources/Shop/Services/UserService.swift
- `run()` --calls--> `log()`  [INFERRED]
  Sources/App/main.swift → Sources/Shop/Services/UserService.swift
- `run()` --calls--> `Circle`  [INFERRED]
  Sources/App/main.swift → Sources/Shop/Models/Shape.swift
- `Circle` --references--> `String`  [EXTRACTED]
  Sources/Shop/Models/Shape.swift → Sources/Shop/Extensions/User+Format.swift
- `Shape` --references--> `String`  [EXTRACTED]
  Sources/Shop/Models/Shape.swift → Sources/Shop/Extensions/User+Format.swift

## Import Cycles
- None detected.

## Communities (7 total, 0 thin omitted)

### Community 0 - "User"
Cohesion: 0.18
Nodes (5): .description, .summary, User, .displayName, log()

### Community 1 - "String"
Cohesion: 0.17
Nodes (6): String, Payment, failed, paid, pending, Cache

### Community 2 - "UserService"
Cohesion: 0.19
Nodes (7): Combine, Repository, ShopError, invalid, missing, UserRepository, UserService

### Community 3 - "Circle"
Cohesion: 0.20
Nodes (6): run(), Box, Circle, .summary, Pair, Shape

### Community 4 - "Foundation"
Cohesion: 0.20
Nodes (4): Foundation, Array, Describable, Entity

### Community 5 - "Role"
Cohesion: 0.25
Nodes (6): Address, Role, admin, customer, guest, .isPrivileged

### Community 6 - "UserServiceTests"
Cohesion: 0.29
Nodes (3): Shop, UserServiceTests, XCTest

## Knowledge Gaps
- **13 isolated node(s):** `.summary`, `admin`, `customer`, `guest`, `.isPrivileged` (+8 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 25 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `String` connect `String` to `User`, `UserService`, `Circle`, `Foundation`, `Role`?**
  _High betweenness centrality (0.417) - this node is a cross-community bridge._
- **What connects `.summary`, `admin`, `customer` to the rest of the system?**
  _13 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `User` connect `User` to `String`, `UserService`, `Foundation`, `Role`?**
  _High betweenness centrality (0.320) - this node is a cross-community bridge._
- **Why does `Payment` connect `String` to `Role`, `UserServiceTests`?**
  _High betweenness centrality (0.132) - this node is a cross-community bridge._
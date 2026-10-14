# Graph Report - swift  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 91 nodes · 173 edges · 9 communities (6 shown, 3 thin omitted)
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
- Community 7
- Community 8

## God Nodes (most connected - your core abstractions)
1. `User` - 32 edges
2. `UserService` - 11 edges
3. `Role` - 11 edges
4. `UserRepository` - 10 edges
5. `Circle` - 9 edges
6. `Payment` - 9 edges
7. `ShopError` - 6 edges
8. `Shape` - 6 edges
9. `UserServiceTests` - 6 edges
10. `Cache` - 6 edges

## Surprising Connections (you probably didn't know these)
- `UserServiceTests` --references--> `UserService`  [EXTRACTED]
  Tests/ShopTests/UserServiceTests.swift → Sources/Shop/Services/UserService.swift
- `run()` --calls--> `log()`  [INFERRED]
  Sources/App/main.swift → Sources/Shop/Services/UserService.swift
- `ShopError` --references--> `String`  [EXTRACTED]
  Sources/Shop/Services/UserService.swift → Sources/Shop/Extensions/User+Format.swift
- `UserRepository` --references--> `String`  [EXTRACTED]
  Sources/Shop/Services/UserService.swift → Sources/Shop/Extensions/User+Format.swift
- `UserRepository` --references--> `User`  [EXTRACTED]
  Sources/Shop/Services/UserService.swift → Sources/Shop/Models/User.swift

## Import Cycles
- None detected.

## Communities (9 total, 3 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.16
Nodes (8): Combine, log(), Repository, ShopError, invalid, missing, UserRepository, UserService

### Community 1 - "Community 1"
Cohesion: 0.22
Nodes (5): run(), Circle, .summary, Pair, Shape

### Community 2 - "Community 2"
Cohesion: 0.15
Nodes (6): Payment, failed, paid, pending, UserServiceTests, XCTest

### Community 3 - "Community 3"
Cohesion: 0.27
Nodes (4): .description, .summary, User, .displayName

### Community 5 - "Community 5"
Cohesion: 0.25
Nodes (6): Address, Role, admin, customer, guest, .isPrivileged

### Community 6 - "Community 6"
Cohesion: 0.25
Nodes (4): Foundation, Shop, Describable, Entity

## Knowledge Gaps
- **13 isolated node(s):** `Combine`, `invalid`, `missing`, `.summary`, `failed` (+8 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 24 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `String` connect `Community 4` to `Community 0`, `Community 1`, `Community 2`, `Community 3`, `Community 5`, `Community 8`?**
  _High betweenness centrality (0.417) - this node is a cross-community bridge._
- **What connects `Combine`, `invalid`, `missing` to the rest of the system?**
  _13 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `User` connect `Community 3` to `Community 0`, `Community 4`, `Community 5`, `Community 6`, `Community 8`?**
  _High betweenness centrality (0.305) - this node is a cross-community bridge._
- **Why does `Circle` connect `Community 1` to `Community 4`, `Community 6`?**
  _High betweenness centrality (0.132) - this node is a cross-community bridge._
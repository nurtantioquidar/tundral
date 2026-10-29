# Graph Report - csharp  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 93 nodes · 142 edges · 8 communities (6 shown, 2 thin omitted)
- Extraction: 89% EXTRACTED · 11% INFERRED · 0% AMBIGUOUS · INFERRED: 16 edges (avg confidence: 0.85)
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
1. `User` - 24 edges
2. `UserService` - 14 edges
3. `Entity` - 8 edges
4. `IRepository` - 7 edges
5. `Shop.Models` - 7 edges
6. `InMemoryRepository` - 6 edges
7. `UserServiceTests` - 5 edges
8. `Point` - 5 edges
9. `Status` - 5 edges
10. `Address` - 4 edges

## Surprising Connections (you probably didn't know these)
- `UserServiceTests` --references--> `UserService`  [EXTRACTED]
  tests/Shop.Tests/UserServiceTests.cs → src/Shop/Services/UserService.cs
- `NullLogger` --implements--> `ILogger`  [EXTRACTED]
  tests/Shop.Tests/UserServiceTests.cs → src/Shop/Services/UserService.cs
- `User` --inherits--> `Entity`  [EXTRACTED]
  src/Shop/Models/User.Validation.cs → src/Shop/Models/Entity.cs
- `User` --references--> `Status`  [EXTRACTED]
  src/Shop/Models/User.Validation.cs → src/Shop/Models/User.cs
- `User` --defines--> `Name`  [EXTRACTED]
  src/Shop/Models/User.Validation.cs → src/Shop/Models/User.cs

## Import Cycles
- None detected.

## Communities (8 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.16
Nodes (5): Address, Street, Name, Status, User

### Community 1 - "Community 1"
Cohesion: 0.12
Nodes (5): Shop.Services, Shop.Tests, Formatter, StringExtensions, UserServiceTests

### Community 2 - "Community 2"
Cohesion: 0.18
Nodes (6): Shop.Models, Entity, CreatedAt, Id, IEntity, Id

### Community 3 - "Community 3"
Cohesion: 0.19
Nodes (4): ILogger, IUserService, UserService, NullLogger

### Community 5 - "Community 5"
Cohesion: 0.22
Nodes (6): Money, Point, X, Y, Range, UserChanged

### Community 7 - "Community 7"
Cohesion: 0.50
Nodes (4): Status, Active, Deleted, Suspended

## Knowledge Gaps
- **11 isolated node(s):** `Street`, `Name`, `Status`, `CreatedAt`, `Id` (+6 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 36 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `Community 0` to `Community 2`, `Community 4`, `Community 7`?**
  _High betweenness centrality (0.368) - this node is a cross-community bridge._
- **What connects `Street`, `Name`, `Status` to the rest of the system?**
  _11 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.125 - nodes in this community are weakly interconnected._
- **Why does `UserService` connect `Community 3` to `Community 0`, `Community 1`, `Community 4`, `Community 6`?**
  _High betweenness centrality (0.251) - this node is a cross-community bridge._
- **Why does `Shop.Models` connect `Community 2` to `Community 1`, `Community 3`, `Community 4`, `Community 5`?**
  _High betweenness centrality (0.186) - this node is a cross-community bridge._
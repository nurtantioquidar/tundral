# Graph Report - csharp  (2026-10-14)

## Corpus Check
- 8 files · ~713 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 91 nodes · 136 edges · 8 communities (6 shown, 2 thin omitted)
- Extraction: 90% EXTRACTED · 10% INFERRED · 0% AMBIGUOUS · INFERRED: 13 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- User
- UserServiceTests.cs
- IRepository
- UserService
- UserService.cs
- Entity
- Point
- Status

## God Nodes (most connected - your core abstractions)
1. `User` - 23 edges
2. `UserService` - 13 edges
3. `Entity` - 8 edges
4. `Shop.Models` - 7 edges
5. `IRepository` - 7 edges
6. `InMemoryRepository` - 6 edges
7. `Status` - 5 edges
8. `Point` - 5 edges
9. `UserServiceTests` - 5 edges
10. `IEntity` - 4 edges

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

### Community 0 - "User"
Cohesion: 0.15
Nodes (5): Address, Street, Name, Status, User

### Community 1 - "UserServiceTests.cs"
Cohesion: 0.12
Nodes (5): Shop.Services, Shop.Tests, Formatter, StringExtensions, UserServiceTests

### Community 3 - "UserService"
Cohesion: 0.22
Nodes (4): ILogger, IUserService, UserService, NullLogger

### Community 5 - "Entity"
Cohesion: 0.25
Nodes (5): Entity, CreatedAt, Id, IEntity, Id

### Community 6 - "Point"
Cohesion: 0.25
Nodes (5): Money, Point, X, Y, Range

### Community 7 - "Status"
Cohesion: 0.50
Nodes (4): Status, Active, Deleted, Suspended

## Knowledge Gaps
- **11 isolated node(s):** `Id`, `Id`, `CreatedAt`, `Active`, `Suspended` (+6 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 35 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `User` to `UserService`, `UserService.cs`, `Entity`, `Status`?**
  _High betweenness centrality (0.386) - this node is a cross-community bridge._
- **What connects `Id`, `Id`, `CreatedAt` to the rest of the system?**
  _11 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `UserServiceTests.cs` be split into smaller, more focused modules?**
  _Cohesion score 0.125 - nodes in this community are weakly interconnected._
- **Why does `UserService` connect `UserService` to `UserServiceTests.cs`, `IRepository`, `UserService.cs`?**
  _High betweenness centrality (0.235) - this node is a cross-community bridge._
- **Why does `Shop.Models` connect `UserService.cs` to `UserServiceTests.cs`, `IRepository`, `Point`?**
  _High betweenness centrality (0.166) - this node is a cross-community bridge._
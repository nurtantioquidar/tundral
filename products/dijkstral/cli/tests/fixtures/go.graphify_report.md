# Graph Report - go  (2026-10-13)

## Corpus Check
- 6 files · ~568 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 45 nodes · 71 edges · 10 communities (2 shown, 8 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- service.go
- main
- helpers.go
- TestRegister
- User
- Store
- T
- ID
- Entity
- example.com/shop

## God Nodes (most connected - your core abstractions)
1. `Store` - 13 edges
2. `User` - 7 edges
3. `UserService` - 7 edges
4. `ID` - 6 edges
5. `main()` - 4 edges
6. `NewUserService()` - 4 edges
7. `TestRegister()` - 4 edges
8. `Normalize()` - 4 edges
9. `Model` - 4 edges
10. `Sum()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `User` --embeds--> `Model`  [EXTRACTED]
  service/service.go → store/store.go
- `UserService` --references--> `Store`  [EXTRACTED]
  service/service.go → store/store.go
- `main()` --calls--> `NewUserService()`  [EXTRACTED]
  cmd/app/main.go → service/service.go
- `main()` --calls--> `Format()`  [EXTRACTED]
  cmd/app/main.go → store/helpers.go
- `main()` --calls--> `Sum()`  [EXTRACTED]
  cmd/app/main.go → store/store.go

## Import Cycles
- None detected.

## Communities (10 total, 8 thin omitted)

### Community 1 - "main"
Cohesion: 0.40
Nodes (4): main(), NewUserService(), Format(), Sum()

### Community 5 - "Store"
Cohesion: 0.50
Nodes (3): Number, Store, NewStore()

## Knowledge Gaps
- **1 isolated node(s):** `example.com/shop`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 13 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **8 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Store` connect `Store` to `service.go`, `main`, `User`, `T`, `ID`, `Entity`?**
  _High betweenness centrality (0.393) - this node is a cross-community bridge._
- **What connects `example.com/shop` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `ID` connect `ID` to `Entity`, `helpers.go`, `Store`, `T`?**
  _High betweenness centrality (0.110) - this node is a cross-community bridge._
- **Why does `UserService` connect `User` to `service.go`, `main`, `Store`?**
  _High betweenness centrality (0.094) - this node is a cross-community bridge._
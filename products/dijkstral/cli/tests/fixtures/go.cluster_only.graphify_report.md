# Graph Report - go  (2026-10-13)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 45 nodes · 71 edges · 10 communities (2 shown, 8 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
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
- Community 9

## God Nodes (most connected - your core abstractions)
1. `Store` - 13 edges
2. `User` - 7 edges
3. `UserService` - 7 edges
4. `ID` - 6 edges
5. `main()` - 4 edges
6. `NewUserService()` - 4 edges
7. `Normalize()` - 4 edges
8. `TestRegister()` - 4 edges
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

### Community 1 - "Community 1"
Cohesion: 0.40
Nodes (4): main(), NewUserService(), Format(), Sum()

### Community 5 - "Community 5"
Cohesion: 0.50
Nodes (3): Number, Store, NewStore()

## Knowledge Gaps
- **1 isolated node(s):** `example.com/shop`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 13 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **8 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Store` connect `Community 5` to `Community 0`, `Community 1`, `Community 4`, `Community 6`, `Community 7`, `Community 8`?**
  _High betweenness centrality (0.393) - this node is a cross-community bridge._
- **What connects `example.com/shop` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `ID` connect `Community 7` to `Community 8`, `Community 2`, `Community 5`, `Community 6`?**
  _High betweenness centrality (0.110) - this node is a cross-community bridge._
- **Why does `UserService` connect `Community 4` to `Community 0`, `Community 1`, `Community 5`?**
  _High betweenness centrality (0.094) - this node is a cross-community bridge._
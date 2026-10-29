# Graph Report - go  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 43 nodes · 67 edges · 13 communities (3 shown, 10 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 2
- Community 3
- Community 4
- Community 5
- Community 7
- Community 9
- Community 10
- Community 11
- Community 12

## God Nodes (most connected - your core abstractions)
1. `Store` - 15 edges
2. `User` - 7 edges
3. `UserService` - 7 edges
4. `ID` - 6 edges
5. `Model` - 4 edges
6. `Normalize()` - 4 edges
7. `NewUserService()` - 4 edges
8. `TestRegister()` - 4 edges
9. `main()` - 4 edges
10. `Entity` - 2 edges

## Surprising Connections (you probably didn't know these)
- `UserService` --references--> `Store`  [EXTRACTED]
  service/service.go → store/store.go
- `User` --embeds--> `Model`  [EXTRACTED]
  service/service.go → store/store.go
- `main()` --calls--> `NewUserService()`  [EXTRACTED]
  cmd/app/main.go → service/service.go
- `main()` --calls--> `Format()`  [EXTRACTED]
  cmd/app/main.go → store/helpers.go
- `main()` --calls--> `Sum()`  [EXTRACTED]
  cmd/app/main.go → store/store.go

## Import Cycles
- None detected.

## Communities (13 total, 10 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.33
Nodes (3): Key, Number, Store

### Community 3 - "Community 3"
Cohesion: 0.50
Nodes (3): clean(), NewUserService(), TestRegister()

### Community 7 - "Community 7"
Cohesion: 0.67
Nodes (3): main(), Format(), Sum()

## Knowledge Gaps
- **1 isolated node(s):** `example.com/shop`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 14 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **10 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Store` connect `Community 0` to `Community 1`, `Community 4`, `Community 5`, `Community 6`, `Community 7`, `Community 11`?**
  _High betweenness centrality (0.395) - this node is a cross-community bridge._
- **What connects `example.com/shop` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `UserService` connect `Community 4` to `Community 0`, `Community 1`, `Community 10`, `Community 3`?**
  _High betweenness centrality (0.101) - this node is a cross-community bridge._
- **Why does `User` connect `Community 4` to `Community 9`, `Community 10`, `Community 5`, `Community 1`?**
  _High betweenness centrality (0.077) - this node is a cross-community bridge._
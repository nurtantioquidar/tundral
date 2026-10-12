# Graph Report - python  (2026-10-13)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 59 nodes · 107 edges · 9 communities (2 shown, 7 thin omitted)
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 9 edges (avg confidence: 0.91)
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
1. `User` - 17 edges
2. `UserService` - 12 edges
3. `run()` - 11 edges
4. `Repo` - 8 edges
5. `Base` - 6 edges
6. `slugify()` - 6 edges
7. `Admin` - 5 edges
8. `TestUserService` - 5 edges
9. `helper()` - 3 edges
10. `traced()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `TestUserService` --uses--> `User`  [INFERRED]
  tests/test_service.py → pkg/models.py
- `TestUserService` --uses--> `UserService`  [INFERRED]
  tests/test_service.py → pkg/service.py
- `main()` --uses--> `UserService`  [INFERRED]
  main.py → pkg/service.py
- `service()` --calls--> `run()`  [EXTRACTED]
  tests/test_service.py → pkg/service.py
- `UserService` --uses--> `User`  [INFERRED]
  pkg/service.py → pkg/models.py

## Import Cycles
- None detected.

## Communities (9 total, 7 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.24
Nodes (3): helper(), slugify(), traced()

### Community 3 - "Community 3"
Cohesion: 0.60
Nodes (3): main(), run(), UserService

## Knowledge Gaps
- **7 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `Community 1` to `Community 0`, `Community 2`, `Community 3`, `Community 4`, `Community 5`, `Community 6`, `Community 7`?**
  _High betweenness centrality (0.304) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `User` (e.g. with `UserService` and `TestUserService`) actually correct?**
  _`User` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Why does `Repo` connect `Community 5` to `Community 0`, `Community 2`, `Community 3`?**
  _High betweenness centrality (0.142) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `UserService` (e.g. with `main()` and `Repo`) actually correct?**
  _`UserService` has 4 INFERRED edges - model-reasoned connections that need verification._
- **Why does `UserService` connect `Community 3` to `Community 0`, `Community 1`, `Community 5`, `Community 6`, `Community 7`, `Community 8`?**
  _High betweenness centrality (0.102) - this node is a cross-community bridge._
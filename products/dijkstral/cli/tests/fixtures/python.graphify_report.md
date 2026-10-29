# Graph Report - python  (2026-10-29)

## Corpus Check
- 6 files · ~454 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 59 nodes · 108 edges · 9 communities (2 shown, 7 thin omitted)
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 9 edges (avg confidence: 0.91)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- service.py
- User
- models.py
- UserService
- Base
- Repo
- test_service.py
- TestUserService
- helper

## God Nodes (most connected - your core abstractions)
1. `User` - 17 edges
2. `UserService` - 12 edges
3. `run()` - 11 edges
4. `Repo` - 8 edges
5. `Base` - 6 edges
6. `slugify()` - 6 edges
7. `Admin` - 5 edges
8. `TestUserService` - 5 edges
9. `validate()` - 4 edges
10. `main()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `TestUserService` --uses--> `User`  [INFERRED]
  tests/test_service.py → pkg/models.py
- `TestUserService` --uses--> `UserService`  [INFERRED]
  tests/test_service.py → pkg/service.py
- `main()` --uses--> `UserService`  [INFERRED]
  main.py → pkg/service.py
- `service()` --calls--> `run()`  [EXTRACTED]
  tests/test_service.py → pkg/service.py
- `main()` --calls--> `run()`  [EXTRACTED]
  main.py → pkg/service.py

## Import Cycles
- None detected.

## Communities (9 total, 7 thin omitted)

### Community 0 - "service.py"
Cohesion: 0.24
Nodes (3): helper(), slugify(), traced()

### Community 3 - "UserService"
Cohesion: 0.60
Nodes (3): main(), run(), UserService

## Knowledge Gaps
- **7 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `User` to `service.py`, `models.py`, `UserService`, `Base`, `Repo`, `test_service.py`, `TestUserService`?**
  _High betweenness centrality (0.296) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `User` (e.g. with `UserService` and `TestUserService`) actually correct?**
  _`User` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Why does `Repo` connect `Repo` to `service.py`, `models.py`, `UserService`?**
  _High betweenness centrality (0.141) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `UserService` (e.g. with `main()` and `Repo`) actually correct?**
  _`UserService` has 4 INFERRED edges - model-reasoned connections that need verification._
- **Why does `UserService` connect `UserService` to `service.py`, `User`, `Repo`, `test_service.py`, `TestUserService`, `helper`?**
  _High betweenness centrality (0.103) - this node is a cross-community bridge._
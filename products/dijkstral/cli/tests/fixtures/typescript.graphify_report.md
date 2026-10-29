# Graph Report - typescript  (2026-10-29)

## Corpus Check
- 7 files · ~644 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 64 nodes · 111 edges · 7 communities (6 shown, 1 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- user.ts
- Repo
- User
- service.ts
- App.tsx
- util.ts
- UserService

## God Nodes (most connected - your core abstractions)
1. `User` - 17 edges
2. `UserService` - 11 edges
3. `Repo` - 10 edges
4. `run()` - 8 edges
5. `Base` - 5 edges
6. `Admin` - 5 edges
7. `Entity` - 4 edges
8. `Named` - 4 edges
9. `slugify()` - 4 edges
10. `Role` - 3 edges

## Surprising Connections (you probably didn't know these)
- `makeUser()` --calls--> `User`  [EXTRACTED]
  src/__tests__/service.test.ts → src/models/user.ts
- `BadgeProps` --references--> `User`  [EXTRACTED]
  src/ui/App.tsx → src/models/user.ts
- `Base` --implements--> `Entity`  [EXTRACTED]
  src/models/user.ts → src/models/repo.ts
- `run()` --calls--> `Repo`  [EXTRACTED]
  src/service.ts → src/models/repo.ts
- `UserService` --references--> `Repo`  [EXTRACTED]
  src/service.ts → src/models/repo.ts

## Import Cycles
- None detected.

## Communities (7 total, 1 thin omitted)

### Community 0 - "user.ts"
Cohesion: 0.15
Nodes (8): Base, formatName(), helper(), Role, Admin, Member, UserId, validate()

### Community 1 - "Repo"
Cohesion: 0.24
Nodes (4): Entity, Repo, sealed(), Storage

### Community 2 - "User"
Cohesion: 0.28
Nodes (3): Admin, Named, User

### Community 4 - "App.tsx"
Cohesion: 0.32
Nodes (4): App, Badge(), BadgeProps, UserList()

### Community 5 - "util.ts"
Cohesion: 0.33
Nodes (3): helper(), { join }, slugify()

### Community 6 - "UserService"
Cohesion: 0.43
Nodes (3): run(), UserService, makeUser()

## Knowledge Gaps
- **2 isolated node(s):** `Member`, `Admin`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 25 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `User` to `user.ts`, `Repo`, `service.ts`, `App.tsx`, `UserService`?**
  _High betweenness centrality (0.257) - this node is a cross-community bridge._
- **What connects `Member`, `Admin` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Repo` connect `Repo` to `service.ts`, `UserService`?**
  _High betweenness centrality (0.137) - this node is a cross-community bridge._
- **Why does `UserService` connect `UserService` to `Repo`, `User`, `service.ts`, `App.tsx`?**
  _High betweenness centrality (0.080) - this node is a cross-community bridge._
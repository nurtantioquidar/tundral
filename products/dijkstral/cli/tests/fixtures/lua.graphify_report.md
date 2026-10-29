# Graph Report - lua  (2026-10-29)

## Corpus Check
- 8 files · ~458 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 34 nodes · 49 edges · 5 communities (4 shown, 1 thin omitted)
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 8 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- user.lua
- util.lua
- init.lua
- service.lua
- server.luau

## God Nodes (most connected - your core abstractions)
1. `User.new()` - 3 edges
2. `Service.new()` - 3 edges
3. `Service:names()` - 3 edges
4. `clamp()` - 3 edges
5. `M.format_price()` - 3 edges
6. `slugify()` - 3 edges
7. `helper()` - 3 edges
8. `make_service()` - 3 edges
9. `M.new()` - 2 edges
10. `Base.new()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `make_service()` --calls--> `User.new()`  [INFERRED]
  spec/service_spec.lua → lib/shop/models/user.lua
- `helper()` --calls--> `Service.new()`  [INFERRED]
  spec/service_spec.lua → lib/shop/service.lua
- `User:display()` --calls--> `M.format_price()`  [INFERRED]
  lib/shop/models/user.lua → lib/shop/util.lua
- `Service:names()` --calls--> `slugify()`  [INFERRED]
  lib/shop/service.lua → lib/shop/util.lua
- `M.new()` --calls--> `Service.new()`  [INFERRED]
  lib/shop/init.lua → lib/shop/service.lua

## Import Cycles
- None detected.

## Communities (5 total, 1 thin omitted)

### Community 0 - "user.lua"
Cohesion: 0.24
Nodes (7): Base:describe(), Base:id(), Base.new(), User.Admin.promote(), User:display(), User.new(), Service:promote_all()

### Community 1 - "util.lua"
Cohesion: 0.38
Nodes (4): clamp(), M.format_price(), M.round(), slugify()

### Community 2 - "init.lua"
Cohesion: 0.38
Nodes (4): M.new(), Service.new(), helper(), make_service()

### Community 3 - "service.lua"
Cohesion: 0.47
Nodes (4): Service:add(), Service:export(), Service:log(), Service:names()

## Knowledge Gaps
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `slugify()` connect `util.lua` to `service.lua`?**
  _High betweenness centrality (0.112) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `User.new()` (e.g. with `Base.new()` and `make_service()`) actually correct?**
  _`User.new()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Why does `Service:names()` connect `service.lua` to `util.lua`?**
  _High betweenness centrality (0.051) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `Service.new()` (e.g. with `M.new()` and `helper()`) actually correct?**
  _`Service.new()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Why does `User.new()` connect `user.lua` to `init.lua`?**
  _High betweenness centrality (0.050) - this node is a cross-community bridge._
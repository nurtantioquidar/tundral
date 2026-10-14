# Graph Report - ruby  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 56 nodes · 71 edges · 8 communities (2 shown, 6 thin omitted)
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

## God Nodes (most connected - your core abstractions)
1. `Shop::Service::UserService` - 10 edges
2. `Shop::Models::User` - 9 edges
3. `Shop::Models::Repo` - 6 edges
4. `ServiceTest` - 5 edges
5. `Shop::Models::Base` - 5 edges
6. `Shop::Loggable` - 4 edges
7. `Shop::Models::Guest` - 3 edges
8. `Shop::Models::User::Admin` - 3 edges
9. `Shop::Models` - 3 edges
10. `Shop::Service` - 3 edges

## Surprising Connections (you probably didn't know these)
- `Shop::Models::User` --mixes_in--> `Shop::Loggable`  [EXTRACTED]
  lib/shop/models/user.rb → lib/shop/util.rb
- `Shop::Service::UserService` --mixes_in--> `Shop::Loggable`  [EXTRACTED]
  lib/shop/service.rb → lib/shop/util.rb

## Import Cycles
- None detected.

## Communities (8 total, 6 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.22
Nodes (5): Shop, Shop::Models, Shop::Models::Guest, Shop::Models::User, Shop::Models::User::Admin

### Community 1 - "Community 1"
Cohesion: 0.20
Nodes (4): Shop, Shop::Models, Shop::Models::Repo, Shop

## Knowledge Gaps
- **6 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Shop::Models::User` connect `Community 0` to `Community 2`, `Community 3`, `Community 4`, `Community 5`, `Community 6`?**
  _High betweenness centrality (0.406) - this node is a cross-community bridge._
- **Why does `Shop::Service::UserService` connect `Community 2` to `Community 1`, `Community 3`, `Community 6`?**
  _High betweenness centrality (0.351) - this node is a cross-community bridge._
- **Why does `Shop::Models::Repo` connect `Community 1` to `Community 2`?**
  _High betweenness centrality (0.213) - this node is a cross-community bridge._
# Graph Report - upd  (2026-10-14)

## Corpus Check
- 9 files · ~406 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 53 nodes · 67 edges · 8 communities (6 shown, 2 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 2 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Shop.Util
- Shop.Format
- Shop.Repo
- Shop.User
- shop.ex
- Shop
- Shop.MixProject

## God Nodes (most connected - your core abstractions)
1. `Shop.User` - 10 edges
2. `Shop.Repo` - 8 edges
3. `Shop` - 6 edges
4. `Shop.Util` - 6 edges
5. `Shop.Format` - 5 edges
6. `register()` - 4 edges
7. `line()` - 4 edges
8. `stamp()` - 4 edges
9. `Stats` - 3 edges
10. `summary()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `stamped()` --calls--> `stamp()`  [INFERRED]
  test/shop_test.exs → lib/shop/util.ex
- `register()` --calls--> `insert()`  [EXTRACTED]
  lib/shop.ex → lib/shop/repo.ex
- `register()` --calls--> `new()`  [EXTRACTED]
  lib/shop.ex → lib/shop/user.ex
- `insert()` --calls--> `stamp()`  [INFERRED]
  lib/shop/repo.ex → lib/shop/util.ex
- `describe()` --calls--> `line()`  [EXTRACTED]
  lib/shop.ex → lib/shop/format.ex

## Import Cycles
- None detected.

## Communities (8 total, 2 thin omitted)

### Community 0 - "Shop.Util"
Cohesion: 0.24
Nodes (6): insert(), Shop.Util, now(), stamp(), ShopTest, stamped()

### Community 1 - "Shop.Format"
Cohesion: 0.27
Nodes (8): Printable (for User), print(), String.Chars (for Shop.User), to_string(), Printable, Shop.Format, line(), describe()

### Community 3 - "Shop.User"
Cohesion: 0.25
Nodes (3): Shop.User, new(), normalize()

### Community 4 - "shop.ex"
Cohesion: 0.40
Nodes (4): summary(), Stats, count(), summary()

### Community 5 - "Shop"
Cohesion: 0.50
Nodes (5): Shop, admins(), log_result(), register(), admin?()

### Community 6 - "Shop.MixProject"
Cohesion: 0.50
Nodes (3): Shop.MixProject, deps(), project()

## Knowledge Gaps
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Shop.User` connect `Shop.User` to `Shop.Util`, `Shop.Format`, `shop.ex`, `Shop`?**
  _High betweenness centrality (0.328) - this node is a cross-community bridge._
- **Why does `Shop.Repo` connect `Shop.Repo` to `Shop.Util`, `shop.ex`?**
  _High betweenness centrality (0.232) - this node is a cross-community bridge._
- **Why does `Shop.Format` connect `Shop.Format` to `shop.ex`?**
  _High betweenness centrality (0.162) - this node is a cross-community bridge._
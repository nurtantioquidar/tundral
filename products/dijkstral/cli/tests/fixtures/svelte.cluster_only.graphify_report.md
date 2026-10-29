# Graph Report - svelte  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 28 nodes · 46 edges · 5 communities (2 shown, 3 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3

## God Nodes (most connected - your core abstractions)
1. `sumPrices()` - 5 edges
2. `money()` - 5 edges
3. `Item` - 3 edges
4. `announce()` - 3 edges
5. `label()` - 3 edges
6. `plural()` - 3 edges
7. `addItem()` - 3 edges
8. `total` - 3 edges
9. `clear()` - 2 edges
10. `emptyMessage()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `label()` --calls--> `money()`  [EXTRACTED]
  src/lib/Cart.svelte → src/lib/format.js
- `load()` --calls--> `sumPrices()`  [EXTRACTED]
  src/routes/+page.ts → src/lib/stores.ts
- `price()` --calls--> `money()`  [EXTRACTED]
  src/lib/CartRow.svelte → src/lib/format.js
- `emptyMessage()` --calls--> `plural()`  [EXTRACTED]
  src/lib/Cart.svelte → src/lib/format.js
- `buy()` --calls--> `addItem()`  [EXTRACTED]
  src/routes/+page.svelte → src/lib/stores.ts

## Import Cycles
- None detected.

## Communities (5 total, 3 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.31
Nodes (8): announce(), CART_KEY, clear(), compact, emptyMessage(), label(), title, plural()

### Community 1 - "Community 1"
Cohesion: 0.39
Nodes (5): cart, Item, sumPrices(), total, load()

## Knowledge Gaps
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `sumPrices()` connect `Community 1` to `Community 0`?**
  _High betweenness centrality (0.068) - this node is a cross-community bridge._
- **Why does `addItem()` connect `Community 2` to `Community 1`?**
  _High betweenness centrality (0.031) - this node is a cross-community bridge._
- **Why does `money()` connect `Community 3` to `Community 0`?**
  _High betweenness centrality (0.026) - this node is a cross-community bridge._
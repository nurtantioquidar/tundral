# Graph Report - svelte  (2026-10-29)

## Corpus Check
- 7 files · ~301 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 28 nodes · 46 edges · 5 communities (2 shown, 3 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Cart.svelte
- stores.ts
- +page.svelte
- CartRow.svelte

## God Nodes (most connected - your core abstractions)
1. `money()` - 5 edges
2. `sumPrices()` - 5 edges
3. `label()` - 3 edges
4. `announce()` - 3 edges
5. `plural()` - 3 edges
6. `Item` - 3 edges
7. `total` - 3 edges
8. `addItem()` - 3 edges
9. `emptyMessage()` - 2 edges
10. `clear()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `label()` --calls--> `money()`  [EXTRACTED]
  src/lib/Cart.svelte → src/lib/format.js
- `price()` --calls--> `money()`  [EXTRACTED]
  src/lib/CartRow.svelte → src/lib/format.js
- `load()` --calls--> `sumPrices()`  [EXTRACTED]
  src/routes/+page.ts → src/lib/stores.ts
- `emptyMessage()` --calls--> `plural()`  [EXTRACTED]
  src/lib/Cart.svelte → src/lib/format.js
- `buy()` --calls--> `addItem()`  [EXTRACTED]
  src/routes/+page.svelte → src/lib/stores.ts

## Import Cycles
- None detected.

## Communities (5 total, 3 thin omitted)

### Community 0 - "Cart.svelte"
Cohesion: 0.31
Nodes (8): announce(), CART_KEY, clear(), compact, emptyMessage(), label(), title, plural()

### Community 1 - "stores.ts"
Cohesion: 0.39
Nodes (5): cart, Item, sumPrices(), total, load()

## Knowledge Gaps
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `sumPrices()` connect `stores.ts` to `Cart.svelte`?**
  _High betweenness centrality (0.068) - this node is a cross-community bridge._
- **Why does `addItem()` connect `+page.svelte` to `stores.ts`?**
  _High betweenness centrality (0.031) - this node is a cross-community bridge._
- **Why does `money()` connect `CartRow.svelte` to `Cart.svelte`?**
  _High betweenness centrality (0.026) - this node is a cross-community bridge._
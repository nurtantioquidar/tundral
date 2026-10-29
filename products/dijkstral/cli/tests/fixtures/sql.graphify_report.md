# Graph Report - sql  (2026-10-29)

## Corpus Check
- 8 files · ~554 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 37 nodes · 55 edges · 8 communities (5 shown, 3 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- order_items
- orders
- 003_functions.sql
- billing.invoices
- legacy.sql
- procedures.sql

## God Nodes (most connected - your core abstractions)
1. `orders` - 8 edges
2. `order_items` - 8 edges
3. `products` - 5 edges
4. `order_totals` - 5 edges
5. `billing.invoices` - 5 edges
6. `customers` - 4 edges
7. `big_customers` - 4 edges
8. `refresh_totals()` - 4 edges
9. `all_people` - 3 edges
10. `touch_order()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `order_total()` --reads_from--> `order_totals`  [EXTRACTED]
  db/migrations/003_functions.sql → db/migrations/002_views.sql
- `invoices_close` --triggers--> `billing.invoices`  [EXTRACTED]
  db/migrations/003_functions.sql → db/schema.sql
- `billing.close_invoice()` --writes_to--> `billing.invoices`  [EXTRACTED]
  db/procedures.sql → db/schema.sql
- `touch_order()` --reads_from--> `orders`  [EXTRACTED]
  db/migrations/003_functions.sql → db/migrations/001_init.sql
- `archive_orders()` --writes_to--> `orders`  [EXTRACTED]
  db/procedures.sql → db/migrations/001_init.sql

## Import Cycles
- None detected.

## Communities (8 total, 3 thin omitted)

### Community 0 - "order_items"
Cohesion: 0.31
Nodes (5): customers, order_items, order_items_order_idx, products, products_sku_idx

### Community 1 - "orders"
Cohesion: 0.39
Nodes (5): orders, all_people, big_customers, order_totals, refresh_totals()

### Community 2 - "003_functions.sql"
Cohesion: 0.50
Nodes (4): invoices_close, order_items_touch, order_total(), touch_order()

### Community 3 - "billing.invoices"
Cohesion: 0.50
Nodes (3): billing.invoices, invoices_order_idx, public.orders_archive

### Community 4 - "legacy.sql"
Cohesion: 1.00
Nodes (3): `customers`, `team_sizes`, `teams`

## Knowledge Gaps
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `order_items` connect `order_items` to `orders`, `003_functions.sql`?**
  _High betweenness centrality (0.226) - this node is a cross-community bridge._
- **Why does `orders` connect `orders` to `order_items`, `003_functions.sql`, `procedures.sql`?**
  _High betweenness centrality (0.170) - this node is a cross-community bridge._
- **Why does `refresh_totals()` connect `orders` to `order_items`, `procedures.sql`?**
  _High betweenness centrality (0.142) - this node is a cross-community bridge._
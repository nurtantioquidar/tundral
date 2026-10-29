# Graph Report - sql  (2026-10-29)

## Corpus Check
- 10 files · ~792 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 48 nodes · 69 edges · 9 communities (6 shown, 3 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- orders
- billing.invoices
- order_items
- plpgsql.sql
- 003_functions.sql
- legacy.sql

## God Nodes (most connected - your core abstractions)
1. `orders` - 10 edges
2. `order_items` - 8 edges
3. `products` - 5 edges
4. `order_totals` - 5 edges
5. `"public"."ledger"` - 5 edges
6. `billing.invoices` - 5 edges
7. `customers` - 4 edges
8. `big_customers` - 4 edges
9. `purge_archive()` - 4 edges
10. `refresh_totals()` - 4 edges

## Surprising Connections (you probably didn't know these)
- `order_total()` --reads_from--> `order_totals`  [EXTRACTED]
  db/migrations/003_functions.sql → db/migrations/002_views.sql
- `invoices_close` --triggers--> `billing.invoices`  [EXTRACTED]
  db/migrations/003_functions.sql → db/schema.sql
- `touch_order()` --reads_from--> `orders`  [EXTRACTED]
  db/migrations/003_functions.sql → db/migrations/001_init.sql
- `"public"."ledger"` --references--> `orders`  [EXTRACTED]
  db/plpgsql.sql → db/migrations/001_init.sql
- `archive_orders()` --writes_to--> `orders`  [EXTRACTED]
  db/procedures.sql → db/migrations/001_init.sql

## Import Cycles
- None detected.

## Communities (9 total, 3 thin omitted)

### Community 0 - "orders"
Cohesion: 0.27
Nodes (7): nightly_maintenance(), customers, orders, all_people, big_customers, order_totals, refresh_totals()

### Community 1 - "billing.invoices"
Cohesion: 0.25
Nodes (5): archive_orders(), billing.close_invoice(), billing.invoices, invoices_order_idx, public.orders_archive

### Community 2 - "order_items"
Cohesion: 0.36
Nodes (4): order_items, order_items_order_idx, products, products_sku_idx

### Community 3 - "plpgsql.sql"
Cohesion: 0.36
Nodes (4): close_year(), "public"."ledger", purge_archive(), reconcile()

### Community 4 - "003_functions.sql"
Cohesion: 0.50
Nodes (4): invoices_close, order_items_touch, order_total(), touch_order()

### Community 5 - "legacy.sql"
Cohesion: 1.00
Nodes (3): `customers`, `team_sizes`, `teams`

## Knowledge Gaps
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `orders` connect `orders` to `billing.invoices`, `order_items`, `plpgsql.sql`, `003_functions.sql`?**
  _High betweenness centrality (0.374) - this node is a cross-community bridge._
- **Why does `"public"."ledger"` connect `plpgsql.sql` to `orders`?**
  _High betweenness centrality (0.217) - this node is a cross-community bridge._
- **Why does `order_items` connect `order_items` to `orders`, `003_functions.sql`?**
  _High betweenness centrality (0.163) - this node is a cross-community bridge._
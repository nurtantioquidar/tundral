# Graph Report - powershell  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 43 nodes · 64 edges · 8 communities (3 shown, 5 thin omitted)
- Extraction: 89% EXTRACTED · 11% INFERRED · 0% AMBIGUOUS · INFERRED: 7 edges (avg confidence: 0.85)
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
1. `Order` - 10 edges
2. `Invoke-Build()` - 5 edges
3. `New-ShopOrder()` - 5 edges
4. `PriorityOrder` - 5 edges
5. `OrderStatus` - 5 edges
6. `Write-BuildLog()` - 4 edges
7. `TaxRule` - 4 edges
8. `Test-Manifest()` - 3 edges
9. `New-TestOrder()` - 3 edges
10. `Get-TaxedAmount()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `Invoke-Build()` --calls--> `New-ShopOrder()`  [INFERRED]
  build.ps1 → Modules/Shop/Shop.psm1
- `New-TestOrder()` --calls--> `Write-BuildLog()`  [INFERRED]
  tests/Shop.Tests.ps1 → scripts/Common.ps1
- `Invoke-Build()` --calls--> `Write-BuildLog()`  [INFERRED]
  build.ps1 → scripts/Common.ps1
- `Test-Manifest()` --calls--> `Write-BuildLog()`  [INFERRED]
  build.ps1 → scripts/Common.ps1
- `New-TestOrder()` --calls--> `New-ShopOrder()`  [INFERRED]
  tests/Shop.Tests.ps1 → Modules/Shop/Shop.psm1

## Import Cycles
- None detected.

## Communities (8 total, 5 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.39
Nodes (4): Format-Result(), Invoke-Build(), Test-Manifest(), Write-BuildLog()

### Community 1 - "Community 1"
Cohesion: 0.32
Nodes (3): New-ShopOrder(), Write-ShopLog(), New-TestOrder()

### Community 6 - "Community 6"
Cohesion: 0.50
Nodes (4): OrderStatus, Open, Paid, Shipped

## Knowledge Gaps
- **3 isolated node(s):** `Open`, `Paid`, `Shipped`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 11 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Order` connect `Community 3` to `Community 1`, `Community 4`, `Community 6`, `Community 7`?**
  _High betweenness centrality (0.561) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `Invoke-Build()` (e.g. with `New-ShopOrder()` and `Write-BuildLog()`) actually correct?**
  _`Invoke-Build()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Open`, `Paid`, `Shipped` to the rest of the system?**
  _3 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `New-ShopOrder()` connect `Community 1` to `Community 0`?**
  _High betweenness centrality (0.379) - this node is a cross-community bridge._
- **Are the 3 inferred relationships involving `New-ShopOrder()` (e.g. with `Invoke-Build()` and `Shop.Tests.ps1`) actually correct?**
  _`New-ShopOrder()` has 3 INFERRED edges - model-reasoned connections that need verification._
- **Why does `Get-TaxedAmount()` connect `Community 2` to `Community 3`?**
  _High betweenness centrality (0.372) - this node is a cross-community bridge._
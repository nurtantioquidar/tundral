# Graph Report - javascript  (2026-10-13)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 36 nodes · 52 edges · 7 communities (2 shown, 5 thin omitted)
- Extraction: 94% EXTRACTED · 6% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.85)
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
1. `Store` - 9 edges
2. `CachedStore` - 5 edges
3. `formatLabel()` - 5 edges
4. `App` - 4 edges
5. `formatAll()` - 4 edges
6. `List()` - 3 edges
7. `Row()` - 3 edges
8. `fixture()` - 2 edges
9. `helper()` - 2 edges
10. `clear()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `fixture()` --calls--> `Store`  [EXTRACTED]
  test/store.test.js → lib/store.js
- `Row()` --calls--> `formatLabel()`  [EXTRACTED]
  components/App.jsx → lib/format.mjs

## Import Cycles
- None detected.

## Communities (7 total, 5 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.29
Nodes (3): Store, fixture(), { Store }

### Community 2 - "Community 2"
Cohesion: 0.40
Nodes (3): CachedStore, clear(), utils

## Knowledge Gaps
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Store` connect `Community 0` to `Community 2`, `Community 4`?**
  _High betweenness centrality (0.277) - this node is a cross-community bridge._
- **Why does `CachedStore` connect `Community 2` to `Community 0`, `Community 3`, `Community 5`?**
  _High betweenness centrality (0.129) - this node is a cross-community bridge._
- **Why does `formatLabel()` connect `Community 6` to `Community 1`, `Community 3`, `Community 5`?**
  _High betweenness centrality (0.066) - this node is a cross-community bridge._
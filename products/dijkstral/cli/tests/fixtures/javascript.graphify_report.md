# Graph Report - javascript  (2026-10-29)

## Corpus Check
- 5 files · ~323 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 34 nodes · 52 edges · 6 communities (2 shown, 4 thin omitted)
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 4 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- App.jsx
- store.js
- format.mjs
- Store
- default.cjs
- helper

## God Nodes (most connected - your core abstractions)
1. `Store` - 9 edges
2. `formatLabel()` - 5 edges
3. `CachedStore` - 5 edges
4. `App` - 4 edges
5. `formatAll()` - 4 edges
6. `Row()` - 3 edges
7. `List()` - 3 edges
8. `helper()` - 2 edges
9. `clear()` - 2 edges
10. `helper()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `fixture()` --calls--> `Store`  [EXTRACTED]
  test/store.test.js → lib/store.js
- `Row()` --calls--> `formatLabel()`  [EXTRACTED]
  components/App.jsx → lib/format.mjs

## Import Cycles
- None detected.

## Communities (6 total, 4 thin omitted)

### Community 0 - "App.jsx"
Cohesion: 0.31
Nodes (5): App, List(), Row(), formatAll(), formatLabel()

### Community 1 - "store.js"
Cohesion: 0.33
Nodes (3): CachedStore, clear(), utils

## Knowledge Gaps
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Store` connect `Store` to `store.js`, `default.cjs`, `helper`?**
  _High betweenness centrality (0.278) - this node is a cross-community bridge._
- **Why does `CachedStore` connect `store.js` to `App.jsx`, `Store`?**
  _High betweenness centrality (0.136) - this node is a cross-community bridge._
- **Why does `formatLabel()` connect `App.jsx` to `format.mjs`?**
  _High betweenness centrality (0.070) - this node is a cross-community bridge._
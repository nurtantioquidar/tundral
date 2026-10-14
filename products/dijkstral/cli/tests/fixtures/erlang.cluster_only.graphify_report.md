# Graph Report - co  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 9 nodes · 14 edges · 2 communities (1 shown, 1 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1

## God Nodes (most connected - your core abstractions)
1. `release()` - 4 edges
2. `current()` - 4 edges
3. `bump()` - 3 edges
4. `main()` - 2 edges
5. `parse()` - 2 edges
6. `Release helper for the shop application.` - 1 edges
7. `Version arithmetic for releases.` - 1 edges

## Surprising Connections (you probably didn't know these)
- `release()` --calls--> `current()`  [EXTRACTED]
  tools/release.py → tools/versions.py
- `release()` --calls--> `bump()`  [EXTRACTED]
  tools/release.py → tools/versions.py

## Import Cycles
- None detected.

## Communities (2 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.60
Nodes (3): main(), release(), bump()

## Knowledge Gaps
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `current()` connect `Community 1` to `Community 0`?**
  _High betweenness centrality (0.119) - this node is a cross-community bridge._
- **Why does `release()` connect `Community 0` to `Community 1`?**
  _High betweenness centrality (0.060) - this node is a cross-community bridge._
- **Why does `bump()` connect `Community 0` to `Community 1`?**
  _High betweenness centrality (0.024) - this node is a cross-community bridge._
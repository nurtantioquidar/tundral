# Graph Report - c  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 36 nodes · 56 edges · 5 communities (2 shown, 3 thin omitted)
- Extraction: 82% EXTRACTED · 18% INFERRED · 0% AMBIGUOUS · INFERRED: 10 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4

## God Nodes (most connected - your core abstractions)
1. `shape_new()` - 8 edges
2. `main()` - 6 edges
3. `shape_free()` - 5 edges
4. `test_area()` - 5 edges
5. `shape_area()` - 4 edges
6. `pick_area()` - 4 edges
7. `clamp()` - 3 edges
8. `util_report()` - 3 edges
9. `apply()` - 3 edges
10. `util_max()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `main()` --calls--> `shape_free()`  [INFERRED]
  main.c → src/shape.c
- `test_area()` --calls--> `shape_free()`  [INFERRED]
  tests/test_shape.c → src/shape.c
- `main()` --calls--> `util_report()`  [INFERRED]
  main.c → src/util.c
- `main()` --calls--> `shape_new()`  [INFERRED]
  main.c → src/shape.c
- `test_area()` --calls--> `shape_area()`  [INFERRED]
  tests/test_shape.c → src/shape.c

## Import Cycles
- None detected.

## Communities (5 total, 3 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.31
Nodes (6): util_max(), shape_free(), clamp(), log_line(), util_report(), xmalloc()

### Community 1 - "Community 1"
Cohesion: 0.29
Nodes (3): apply(), main(), shape_area()

## Knowledge Gaps
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `shape_new()` connect `Community 4` to `Community 0`, `Community 1`, `Community 2`, `Community 3`?**
  _High betweenness centrality (0.297) - this node is a cross-community bridge._
- **Are the 3 inferred relationships involving `shape_new()` (e.g. with `main()` and `xmalloc()`) actually correct?**
  _`shape_new()` has 3 INFERRED edges - model-reasoned connections that need verification._
- **Why does `main()` connect `Community 1` to `Community 0`, `Community 4`?**
  _High betweenness centrality (0.267) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `main()` (e.g. with `shape_area()` and `shape_free()`) actually correct?**
  _`main()` has 4 INFERRED edges - model-reasoned connections that need verification._
- **Why does `shape_free()` connect `Community 0` to `Community 1`, `Community 2`, `Community 3`?**
  _High betweenness centrality (0.167) - this node is a cross-community bridge._
- **Are the 3 inferred relationships involving `shape_free()` (e.g. with `main()` and `clamp()`) actually correct?**
  _`shape_free()` has 3 INFERRED edges - model-reasoned connections that need verification._
- **Are the 3 inferred relationships involving `test_area()` (e.g. with `shape_area()` and `shape_free()`) actually correct?**
  _`test_area()` has 3 INFERRED edges - model-reasoned connections that need verification._
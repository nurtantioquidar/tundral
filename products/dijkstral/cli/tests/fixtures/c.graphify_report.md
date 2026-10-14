# Graph Report - c  (2026-10-14)

## Corpus Check
- 6 files · ~444 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 36 nodes · 56 edges · 5 communities (2 shown, 3 thin omitted)
- Extraction: 82% EXTRACTED · 18% INFERRED · 0% AMBIGUOUS · INFERRED: 10 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- util.c
- main.c
- shape.c
- test_area
- shape_new

## God Nodes (most connected - your core abstractions)
1. `shape_new()` - 8 edges
2. `main()` - 6 edges
3. `shape_free()` - 5 edges
4. `test_area()` - 5 edges
5. `pick_area()` - 4 edges
6. `shape_area()` - 4 edges
7. `apply()` - 3 edges
8. `clamp()` - 3 edges
9. `util_report()` - 3 edges
10. `util_max()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `main()` --calls--> `shape_free()`  [INFERRED]
  main.c → src/shape.c
- `main()` --calls--> `shape_new()`  [INFERRED]
  main.c → src/shape.c
- `main()` --calls--> `util_report()`  [INFERRED]
  main.c → src/util.c
- `test_area()` --calls--> `shape_new()`  [INFERRED]
  tests/test_shape.c → src/shape.c
- `test_area()` --calls--> `shape_free()`  [INFERRED]
  tests/test_shape.c → src/shape.c

## Import Cycles
- None detected.

## Communities (5 total, 3 thin omitted)

### Community 0 - "util.c"
Cohesion: 0.31
Nodes (6): util_max(), shape_free(), clamp(), log_line(), util_report(), xmalloc()

### Community 1 - "main.c"
Cohesion: 0.29
Nodes (3): apply(), main(), shape_area()

## Knowledge Gaps
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `shape_new()` connect `shape_new` to `util.c`, `main.c`, `shape.c`, `test_area`?**
  _High betweenness centrality (0.297) - this node is a cross-community bridge._
- **Are the 3 inferred relationships involving `shape_new()` (e.g. with `main()` and `xmalloc()`) actually correct?**
  _`shape_new()` has 3 INFERRED edges - model-reasoned connections that need verification._
- **Why does `main()` connect `main.c` to `util.c`, `shape_new`?**
  _High betweenness centrality (0.267) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `main()` (e.g. with `shape_area()` and `shape_free()`) actually correct?**
  _`main()` has 4 INFERRED edges - model-reasoned connections that need verification._
- **Why does `shape_free()` connect `util.c` to `main.c`, `shape.c`, `test_area`?**
  _High betweenness centrality (0.167) - this node is a cross-community bridge._
- **Are the 3 inferred relationships involving `shape_free()` (e.g. with `main()` and `clamp()`) actually correct?**
  _`shape_free()` has 3 INFERRED edges - model-reasoned connections that need verification._
- **Are the 3 inferred relationships involving `test_area()` (e.g. with `shape_area()` and `shape_free()`) actually correct?**
  _`test_area()` has 3 INFERRED edges - model-reasoned connections that need verification._
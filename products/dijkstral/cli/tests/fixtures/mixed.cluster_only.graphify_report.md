# Graph Report - mixed  (2026-10-13)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 31 nodes · 31 edges · 7 communities (4 shown, 3 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
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
1. `build()` - 5 edges
2. `clean()` - 3 edges
3. `log()` - 3 edges
4. `compilerOptions` - 3 edges
5. `helper()` - 2 edges
6. `main()` - 2 edges
7. `scripts` - 2 edges
8. `compile_all()` - 2 edges
9. `build.sh script` - 2 edges
10. `typescript` - 1 edges

## Surprising Connections (you probably didn't know these)
- `build()` --calls--> `log()`  [EXTRACTED]
  scripts/build.sh → scripts/common.sh
- `clean()` --calls--> `log()`  [EXTRACTED]
  scripts/build.sh → scripts/common.sh

## Import Cycles
- None detected.

## Communities (7 total, 3 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.20
Nodes (9): devDependencies, typescript, name, scripts, build, version, react, typescript (+1 more)

### Community 1 - "Community 1"
Cohesion: 0.43
Nodes (6): build(), clean(), compile_all(), build.sh script, log(), common.sh script

### Community 2 - "Community 2"
Cohesion: 0.40
Nodes (4): compilerOptions, strict, target, include

### Community 4 - "Community 4"
Cohesion: 0.67
Nodes (3): dependencies, react, zod

## Knowledge Gaps
- **5 isolated node(s):** `react`, `typescript`, `zod`, `mixed-fixture`, `mixed-fixture`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 15 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `dependencies` connect `Community 4` to `Community 0`?**
  _High betweenness centrality (0.048) - this node is a cross-community bridge._
- **What connects `react`, `typescript`, `zod` to the rest of the system?**
  _5 weakly-connected nodes found - possible documentation gaps or missing edges._
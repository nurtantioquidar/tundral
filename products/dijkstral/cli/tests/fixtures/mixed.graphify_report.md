# Graph Report - mixed  (2026-10-29)

## Corpus Check
- 7 files · ~132 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 31 nodes · 34 edges · 7 communities (4 shown, 3 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- package.json
- build.sh
- dependencies
- compilerOptions
- main.py
- mixed-fixture
- mixed-fixture

## God Nodes (most connected - your core abstractions)
1. `build()` - 5 edges
2. `clean()` - 3 edges
3. `log()` - 3 edges
4. `compilerOptions` - 3 edges
5. `main()` - 2 edges
6. `helper()` - 2 edges
7. `scripts` - 2 edges
8. `react@^18.3.1` - 2 edges
9. `react` - 2 edges
10. `zod@3.23.8` - 2 edges

## Surprising Connections (you probably didn't know these)
- `clean()` --calls--> `log()`  [EXTRACTED]
  scripts/build.sh → scripts/common.sh
- `build()` --calls--> `log()`  [EXTRACTED]
  scripts/build.sh → scripts/common.sh

## Import Cycles
- None detected.

## Communities (7 total, 3 thin omitted)

### Community 0 - "package.json"
Cohesion: 0.29
Nodes (7): devDependencies, typescript@5.6.3, name, scripts, build, version, typescript

### Community 1 - "build.sh"
Cohesion: 0.43
Nodes (6): build(), clean(), compile_all(), build.sh script, log(), common.sh script

### Community 2 - "dependencies"
Cohesion: 0.40
Nodes (5): dependencies, react@^18.3.1, zod@3.23.8, react, zod

### Community 3 - "compilerOptions"
Cohesion: 0.40
Nodes (4): compilerOptions, strict, target, include

## Knowledge Gaps
- **2 isolated node(s):** `mixed-fixture`, `mixed-fixture`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 9 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `dependencies` connect `dependencies` to `package.json`?**
  _High betweenness centrality (0.024) - this node is a cross-community bridge._
- **What connects `mixed-fixture`, `mixed-fixture` to the rest of the system?**
  _2 weakly-connected nodes found - possible documentation gaps or missing edges._
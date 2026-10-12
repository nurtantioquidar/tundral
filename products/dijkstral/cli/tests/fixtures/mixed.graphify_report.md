# Graph Report - mixed  (2026-10-13)

## Corpus Check
- 7 files · ~132 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 31 nodes · 31 edges · 7 communities (4 shown, 3 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- package.json
- build.sh
- compilerOptions
- main.py
- dependencies
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
8. `build.sh script` - 2 edges
9. `compile_all()` - 2 edges
10. `build` - 1 edges

## Surprising Connections (you probably didn't know these)
- `clean()` --calls--> `log()`  [EXTRACTED]
  scripts/build.sh → scripts/common.sh
- `build()` --calls--> `log()`  [EXTRACTED]
  scripts/build.sh → scripts/common.sh

## Import Cycles
- None detected.

## Communities (7 total, 3 thin omitted)

### Community 0 - "package.json"
Cohesion: 0.20
Nodes (9): devDependencies, typescript, name, scripts, build, version, react, typescript (+1 more)

### Community 1 - "build.sh"
Cohesion: 0.43
Nodes (6): build(), clean(), compile_all(), build.sh script, log(), common.sh script

### Community 2 - "compilerOptions"
Cohesion: 0.40
Nodes (4): compilerOptions, strict, target, include

### Community 4 - "dependencies"
Cohesion: 0.67
Nodes (3): dependencies, react, zod

## Knowledge Gaps
- **5 isolated node(s):** `mixed-fixture`, `react`, `zod`, `typescript`, `mixed-fixture`
  These have ≤1 connection - possible missing edges. (Counts symbols only; 15 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `dependencies` connect `dependencies` to `package.json`?**
  _High betweenness centrality (0.048) - this node is a cross-community bridge._
- **What connects `mixed-fixture`, `react`, `zod` to the rest of the system?**
  _5 weakly-connected nodes found - possible documentation gaps or missing edges._
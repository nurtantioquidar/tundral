# Graph Report - astro  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 22 nodes · 32 edges · 4 communities (2 shown, 2 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3

## God Nodes (most connected - your core abstractions)
1. `getPosts()` - 4 edges
2. `readingTime()` - 4 edges
3. `sortByDate()` - 4 edges
4. `getStaticPaths()` - 3 edges
5. `Post` - 2 edges
6. `track()` - 2 edges
7. `bump()` - 2 edges
8. `Like()` - 2 edges
9. `Props` - 1 edges
10. `Props` - 1 edges

## Surprising Connections (you probably didn't know these)
- `getStaticPaths()` --calls--> `getPosts()`  [EXTRACTED]
  src/pages/index.astro → src/utils/posts.ts
- `getStaticPaths()` --calls--> `sortByDate()`  [EXTRACTED]
  src/pages/index.astro → src/utils/posts.ts
- `track()` --calls--> `readingTime()`  [EXTRACTED]
  src/pages/index.astro → src/utils/posts.ts

## Import Cycles
- None detected.

## Communities (4 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.62
Nodes (5): getStaticPaths(), track(), getPosts(), readingTime(), sortByDate()

### Community 2 - "Community 2"
Cohesion: 0.40
Nodes (3): minutes, Props, Post

## Knowledge Gaps
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `readingTime()` connect `Community 0` to `Community 2`?**
  _High betweenness centrality (0.014) - this node is a cross-community bridge._
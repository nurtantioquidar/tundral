# Graph Report - astro  (2026-10-14)

## Corpus Check
- 7 files · ~289 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 22 nodes · 32 edges · 4 communities (2 shown, 2 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- index.astro
- Base.astro
- PostCard.astro
- Like.tsx

## God Nodes (most connected - your core abstractions)
1. `sortByDate()` - 4 edges
2. `readingTime()` - 4 edges
3. `getPosts()` - 4 edges
4. `getStaticPaths()` - 3 edges
5. `Like()` - 2 edges
6. `bump()` - 2 edges
7. `track()` - 2 edges
8. `Post` - 2 edges
9. `Props` - 1 edges
10. `minutes` - 1 edges

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

### Community 0 - "index.astro"
Cohesion: 0.62
Nodes (5): getStaticPaths(), track(), getPosts(), readingTime(), sortByDate()

### Community 2 - "PostCard.astro"
Cohesion: 0.40
Nodes (3): minutes, Props, Post

## Knowledge Gaps
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `readingTime()` connect `index.astro` to `PostCard.astro`?**
  _High betweenness centrality (0.014) - this node is a cross-community bridge._
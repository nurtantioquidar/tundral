# Graph Report - mixed  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 24 nodes · 25 edges · 5 communities (2 shown, 3 thin omitted)
- Extraction: 64% EXTRACTED · 8% INFERRED · 28% AMBIGUOUS · INFERRED: 2 edges (avg confidence: 0.7)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3

## God Nodes (most connected - your core abstractions)
1. `JobQueue` - 5 edges
2. `run_job()` - 3 edges
3. `slugify()` - 3 edges
4. `queue` - 3 edges
5. `cache` - 2 edges
6. `Alpha` - 2 edges
7. `Bravo` - 2 edges
8. `Charlie` - 2 edges
9. `Delta` - 2 edges
10. `Echo` - 2 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Import Cycles
- None detected.

## Hyperedges (group relationships)
- **Aaa group** — readme_e, readme_f, readme_g [AMBIGUOUS]
- **No id group** — readme_e, readme_h [EXTRACTED 1.00]
- **Bbb group** — readme_a, readme_b [INFERRED 0.33]
- **Ccc group** — readme_c, readme_d [INFERRED]

## Communities (5 total, 3 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.29
Nodes (8): Alpha, Bravo, Charlie, Delta, Echo, Foxtrot, Golf, Hotel

### Community 1 - "Community 1"
Cohesion: 0.33
Nodes (5): cache, policy, queue, max_items, name

## Ambiguous Edges - Review These
- `Alpha` → `Hotel`  [AMBIGUOUS]
  README.md · relation: rel_one
- `Bravo` → `Charlie`  [AMBIGUOUS]
  README.md · relation: rel_two
- `Charlie` → `Delta`  [AMBIGUOUS]
  README.md · relation: rel_three
- `Delta` → `Echo`  [AMBIGUOUS]
  README.md · relation: rel_four
- `Echo` → `Foxtrot`  [AMBIGUOUS]
  README.md · relation: rel_five
- `Foxtrot` → `Golf`  [AMBIGUOUS]
  README.md · relation: rel_six
- `Golf` → `Hotel`  [AMBIGUOUS]
  README.md · relation: rel_seven

## Knowledge Gaps
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.
- **High ambiguity: 28% of edges are AMBIGUOUS.** Review the Ambiguous Edges section above.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `JobQueue` connect `Community 2` to `Community 3`?**
  _High betweenness centrality (0.054) - this node is a cross-community bridge._

### Low-confidence Hints
_AMBIGUOUS edges — the extractor was unsure. Verify before acting on these._

- **What is the exact relationship between `Alpha` and `Hotel`?**
  _Edge tagged AMBIGUOUS (relation: rel_one) - confidence is low._
- **What is the exact relationship between `Bravo` and `Charlie`?**
  _Edge tagged AMBIGUOUS (relation: rel_two) - confidence is low._
- **What is the exact relationship between `Charlie` and `Delta`?**
  _Edge tagged AMBIGUOUS (relation: rel_three) - confidence is low._
- **What is the exact relationship between `Delta` and `Echo`?**
  _Edge tagged AMBIGUOUS (relation: rel_four) - confidence is low._
- **What is the exact relationship between `Echo` and `Foxtrot`?**
  _Edge tagged AMBIGUOUS (relation: rel_five) - confidence is low._
- **What is the exact relationship between `Foxtrot` and `Golf`?**
  _Edge tagged AMBIGUOUS (relation: rel_six) - confidence is low._
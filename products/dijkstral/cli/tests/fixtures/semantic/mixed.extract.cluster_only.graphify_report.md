# Graph Report - mixed  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 31 nodes · 48 edges · 6 communities (5 shown, 1 thin omitted)
- Extraction: 94% EXTRACTED · 4% INFERRED · 2% AMBIGUOUS · INFERRED: 2 edges (avg confidence: 0.9)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Community 0
- Community 1
- Community 2
- Community 3
- Community 4

## God Nodes (most connected - your core abstractions)
1. `JobQueue` - 7 edges
2. `Write-through cache` - 7 edges
3. `Job Service README` - 7 edges
4. `run_job` - 6 edges
5. `slugify` - 6 edges
6. `JobQueue` - 5 edges
7. `Write-Through Caching for Job Queues` - 5 edges
8. `Architecture diagram: README → run_job → JobQueue; run_job → slugify → Cache` - 5 edges
9. `run_job()` - 3 edges
10. `slugify()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `Write-Through Caching for Job Queues` --references--> `JobQueue.push`  [EXTRACTED]
  papers/write_through.pdf → docs/guide.md
- `Write-Through Caching for Job Queues` --conceptually_related_to--> `slugify`  [INFERRED]
  papers/write_through.pdf → README.md
- `Architecture diagram: README → run_job → JobQueue; run_job → slugify → Cache` --references--> `JobQueue`  [EXTRACTED]
  images/architecture.png → README.md
- `Architecture diagram: README → run_job → JobQueue; run_job → slugify → Cache` --references--> `run_job`  [EXTRACTED]
  images/architecture.png → README.md
- `Write-Through Caching for Job Queues` --references--> `Write-through cache`  [EXTRACTED]
  papers/write_through.pdf → README.md

## Import Cycles
- None detected.

## Hyperedges (group relationships)
- **Job execution connects queue processing with slug normalization and caching** — readme_run_job, readme_jobqueue, readme_slugify, readme_cache [EXTRACTED 0.95]
- **Jobs are pushed onto JobQueue and then drained by run_job** — docs_glossary_job, docs_guide_jobqueue_push, readme_jobqueue, readme_run_job [EXTRACTED 1.00]

## Communities (6 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.33
Nodes (3): JobQueue, run_job(), slugify()

### Community 1 - "Community 1"
Cohesion: 0.33
Nodes (5): cache, policy, queue, max_items, name

### Community 2 - "Community 2"
Cohesion: 0.47
Nodes (6): Operator Guide, JobQueue.push, Converted capacity sheet: JobQueue 120 jobs/s; Cache 900 jobs/s, Queue Runbook: process pending jobs, monitor cache hit rate, page JobQueue owner when latency exceeds 2 seconds, JobQueue, run_job

### Community 3 - "Community 3"
Cohesion: 0.67
Nodes (6): Architecture diagram: README → run_job → JobQueue; run_job → slugify → Cache, Write-Through Caching for Job Queues, Lazy Invalidation in Distributed Caches (2019), Job Service README, Write-through cache, slugify

### Community 4 - "Community 4"
Cohesion: 0.67
Nodes (3): Job service glossary, Job: unit of work pushed onto JobQueue, Slug: normalized job name produced by slugify

## Ambiguous Edges - Review These
- `run_job` → `slugify`  [AMBIGUOUS]
  images/architecture.png · relation: calls

## Knowledge Gaps
- **1 isolated node(s):** `Lazy Invalidation in Distributed Caches (2019)`
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 8 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Write-through cache` connect `Community 3` to `Community 2`, `Community 4`?**
  _High betweenness centrality (0.045) - this node is a cross-community bridge._
- **What connects `Lazy Invalidation in Distributed Caches (2019)` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `JobQueue` connect `Community 2` to `Community 3`, `Community 4`?**
  _High betweenness centrality (0.040) - this node is a cross-community bridge._
- **Why does `Write-Through Caching for Job Queues` connect `Community 3` to `Community 2`?**
  _High betweenness centrality (0.039) - this node is a cross-community bridge._

### Low-confidence Hints
_AMBIGUOUS edges — the extractor was unsure. Verify before acting on these._

- **What is the exact relationship between `run_job` and `slugify`?**
  _Edge tagged AMBIGUOUS (relation: calls) - confidence is low._
# Graph Report - gold-upd  (2026-10-29)

## Corpus Check
- 8 files · ~317 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 38 nodes · 60 edges · 11 communities (5 shown, 6 thin omitted)
- Extraction: 87% EXTRACTED · 12% INFERRED · 2% AMBIGUOUS · INFERRED: 7 edges (avg confidence: 0.94)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- service.py
- queue
- JobQueue
- README.md
- Job service glossary
- Write-Through Caching for Job Queues
- run_job
- Job Service
- JobQueue
- capacity_e7fd8f6a.md

## God Nodes (most connected - your core abstractions)
1. `JobQueue` - 7 edges
2. `Write-through cache` - 7 edges
3. `JobQueue` - 6 edges
4. `run_job` - 6 edges
5. `slugify` - 6 edges
6. `run_job()` - 5 edges
7. `Write-Through Caching for Job Queues` - 5 edges
8. `Architecture diagram: README → run_job → JobQueue; run_job → slugify → Cache` - 5 edges
9. `slugify()` - 4 edges
10. `Job Service` - 4 edges

## Surprising Connections (you probably didn't know these)
- `Job Service` --references--> `JobQueue`  [INFERRED]
  README.md → src/app/service.py
- `Job Service` --references--> `run_job()`  [INFERRED]
  README.md → src/app/service.py
- `Caching` --references--> `slugify()`  [INFERRED]
  README.md → src/app/util.py
- `Write-Through Caching for Job Queues` --conceptually_related_to--> `slugify`  [INFERRED]
  papers/write_through.pdf → README.md
- `Running jobs` --references--> `run_job()`  [INFERRED]
  docs/guide.md → src/app/service.py

## Import Cycles
- None detected.

## Hyperedges (group relationships)
- **Job execution connects queue processing with slug normalization and caching** — readme_run_job, readme_jobqueue, readme_slugify, readme_cache [EXTRACTED 0.95]
- **Jobs are pushed onto JobQueue and then drained by run_job** — docs_glossary_job, docs_guide_jobqueue_push, readme_jobqueue, readme_run_job [EXTRACTED 1.00]

## Communities (11 total, 6 thin omitted)

### Community 1 - "queue"
Cohesion: 0.33
Nodes (5): cache, policy, queue, max_items, name

### Community 3 - "README.md"
Cohesion: 1.00
Nodes (3): Architecture diagram: README → run_job → JobQueue; run_job → slugify → Cache, Write-through cache, slugify

### Community 4 - "Job service glossary"
Cohesion: 0.67
Nodes (3): Job service glossary, Job: unit of work pushed onto JobQueue, Slug: normalized job name produced by slugify

### Community 6 - "Write-Through Caching for Job Queues"
Cohesion: 0.33
Nodes (5): Capacity, JobQueue.push, Operator Guide, Write-Through Caching for Job Queues, Lazy Invalidation in Distributed Caches (2019)

### Community 8 - "Job Service"
Cohesion: 0.67
Nodes (3): Caching, Decision, Job Service

## Ambiguous Edges - Review These
- `run_job` → `slugify`  [AMBIGUOUS]
  images/architecture.png · relation: calls

## Knowledge Gaps
- **1 isolated node(s):** `Lazy Invalidation in Distributed Caches (2019)`
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 11 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **6 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Job Service` connect `Job Service` to `JobQueue`, `README.md`, `run_job`?**
  _High betweenness centrality (0.280) - this node is a cross-community bridge._
- **What connects `Lazy Invalidation in Distributed Caches (2019)` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `JobQueue` connect `JobQueue` to `Job Service`, `service.py`, `run_job`?**
  _High betweenness centrality (0.131) - this node is a cross-community bridge._
- **Why does `JobQueue` connect `JobQueue` to `capacity_e7fd8f6a.md`, `README.md`, `Job service glossary`, `Write-Through Caching for Job Queues`?**
  _High betweenness centrality (0.092) - this node is a cross-community bridge._

### Low-confidence Hints
_AMBIGUOUS edges — the extractor was unsure. Verify before acting on these._

- **What is the exact relationship between `run_job` and `slugify`?**
  _Edge tagged AMBIGUOUS (relation: calls) - confidence is low._
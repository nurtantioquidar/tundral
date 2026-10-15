# Graph Report - gold-upd  (2026-10-15)

## Corpus Check
- 7 files · ~823 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 33 nodes · 57 edges · 9 communities (5 shown, 4 thin omitted)
- Extraction: 84% EXTRACTED · 14% INFERRED · 2% AMBIGUOUS · INFERRED: 8 edges (avg confidence: 0.94)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- README.md
- service.py
- JobQueue
- run_job
- Job service glossary
- Job Service
- JobQueue
- capacity_e7fd8f6a.md

## God Nodes (most connected - your core abstractions)
1. `Write-through cache` - 8 edges
2. `JobQueue` - 8 edges
3. `JobQueue` - 6 edges
4. `slugify` - 6 edges
5. `run_job` - 6 edges
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
- `Running jobs` --references--> `run_job()`  [INFERRED]
  docs/guide.md → src/app/service.py
- `Write-Through Caching for Job Queues` --references--> `JobQueue.push`  [EXTRACTED]
  papers/write_through.pdf → docs/guide.md

## Import Cycles
- None detected.

## Hyperedges (group relationships)
- **Job execution connects queue processing with slug normalization and caching** — readme_run_job, readme_jobqueue, readme_slugify, readme_cache [EXTRACTED 0.95]
- **Jobs are pushed onto JobQueue and then drained by run_job** — docs_glossary_job, docs_guide_jobqueue_push, readme_jobqueue, readme_run_job [EXTRACTED 1.00]

## Communities (9 total, 4 thin omitted)

### Community 0 - "README.md"
Cohesion: 0.67
Nodes (5): Architecture diagram: README → run_job → JobQueue; run_job → slugify → Cache, Write-Through Caching for Job Queues, Lazy Invalidation in Distributed Caches (2019), Write-through cache, slugify

### Community 2 - "JobQueue"
Cohesion: 0.47
Nodes (4): Queue configuration: jobs, maximum 1000 items, write-through cache, JobQueue.push, JobQueue, run_job

### Community 4 - "run_job"
Cohesion: 0.50
Nodes (4): Capacity, Operator Guide, Running jobs, run_job()

### Community 5 - "Job service glossary"
Cohesion: 0.67
Nodes (3): Job service glossary, Job: unit of work pushed onto JobQueue, Slug: normalized job name produced by slugify

### Community 6 - "Job Service"
Cohesion: 0.67
Nodes (3): Caching, Decision, Job Service

## Ambiguous Edges - Review These
- `slugify` → `run_job`  [AMBIGUOUS]
  images/architecture.png · relation: calls

## Knowledge Gaps
- **1 isolated node(s):** `Lazy Invalidation in Distributed Caches (2019)`
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 8 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Job Service` connect `Job Service` to `README.md`, `run_job`, `JobQueue`?**
  _High betweenness centrality (0.399) - this node is a cross-community bridge._
- **What connects `Lazy Invalidation in Distributed Caches (2019)` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `JobQueue` connect `JobQueue` to `service.py`, `run_job`, `Job Service`?**
  _High betweenness centrality (0.183) - this node is a cross-community bridge._
- **Why does `JobQueue` connect `JobQueue` to `capacity_e7fd8f6a.md`, `README.md`, `Job service glossary`?**
  _High betweenness centrality (0.153) - this node is a cross-community bridge._

### Low-confidence Hints
_AMBIGUOUS edges — the extractor was unsure. Verify before acting on these._

- **What is the exact relationship between `slugify` and `run_job`?**
  _Edge tagged AMBIGUOUS (relation: calls) - confidence is low._
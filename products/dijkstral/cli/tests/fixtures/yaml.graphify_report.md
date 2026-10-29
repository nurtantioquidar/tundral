# Graph Report - yaml  (2026-10-29)

## Corpus Check
- 17 files · ~253 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 108 nodes · 105 edges · 13 communities (9 shown, 4 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- Deployment shop
- config/app.yaml
- steps
- cache
- job.yaml
- server
- services
- workers
- Service shop
- k
- k
- .travis.yml

## God Nodes (most connected - your core abstractions)
1. `steps` - 5 edges
2. `workers` - 5 edges
3. `Deployment shop` - 5 edges
4. `Service shop` - 5 edges
5. `server` - 4 edges
6. `on` - 3 edges
7. `test` - 3 edges
8. `tls` - 3 edges
9. `database` - 3 edges
10. `pool` - 3 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Import Cycles
- None detected.

## Communities (13 total, 4 thin omitted)

### Community 0 - "Deployment shop"
Cohesion: 0.11
Nodes (18): data, kind, mode, apiVersion, kind, metadata, spec, Deployment shop (+10 more)

### Community 1 - "config/app.yaml"
Cohesion: 0.13
Nodes (14): 0x1F, esc"aped, csv, features, export, search, name, notes (+6 more)

### Community 2 - "steps"
Cohesion: 0.13
Nodes (14): CI, jobs, test, name, on, pull_request, push, branches (+6 more)

### Community 3 - "cache"
Cohesion: 0.22
Nodes (11): cache, driver, pool, database, driver, pool, max, min (+3 more)

### Community 4 - "job.yaml"
Cohesion: 0.18
Nodes (10): apiVersion, image, name, kind, metadata, name, spec, containers (+2 more)

### Community 5 - "server"
Cohesion: 0.29
Nodes (7): ciphers, server, host, port, tls, cert, options

### Community 6 - "services"
Cohesion: 0.29
Nodes (6): image, services, db, web, depends_on, image

### Community 7 - "workers"
Cohesion: 0.33
Nodes (6): attempts, workers, name, nested, queue, retry

### Community 8 - "Service shop"
Cohesion: 0.33
Nodes (6): Service shop, port, apiVersion, kind, spec, ports

## Knowledge Gaps
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `server` connect `server` to `config/app.yaml`?**
  _High betweenness centrality (0.035) - this node is a cross-community bridge._
- **Should `Deployment shop` be split into smaller, more focused modules?**
  _Cohesion score 0.1111111111111111 - nodes in this community are weakly interconnected._
- **Why does `workers` connect `workers` to `config/app.yaml`?**
  _High betweenness centrality (0.031) - this node is a cross-community bridge._
- **Should `config/app.yaml` be split into smaller, more focused modules?**
  _Cohesion score 0.13333333333333333 - nodes in this community are weakly interconnected._
- **Should `steps` be split into smaller, more focused modules?**
  _Cohesion score 0.13333333333333333 - nodes in this community are weakly interconnected._
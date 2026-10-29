# Graph Report - yaml  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 108 nodes · 105 edges · 13 communities (9 shown, 4 thin omitted)
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
- Community 7
- Community 8
- Community 9
- Community 10
- Community 11

## God Nodes (most connected - your core abstractions)
1. `Deployment shop` - 5 edges
2. `steps` - 5 edges
3. `workers` - 5 edges
4. `Service shop` - 5 edges
5. `server` - 4 edges
6. `metadata` - 3 edges
7. `ConfigMap` - 3 edges
8. `doc 3` - 3 edges
9. `name` - 3 edges
10. `features` - 3 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Import Cycles
- None detected.

## Communities (13 total, 4 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.11
Nodes (18): data, kind, mode, apiVersion, kind, metadata, spec, Deployment shop (+10 more)

### Community 1 - "Community 1"
Cohesion: 0.13
Nodes (14): 0x1F, esc"aped, csv, features, export, search, name, notes (+6 more)

### Community 2 - "Community 2"
Cohesion: 0.13
Nodes (14): CI, jobs, test, name, on, pull_request, push, branches (+6 more)

### Community 3 - "Community 3"
Cohesion: 0.22
Nodes (11): cache, driver, pool, database, driver, pool, max, min (+3 more)

### Community 4 - "Community 4"
Cohesion: 0.18
Nodes (10): apiVersion, image, name, kind, metadata, name, spec, containers (+2 more)

### Community 5 - "Community 5"
Cohesion: 0.29
Nodes (7): ciphers, server, host, port, tls, cert, options

### Community 6 - "Community 6"
Cohesion: 0.29
Nodes (6): image, services, db, web, depends_on, image

### Community 7 - "Community 7"
Cohesion: 0.33
Nodes (6): attempts, workers, name, nested, queue, retry

### Community 8 - "Community 8"
Cohesion: 0.33
Nodes (6): Service shop, port, apiVersion, kind, spec, ports

## Knowledge Gaps
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `server` connect `Community 5` to `Community 1`?**
  _High betweenness centrality (0.035) - this node is a cross-community bridge._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.1111111111111111 - nodes in this community are weakly interconnected._
- **Why does `workers` connect `Community 7` to `Community 1`?**
  _High betweenness centrality (0.031) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.13333333333333333 - nodes in this community are weakly interconnected._
- **Should `Community 2` be split into smaller, more focused modules?**
  _Cohesion score 0.13333333333333333 - nodes in this community are weakly interconnected._
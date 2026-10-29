# Graph Report - dart  (2026-10-29)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 67 nodes · 72 edges · 7 communities (5 shown, 2 thin omitted)
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

## God Nodes (most connected - your core abstractions)
1. `User` - 9 edges
2. `Entity` - 3 edges
3. `Printable` - 3 edges
4. `Account` - 3 edges
5. `Comparable2` - 2 edges
6. `UserLabels` - 2 edges
7. `Repository` - 2 edges
8. `UserService` - 2 edges
9. `Address` - 1 edges
10. `city` - 1 edges

## Surprising Connections (you probably didn't know these)
- `User` --implements--> `Comparable2`  [EXTRACTED]
  lib/src/models/user.dart → lib/src/models/base.dart
- `User` --inherits--> `Entity`  [EXTRACTED]
  lib/src/models/user.dart → lib/src/models/base.dart
- `User` --mixes_in--> `Printable`  [EXTRACTED]
  lib/src/models/user.dart → lib/src/models/base.dart

## Import Cycles
- None detected.

## Communities (7 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.12
Nodes (12): Address, city, compareTo, display, guest, home, id, label (+4 more)

### Community 1 - "Community 1"
Cohesion: 0.13
Nodes (9): all, everything, find, isUser, owner, register, save, store (+1 more)

### Community 2 - "Community 2"
Cohesion: 0.19
Nodes (10): Comparable2, describe, Entity, id, Printable, printMe, sameAs, Account (+2 more)

### Community 3 - "Community 3"
Cohesion: 0.18
Nodes (4): loadLibrary, run, service, shop

### Community 4 - "Community 4"
Cohesion: 0.40
Nodes (4): defaultWidth, log, stamp, title

## Knowledge Gaps
- **34 isolated node(s):** `Address`, `city`, `compareTo`, `display`, `guest` (+29 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 52 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `User` connect `Community 2` to `Community 0`, `Community 1`, `Community 5`?**
  _High betweenness centrality (0.280) - this node is a cross-community bridge._
- **What connects `Address`, `city`, `compareTo` to the rest of the system?**
  _34 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.11764705882352941 - nodes in this community are weakly interconnected._
- **Why does `Account` connect `Community 2` to `Community 0`, `Community 1`?**
  _High betweenness centrality (0.071) - this node is a cross-community bridge._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.13333333333333333 - nodes in this community are weakly interconnected._
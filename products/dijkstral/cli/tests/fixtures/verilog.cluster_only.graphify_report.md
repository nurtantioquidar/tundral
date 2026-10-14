# Graph Report - verilog  (2026-10-14)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 41 nodes · 42 edges · 9 communities (4 shown, 5 thin omitted)
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
1. `top` - 12 edges
2. `alu_txn` - 6 edges
3. `ref_model` - 4 edges
4. `fifo` - 4 edges
5. `ram` - 3 edges
6. `alu` - 3 edges
7. `base_txn` - 3 edges
8. `counter` - 2 edges
9. `top_ctrl` - 2 edges
10. `fifo_flags` - 2 edges

## Surprising Connections (you probably didn't know these)
- `tb_top` --instantiates--> `top`  [EXTRACTED]
  tb/tb_top.v → rtl/top.v
- `top` --instantiates--> `ref_model`  [EXTRACTED]
  rtl/top.v → tb/model.py
- `top` --instantiates--> `counter`  [EXTRACTED]
  rtl/top.v → counter.v
- `fifo` --instantiates--> `ram`  [EXTRACTED]
  rtl/fifo.v → mem/ram.v
- `fifo_flags` --instantiates--> `ram`  [EXTRACTED]
  rtl/fifo.v → mem/ram.v

## Import Cycles
- None detected.

## Communities (9 total, 5 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.22
Nodes (5): counter, keep_hierarchy, top, top_ctrl, reset_all

### Community 1 - "Community 1"
Cohesion: 0.29
Nodes (5): dup, ram, dup, fifo, fifo_flags

### Community 3 - "Community 3"
Cohesion: 0.40
Nodes (5): alu_txn, show, width, cloneable, printable

### Community 5 - "Community 5"
Cohesion: 0.50
Nodes (3): later_txn, pending, tb_top

## Knowledge Gaps
- **9 isolated node(s):** `keep_hierarchy`, `dup`, `dup`, `show`, `width` (+4 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 23 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `top` connect `Community 0` to `Community 1`, `Community 2`, `Community 4`, `Community 5`?**
  _High betweenness centrality (0.763) - this node is a cross-community bridge._
- **What connects `keep_hierarchy`, `dup`, `dup` to the rest of the system?**
  _9 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `tb_top` connect `Community 5` to `Community 0`?**
  _High betweenness centrality (0.359) - this node is a cross-community bridge._
- **Why does `fifo` connect `Community 1` to `Community 0`?**
  _High betweenness centrality (0.250) - this node is a cross-community bridge._
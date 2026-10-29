# Graph Report - verilog  (2026-10-29)

## Corpus Check
- 9 files · ~431 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 42 nodes · 43 edges · 9 communities (4 shown, 5 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- top
- fifo
- alu.v
- alu_txn
- ref_model
- tb_top.v
- base_txn

## God Nodes (most connected - your core abstractions)
1. `top` - 12 edges
2. `alu_txn` - 7 edges
3. `fifo` - 4 edges
4. `ref_model` - 4 edges
5. `ram` - 3 edges
6. `alu` - 3 edges
7. `base_txn` - 3 edges
8. `counter` - 2 edges
9. `fifo_flags` - 2 edges
10. `top_ctrl` - 2 edges

## Surprising Connections (you probably didn't know these)
- `tb_top` --instantiates--> `top`  [EXTRACTED]
  tb/tb_top.v → rtl/top.v
- `top` --instantiates--> `counter`  [EXTRACTED]
  rtl/top.v → counter.v
- `top` --instantiates--> `ref_model`  [EXTRACTED]
  rtl/top.v → tb/model.py
- `fifo` --instantiates--> `ram`  [EXTRACTED]
  rtl/fifo.v → mem/ram.v
- `fifo_flags` --instantiates--> `ram`  [EXTRACTED]
  rtl/fifo.v → mem/ram.v

## Import Cycles
- None detected.

## Communities (9 total, 5 thin omitted)

### Community 0 - "top"
Cohesion: 0.22
Nodes (5): counter, keep_hierarchy, top, top_ctrl, reset_all

### Community 1 - "fifo"
Cohesion: 0.29
Nodes (5): dup, ram, dup, fifo, fifo_flags

### Community 3 - "alu_txn"
Cohesion: 0.33
Nodes (6): alu_txn, drive, show, width, cloneable, printable

### Community 5 - "tb_top.v"
Cohesion: 0.50
Nodes (3): later_txn, pending, tb_top

## Knowledge Gaps
- **10 isolated node(s):** `dup`, `dup`, `keep_hierarchy`, `show`, `printable` (+5 more)
  These have ≤1 connection - possible missing edges. (Counts symbols only; 24 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `top` connect `top` to `fifo`, `alu.v`, `ref_model`, `tb_top.v`?**
  _High betweenness centrality (0.759) - this node is a cross-community bridge._
- **What connects `dup`, `dup`, `keep_hierarchy` to the rest of the system?**
  _10 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `tb_top` connect `tb_top.v` to `top`?**
  _High betweenness centrality (0.376) - this node is a cross-community bridge._
- **Why does `fifo` connect `fifo` to `top`?**
  _High betweenness centrality (0.245) - this node is a cross-community bridge._
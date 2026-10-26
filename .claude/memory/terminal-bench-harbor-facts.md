---
name: terminal-bench-harbor-facts
description: Terminal-Bench runs via Harbor (`uvx --from harbor harbor`); dataset id terminal-bench/terminal-bench@4.0.0 (66 tasks); Butterfly adapter lives in products/butterfly/bench/harbor
metadata:
  type: reference
---

Terminal-Bench is run through Harbor: `uvx --from harbor harbor run -d
terminal-bench/terminal-bench@4.0.0 -a <agent> -m provider/model -k 5` (official page
https://www.tbench.ai/run). Verified 2026-09-03: Harbor 0.22.0, TB 4.0.0 has 66 tasks (a few
declare GPUs); the legacy registry also lists `terminal-bench@2.0` (89) and
`terminal-bench-sample@2.0` (10). Butterfly's adapter, static Linux build script, dev subset,
matrix runner, and summarizer are in `products/butterfly/bench/harbor/` (see its README). No paid
run has been executed as of 2026-09-04. The user calls this track "TB4".

---
name: butterfly-release-state
description: Butterfly's measured state and defaults at the 2026-09-16 release to five engineers; what to compare future results against
metadata:
  type: project
---

Released to five engineers on 2026-09-16 at commit e393a96. Defaults: lean on (`--no-lean` opts out),
outline reads at 400 lines, proven first claim on, requirements required in complete_task, completion
gate opt-in. Reference numbers (private suites): Flink GLM 18/18 at $0.033/trial and 20 calls, DeepSeek
18/18 at ~$0.017 off-peak; services (12 rewritten tasks) GLM 20/24 $0.053/success, DeepSeek 19/24.
OpenCode on the same tasks: 17/18 Flink at $0.048 (GLM), 19/24 services at $0.065.

**Why:** later feedback from the engineers and any public-suite run should be read against these figures;
statement quality (thin check) explains nearly every miss.

**How to apply:** onboarding is `docs/butterfly-onboarding.md`; ask engineers for statement + run summary +
`telemetry.jsonl`. Do not re-run verifier/clarify/output-cap experiments (measured dead). See
[[private-task-suite]] and [[butterfly-nimble-direction]].

---
name: butterfly-nimble-direction
description: 2026-09-08 decision - Butterfly is the sellable product (strong harness); Nimble is hard to sell alone, routing likely becomes an Inver feature; stop chasing Muse Spark
metadata:
  type: project
---

On 2026-09-08 the user concluded they cannot sell Nimble on its own and want Butterfly positioned as
a strong harness. All Terminal-Bench jobs and the Nimble router were stopped at their request.

**Why:** two days of benchmarking showed the harness moves reliability and cost (half the tokens,
97% cache, truthful verdicts, no crashes) but pass rate is decided by the model; Nimble matched but
did not beat the best fixed model (GLM-5.3-Flash, 5/12) because the pool had no stronger model to
escalate to. Muse Spark 1.3 Contributor (Meta, $0.10/$0.20) went 0/6 on tasks GLM passes, cannot use
`reasoning_effort: max` (standard tier only), and thinks in silence for minutes.

**How to apply:** frame Butterfly work around cost per verified task versus mini-swe-agent on the
same model (baseline still unmeasured beyond 1 trial: mini-swe-agent+GLM). Next harness levers in
order: independent verifier pass (raises first-attempt success and gives an early-stop/escalation
signal), persistent terminal tool, sequential fresh attempts behind a cost-capped flag (parallel
best-of-n raises cost per success). Routing's home is a feature of Inver (metered gateway) via the existing
nimble/v1 contract; recorded 2026-09-16 in `docs/nimble-routing-decision.md`, which also states the
three conditions that reopen it and the migration shape. Nimble's router is frozen at v4. See
[[butterfly-product]], [[nimble-product]].

---
name: show-tradeoffs-before-side-effects
description: User wants trade-offs presented for a choice BEFORE applying any change with side effects (e.g. cache-prefix invalidation in hx)
metadata: 
  node_type: memory
  type: feedback
  originSessionId: e85766da-7000-4adb-b0f1-683641ec9bf9
  modified: 2026-08-30T07:09:43.608Z
---

When a fix or improvement carries a side effect — even a small, quantified one — do not apply it
and report the side effect afterward. Present the trade-off first and let the user choose.

**Why:** In the hx harness (2026-08-30), a system-prompt improvement was applied that would cost a
one-time ~$0.0004 prefix re-cache; the user said "I don't want any side effect, show me the
trade-off first let me choose" and chose to revert. hx AGENTS.md invariant 9 now codifies this for
prompt-prefix bytes specifically.

**How to apply:** Separate zero-side-effect changes (apply freely) from side-effectful ones (ask
with quantified cost/benefit via options). Applies especially to anything touching
[[harness-experimental-rust-cli]] cached prompt bytes, but treat it as a general working style.

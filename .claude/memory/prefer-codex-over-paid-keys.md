---
name: prefer-codex-over-paid-keys
description: "For LLM calls in benchmarks/tests, use the user's Codex subscription (codex exec), not paid API keys like DeepSeek"
metadata:
  node_type: memory
  type: feedback
  originSessionId: ec8aa4f3-bd29-4a61-bf8d-d0a3ac062b7f
  modified: 2026-10-15T08:46:24.741Z
---

2026-10-15, dijkstral stage 5: asked for a DeepSeek spend cap, the user answered "prefer to use codex then". Codex CLI is installed at ~/.local/bin/codex and logged in with ChatGPT. Default semantic backend = Codex when logged in (user picked this).

**Why:** the subscription is already paid for; per-token API spend is not wanted.
**How to apply:** route model calls through `codex exec` (for tools that only speak OpenAI-compatible HTTP, e.g. Graphify, use the bench shim `products/dijkstral/bench/llm_shim.py`). Do not read DeepSeek/OpenAI keys from 1Password for automated runs unless the user asks again; keep request volume modest because plan usage limits apply. See [[dijkstral-product]], [[provider-keys-1password]].

2026-10-16, stage 7: the user said "if my quota runs out, just ping me I have codex banked reset quota" — on a Codex usage-limit error, stop the run cleanly (resumable), notify the user (PushNotification if away) and wait; never fall back to paid keys.

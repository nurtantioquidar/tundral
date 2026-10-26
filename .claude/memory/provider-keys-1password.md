---
name: provider-keys-1password
description: Provider API keys live in 1Password (vault Personal, items api_key_{deepseek,zai,openai}__monet.harness, field password); load via op read, never print or persist
metadata:
  type: reference
---

The user's provider keys are in 1Password, vault `Personal`: `api_key_deepseek__monet.harness`,
`api_key_zai__monet.harness` (Z.ai / GLM), `api_key_openai__monet.harness`; each is a PASSWORD
item whose `password` field is the key. The `op` CLI (2.34) works through the desktop-app
integration even when `op whoami` says not signed in. Load with
`op read "op://Personal/<item>/password"` straight into a child process environment —
`products/butterfly/bench/harbor/with-keys.sh <cmd>` does exactly that. Never echo, log, or
write the values; the user authorized using them for live smoke tests and benchmarks (2026-09-04).

Meta Model API (Muse Spark, added 2026-09-06): base `https://api.meta.ai/v1`, OpenAI-compatible
chat with tools + `reasoning_effort`; Butterfly/Nimble/bench read `META_API_KEY` (Meta's docs call
it MODEL_API_KEY). Env-file path for unattended runs: `~/.config/monet/bench.env` (0600), read by
`with-keys.sh` before Keychain and 1Password (item name `api_key_meta__monet.harness` if stored
there). Models: `muse-spark-1.3-contributor` ($0.10/$0.002/$0.20 per 1M, trains on prompts) and
`muse-spark-1.3` ($1.25/$0.15/$4.25), 1,048,576 context.

---
name: typesafe-jev-classifier
description: TypeSafe Jev classifier key lives in 1Password api_key_typesafeai; user-authorised for classifier needs
metadata:
  node_type: memory
  type: reference
  originSessionId: ec8aa4f3-bd29-4a61-bf8d-d0a3ac062b7f
  modified: 2026-10-17T15:19:17.167Z
---

TypeSafe AI's Jev ("System One") is a hosted classifier: `POST https://api.typesafe.ai/v1/systemone`,
body `{state, model: "jev-latest", questions: {name: {type, instructions, ...}}}`, answers carry
per-option probabilities. Models `jev-1.13.0`, `jev-1.14.0`, `jev-latest`, `jev-preview`. Docs:
https://docs.typesafe.ai/api.

Key: 1Password item `api_key_typesafeai` (read with `op read` at runtime, never print). The user
offered it on 2026-10-17 "in case you need classifier model" — authorised, not mandated.

Sending code to it is a third-party upload, so any Dijkstral use must stay explicit opt-in (code
extraction never calls a model). See [[dijkstral-product]], [[prefer-codex-over-paid-keys]].

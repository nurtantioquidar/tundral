---
title: Job Service
owner: platform
---
# Job Service

The service exposes `run_job` and keeps work in a `JobQueue`.
Design background is in the [guide](docs/guide.md) and the paper in `papers/write_through.pdf`.

## Caching

Pushed jobs pass through `slugify()` before storage. We adopted write-through caching
because lazy invalidation caused stale reads.

### Decision

Keep the cache write-through; revisit if write latency exceeds 2 seconds.

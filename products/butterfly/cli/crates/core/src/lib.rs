//! butterfly-core: UI-free engine of butterfly, the Harness coding agent.
//!
//! Layering: `domain` (types, money) → `model` (descriptors, capabilities) →
//! `provider` (OpenAI-compatible streaming, per-dialect) →
//! `tools` (policy-confined execution) → `context` (cache-aware history and
//! compaction) → `session` (the deterministic agent loop and persistence).

pub mod context;
pub mod domain;
pub mod failure;
pub mod model;
pub mod provider;
pub mod session;
pub mod telemetry;
pub mod tools;
pub mod triage;
pub mod verify;

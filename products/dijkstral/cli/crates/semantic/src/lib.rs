//! dijkstral's semantic pass (`docs/graphify-semantic.md`): documents, papers, office files and
//! images become graph nodes through a model, merged into the code graph.
//!
//! Stages and ownership (one module each; `types` is the only shared surface):
//! - `ingest`: which files go to the model, office/PDF text, image bytes, token estimates,
//!   chunking (§2, §4.1–4.2).
//! - `run`: prompts, request framing, backends (Codex CLI default, OpenAI-compatible, Anthropic,
//!   Gemini, replay, skill-driven hand-off), validation and repair, retries, concurrency, the
//!   per-file cache, community naming (§4.3–4.4, §5, §6, §9, §10).
//! - `merge`: replies into the graph, hyperedges, outputs that change (§7, §8).
//! - `pass`: the three as a build stage (`dijkstral_core::pipeline::Semantic`) and `label`;
//!   `standin` stands in for `ingest` until it lands; `add`: `dijkstral add <url>` (§11).

pub mod add;
pub mod ingest;
pub mod merge;
pub mod pass;
pub mod run;
pub mod standin;
pub mod types;

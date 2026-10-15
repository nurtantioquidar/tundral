//! Running the semantic pass through a model (§4.3–4.4, §5, §6, §9, §10 of
//! `docs/graphify-semantic.md`).
//!
//! ```text
//! backend::select(settings, env)        Codex CLI (default), OpenAI-compatible, Anthropic, Gemini,
//!                                       replay, or the skill-driven hand-off
//! extract(chunks, docs, backend, opts)  cache lookup → requests (retries, bisection, concurrency)
//!                                       → validated replies + per-file cache + usage.json
//! name_communities(communities, ...)    community names for `label` / `cluster-only`
//! handoff::import(out_dir, replies)     the host assistant's replies into the cache
//! ```
//!
//! Prompts are dijkstral's own (`prompt`, versioned by `PROMPT_VERSION`). Replies are repaired and
//! validated by `repair`: the first JSON object of the text is used, required fields and enums are
//! checked (warnings only), an empty or invalid reply is retried twice (2 s, 8 s), and a truncated
//! multi-file request is split in halves down to single files.
//!
//! Deliberate differences from Graphify: the cache scope includes the backend and the model
//! (`cache`), a file the model keeps leaving out is cached as empty after its second omission, and
//! `./README.md` attributes items to `README.md` in the cache (Graphify's quirk S6 lets it
//! overwrite the canonical entry).

pub mod backend;
pub mod cache;
pub mod extract;
pub mod label;
pub mod prompt;
pub mod repair;
pub mod usage;

pub use backend::handoff::{import, ImportReport};
pub use backend::{select, BackendKind, BackendSettings, Env};
pub use cache::{clear_needs_update, mark_needs_update, pending_changes};
pub use extract::{
    cache_misses, default_out_dir, extract, ExtractOptions, ExtractOutcome, Extracted, FileState,
    Shrink,
};
pub use label::{name_communities, Community, LabelOptions, LabelOutcome};
pub use prompt::PROMPT_VERSION;
pub use usage::{UsageRecord, UsageReport};

/// `--mode`: normal, or `deep` (finer concepts, more inferred links; its own cache).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Normal,
    Deep,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::Deep => "deep",
        }
    }
}

#[cfg(test)]
mod tests;

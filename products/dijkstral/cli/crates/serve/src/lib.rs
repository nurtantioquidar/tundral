//! Dijkstral's query engine: Graphify's `query`, `path`, `explain`, `affected`, `god-nodes` and
//! `benchmark` (`docs/graphify-query.md`) over a [`dijkstral_core::index::Index`].
//!
//! Every command is split into a function that returns a structured answer and a renderer that
//! produces Graphify's text (without the trailing newline the CLI adds), so the CLI and the MCP
//! server share one implementation.

pub mod affected;
pub mod benchmark;
pub mod explain;
pub mod god_nodes;
pub mod load;
pub mod path;
pub mod query;
pub mod resolve;
pub mod text;

pub use dijkstral_core::index::Index;
pub use load::{load, GraphSource};

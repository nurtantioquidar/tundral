//! Dijkstral's query engine: Graphify's `query`, `path`, `explain`, `affected`, `god-nodes` and
//! `benchmark`, and the MCP server (`docs/graphify-query.md`), over a [`dijkstral_core::index::Index`];
//! plus the tools beside the graph (`docs/graphify-memory.md`): saved results and `reflect`
//! (`memory`, `reflect`), the PR dashboard and MCP PR tools (`prs`) and `diagnose multigraph`
//! (`diagnose`).
//!
//! Every command is split into a function that returns a structured answer and a renderer that
//! produces Graphify's text (without the trailing newline the CLI adds), so the CLI and the MCP
//! server share one implementation.

pub mod affected;
pub mod benchmark;
pub mod clock;
pub mod diagnose;
pub mod explain;
pub mod god_nodes;
pub mod insights;
pub mod load;
pub mod mcp;
pub mod memory;
pub mod path;
pub mod prs;
pub mod query;
pub mod reflect;
pub mod resolve;
pub mod text;

pub use dijkstral_core::index::Index;
pub use load::{load, GraphSource};

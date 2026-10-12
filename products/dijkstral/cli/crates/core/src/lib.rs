//! Dijkstral core: discovery, extraction, cache, resolution and graph output.

pub mod cache;
pub mod discover;
pub mod extract;
pub mod facts;
pub mod graph;
pub mod ids;
pub mod lang;
pub mod pipeline;
pub mod resolve;

pub use pipeline::{build, BuildOptions, BuildReport};

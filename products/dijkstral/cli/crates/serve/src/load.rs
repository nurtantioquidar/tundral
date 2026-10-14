//! Opening a graph for queries: dijkstral's memory-mapped `graph.dij`, or any Graphify-format
//! `graph.json` (Graphify's own included), indexed in memory on load.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dijkstral_core::index::{Index, INDEX_FILE};

/// Where an opened graph came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphSource {
    /// A `graph.dij`, memory-mapped.
    Index(PathBuf),
    /// A `graph.json`, parsed and indexed in memory.
    Json(PathBuf),
}

/// Open `path`. A `graph.dij` is memory-mapped. A `graph.json` is served from the `graph.dij`
/// dijkstral wrote next to it when that index is at least as new as the JSON (a build writes the
/// index last; anything that rewrites `graph.json` afterwards makes the JSON authoritative), and
/// is otherwise parsed and indexed in memory.
pub fn load(path: &Path) -> Result<(Index, GraphSource)> {
    let is_dij = path.extension().is_some_and(|e| e == "dij");
    if is_dij {
        return Ok((Index::open(path)?, GraphSource::Index(path.to_path_buf())));
    }
    if let Some(sibling) = fresh_sibling_index(path) {
        if let Ok(ix) = Index::open(&sibling) {
            return Ok((ix, GraphSource::Index(sibling)));
        }
    }
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let ix = Index::from_graph_json(&bytes)
        .with_context(|| format!("could not load graph.json at {}", path.display()))?;
    Ok((ix, GraphSource::Json(path.to_path_buf())))
}

fn fresh_sibling_index(json: &Path) -> Option<PathBuf> {
    if json.file_name()? != "graph.json" {
        return None;
    }
    let sibling = json.with_file_name(INDEX_FILE);
    let idx_time = fs::metadata(&sibling).ok()?.modified().ok()?;
    let json_time = fs::metadata(json).ok()?.modified().ok()?;
    (idx_time >= json_time).then_some(sibling)
}

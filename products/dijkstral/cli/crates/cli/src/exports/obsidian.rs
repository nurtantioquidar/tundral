//! `dijkstral export obsidian` (`docs/graphify-exports.md`, Obsidian section): an Obsidian vault
//! with one note per node and per community, plus `graph.canvas`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use dijkstral_export::nx::NxGraph;
use dijkstral_export::obsidian::{self, CANVAS_FILE, OBSIDIAN_DIR};

use crate::export_cmd::{graph_path, nx_names, sibling, Common};

#[derive(Args, Debug)]
pub struct ObsidianArgs {
    #[command(flatten)]
    pub common: Common,
    /// Vault directory. Default: obsidian/ next to the graph. Existing notes the exporter did not
    /// write are never overwritten.
    #[arg(long, value_name = "PATH")]
    pub dir: Option<PathBuf>,
}

pub fn run(a: ObsidianArgs) -> Result<String> {
    let path = graph_path(&a.common.graph);
    let graph = NxGraph::read(&path)?;
    let names = nx_names(&a.common, &graph, &sibling(&path, ""))?;
    let dir = a.dir.unwrap_or_else(|| sibling(&path, OBSIDIAN_DIR));
    let done = obsidian::write(&graph, &names, &dir)
        .with_context(|| format!("writing the vault in {}", dir.display()))?;
    if done.pruned > 0 {
        eprintln!(
            "pruned {} note(s) for nodes no longer in the graph",
            done.pruned
        );
    }
    if !done.skipped.is_empty() {
        eprintln!(
            "warning: skipped {} existing file(s) dijkstral did not create, to avoid overwriting \
             your notes: {}. Export into an empty directory to get the full vault.",
            done.skipped.len(),
            done.skipped.join(", ")
        );
    }
    let shown = dir.display();
    Ok(format!(
        "Obsidian vault: {} notes in {shown}/\nCanvas: {}\nOpen {shown}/ as a vault in Obsidian.",
        done.notes,
        dir.join(CANVAS_FILE).display()
    ))
}

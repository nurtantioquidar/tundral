//! `dijkstral tree` (`docs/graphify-exports.md`, tree section): `GRAPH_TREE.html`, a collapsible
//! D3 tree of the graph's directories, files and symbols, next to the graph by default.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use dijkstral_export::nx::NxGraph;
use dijkstral_export::tree::{
    self, TreeOptions, DEFAULT_MAX_CHILDREN, DEFAULT_TOP_K_EDGES, TREE_FILE,
};

use crate::export_cmd::{graph_path, sibling, GraphOnly};

#[derive(Args, Debug)]
pub struct TreeArgs {
    #[command(flatten)]
    pub graph: GraphOnly,
    /// Page to write. Default: GRAPH_TREE.html next to the graph; a directory gets
    /// GRAPH_TREE.html inside it.
    #[arg(long, value_name = "HTML")]
    pub output: Option<PathBuf>,
    /// Root of the hierarchy, matched against the graph's source_file paths (as stored, usually
    /// repo-relative). Default: their common directory.
    #[arg(long, value_name = "PATH")]
    pub root: Option<String>,
    /// Symbols listed per file before a "(+N more)" entry.
    #[arg(long, value_name = "N", default_value_t = DEFAULT_MAX_CHILDREN)]
    pub max_children: usize,
    /// Outbound links listed per symbol in the inspector.
    #[arg(long, value_name = "N", default_value_t = DEFAULT_TOP_K_EDGES)]
    pub top_k_edges: usize,
    /// Name of the root (page title). Default: the root directory's name, else "/".
    #[arg(long, value_name = "NAME")]
    pub label: Option<String>,
}

pub fn run(a: TreeArgs) -> Result<String> {
    let path = graph_path(&a.graph);
    let graph = NxGraph::read(&path)?;
    let out = match a.output {
        Some(o) if o.is_dir() => o.join(TREE_FILE),
        Some(o) => o,
        None => sibling(&path, TREE_FILE),
    };
    let opts = TreeOptions {
        root: a.root,
        max_children: a.max_children,
        top_k_edges: a.top_k_edges,
        label: a.label,
    };
    let stats = tree::write(&graph, &opts, &out)?;
    Ok(format!(
        "wrote {} ({} files, {} symbols)",
        out.display(),
        stats.files,
        stats.symbols
    ))
}

pub fn run_cmd(a: TreeArgs) -> ExitCode {
    match run(a) {
        Ok(msg) => {
            println!("{msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

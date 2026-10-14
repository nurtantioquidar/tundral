//! Exporters for dijkstral graphs, specified in `docs/graphify-exports.md`: the interactive
//! `graph.html`, the Markdown wiki and GraphML. They read dijkstral's build in memory or any
//! Graphify-format `graph.json`.

pub mod graphml;
pub mod html;
pub mod view;
pub mod wiki;

use std::path::PathBuf;

use anyhow::Result;
use dijkstral_core::pipeline::{Built, Exporter};

pub use html::{HtmlOptions, DEFAULT_NODE_LIMIT, HTML_FILE};
pub use view::{Loaded, Names, View, LABELS_FILE};
pub use wiki::{INDEX_FILE, WIKI_DIR};

/// `graph.html` written by `extract`/`update`.
pub struct HtmlExport {
    pub options: HtmlOptions,
}

impl Exporter for HtmlExport {
    fn name(&self) -> &'static str {
        "html"
    }

    fn outputs(&self) -> Vec<PathBuf> {
        vec![PathBuf::from(HTML_FILE)]
    }

    fn write(&self, built: &Built) -> Result<()> {
        html::write(
            &View::of_build(built),
            &self.options,
            &built.out_dir.join(HTML_FILE),
        )
    }
}

/// The wiki written by `extract --wiki`/`update --wiki`.
pub struct WikiExport;

impl Exporter for WikiExport {
    fn name(&self) -> &'static str {
        "wiki"
    }

    fn outputs(&self) -> Vec<PathBuf> {
        vec![PathBuf::from(WIKI_DIR).join(INDEX_FILE)]
    }

    fn write(&self, built: &Built) -> Result<()> {
        wiki::write(&View::of_build(built), &built.out_dir.join(WIKI_DIR)).map(drop)
    }
}

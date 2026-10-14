//! `dijkstral export svg` (`docs/graphify-exports.md`, SVG section): `graph.svg` next to the
//! graph.

use anyhow::{Context, Result};
use dijkstral_export::nx::NxGraph;
use dijkstral_export::svg::{self, Layout, SVG_FILE};

use crate::export_cmd::{graph_path, nx_names, sibling, Common};

pub fn run(c: Common) -> Result<String> {
    let path = graph_path(&c.graph);
    let graph = NxGraph::read(&path)?;
    let names = nx_names(&c, &graph, &sibling(&path, ""))?;
    let out = sibling(&path, SVG_FILE);
    let stats =
        svg::write(&graph, &names, &out).with_context(|| format!("writing {}", out.display()))?;
    let layout = match stats.layout {
        Layout::Force => "force layout",
        Layout::Packed => "community-packed layout",
    };
    let mut msg = format!(
        "wrote {} ({} nodes, {layout}, {} × {} pt)",
        out.display(),
        stats.nodes,
        stats.width,
        stats.height
    );
    if stats.edges_drawn < stats.edges {
        msg.push_str(&format!(
            "; drew {} of {} links",
            stats.edges_drawn, stats.edges
        ));
    }
    if stats.labels_drawn < stats.nodes {
        msg.push_str(&format!(
            "; labelled the {} best-connected nodes",
            stats.labels_drawn
        ));
    }
    Ok(msg)
}

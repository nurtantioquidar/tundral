//! `dijkstral export html|wiki|graphml|cypher|neo4j|falkordb|obsidian|svg|callflow-html` on any
//! Graphify-format `graph.json`, and the exporters `extract`/`update` run
//! (`docs/graphify-exports.md`). The stage-4b formats live in `crate::exports`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use dijkstral_core::pipeline::{Exporter, OUT_DIR};
use dijkstral_export::html::{self, HtmlOptions};
use dijkstral_export::nx::NxGraph;
use dijkstral_export::{
    graphml, wiki, HtmlExport, Loaded, Names, WikiExport, DEFAULT_NODE_LIMIT, HTML_FILE,
    LABELS_FILE, WIKI_DIR,
};

const GRAPHIFY_DEFAULT: &str = "graphify-out/graph.json";

/// Output flags of `extract` and `update`.
#[derive(Args, Debug)]
pub struct BuildExports {
    /// Do not write graph.html.
    #[arg(long)]
    no_viz: bool,
    /// Also write the Markdown wiki to dijkstral-out/wiki/.
    #[arg(long, conflicts_with = "no_cluster")]
    wiki: bool,
    /// Above this many nodes graph.html uses its WebGL view.
    #[arg(long, value_name = "N", default_value_t = DEFAULT_NODE_LIMIT)]
    node_limit: usize,
}

/// The exporters a build runs.
pub fn exporters(a: &BuildExports, root: &Path) -> Vec<Box<dyn Exporter>> {
    let mut out: Vec<Box<dyn Exporter>> = Vec::new();
    if !a.no_viz {
        let title = root
            .canonicalize()
            .ok()
            .and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        out.push(Box::new(HtmlExport {
            options: HtmlOptions {
                node_limit: a.node_limit,
                title,
            },
        }));
    }
    if a.wiki {
        out.push(Box::new(WikiExport));
    }
    out
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    #[command(subcommand)]
    format: Format,
}

#[derive(Subcommand, Debug)]
enum Format {
    /// Interactive graph.html next to the graph (vis-network up to --node-limit nodes, WebGL
    /// above).
    Html(HtmlArgs),
    /// Markdown wiki (index, one article per community and per god node) in wiki/ next to the
    /// graph.
    Wiki(Common),
    /// graph.graphml next to the graph (Gephi, yEd, any GraphML tool).
    Graphml(GraphOnly),
    /// cypher.txt next to the graph (OpenCypher, for Neo4j or FalkorDB).
    Cypher(GraphOnly),
    /// cypher.txt for Neo4j, or --push the graph to a Neo4j server over Bolt.
    Neo4j(crate::exports::cypher::PushArgs),
    /// cypher.txt for FalkorDB, or --push the graph to a FalkorDB server.
    Falkordb(crate::exports::cypher::PushArgs),
    /// Obsidian vault: one note per node and per community, plus graph.canvas.
    Obsidian(crate::exports::obsidian::ObsidianArgs),
    /// graph.svg next to the graph: a static drawing coloured by community.
    Svg(Common),
    /// Mermaid architecture and call-flow HTML page.
    CallflowHtml(crate::exports::callflow::CallflowArgs),
}

#[derive(Args, Debug)]
pub(crate) struct GraphOnly {
    /// Graph to export (any Graphify-format graph.json). Default: dijkstral-out/graph.json, else
    /// graphify-out/graph.json.
    #[arg(long, value_name = "PATH")]
    pub(crate) graph: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub(crate) struct Common {
    #[command(flatten)]
    pub(crate) graph: GraphOnly,
    /// Community names (Graphify's .graphify_labels.json format). Default: that file next to the
    /// graph, else each community's best-connected member, as in GRAPH_REPORT.md.
    #[arg(long, value_name = "PATH")]
    pub(crate) labels: Option<PathBuf>,
    /// Name communities "Community N", as Graphify does without a labels file.
    #[arg(long)]
    pub(crate) no_label: bool,
}

#[derive(Args, Debug)]
struct HtmlArgs {
    #[command(flatten)]
    common: Common,
    /// Above this many nodes the page switches to its WebGL view.
    #[arg(long, value_name = "N", default_value_t = DEFAULT_NODE_LIMIT)]
    node_limit: usize,
    /// Write nothing (accepted for Graphify compatibility).
    #[arg(long)]
    no_viz: bool,
}

pub fn run(a: ExportArgs) -> ExitCode {
    match export(a) {
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

pub(crate) fn graph_path(g: &GraphOnly) -> PathBuf {
    match &g.graph {
        Some(p) => p.clone(),
        None => {
            let ours = Path::new(OUT_DIR).join("graph.json");
            if !ours.is_file() && Path::new(GRAPHIFY_DEFAULT).is_file() {
                PathBuf::from(GRAPHIFY_DEFAULT)
            } else {
                ours
            }
        }
    }
}

fn names(c: &Common, loaded: &Loaded, dir: &Path) -> Result<Names> {
    if let Some(p) = &c.labels {
        return Names::read(p);
    }
    if c.no_label {
        return Ok(Names::Placeholder);
    }
    let sidecar = dir.join(LABELS_FILE);
    if sidecar.is_file() {
        return Names::read(&sidecar);
    }
    Ok(loaded.view(Names::Placeholder).hub_names())
}

/// Community names for the stage-4b exporters (same precedence as `names`).
pub(crate) fn nx_names(c: &Common, g: &NxGraph, dir: &Path) -> Result<Names> {
    if let Some(p) = &c.labels {
        return Names::read(p);
    }
    if c.no_label {
        return Ok(Names::Placeholder);
    }
    let sidecar = dir.join(LABELS_FILE);
    if sidecar.is_file() {
        return Names::read(&sidecar);
    }
    Ok(g.hub_names())
}

fn export(a: ExportArgs) -> Result<String> {
    match a.format {
        Format::Cypher(g) => crate::exports::cypher::run_file(&g),
        Format::Neo4j(p) => crate::exports::cypher::run(p, crate::exports::cypher::Target::Neo4j),
        Format::Falkordb(p) => {
            crate::exports::cypher::run(p, crate::exports::cypher::Target::FalkorDb)
        }
        Format::Obsidian(o) => crate::exports::obsidian::run(o),
        Format::Svg(c) => crate::exports::svg::run(c),
        Format::CallflowHtml(c) => crate::exports::callflow::run(c),
        Format::Graphml(g) => {
            let path = graph_path(&g);
            let (_, doc) = Loaded::read(&path)?;
            let out = sibling(&path, graphml::GRAPHML_FILE);
            graphml::write(&doc, &out)?;
            Ok(format!("wrote {}", out.display()))
        }
        Format::Wiki(c) => {
            let path = graph_path(&c.graph);
            let (loaded, doc) = Loaded::read(&path)?;
            drop(doc);
            let names = names(&c, &loaded, &sibling(&path, ""))?;
            let dir = sibling(&path, WIKI_DIR);
            let n = wiki::write(&loaded.view(names), &dir)?;
            Ok(format!(
                "wrote {n} articles to {} (start at {})",
                dir.display(),
                dir.join(dijkstral_export::INDEX_FILE).display()
            ))
        }
        Format::Html(h) => {
            if h.no_viz {
                return Ok("--no-viz: graph.html not written".into());
            }
            let path = graph_path(&h.common.graph);
            let (loaded, doc) = Loaded::read(&path)?;
            drop(doc);
            let names = names(&h.common, &loaded, &sibling(&path, ""))?;
            let view = loaded.view(names);
            let opts = HtmlOptions {
                node_limit: h.node_limit,
                title: project_name(&path),
            };
            let out = sibling(&path, HTML_FILE);
            html::write(&view, &opts, &out)
                .with_context(|| format!("writing {}", out.display()))?;
            let how = match html::mode(&view, &opts) {
                html::Mode::Vis => "vis-network",
                html::Mode::WebGl => "WebGL",
            };
            Ok(format!(
                "wrote {} ({} nodes, {how} view)",
                out.display(),
                view.nodes.len()
            ))
        }
    }
}

/// `name` in the graph's directory.
pub(crate) fn sibling(graph: &Path, name: &str) -> PathBuf {
    let dir = graph.parent().unwrap_or(Path::new(""));
    if name.is_empty() {
        dir.to_path_buf()
    } else {
        dir.join(name)
    }
}

/// The project a graph describes: the directory holding its `dijkstral-out/` or
/// `graphify-out/`, else the graph's own directory.
pub(crate) fn project_name(graph: &Path) -> String {
    let dir = graph
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let project = match dir.file_name().and_then(|n| n.to_str()) {
        Some(OUT_DIR) | Some("graphify-out") => dir.parent().map(Path::to_path_buf),
        _ => Some(dir),
    };
    project
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

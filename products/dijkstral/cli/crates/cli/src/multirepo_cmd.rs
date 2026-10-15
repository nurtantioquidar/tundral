//! Cross-repo graphs: `merge-graphs`, `global add|remove|list|path` and `extract --global`
//! (`docs/graphify-multirepo.md` §3–§6), over `dijkstral_core::multirepo`.
//!
//! Differences from Graphify, by design (§7):
//! - The global graph lives in `$DIJKSTRAL_HOME` (default `~/.dijkstral`), not `~/.graphify`.
//! - Messages are dijkstral's own; argument errors from clap exit 2 where Graphify exits 1.
//! - Colliding default tags take the first free `-N` suffix (Graphify can hand out the same tag
//!   twice, merging two repositories' nodes).
//! - `update --global` is accepted as well as `extract --global`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use dijkstral_core::multirepo::{
    dijkstral_home, global_default_tag, merge, merge_tags, read_graph, AddOutcome, GlobalStore,
    MergeInput,
};
use dijkstral_core::nodelink::InputGraph;
use dijkstral_core::pipeline::write_atomic;

const DEFAULT_MERGED: &str = "dijkstral-out/merged-graph.json";

#[derive(Args, Debug)]
pub struct MergeArgs {
    /// Two or more graph.json files (dijkstral's or Graphify's). Each repository is tagged with
    /// the directory holding its output directory (<repo>/dijkstral-out/graph.json → repo).
    #[arg(value_name = "GRAPH", num_args = 2.., required = true)]
    graphs: Vec<PathBuf>,
    /// Where to write the merged graph.
    #[arg(long, value_name = "PATH", default_value = DEFAULT_MERGED)]
    out: PathBuf,
    /// A previous merged graph whose community ids to restore by node id (nodes it does not
    /// cover get none), so re-clustering keeps community names stable.
    #[arg(long, value_name = "PATH")]
    previous: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct GlobalArgs {
    #[command(subcommand)]
    command: GlobalCommand,
}

#[derive(Subcommand, Debug)]
enum GlobalCommand {
    /// Add a project's graph.json to the global graph, replacing what its tag held before.
    Add(GlobalAddArgs),
    /// Remove a project's nodes from the global graph.
    Remove {
        /// The project's tag.
        tag: String,
    },
    /// List the projects in the global graph.
    List,
    /// Print the global graph's path ($DIJKSTRAL_HOME/global-graph.json).
    Path,
}

#[derive(Args, Debug)]
struct GlobalAddArgs {
    /// The project's graph.json.
    graph: PathBuf,
    /// Tag for the project (default: the directory holding the graph's output directory).
    #[arg(long = "as", value_name = "TAG")]
    tag: Option<String>,
}

/// `extract --global` / `update --global`.
#[derive(Args, Debug, Default)]
pub struct GlobalFlags {
    /// Also add the built graph to the global graph ($DIJKSTRAL_HOME/global-graph.json).
    #[arg(long)]
    global: bool,
    /// Tag for --global (default: the indexed directory's name).
    #[arg(long = "as", value_name = "TAG", requires = "global")]
    tag: Option<String>,
}

fn fail(e: anyhow::Error) -> ExitCode {
    eprintln!("error: {e:#}");
    ExitCode::FAILURE
}

fn store() -> Result<GlobalStore> {
    Ok(GlobalStore::new(dijkstral_home()?))
}

pub fn run_merge(a: MergeArgs) -> ExitCode {
    match merge_files(&a) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

fn merge_files(a: &MergeArgs) -> Result<()> {
    let previous = match &a.previous {
        Some(p) => Some(
            read_graph(p)
                .with_context(|| format!("--previous {}", p.display()))?
                .0,
        ),
        None => None,
    };
    let (tags, collided) = merge_tags(&a.graphs);
    let mut inputs = Vec::with_capacity(a.graphs.len());
    for (path, tag) in a.graphs.iter().zip(tags.iter()) {
        let (doc, _) = read_graph(path)?;
        let graph = InputGraph::load(&doc).with_context(|| format!("{}", path.display()))?;
        inputs.push(MergeInput {
            tag: tag.clone(),
            graph,
        });
    }
    if collided {
        println!(
            "several inputs share a directory name; tagged them {}",
            tags.join(", ")
        );
    }
    let merged = merge(&inputs, previous.as_ref());
    if let Some(parent) = a.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    write_atomic(&a.out, merged.json.as_bytes())
        .with_context(|| format!("writing {}", a.out.display()))?;
    if let (Some(n), Some(p)) = (merged.restored, &a.previous) {
        println!("{n} node(s) keep their community from {}", p.display());
    }
    println!(
        "{} graphs merged: {} nodes, {} edges\nwrote {}",
        inputs.len(),
        merged.nodes,
        merged.edges,
        a.out.display()
    );
    Ok(())
}

pub fn run_global(a: GlobalArgs) -> ExitCode {
    let res = match a.command {
        GlobalCommand::Add(a) => {
            let tag = a.tag.unwrap_or_else(|| global_default_tag(&a.graph));
            add(&a.graph, &tag)
        }
        GlobalCommand::Remove { tag } => remove(&tag),
        GlobalCommand::List => list(),
        GlobalCommand::Path => store().map(|s| println!("{}", s.graph_path().display())),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

fn add(graph: &Path, tag: &str) -> Result<()> {
    let store = store()?;
    if !graph.is_file() {
        anyhow::bail!("no graph at {}", graph.display());
    }
    let (doc, bytes) = read_graph(graph)?;
    let source = graph
        .canonicalize()
        .with_context(|| format!("resolving {}", graph.display()))?;
    let added = store
        .add(&doc, &bytes, &source, tag)
        .with_context(|| format!("adding {}", graph.display()))?;
    if let Some(old) = &added.moved_from {
        eprintln!(
            "warning: tag '{tag}' was added from {old}; it now follows {} (use --as to keep both)",
            source.display()
        );
    }
    match added.outcome {
        AddOutcome::Unchanged => {
            println!("'{tag}' has not changed since it was added; global graph left as is")
        }
        AddOutcome::Added { added, pruned } => println!(
            "'{tag}' is in the global graph: {added} nodes added, {pruned} old nodes removed\n\
             wrote {}",
            store.graph_path().display()
        ),
    }
    Ok(())
}

fn remove(tag: &str) -> Result<()> {
    let store = store()?;
    let pruned = store.remove(tag)?;
    println!("'{tag}' removed from the global graph ({pruned} nodes)");
    Ok(())
}

fn list() -> Result<()> {
    let store = store()?;
    let repos = store.repos()?;
    if repos.is_empty() {
        println!("the global graph has no projects yet; add one with `dijkstral global add <graph.json>`");
        return Ok(());
    }
    println!("{}", store.graph_path().display());
    for r in repos {
        let day = r.added_at.get(..10).unwrap_or(&r.added_at);
        println!(
            "  {}  {} nodes, {} edges  added {day}  from {}",
            r.tag, r.node_count, r.edge_count, r.source_path
        );
    }
    Ok(())
}

/// After a successful build with `--global`: add `<out_dir>/graph.json` under `--as`, or the
/// indexed directory's name. Returns false (after reporting why) when that fails.
pub fn after_build(flags: &GlobalFlags, root: &Path, out_dir: &Path) -> bool {
    if !flags.global {
        return true;
    }
    let tag = flags.tag.clone().unwrap_or_else(|| {
        root.canonicalize()
            .ok()
            .and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "repo".into())
    });
    match add(&out_dir.join("graph.json"), &tag) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("error: global graph not updated: {e:#}");
            false
        }
    }
}

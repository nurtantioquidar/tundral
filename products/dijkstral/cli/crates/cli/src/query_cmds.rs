//! The query surface: `query`, `path`, `explain`, `affected`, `god-nodes` and `benchmark`, with
//! Graphify's outputs, streams and exit codes (`docs/graphify-query.md` §2–§9).
//!
//! Differences from Graphify, by design:
//! - Nothing is written next to the graph. Graphify's `query`, `path` and `explain` leave
//!   `cache/last_query_stamp` beside it; nothing reads that file (§2.3), so dijkstral skips it.
//! - No size cap on `graph.json` (`GRAPHIFY_MAX_GRAPH_BYTES`): dijkstral indexes it without
//!   building Python objects.
//! - BFS `EDGE` lines are in a deterministic order: reached nodes in visit order, each node's
//!   links in graph.json order (Graphify's depends on Python's hash seed, §3.9).
//! - Argument errors from clap (unknown flags) exit 2 where Graphify ignores them.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use dijkstral_serve::{affected, benchmark, explain, god_nodes, path, query, resolve, Index};

const DEFAULT_INDEX: &str = "dijkstral-out/graph.dij";
const GRAPHIFY_DEFAULT: &str = "graphify-out/graph.json";

#[derive(Subcommand, Debug)]
pub enum QueryCommand {
    /// Answer a question: score nodes against its terms, traverse from the best matches, print
    /// nodes and edges within a token budget (exit 0 answer, 4 no match).
    Query(QueryArgs),
    /// Shortest path between two nodes (exit 3 ambiguous name, 4 unknown name).
    Path(PathArgs),
    /// A node and its connections (exit 3 ambiguous name, 4 unknown name).
    Explain(ExplainArgs),
    /// Nodes that depend on a node, by reverse traversal of dependency relations.
    Affected(AffectedArgs),
    /// The most connected symbols.
    GodNodes(GodNodesArgs),
    /// Estimate the token reduction of graph queries against reading the corpus.
    Benchmark(BenchmarkArgs),
}

/// Graph selection shared by every query command.
#[derive(Args, Debug)]
pub struct GraphArg {
    /// Graph to read: a graph.dij, or any graph.json (Graphify's included). Default:
    /// dijkstral-out/graph.dij, else graphify-out/graph.json.
    #[arg(long, value_name = "PATH")]
    graph: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct QueryArgs {
    /// The question.
    question: Option<String>,
    /// Depth-first traversal instead of breadth-first.
    #[arg(long)]
    dfs: bool,
    /// Follow only links with this context (repeatable): call, import, field, …
    #[arg(long, value_name = "C")]
    context: Vec<String>,
    /// Token budget for the answer.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    budget: Option<String>,
    #[command(flatten)]
    graph: GraphArg,
}

#[derive(Args, Debug)]
pub struct PathArgs {
    source: Option<String>,
    target: Option<String>,
    /// Ignore link direction.
    #[arg(long)]
    undirected: bool,
    /// Follow link direction (the default).
    #[arg(long, conflicts_with = "undirected")]
    directed: bool,
    #[command(flatten)]
    graph: GraphArg,
}

#[derive(Args, Debug)]
pub struct ExplainArgs {
    node: Option<String>,
    /// List every connection (default: the first 20 and a summary by file).
    #[arg(long)]
    all: bool,
    #[command(flatten)]
    graph: GraphArg,
}

#[derive(Args, Debug)]
pub struct AffectedArgs {
    node: Option<String>,
    /// Relation to follow (repeatable; replaces the default dependency relations).
    #[arg(long, value_name = "R")]
    relation: Vec<String>,
    /// Layers to traverse.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    depth: Option<String>,
    #[command(flatten)]
    graph: GraphArg,
}

#[derive(Args, Debug)]
pub struct GodNodesArgs {
    /// How many nodes to list (default 10).
    #[arg(long, value_name = "N", num_args = 0..=1, allow_hyphen_values = true)]
    top: Option<Option<String>>,
    /// JSON output.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    graph: GraphArg,
}

#[derive(Args, Debug)]
pub struct BenchmarkArgs {
    /// Graph to measure (default as for the other commands).
    graph: Option<PathBuf>,
}

/// Print `text` and a newline to stdout or stderr and exit with `code`.
fn emit(text: &str, stderr: bool, code: u8) -> ExitCode {
    if stderr {
        eprintln!("{text}");
    } else {
        println!("{text}");
    }
    ExitCode::from(code)
}

fn fail(msg: &str) -> ExitCode {
    emit(msg, true, 1)
}

fn absolute(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        env::current_dir().unwrap_or_default().join(p)
    };
    abs.canonicalize().unwrap_or(abs)
}

/// The path Graphify prints: relative to the cwd when inside it, else the absolute real path.
fn display_path(p: &Path) -> String {
    let real = absolute(p);
    let cwd = env::current_dir()
        .and_then(|c| c.canonicalize())
        .unwrap_or_default();
    match real.strip_prefix(&cwd) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
        _ => real.display().to_string(),
    }
}

/// Open the selected graph, or report the error Graphify would (exit 1).
fn open(graph: Option<&Path>) -> Result<(Index, String), ExitCode> {
    let path = match graph {
        Some(p) => p.to_path_buf(),
        None if Path::new(DEFAULT_INDEX).is_file() => PathBuf::from(DEFAULT_INDEX),
        None if Path::new(GRAPHIFY_DEFAULT).is_file() => PathBuf::from(GRAPHIFY_DEFAULT),
        None => PathBuf::from(DEFAULT_INDEX),
    };
    if !path.is_file() {
        return Err(fail(&format!(
            "error: graph file not found: {}",
            absolute(&path).display()
        )));
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext != "json" && ext != "dij" {
        return Err(fail("error: graph file must be a .json or .dij file"));
    }
    match dijkstral_serve::load(&path) {
        Ok((ix, _)) => Ok((ix, display_path(&path))),
        Err(e) => Err(fail(&format!("error: {e:#}"))),
    }
}

fn parse_int(v: &str, flag: &str) -> Result<i64, ExitCode> {
    v.trim()
        .parse::<i64>()
        .map_err(|_| fail(&format!("error: {flag} must be an integer")))
}

pub fn run(cmd: QueryCommand) -> ExitCode {
    let r = match cmd {
        QueryCommand::Query(a) => run_query(a),
        QueryCommand::Path(a) => run_path(a),
        QueryCommand::Explain(a) => run_explain(a),
        QueryCommand::Affected(a) => run_affected(a),
        QueryCommand::GodNodes(a) => run_god_nodes(a),
        QueryCommand::Benchmark(a) => run_benchmark(a),
    };
    r.unwrap_or_else(|code| code)
}

fn run_query(a: QueryArgs) -> Result<ExitCode, ExitCode> {
    let Some(question) = a.question else {
        return Err(fail(
            "Usage: dijkstral query \"<question>\" [--dfs] [--context C] [--budget N] [--graph path]",
        ));
    };
    let budget = match &a.budget {
        Some(b) => parse_int(b, "--budget")?,
        None => 2000,
    };
    let (ix, shown) = open(a.graph.graph.as_deref())?;
    let opts = query::QueryOptions {
        dfs: a.dfs,
        depth: 2,
        context: (!a.context.is_empty()).then_some(a.context),
        ..Default::default()
    };
    Ok(match query::query(&ix, &question, &opts) {
        None => emit(query::NO_MATCH, false, 4),
        Some(ans) => emit(&query::render(&ix, &shown, &ans, budget), false, 0),
    })
}

/// Resolve a name, printing Graphify's ambiguity (exit 3) or not-found (exit 4) text.
fn resolve_or_exit(ix: &Index, name: &str, stderr: bool) -> Result<u32, ExitCode> {
    match resolve::resolve(ix, name) {
        resolve::Resolved::Found(i) => Ok(i),
        resolve::Resolved::Ambiguous(c) => Err(emit(&resolve::ambiguous(ix, name, &c), stderr, 3)),
        resolve::Resolved::NotFound => Err(emit(&resolve::not_found(name), stderr, 4)),
    }
}

fn run_path(a: PathArgs) -> Result<ExitCode, ExitCode> {
    let (Some(src), Some(dst)) = (a.source, a.target) else {
        return Err(fail(
            "Usage: dijkstral path \"<source>\" \"<target>\" [--graph path] [--directed|--undirected]",
        ));
    };
    let (ix, _) = open(a.graph.graph.as_deref())?;
    let from = resolve_or_exit(&ix, &src, true)?;
    let to = resolve_or_exit(&ix, &dst, true)?;
    if from == to {
        return Err(emit(&path::same_node(&src, &dst, ix.id(from)), true, 3));
    }
    Ok(match path::shortest_path(&ix, from, to, a.undirected) {
        Some(p) => emit(&path::render(&ix, &p), false, 0),
        None => emit(&path::no_path(&src, &dst, a.undirected), false, 0),
    })
}

fn run_explain(a: ExplainArgs) -> Result<ExitCode, ExitCode> {
    let Some(name) = a.node else {
        return Err(fail(
            "Usage: dijkstral explain \"<node>\" [--all] [--graph path]",
        ));
    };
    let (ix, _) = open(a.graph.graph.as_deref())?;
    let node = resolve_or_exit(&ix, &name, false)?;
    Ok(emit(&explain::render(&ix, node, a.all), false, 0))
}

fn run_affected(a: AffectedArgs) -> Result<ExitCode, ExitCode> {
    let Some(name) = a.node else {
        return Err(fail(
            "Usage: dijkstral affected \"<node>\" [--relation R] [--depth N] [--graph path]",
        ));
    };
    let depth = match &a.depth {
        Some(d) => parse_int(d, "--depth")?,
        None => affected::DEFAULT_DEPTH,
    };
    let (ix, _) = open(a.graph.graph.as_deref())?;
    let Some(node) = affected::resolve(&ix, &name) else {
        return Ok(emit(&affected::no_unique(&name), false, 0));
    };
    let relations: Vec<String> = if a.relation.is_empty() {
        affected::DEFAULT_RELATIONS
            .iter()
            .map(|r| r.to_string())
            .collect()
    } else {
        a.relation
    };
    let found = affected::affected(&ix, node, &relations, depth);
    Ok(emit(
        &affected::render(&ix, node, &relations, depth, &found),
        false,
        0,
    ))
}

fn run_god_nodes(a: GodNodesArgs) -> Result<ExitCode, ExitCode> {
    let top = match a.top.flatten() {
        Some(t) => parse_int(&t, "--top")?,
        None => god_nodes::DEFAULT_TOP,
    };
    let (ix, _) = open(a.graph.graph.as_deref())?;
    let nodes = god_nodes::god_nodes(&ix, top);
    let text = if a.json {
        god_nodes::render_json(&ix, &nodes)
    } else {
        god_nodes::render_text(&ix, &nodes)
    };
    Ok(emit(&text, false, 0))
}

fn run_benchmark(a: BenchmarkArgs) -> Result<ExitCode, ExitCode> {
    let (ix, _) = open(a.graph.as_deref())?;
    let b = benchmark::benchmark(&ix);
    Ok(emit(&benchmark::render(&b), false, 0))
}

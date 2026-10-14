//! The MCP server behind `dijkstral mcp`: Graphify's `graphify-mcp` tools and resources over
//! stdio (`docs/graphify-query.md` §10), answered by the query engine in this crate.
//!
//! Transport: newline-delimited JSON-RPC 2.0, one compact object per line, read synchronously
//! from stdin. Nothing but protocol messages is written to stdout.
//!
//! The startup graph is loaded on a background thread while the handshake is answered; the first
//! call that needs it waits for it. Every call re-checks the graph file (size, mtime, inode, and
//! the `graph.dij` beside a `graph.json`) and reloads it when it changed, so no stale copy is ever
//! served. One index is cached per resolved graph path.
//!
//! Differences from Graphify, by design (§10.9):
//! - Failures set `isError: true` (Graphify reports every graph-tool failure with `false`). The
//!   failure texts are Graphify's.
//! - Echoed names keep their case (Graphify lowercases them in `get_node`/`get_neighbors`).
//! - A string `context_filter` is one context (Graphify iterates its characters); an unknown
//!   `mode` or an out-of-range `exclude_hubs_percentile` is an argument error.
//! - A line that is not JSON gets a JSON-RPC parse error (Graphify stays silent).
//! - The PR tools (`list_prs`, `get_pr_impact`, `triage_prs`) are not listed: they arrive with
//!   stage 6. Calling one returns an error saying so.
//! - `serverInfo` names dijkstral; resource URIs and tool names stay Graphify's.
//! - A project (the server's cwd, or `project_path`) is served from `dijkstral-out/graph.dij`,
//!   else `dijkstral-out/graph.json`, else `graphify-out/graph.json`.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime};

use dijkstral_core::index::{Index, INDEX_FILE};
use serde_json::{json, Map, Value};

use crate::text::{line_label, py_repr};
use crate::{god_nodes, insights, path, query, resolve};

pub const SERVER_NAME: &str = "dijkstral";
const PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
const LATEST_PROTOCOL: &str = "2025-11-25";
/// Project-relative graph locations, in preference order.
const PROJECT_GRAPHS: &[&str] = &[
    "dijkstral-out/graph.dij",
    "dijkstral-out/graph.json",
    "graphify-out/graph.json",
];
const PR_TOOLS: &[&str] = &["list_prs", "get_pr_impact", "triage_prs"];
/// Indexes kept for `project_path` graphs besides the startup graph.
const CACHE_LIMIT: usize = 8;

const NEIGHBORS_HINT: &str = "Narrow with relation_filter or use get_node for a specific symbol";
const COMMUNITY_HINT: &str = "Raise token_budget or use get_node for specific members";

#[derive(Debug, Clone)]
pub struct Options {
    /// The startup graph file; `None` serves the project in `cwd`.
    pub graph: Option<PathBuf>,
    /// The directory relative paths (and `project_path`) resolve against.
    pub cwd: PathBuf,
    /// Log requests and load times to stderr.
    pub verbose: bool,
}

/// Where a call's graph comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Target {
    File(PathBuf),
    Project(PathBuf),
}

/// What identifies a version of a graph on disk.
type Fingerprint = Vec<Option<(u64, Option<SystemTime>, u64)>>;

struct Entry {
    ix: Index,
    fp: Fingerprint,
    /// The graph path as `query_graph`'s header shows it.
    shown: String,
    /// The directory holding the graph (for `GRAPH_REPORT.md`).
    dir: PathBuf,
    /// `built_at_commit` of the graph.json.
    commit: Option<String>,
    used: u64,
}

type Loaded = (PathBuf, Result<Entry, String>);

pub struct Server {
    opts: Options,
    default: Target,
    cache: HashMap<PathBuf, Entry>,
    pending: Option<JoinHandle<Option<Loaded>>>,
    initialized: bool,
    clock: u64,
}

fn real(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn stat(p: &Path) -> Option<(u64, Option<SystemTime>, u64)> {
    let m = fs::metadata(p).ok()?;
    #[cfg(unix)]
    let ino = std::os::unix::fs::MetadataExt::ino(&m);
    #[cfg(not(unix))]
    let ino = 0;
    Some((m.len(), m.modified().ok(), ino))
}

fn fingerprint(file: &Path) -> Fingerprint {
    let mut fp = vec![stat(file)];
    if file.file_name().is_some_and(|n| n == "graph.json") {
        fp.push(stat(&file.with_file_name(INDEX_FILE)));
    }
    fp
}

/// The graph file a target names, or the error text for a missing one.
fn locate(t: &Target) -> Result<PathBuf, String> {
    match t {
        Target::File(p) => {
            if p.is_file() {
                Ok(real(p))
            } else {
                let name = p.file_name().map_or_else(
                    || "graph".to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                Err(format!("{name} not found: {}", real(p).display()))
            }
        }
        Target::Project(dir) => PROJECT_GRAPHS
            .iter()
            .map(|g| dir.join(g))
            .find(|p| p.is_file())
            .map(|p| real(&p))
            .ok_or_else(|| {
                format!(
                    "graph.json not found: {} (run `dijkstral extract` there, or point project_path at a directory holding dijkstral-out/ or graphify-out/graph.json)",
                    real(dir).join("graphify-out/graph.json").display()
                )
            }),
    }
}

/// Graphify's header path: relative to `cwd` when the graph is under it, else absolute.
fn shown(file: &Path, cwd: &Path) -> String {
    let cwd = real(cwd);
    match file.strip_prefix(&cwd) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
        _ => file.display().to_string(),
    }
}

fn load_entry(file: &Path, cwd: &Path) -> Result<Entry, String> {
    let fp = fingerprint(file);
    let name = file
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let (ix, _) = crate::load(file)
        .map_err(|e| format!("could not load {name} at {}: {e:#}", file.display()))?;
    Ok(Entry {
        ix,
        fp,
        shown: shown(file, cwd),
        dir: file.parent().map_or_else(PathBuf::new, Path::to_path_buf),
        commit: built_at_commit(file),
        used: 0,
    })
}

/// The top-level `built_at_commit` of a graph.json (for a graph.dij, of the graph.json beside
/// it). Graphify and dijkstral write it as the last key, so only the file's tail is read.
fn built_at_commit(file: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let json = if file.extension().is_some_and(|e| e == "dij") {
        file.with_file_name("graph.json")
    } else {
        file.to_path_buf()
    };
    let mut f = fs::File::open(json).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(1024))).ok()?;
    let mut tail = Vec::new();
    f.read_to_end(&mut tail).ok()?;
    let tail = String::from_utf8_lossy(&tail);
    const KEY: &str = "\"built_at_commit\"";
    let rest = &tail[tail.rfind(KEY)? + KEY.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let commit = &rest[..rest.find('"')?];
    (!commit.is_empty()).then(|| commit.to_string())
}

/// A tool's text and whether it reports a failure.
struct Reply {
    text: String,
    is_error: bool,
}

fn ok(text: impl Into<String>) -> Reply {
    Reply {
        text: text.into(),
        is_error: false,
    }
}

fn fail(text: impl Into<String>) -> Reply {
    Reply {
        text: text.into(),
        is_error: true,
    }
}

/// An argument problem, reported as `Error executing <tool>: <text>`.
struct ArgError(String);

type Args = Map<String, Value>;

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// Python's `int(v)`.
fn py_int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| {
                n.as_f64()
                    .filter(|f| f.is_finite())
                    .map(|f| f.trunc() as i64)
            })
            .ok_or_else(|| format!("integer out of range: {n}")),
        Value::Bool(b) => Ok(*b as i64),
        Value::String(s) => {
            let t = s.trim();
            let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
            let valid = !digits.is_empty()
                && !digits.starts_with('_')
                && !digits.ends_with('_')
                && !digits.contains("__")
                && digits.chars().all(|c| c.is_ascii_digit() || c == '_');
            valid
                .then(|| t.replace('_', "").parse::<i64>().ok())
                .flatten()
                .ok_or_else(|| format!("invalid literal for int() with base 10: {}", py_repr(s)))
        }
        other => Err(format!(
            "int() argument must be a string, a bytes-like object or a real number, not '{}'",
            type_name(other)
        )),
    }
}

fn int_arg(args: &Args, key: &str, default: i64) -> Result<i64, ArgError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => py_int(v).map_err(ArgError),
    }
}

fn required<'a>(args: &'a Args, key: &str) -> Result<&'a Value, ArgError> {
    args.get(key).ok_or_else(|| ArgError(format!("'{key}'")))
}

fn required_str<'a>(args: &'a Args, key: &str) -> Result<&'a str, ArgError> {
    match required(args, key)? {
        Value::String(s) => Ok(s),
        other => Err(ArgError(format!(
            "{key} must be a string, not '{}'",
            type_name(other)
        ))),
    }
}

/// Python truthiness.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// The node name of a `get_node`/`get_neighbors` call: the first non-empty `label`, `node_id` or
/// `id`.
fn node_name(args: &Args) -> Option<&str> {
    ["label", "node_id", "id"]
        .iter()
        .filter_map(|k| args.get(*k).and_then(Value::as_str))
        .find(|s| !s.is_empty())
}

/// Cut `text` to `budget * 3` characters at a line boundary, with Graphify's notices
/// (§10.6 `get_neighbors`).
pub fn truncate(text: &str, budget: i64, hint: &str) -> String {
    let len = text.chars().count() as i64;
    let chars = budget.saturating_mul(3);
    if len <= chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(chars.max(0) as usize).collect();
    let kept = match cut.rfind('\n') {
        Some(i) => &cut[..i],
        None => cut.as_str(),
    };
    let k = kept.split('\n').count();
    let n = text.split('\n').count();
    let need = (len + 2) / 3;
    format!(
        "[!] TRUNCATED: showing {k} of {n} lines (~{budget}-token budget). Raise the token budget to {need} to get all {n} (token_budget={need}). {hint}\n\n{kept}\n... (truncated — {} more lines cut by ~{budget}-token budget. {hint})",
        n - k
    )
}

/// Resolve a node name for `get_node`/`get_neighbors`.
fn find(ix: &Index, name: &str) -> Result<u32, Reply> {
    match resolve::resolve(ix, name) {
        resolve::Resolved::Found(i) => Ok(i),
        resolve::Resolved::Ambiguous(c) => Err(fail(resolve::ambiguous(ix, name, &c))),
        resolve::Resolved::NotFound => Err(fail(resolve::not_found(name))),
    }
}

fn tool_query_graph(e: &Entry, args: &Args) -> Result<Reply, ArgError> {
    let question = required_str(args, "question")?;
    let dfs = match args.get("mode") {
        None | Some(Value::Null) => false,
        Some(Value::String(m)) if m.eq_ignore_ascii_case("bfs") => false,
        Some(Value::String(m)) if m.eq_ignore_ascii_case("dfs") => true,
        Some(other) => {
            return Err(ArgError(format!(
                "mode must be \"bfs\" or \"dfs\", got {other}"
            )))
        }
    };
    // Above 6 is clamped (Graphify); below 0 traverses nothing, like 0.
    let depth = int_arg(args, "depth", 3)?.clamp(0, 6) as u32;
    let budget = int_arg(args, "token_budget", 2000)?;
    let context = match args.get("context_filter") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => (!s.is_empty()).then(|| vec![s.clone()]),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for v in items {
                match v {
                    Value::String(s) => out.push(s.clone()),
                    other => {
                        return Err(ArgError(format!(
                            "context_filter items must be strings, not '{}'",
                            type_name(other)
                        )))
                    }
                }
            }
            (!out.is_empty()).then_some(out)
        }
        Some(other) => {
            return Err(ArgError(format!(
                "context_filter must be a list of strings, not '{}'",
                type_name(other)
            )))
        }
    };
    let opts = query::QueryOptions {
        dfs,
        depth,
        context,
        order: query::NeighbourOrder::BySource,
    };
    Ok(match query::query(&e.ix, question, &opts) {
        None => ok(query::NO_MATCH),
        Some(ans) => ok(query::render(&e.ix, &e.shown, &ans, budget)),
    })
}

const NO_NODE_ARG: &str = "Provide a node label or id (accepted keys: label, node_id, id).";

fn tool_get_node(e: &Entry, args: &Args) -> Result<Reply, ArgError> {
    let Some(name) = node_name(args) else {
        return Ok(fail(NO_NODE_ARG));
    };
    let ix = &e.ix;
    let i = match find(ix, name) {
        Ok(i) => i,
        Err(r) => return Ok(r),
    };
    let n = ix.node(i);
    Ok(ok(format!(
        "Node: {}\n  ID: {}\n  Source: {} {}\n  Type: {}\n  Community: {}\n  Degree: {}",
        line_label(n.label),
        n.id,
        n.source_file,
        n.source_location,
        n.file_type,
        n.community,
        n.degree
    )))
}

fn tool_get_neighbors(e: &Entry, args: &Args) -> Result<Reply, ArgError> {
    let Some(name) = node_name(args) else {
        return Ok(fail(NO_NODE_ARG));
    };
    let budget = int_arg(args, "token_budget", 2000)?;
    let filter = match args.get("relation_filter") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s)) => Some(s.to_lowercase()),
        Some(other) => {
            return Err(ArgError(format!(
                "relation_filter must be a string, not '{}'",
                type_name(other)
            )))
        }
    };
    let ix = &e.ix;
    let i = match find(ix, name) {
        Ok(i) => i,
        Err(r) => return Ok(r),
    };
    let mut lines = vec![format!("Neighbors of {}:", line_label(ix.label(i)))];
    for outgoing in [true, false] {
        for l in ix.incident(i) {
            let edge = ix.edge(l);
            let (other, arrow) = if outgoing {
                (edge.target, "-->")
            } else {
                (edge.source, "<--")
            };
            if (if outgoing { edge.source } else { edge.target }) != i {
                continue;
            }
            if let Some(f) = &filter {
                if !edge.relation.to_lowercase().contains(f.as_str()) {
                    continue;
                }
            }
            // A link without a location has no `at=` (probed on excalidraw).
            let at = match edge.source_location {
                Some(loc) if !loc.is_empty() => {
                    format!(" at={}:{loc}", edge.source_file.unwrap_or(""))
                }
                _ => String::new(),
            };
            lines.push(format!(
                "  {arrow} {} [{}] [{}]{at}",
                line_label(ix.label(other)),
                edge.relation,
                edge.confidence.unwrap_or(""),
            ));
        }
    }
    Ok(ok(truncate(&lines.join("\n"), budget, NEIGHBORS_HINT)))
}

fn tool_get_community(e: &Entry, args: &Args) -> Result<Reply, ArgError> {
    let id = py_int(required(args, "community_id")?).map_err(ArgError)?;
    let budget = int_arg(args, "token_budget", 2000)?;
    let ix = &e.ix;
    let members: Vec<u32> = (0..ix.node_count() as u32)
        .filter(|&i| ix.node(i).community.id().is_some_and(|c| c as i64 == id))
        .collect();
    if members.is_empty() {
        return Ok(fail(format!("Community {id} not found.")));
    }
    let mut lines = vec![format!("Community {id} ({} nodes):", members.len())];
    for i in members {
        lines.push(format!(
            "  {} [{}]",
            line_label(ix.label(i)),
            ix.source_file(i)
        ));
    }
    Ok(ok(truncate(&lines.join("\n"), budget, COMMUNITY_HINT)))
}

/// The degree cap for `exclude_hubs_percentile`: NumPy's `interpolated_inverted_cdf`
/// percentile (position `p * n / 100 - 1`, clamped), fitted on every fixture and fastapi at
/// p = 0, 0.5, …, 100.
fn hub_percentile(values: &mut [u32], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable();
    let last = values.len() - 1;
    let pos = (p / 100.0 * values.len() as f64 - 1.0).clamp(0.0, last as f64);
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(last);
    let (a, b) = (values[lo] as f64, values[hi] as f64);
    a + (b - a) * (pos - lo as f64)
}

fn tool_god_nodes(e: &Entry, args: &Args) -> Result<Reply, ArgError> {
    let top = int_arg(args, "top_n", god_nodes::DEFAULT_TOP)?.max(1) as usize;
    let p =
        match args.get("exclude_hubs_percentile") {
            None | Some(Value::Null) => None,
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(s)) => Some(s.trim().parse::<f64>().map_err(|_| {
                ArgError(format!("could not convert string to float: {}", py_repr(s)))
            })?),
            Some(other) => {
                return Err(ArgError(format!(
                    "exclude_hubs_percentile must be a number, not '{}'",
                    type_name(other)
                )))
            }
        };
    let ix = &e.ix;
    let nodes = match p {
        None => god_nodes::god_nodes(ix, top as i64),
        Some(p) => {
            if !(0.0..=100.0).contains(&p) {
                return Err(ArgError(
                    "exclude_hubs_percentile must be between 0 and 100".into(),
                ));
            }
            let mut degrees: Vec<u32> = (0..ix.node_count() as u32).map(|i| ix.degree(i)).collect();
            let cap = hub_percentile(&mut degrees, p);
            let mut all = god_nodes::god_nodes(ix, ix.node_count() as i64);
            all.retain(|&i| ix.degree(i) as f64 <= cap);
            all.truncate(top);
            all
        }
    };
    Ok(ok(god_nodes::render_text(ix, &nodes)))
}

fn tool_shortest_path(e: &Entry, args: &Args) -> Result<Reply, ArgError> {
    let src = required_str(args, "source")?;
    let dst = required_str(args, "target")?;
    let max_hops = int_arg(args, "max_hops", 8)?;
    let undirected = args.get("undirected").is_some_and(truthy);
    let ix = &e.ix;
    let mut ends = [0u32; 2];
    for (k, (q, role)) in [(src, "source"), (dst, "target")].into_iter().enumerate() {
        ends[k] = match resolve::resolve(ix, q) {
            resolve::Resolved::Found(i) => i,
            resolve::Resolved::Ambiguous(c) => return Ok(fail(resolve::ambiguous(ix, q, &c))),
            resolve::Resolved::NotFound => {
                return Ok(fail(format!("No node matching {role} '{q}' found.")))
            }
        };
    }
    let [a, b] = ends;
    if a == b {
        return Ok(fail(path::same_node(src, dst, ix.id(a))));
    }
    Ok(match path::shortest_path(ix, a, b, undirected) {
        None if undirected => ok(format!(
            "No path found between '{}' and '{}'.",
            ix.label(a),
            ix.label(b)
        )),
        None => ok(format!(
            "No directed path found between '{}' and '{}'. Retry with undirected=true to search ignoring edge direction.",
            ix.label(a),
            ix.label(b)
        )),
        Some(p) if p.hops.len() as i64 > max_hops => ok(format!(
            "Path exceeds max_hops={max_hops} ({} hops found).",
            p.hops.len()
        )),
        Some(p) => ok(path::render(ix, &p)),
    })
}

impl Server {
    pub fn new(opts: Options) -> Server {
        let default = match &opts.graph {
            Some(p) => Target::File(opts.cwd.join(p)),
            None => Target::Project(opts.cwd.clone()),
        };
        // Load the startup graph while the client handshakes.
        let (t, cwd, verbose) = (default.clone(), opts.cwd.clone(), opts.verbose);
        let pending = std::thread::Builder::new()
            .name("dijkstral-mcp-load".into())
            .spawn(move || {
                let started = Instant::now();
                let file = locate(&t).ok()?;
                let entry = load_entry(&file, &cwd);
                if verbose {
                    match &entry {
                        Ok(e) => eprintln!(
                            "dijkstral mcp: loaded {} ({} nodes, {} links) in {:.1} ms",
                            file.display(),
                            e.ix.node_count(),
                            e.ix.edge_count(),
                            started.elapsed().as_secs_f64() * 1e3
                        ),
                        Err(err) => eprintln!("dijkstral mcp: {err}"),
                    }
                }
                Some((file, entry))
            })
            .ok();
        Server {
            opts,
            default,
            cache: HashMap::new(),
            pending,
            initialized: false,
            clock: 0,
        }
    }

    /// The current index for `target`, loading or reloading it as needed.
    fn graph(&mut self, target: &Target) -> Result<&Entry, String> {
        if let Some(h) = self.pending.take() {
            if let Ok(Some((file, Ok(entry)))) = h.join() {
                self.cache.insert(file, entry);
            }
        }
        let file = locate(target)?;
        self.clock += 1;
        let fresh = self
            .cache
            .get(&file)
            .is_some_and(|e| e.fp == fingerprint(&file));
        if !fresh {
            self.cache.remove(&file);
            let started = Instant::now();
            let entry = load_entry(&file, &self.opts.cwd)?;
            if self.opts.verbose {
                eprintln!(
                    "dijkstral mcp: loaded {} in {:.1} ms",
                    file.display(),
                    started.elapsed().as_secs_f64() * 1e3
                );
            }
            self.evict(&file);
            self.cache.insert(file.clone(), entry);
        }
        let entry = self.cache.get_mut(&file).expect("cached above");
        entry.used = self.clock;
        Ok(entry)
    }

    /// Keep at most [`CACHE_LIMIT`] graphs besides the startup one, dropping the least recently
    /// used.
    fn evict(&mut self, adding: &Path) {
        let pinned = locate(&self.default).ok();
        let others: Vec<(u64, PathBuf)> = self
            .cache
            .iter()
            .filter(|(p, _)| Some(p.as_path()) != pinned.as_deref() && p.as_path() != adding)
            .map(|(p, e)| (e.used, p.clone()))
            .collect();
        if others.len() >= CACHE_LIMIT {
            if let Some((_, p)) = others.into_iter().min() {
                self.cache.remove(&p);
            }
        }
    }

    fn target(&self, args: &Args) -> Target {
        match args.get("project_path").and_then(Value::as_str) {
            Some(p) if !p.is_empty() => Target::Project(self.opts.cwd.join(p)),
            _ => self.default.clone(),
        }
    }

    fn call_tool(&mut self, name: &str, args: &Args) -> Reply {
        if PR_TOOLS.contains(&name) {
            return fail(format!(
                "{name} is not available in dijkstral yet: the pull-request tools ship in a later release. Use query_graph, get_neighbors or shortest_path on the changed symbols instead."
            ));
        }
        let run: fn(&Entry, &Args) -> Result<Reply, ArgError> = match name {
            "query_graph" => tool_query_graph,
            "get_node" => tool_get_node,
            "get_neighbors" => tool_get_neighbors,
            "get_community" => tool_get_community,
            "god_nodes" => tool_god_nodes,
            "graph_stats" => |e, _| Ok(ok(insights::stats(&e.ix, e.commit.as_deref()))),
            "shortest_path" => tool_shortest_path,
            _ => return fail(format!("Unknown tool: {name}")),
        };
        let target = self.target(args);
        let entry = match self.graph(&target) {
            Ok(e) => e,
            Err(e) => return fail(format!("Error executing {name}: {e}")),
        };
        match run(entry, args) {
            Ok(r) => r,
            Err(ArgError(e)) => fail(format!("Error executing {name}: {e}")),
        }
    }

    fn read_resource(&mut self, uri: &str) -> Result<Value, (i64, String)> {
        let Some(&(_, _, mime, _)) = RESOURCES.iter().find(|r| r.0 == uri) else {
            return Err((0, format!("Unknown resource: {uri}")));
        };
        let target = self.default.clone();
        let e = self.graph(&target).map_err(|e| (0, e))?;
        let ix = &e.ix;
        let text = match uri {
            "graphify://report" => fs::read_to_string(e.dir.join("GRAPH_REPORT.md"))
                .unwrap_or_else(|_| {
                    "GRAPH_REPORT.md not found. Run dijkstral extract first.".to_string()
                }),
            "graphify://stats" => insights::stats(ix, e.commit.as_deref()),
            "graphify://god-nodes" => {
                god_nodes::render_text(ix, &god_nodes::god_nodes(ix, god_nodes::DEFAULT_TOP))
            }
            "graphify://surprises" => insights::render_surprises(ix, &insights::surprises(ix)),
            "graphify://audit" => insights::audit(ix),
            "graphify://questions" => insights::render_questions(&insights::questions(ix)),
            _ => unreachable!("listed above"),
        };
        Ok(json!({"contents": [{"mimeType": mime, "text": text, "uri": uri}]}))
    }

    /// Answer one parsed message; `None` for notifications.
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let Some(obj) = msg.as_object() else {
            return Some(error(Value::Null, -32600, "Invalid Request", None));
        };
        let id = obj.get("id").cloned();
        let method = obj.get("method").and_then(Value::as_str).unwrap_or("");
        let Some(id) = id else {
            // A notification (`notifications/initialized`, `notifications/cancelled`, …) or a
            // response to a request we never send: no reply.
            return None;
        };
        let params = obj.get("params").and_then(Value::as_object);
        let empty = Map::new();
        let params = params.unwrap_or(&empty);
        let result: Result<Value, (i64, String, Option<Value>)> = match method {
            "initialize" => {
                self.initialized = true;
                let asked = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let version = if PROTOCOL_VERSIONS.contains(&asked) {
                    asked
                } else {
                    LATEST_PROTOCOL
                };
                Ok(json!({
                    "capabilities": {
                        "resources": {"listChanged": false, "subscribe": false},
                        "tools": {"listChanged": false}
                    },
                    "protocolVersion": version,
                    "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")}
                }))
            }
            "ping" => Ok(json!({})),
            _ if !self.initialized => Err(invalid_params()),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => match params.get("name").and_then(Value::as_str) {
                None => Err(invalid_params()),
                Some(name) => {
                    let args = params
                        .get("arguments")
                        .and_then(Value::as_object)
                        .unwrap_or(&empty);
                    let r = self.call_tool(name, args);
                    Ok(json!({
                        "content": [{"text": r.text, "type": "text"}],
                        "isError": r.is_error
                    }))
                }
            },
            "resources/list" => Ok(json!({
                "resources": RESOURCES
                    .iter()
                    .map(|(uri, name, mime, desc)| json!({
                        "description": desc, "mimeType": mime, "name": name, "uri": uri
                    }))
                    .collect::<Vec<_>>()
            })),
            "resources/read" => match params.get("uri").and_then(Value::as_str) {
                None => Err(invalid_params()),
                Some(uri) => self.read_resource(uri).map_err(|(c, m)| (c, m, None)),
            },
            other => Err((-32601, "Method not found".into(), Some(json!(other)))),
        };
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, message, data)) => error(id, code, &message, data),
        })
    }
}

fn invalid_params() -> (i64, String, Option<Value>) {
    (-32602, "Invalid request parameters".into(), Some(json!("")))
}

fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut err = json!({"code": code, "message": message});
    if let Some(d) = data {
        err["data"] = d;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": err})
}

/// Serve until stdin closes.
pub fn serve(opts: Options, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
    let verbose = opts.verbose;
    let mut server = Server::new(opts);
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let started = Instant::now();
        let (reply, method) = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let method = msg
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                (server.handle(&msg), method)
            }
            Err(_) => (
                Some(error(Value::Null, -32700, "Parse error", None)),
                "<unparseable>".to_string(),
            ),
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut output, &reply)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
        if verbose {
            eprintln!(
                "dijkstral mcp: {method} {:.2} ms",
                started.elapsed().as_secs_f64() * 1e3
            );
        }
    }
    Ok(())
}

/// `(uri, name, mimeType, description)`, in Graphify's order.
const RESOURCES: &[(&str, &str, &str, &str)] = &[
    (
        "graphify://report",
        "Graph Report",
        "text/markdown",
        "The full GRAPH_REPORT.md",
    ),
    (
        "graphify://stats",
        "Graph Stats",
        "text/plain",
        "Node, link and community counts with the share of each confidence tier",
    ),
    (
        "graphify://god-nodes",
        "God Nodes",
        "text/plain",
        "The ten most connected symbols",
    ),
    (
        "graphify://surprises",
        "Surprising Connections",
        "text/plain",
        "Unexpected links between files, directories and communities",
    ),
    (
        "graphify://audit",
        "Confidence Audit",
        "text/plain",
        "Link counts by confidence: extracted, inferred, ambiguous",
    ),
    (
        "graphify://questions",
        "Suggested Questions",
        "text/plain",
        "Questions this graph is well placed to answer",
    ),
];

fn project_path_schema() -> Value {
    json!({
        "type": "string",
        "description": "Absolute path to a project directory containing graphify-out/graph.json. Optional — defaults to the graph this server was started with."
    })
}

/// The tool list: Graphify's names, order and input schemas (§10.3), dijkstral's descriptions.
pub fn tools() -> Vec<Value> {
    let pp = project_path_schema();
    vec![
        json!({
            "description": "Answer a question from the code graph: find the best-matching symbols and return them with their neighbourhood, citing file:line.",
            "inputSchema": {
                "properties": {
                    "question": {"type": "string", "description": "Natural language question or keyword search"},
                    "mode": {"type": "string", "enum": ["bfs", "dfs"], "default": "bfs", "description": "bfs=broad context, dfs=trace a specific path"},
                    "depth": {"type": "integer", "default": 3, "description": "Traversal depth (1-6)"},
                    "token_budget": {"type": "integer", "default": 2000, "description": "Max output tokens"},
                    "context_filter": {"type": "array", "items": {"type": "string"}, "description": "Optional explicit edge-context filter, e.g. ['call', 'field']"},
                    "project_path": pp
                },
                "required": ["question"],
                "type": "object"
            },
            "name": "query_graph"
        }),
        json!({
            "description": "Show one symbol: id, file and line, type, community and link count.",
            "inputSchema": {
                "properties": {
                    "label": {"type": "string", "description": "Node label or ID to look up"},
                    "node_id": {"type": "string", "description": "Alias for label (node id)"},
                    "project_path": pp
                },
                "type": "object"
            },
            "name": "get_node"
        }),
        json!({
            "description": "List a symbol's incoming and outgoing links with relation, confidence and file:line.",
            "inputSchema": {
                "properties": {
                    "label": {"type": "string"},
                    "node_id": {"type": "string", "description": "Alias for label (node id)"},
                    "relation_filter": {"type": "string", "description": "Optional: filter by relation type"},
                    "token_budget": {"type": "integer", "default": 2000, "description": "Max output tokens"},
                    "project_path": pp
                },
                "type": "object"
            },
            "name": "get_neighbors"
        }),
        json!({
            "description": "List the members of one community (a cluster of closely linked symbols).",
            "inputSchema": {
                "properties": {
                    "community_id": {"type": "integer", "description": "Community ID (0-indexed by size)"},
                    "token_budget": {"type": "integer", "default": 2000, "description": "Max output tokens"},
                    "project_path": pp
                },
                "required": ["community_id"],
                "type": "object"
            },
            "name": "get_community"
        }),
        json!({
            "description": "Rank the most connected symbols, optionally leaving out the extreme hubs.",
            "inputSchema": {
                "properties": {
                    "top_n": {"type": "integer", "default": 10},
                    "exclude_hubs_percentile": {"type": "number", "description": "Suppress nodes whose degree exceeds this percentile (0-100) of the degree distribution, matching cluster()'s hub exclusion"},
                    "project_path": pp
                },
                "type": "object"
            },
            "name": "god_nodes"
        }),
        json!({
            "description": "Summarise the graph: node, link and community counts and the share of each confidence tier.",
            "inputSchema": {
                "properties": {"project_path": pp},
                "type": "object"
            },
            "name": "graph_stats"
        }),
        json!({
            "description": "Find the shortest chain of links from one symbol to another, following link direction unless undirected is set.",
            "inputSchema": {
                "properties": {
                    "source": {"type": "string", "description": "Source concept label or keyword"},
                    "target": {"type": "string", "description": "Target concept label or keyword"},
                    "max_hops": {"type": "integer", "default": 8, "description": "Maximum hops to consider"},
                    "undirected": {"type": "boolean", "default": false, "description": "Ignore stored edge direction when searching"},
                    "project_path": pp
                },
                "required": ["source", "target"],
                "type": "object"
            },
            "name": "shortest_path"
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_coercion_follows_python() {
        assert_eq!(py_int(&json!(3)), Ok(3));
        assert_eq!(py_int(&json!(2.9)), Ok(2));
        assert_eq!(py_int(&json!(" 12 ")), Ok(12));
        assert_eq!(py_int(&json!("1_000")), Ok(1000));
        assert_eq!(py_int(&json!(true)), Ok(1));
        assert_eq!(
            py_int(&json!("abc")),
            Err("invalid literal for int() with base 10: 'abc'".into())
        );
        assert!(py_int(&json!("1__0")).is_err());
        assert!(py_int(&json!(null)).is_err());
    }

    #[test]
    fn truncation_matches_graphify() {
        // 4 lines of 9 characters plus newlines: 39 characters, 13 tokens.
        let text = "aaaaaaaaa\nbbbbbbbbb\nccccccccc\nddddddddd";
        assert_eq!(truncate(text, 13, "h"), text);
        let t = truncate(text, 7, "h");
        assert!(t.starts_with(
            "[!] TRUNCATED: showing 2 of 4 lines (~7-token budget). Raise the token budget to 13 to get all 4 (token_budget=13). h\n\naaaaaaaaa\nbbbbbbbbb\n... (truncated — 2 more lines cut by ~7-token budget. h)"
        ));
        // Budget 0 keeps one empty line.
        assert!(truncate(text, 0, "h").contains("showing 1 of 4 lines"));
    }

    #[test]
    fn handshake_and_errors() {
        let mut s = Server::new(Options {
            graph: Some(PathBuf::from("/nonexistent/graph.json")),
            cwd: PathBuf::from("/"),
            verbose: false,
        });
        let before = s
            .handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .unwrap();
        assert_eq!(before["error"]["code"], -32602);
        let init = s
            .handle(&json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}))
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], LATEST_PROTOCOL);
        assert!(s
            .handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .is_none());
        let call = s
            .handle(&json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"graph_stats","arguments":{}}}))
            .unwrap();
        assert_eq!(call["result"]["isError"], true);
        assert_eq!(
            call["result"]["content"][0]["text"],
            "Error executing graph_stats: graph.json not found: /nonexistent/graph.json"
        );
        let pr = s
            .handle(
                &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_prs"}}),
            )
            .unwrap();
        assert_eq!(pr["result"]["isError"], true);
        let unknown = s
            .handle(&json!({"jsonrpc":"2.0","id":5,"method":"prompts/list"}))
            .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        assert_eq!(unknown["error"]["data"], "prompts/list");
    }
}

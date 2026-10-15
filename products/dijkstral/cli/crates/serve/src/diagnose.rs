//! `diagnose multigraph`: how many edges of a graph or extraction JSON collapse when it is loaded
//! as a graph with one edge per endpoint pair (`docs/graphify-memory.md` §6). Read-only.
//!
//! Counters, groups, examples, the suppression scan and the JSON keys are Graphify's; the prose
//! lines are dijkstral's own.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};
use dijkstral_core::pyjson::{self, Json};
use serde_json::Value;

use crate::text::py_repr_list;

pub const DEFAULT_MAX_EXAMPLES: usize = 5;
/// Example lines of scan sites shown in the text report.
const SITE_LINES: usize = 8;

#[derive(Debug, Clone)]
pub struct Options {
    /// `Some` forces the direction; `None` follows the input's `directed` flag.
    pub directed: Option<bool>,
    pub max_examples: usize,
}

/// One endpoint pair with two or more edges.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub source: String,
    pub target: String,
    pub edge_count: usize,
    pub relations: Vec<String>,
    pub source_files: Vec<String>,
    pub source_locations: Vec<String>,
    pub contexts: Vec<String>,
}

/// One place in a source file that could drop duplicates.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub line: usize,
    pub name: String,
    pub tuple_arity: usize,
    pub sample: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scan {
    pub path: String,
    pub sites: Vec<Site>,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub input_path: String,
    pub effective_directed: bool,
    pub node_count: usize,
    pub unverified_node_count: usize,
    pub raw_edge_count: usize,
    pub non_object_edges: usize,
    pub missing_endpoint_edges: usize,
    pub dangling_endpoint_edges: usize,
    pub external_reference_edges: usize,
    pub self_loop_edges: usize,
    pub valid_candidate_edges: usize,
    pub exact_duplicate_edges: usize,
    pub directed_unique_endpoint_pairs: usize,
    pub undirected_unique_endpoint_pairs: usize,
    pub same_endpoint_group_count: usize,
    pub relation_variant_groups: usize,
    pub source_file_variant_groups: usize,
    pub source_location_variant_groups: usize,
    pub context_variant_groups: usize,
    /// `Ok((nodes, edges))`, or why the build would fail.
    pub post_build: std::result::Result<(usize, usize), String>,
    pub examples: Vec<Group>,
    pub scan: Scan,
}

/// Python's `str()` of a JSON value, which is how ids and endpoints are compared.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if !n.is_i64() && !n.is_u64() => pyjson::float_repr(f),
            _ => n.to_string(),
        },
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(py_repr_value).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(m) => {
            let parts: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{}: {}", crate::text::py_repr(k), py_repr_value(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

fn py_repr_value(v: &Value) -> String {
    match v {
        Value::String(s) => crate::text::py_repr(s),
        other => py_str(other),
    }
}

/// A text field of an edge; missing or `null` is `""`.
fn field(e: &serde_json::Map<String, Value>, key: &str) -> String {
    match e.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(v) => py_str(v),
    }
}

/// JSON with object keys sorted at every level: equal values give equal text.
fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Diagnose a parsed input. `Err` when it is not a JSON object.
pub fn diagnose(input_path: &str, data: &Value, o: &Options) -> Result<Report> {
    let Some(obj) = data.as_object() else {
        return Err(anyhow!("diagnostic input must be a JSON object"));
    };
    let mut r = Report {
        input_path: input_path.to_string(),
        effective_directed: o
            .directed
            .unwrap_or(!matches!(obj.get("directed"), Some(Value::Bool(false)))),
        node_count: 0,
        unverified_node_count: 0,
        raw_edge_count: 0,
        non_object_edges: 0,
        missing_endpoint_edges: 0,
        dangling_endpoint_edges: 0,
        external_reference_edges: 0,
        self_loop_edges: 0,
        valid_candidate_edges: 0,
        exact_duplicate_edges: 0,
        directed_unique_endpoint_pairs: 0,
        undirected_unique_endpoint_pairs: 0,
        same_endpoint_group_count: 0,
        relation_variant_groups: 0,
        source_file_variant_groups: 0,
        source_location_variant_groups: 0,
        context_variant_groups: 0,
        post_build: Ok((0, 0)),
        examples: Vec::new(),
        scan: Scan::default(),
    };
    let nodes: &[Value] = obj
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let edges: &[Value] = obj
        .get("edges")
        .or_else(|| obj.get("links"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let ids: HashSet<String> = nodes
        .iter()
        .filter_map(|n| n.as_object()?.get("id").map(py_str))
        .collect();
    r.node_count = ids.len();
    r.raw_edge_count = edges.len();

    let mut seen_exact: HashSet<String> = HashSet::new();
    let mut valid: Vec<(String, String, &serde_json::Map<String, Value>)> = Vec::new();
    let mut build_error: Option<String> = match obj.get("nodes") {
        Some(v) if !v.is_array() => Some("`nodes` is not a list".into()),
        _ => None,
    };
    if build_error.is_none() {
        build_error = nodes
            .iter()
            .position(|n| !n.is_object())
            .map(|i| format!("node {i} is not an object"));
    }
    for (i, e) in edges.iter().enumerate() {
        let Some(m) = e.as_object() else {
            // Every non-object edge counts as the same edge for duplicates.
            if r.non_object_edges > 0 {
                r.exact_duplicate_edges += 1;
            }
            r.non_object_edges += 1;
            build_error.get_or_insert_with(|| format!("edge {i} is not an object"));
            continue;
        };
        let mut text = String::new();
        canonical(e, &mut text);
        if !seen_exact.insert(text) {
            r.exact_duplicate_edges += 1;
        }
        for key in ["source", "target"] {
            if matches!(m.get(key), Some(Value::Null | Value::Bool(_))) {
                build_error
                    .get_or_insert_with(|| format!("edge {i} has a {key} that is not a node id"));
            }
        }
        let end = |key: &str| match m.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            Some(v) => Some(py_str(v)),
        };
        let (Some(s), Some(t)) = (end("source"), end("target")) else {
            r.missing_endpoint_edges += 1;
            continue;
        };
        if !ids.contains(&s) || !ids.contains(&t) {
            r.dangling_endpoint_edges += 1;
            continue;
        }
        if s == t {
            r.self_loop_edges += 1;
        }
        valid.push((s, t, m));
    }
    r.valid_candidate_edges = valid.len();

    // Directed groups, in order of first appearance.
    let mut order: Vec<(String, String)> = Vec::new();
    let mut groups: HashMap<(String, String), Vec<&serde_json::Map<String, Value>>> =
        HashMap::new();
    let mut undirected: HashSet<(String, String)> = HashSet::new();
    for (s, t, m) in &valid {
        let key = (s.clone(), t.clone());
        let g = groups.entry(key.clone()).or_default();
        if g.is_empty() {
            order.push(key);
        }
        g.push(m);
        let pair = if s <= t {
            (s.clone(), t.clone())
        } else {
            (t.clone(), s.clone())
        };
        undirected.insert(pair);
    }
    r.directed_unique_endpoint_pairs = order.len();
    r.undirected_unique_endpoint_pairs = undirected.len();
    let mut multi: Vec<Group> = Vec::new();
    for key in &order {
        let g = &groups[key];
        if g.len() < 2 {
            continue;
        }
        r.same_endpoint_group_count += 1;
        let distinct = |f: &str| -> Vec<String> {
            let set: BTreeSet<String> = g.iter().map(|m| field(m, f)).collect();
            set.into_iter().collect()
        };
        let relations = distinct("relation");
        if relations.len() > 1 {
            r.relation_variant_groups += 1;
        }
        let varies_within_relation = |f: &str| {
            let mut per: HashMap<String, HashSet<String>> = HashMap::new();
            for m in g {
                per.entry(field(m, "relation"))
                    .or_default()
                    .insert(field(m, f));
            }
            per.values().any(|v| v.len() > 1)
        };
        r.source_file_variant_groups += usize::from(varies_within_relation("source_file"));
        r.source_location_variant_groups += usize::from(varies_within_relation("source_location"));
        r.context_variant_groups += usize::from(varies_within_relation("context"));
        multi.push(Group {
            source: key.0.clone(),
            target: key.1.clone(),
            edge_count: g.len(),
            relations,
            source_files: distinct("source_file"),
            source_locations: distinct("source_location"),
            contexts: distinct("context"),
        });
    }
    multi.sort_by_key(|g| std::cmp::Reverse(g.edge_count));
    multi.truncate(o.max_examples);
    r.examples = multi;
    r.post_build = match build_error {
        Some(e) => Err(e),
        None => Ok((
            ids.len(),
            if r.effective_directed {
                r.directed_unique_endpoint_pairs
            } else {
                r.undirected_unique_endpoint_pairs
            },
        )),
    };
    Ok(r)
}

/// [`diagnose`] a file, named `shown` in the report. Errors are one-line messages.
pub fn diagnose_file(path: &Path, shown: &str, o: &Options) -> Result<Report> {
    let bytes = fs::read(path).map_err(|e| anyhow!("cannot read {shown}: {e}"))?;
    let data: Value =
        serde_json::from_slice(&bytes).map_err(|e| anyhow!("{shown} is not valid JSON: {e}"))?;
    diagnose(shown, &data, o)
        .map_err(|_| anyhow!("{shown} holds no JSON object; nothing to diagnose"))
}

/// The suppression scan of a Python source file (§6.7). A missing file is recorded as an error;
/// a file that cannot be read as text fails the command.
pub fn scan(path: &Path, shown: &str) -> Result<Scan> {
    let mut s = Scan {
        path: shown.to_string(),
        ..Scan::default()
    };
    if !path.exists() {
        s.error = "file not found".into();
        return Ok(s);
    }
    let bytes = fs::read(path).map_err(|e| anyhow!("{shown}: {e}"))?;
    let text =
        String::from_utf8(bytes).map_err(|_| anyhow!("{shown}: the file is not UTF-8 text"))?;
    for (i, line) in text.split('\n').enumerate() {
        let stripped = line.trim();
        let Some(rest) = stripped.strip_prefix("seen_") else {
            continue;
        };
        let ident_len = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let after = rest[ident_len..].trim_start();
        let assigns =
            after.starts_with(':') || (after.starts_with('=') && !after.starts_with("=="));
        if !assigns {
            continue;
        }
        let tuple_arity = stripped
            .find("tuple[")
            .map(|at| {
                let inner = &stripped[at + 6..];
                let inner = inner.split(']').next().unwrap_or(inner);
                inner.split(',').count()
            })
            .unwrap_or(0);
        s.sites.push(Site {
            line: i + 1,
            name: format!("seen_{}", &rest[..ident_len]),
            tuple_arity,
            sample: stripped.to_string(),
        });
    }
    Ok(s)
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

/// The text report.
pub fn render_text(r: &Report) -> String {
    let mut out = String::from("[dijkstral] multigraph edge-collapse diagnostic\n");
    let _ = writeln!(out, "input: {}", r.input_path);
    out.push_str(
        "input_stage: the JSON as given (a written graph.json has already been collapsed)\n",
    );
    let _ = writeln!(out, "effective_directed: {}", yes_no(r.effective_directed));
    let counts: [(&str, usize); 18] = [
        ("nodes", r.node_count),
        ("unverified_code_nodes", r.unverified_node_count),
        ("raw_edges", r.raw_edge_count),
        ("valid_candidate_edges", r.valid_candidate_edges),
        ("missing_endpoint_edges", r.missing_endpoint_edges),
        ("dangling_endpoint_edges", r.dangling_endpoint_edges),
        ("external_reference_edges", r.external_reference_edges),
        ("self_loop_edges", r.self_loop_edges),
        ("exact_duplicate_edges", r.exact_duplicate_edges),
        (
            "directed_unique_endpoint_pairs",
            r.directed_unique_endpoint_pairs,
        ),
        (
            "directed_same_endpoint_collapsed_edges",
            r.valid_candidate_edges - r.directed_unique_endpoint_pairs,
        ),
        (
            "undirected_unique_endpoint_pairs",
            r.undirected_unique_endpoint_pairs,
        ),
        (
            "undirected_same_endpoint_collapsed_edges",
            r.valid_candidate_edges - r.undirected_unique_endpoint_pairs,
        ),
        ("same_endpoint_group_count", r.same_endpoint_group_count),
        ("relation_variant_groups", r.relation_variant_groups),
        ("source_file_variant_groups", r.source_file_variant_groups),
        (
            "source_location_variant_groups",
            r.source_location_variant_groups,
        ),
        ("context_variant_groups", r.context_variant_groups),
    ];
    for (k, v) in counts {
        let _ = writeln!(out, "{k}: {v}");
    }
    match &r.post_build {
        Ok((_, edges)) => {
            let _ = writeln!(out, "post_build_graph_type: {}", graph_type(r));
            let _ = writeln!(out, "post_build_edges: {edges}");
        }
        Err(_) => out.push_str("post_build_graph_type: \npost_build_edges: None\n"),
    }
    let _ = writeln!(out, "producer_suppression_sites: {}", r.scan.sites.len());
    if let Err(e) = &r.post_build {
        let _ = writeln!(out, "post_build_error: {e}");
    }
    if !r.scan.error.is_empty() {
        let _ = writeln!(out, "producer_suppression_error: {}", r.scan.error);
    }
    if !r.scan.sites.is_empty() {
        out.push_str("producer_suppression_examples:\n");
        for s in r.scan.sites.iter().take(SITE_LINES) {
            let arity = match s.tuple_arity {
                0 => "unknown".to_string(),
                n => n.to_string(),
            };
            let _ = writeln!(out, "  - L{} {} arity={arity}", s.line, s.name);
        }
    }
    if !r.examples.is_empty() {
        out.push_str("examples:\n");
        for g in &r.examples {
            let _ = writeln!(
                out,
                "  - {} -> {} edges={} relations={} locations={} contexts={}",
                g.source,
                g.target,
                g.edge_count,
                py_repr_list(g.relations.iter().map(String::as_str)),
                py_repr_list(g.source_locations.iter().map(String::as_str)),
                py_repr_list(g.contexts.iter().map(String::as_str)),
            );
        }
    }
    out.push_str(
        "note: graph.json is written after the collapse; measure producer losses on raw extraction output.\n",
    );
    out
}

fn graph_type(r: &Report) -> &'static str {
    if r.effective_directed {
        "DiGraph"
    } else {
        "Graph"
    }
}

fn int(n: usize) -> Json {
    Json::Int(n as i64)
}

fn strs(v: &[String]) -> Json {
    Json::Array(v.iter().map(|s| Json::str(s)).collect())
}

/// The `--json` report (Python `json.dumps(indent=2)` layout, no trailing newline).
pub fn render_json(r: &Report) -> String {
    let (pb_type, pb_nodes, pb_edges, pb_error) = match &r.post_build {
        Ok((n, e)) => (graph_type(r), int(*n), int(*e), String::new()),
        Err(e) => ("", Json::Null, Json::Null, e.clone()),
    };
    let summary = Json::Object(vec![
        ("node_count".into(), int(r.node_count)),
        ("unverified_node_count".into(), int(r.unverified_node_count)),
        ("raw_edge_count".into(), int(r.raw_edge_count)),
        ("non_object_edges".into(), int(r.non_object_edges)),
        (
            "missing_endpoint_edges".into(),
            int(r.missing_endpoint_edges),
        ),
        (
            "dangling_endpoint_edges".into(),
            int(r.dangling_endpoint_edges),
        ),
        (
            "external_reference_edges".into(),
            int(r.external_reference_edges),
        ),
        ("self_loop_edges".into(), int(r.self_loop_edges)),
        ("valid_candidate_edges".into(), int(r.valid_candidate_edges)),
        ("exact_duplicate_edges".into(), int(r.exact_duplicate_edges)),
        (
            "directed_unique_endpoint_pairs".into(),
            int(r.directed_unique_endpoint_pairs),
        ),
        (
            "directed_same_endpoint_collapsed_edges".into(),
            int(r.valid_candidate_edges - r.directed_unique_endpoint_pairs),
        ),
        (
            "undirected_unique_endpoint_pairs".into(),
            int(r.undirected_unique_endpoint_pairs),
        ),
        (
            "undirected_same_endpoint_collapsed_edges".into(),
            int(r.valid_candidate_edges - r.undirected_unique_endpoint_pairs),
        ),
        (
            "same_endpoint_group_count".into(),
            int(r.same_endpoint_group_count),
        ),
        (
            "relation_variant_groups".into(),
            int(r.relation_variant_groups),
        ),
        (
            "source_file_variant_groups".into(),
            int(r.source_file_variant_groups),
        ),
        (
            "source_location_variant_groups".into(),
            int(r.source_location_variant_groups),
        ),
        (
            "context_variant_groups".into(),
            int(r.context_variant_groups),
        ),
        ("post_build_graph_type".into(), Json::str(pb_type)),
        ("post_build_node_count".into(), pb_nodes),
        ("post_build_edge_count".into(), pb_edges),
        ("post_build_error".into(), Json::Str(pb_error)),
        ("input_path".into(), Json::str(&r.input_path)),
        (
            "effective_directed".into(),
            Json::Bool(r.effective_directed),
        ),
    ]);
    let examples = Json::Array(
        r.examples
            .iter()
            .map(|g| {
                Json::Object(vec![
                    ("source".into(), Json::str(&g.source)),
                    ("target".into(), Json::str(&g.target)),
                    ("edge_count".into(), int(g.edge_count)),
                    ("relations".into(), strs(&g.relations)),
                    ("source_files".into(), strs(&g.source_files)),
                    ("source_locations".into(), strs(&g.source_locations)),
                    ("contexts".into(), strs(&g.contexts)),
                ])
            })
            .collect(),
    );
    let sites = Json::Array(
        r.scan
            .sites
            .iter()
            .map(|s| {
                Json::Object(vec![
                    ("line".into(), int(s.line)),
                    ("name".into(), Json::str(&s.name)),
                    ("tuple_arity".into(), int(s.tuple_arity)),
                    ("sample".into(), Json::str(&s.sample)),
                ])
            })
            .collect(),
    );
    let doc = Json::Object(vec![
        ("schema_version".into(), Json::Int(1)),
        ("summary".into(), summary),
        ("examples".into(), examples),
        (
            "producer_suppression".into(),
            Json::Object(vec![
                ("path".into(), Json::str(&r.scan.path)),
                ("total_sites".into(), int(r.scan.sites.len())),
                ("sites".into(), sites),
                ("error".into(), Json::str(&r.scan.error)),
            ]),
        ),
        (
            "notes".into(),
            Json::Array(vec![
                Json::str("Read-only: nothing was written."),
                Json::str(
                    "A written graph.json is already collapsed; raw producer edges cannot be recovered from it.",
                ),
                Json::str("Suppression sites come from a text scan of source code and are only hints."),
            ]),
        ),
    ]);
    pyjson::dumps_indent(&doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opts() -> Options {
        Options {
            directed: None,
            max_examples: 5,
        }
    }

    #[test]
    fn counts_every_edge_class() {
        let g = json!({"nodes": [{"id": "a"}, {"id": "b"}, {"id": 1}], "links": [
            {"source": "a", "target": "b", "relation": "x"},
            {"relation": "x", "target": "b", "source": "a"},
            {"source": "a"},
            {"source": "", "target": "a"},
            {"source": "a", "target": "zz"},
            {"source": "1", "target": "a"},
            {"source": "a", "target": "a"},
            "junk"
        ]});
        let r = diagnose("g.json", &g, &opts()).unwrap();
        assert_eq!(r.node_count, 3);
        assert_eq!(r.non_object_edges, 1);
        assert_eq!(r.missing_endpoint_edges, 2);
        assert_eq!(r.dangling_endpoint_edges, 1);
        assert_eq!(r.valid_candidate_edges, 4);
        assert_eq!(r.exact_duplicate_edges, 1);
        assert_eq!(r.self_loop_edges, 1);
        assert_eq!(r.same_endpoint_group_count, 1);
        assert!(r.post_build.is_err());
    }

    #[test]
    fn variants_are_counted_within_one_relation() {
        let g = json!({"nodes": [{"id": "a"}, {"id": "b"}], "links": [
            {"source": "a", "target": "b", "relation": "x", "source_location": "L1"},
            {"source": "a", "target": "b", "relation": "y", "source_location": "L2"}
        ]});
        let r = diagnose("g.json", &g, &opts()).unwrap();
        assert_eq!(
            (r.relation_variant_groups, r.source_location_variant_groups),
            (1, 0)
        );
    }

    #[test]
    fn scan_finds_seen_assignments() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.py");
        fs::write(
            &p,
            "seen = set()\n  seen_a: set[tuple[str, tuple[a, b]]] = set()\nseen_b.add(1)\nseen_c == 1\nseen_d=[]\n",
        )
        .unwrap();
        let s = scan(&p, "x.py").unwrap();
        let got: Vec<(usize, &str, usize)> = s
            .sites
            .iter()
            .map(|s| (s.line, s.name.as_str(), s.tuple_arity))
            .collect();
        assert_eq!(got, [(2, "seen_a", 3), (5, "seen_d", 0)]);
        let missing = scan(&dir.path().join("nope.py"), "nope.py").unwrap();
        assert_eq!(missing.error, "file not found");
    }
}

//! The assembled graph and its Graphify-compatible `graph.json` encoding (`docs/graphify-format.md`
//! §2, §8).
//!
//! Graphify stores a simple undirected graph: one link per unordered node pair. Edges are folded in
//! emission order (per-file edges in path order, then the cross-file pass) by the rank rules of
//! §8.2. Direction is kept as written by the surviving edge.

use std::collections::{BTreeMap, HashMap, HashSet};

use rayon::prelude::*;
use serde_json::{Map, Value};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::facts::{Attr, Edge, Loc, Node};

pub const SCHEMA_VERSION: u32 = 1;
/// The Graphify version whose output format this crate reproduces.
pub const COMPAT_VERSION: &str = "0.9.86";

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Edges dropped because an endpoint does not exist.
    pub dangling: usize,
}

/// Collapse precedence (§8.2). Unobserved relations rank with the structural ones.
fn rank(relation: &str) -> u8 {
    match relation {
        "references" => 10,
        "uses" => 20,
        "calls" | "case_of" | "dynamic_import" => 30,
        "imports" => 45,
        "imports_from" | "indirect_call" => 50,
        "re_exports" => 60,
        _ => 40, // contains, method, inherits, implements, rationale_for, embeds, defines, …
    }
}

fn is_structural(relation: &str) -> bool {
    relation == "contains" || relation == "method"
}

/// `{**base, **over}`: every field `over` carries wins; fields only `base` has survive.
fn merge_over(base: &Edge, over: &Edge) -> Edge {
    let mut attrs = base.attrs.clone();
    for (k, v) in &over.attrs {
        match attrs.iter_mut().find(|(bk, _)| bk == k) {
            Some(slot) => slot.1 = v.clone(),
            None => attrs.push((k.clone(), v.clone())),
        }
    }
    Edge {
        source: over.source.clone(),
        target: over.target.clone(),
        relation: over.relation.clone(),
        confidence: over.confidence,
        score: over.score.or(base.score),
        context: over.context.clone().or_else(|| base.context.clone()),
        source_file: over.source_file.clone(),
        loc: if over.loc == Loc::Absent {
            base.loc
        } else {
            over.loc
        },
        weight: over.weight.or(base.weight),
        attrs,
    }
}

/// Fold `new` into the link `old` already holding the same unordered pair.
fn fold(old: &mut Edge, new: Edge) {
    let (ro, rn) = (rank(&old.relation), rank(&new.relation));
    if old.source == new.source {
        if rn >= ro {
            *old = if is_structural(&new.relation) {
                new
            } else {
                merge_over(old, &new)
            };
        } else if is_structural(&old.relation) {
            if new.relation == "calls" && old.context.is_none() {
                old.context = Some("call".into());
            }
        } else {
            *old = merge_over(&new, old);
        }
    } else if rn > ro || (rn == ro && new.source < old.source) {
        *old = new;
    }
}

impl Graph {
    /// Merge duplicate node ids (first wins), drop dangling edges, and fold parallel edges to one
    /// link per unordered pair in the given order.
    pub fn assemble(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
        let mut seen = HashSet::with_capacity(nodes.len());
        let nodes: Vec<Node> = nodes
            .into_iter()
            .filter(|n| seen.insert(n.id.clone()))
            .collect();

        let mut slot: HashMap<(String, String), usize> = HashMap::with_capacity(edges.len());
        let mut kept: Vec<Edge> = Vec::with_capacity(edges.len());
        let mut dangling = 0;
        for e in edges {
            if !seen.contains(&e.source) || !seen.contains(&e.target) {
                dangling += 1;
                continue;
            }
            let pair = if e.source <= e.target {
                (e.source.clone(), e.target.clone())
            } else {
                (e.target.clone(), e.source.clone())
            };
            match slot.get(&pair) {
                None => {
                    slot.insert(pair, kept.len());
                    kept.push(e);
                }
                Some(&i) => fold(&mut kept[i], e),
            }
        }
        Graph {
            nodes,
            edges: kept,
            dangling,
        }
    }

    /// Clustered-shape `graph.json` (§2.1): compact, ASCII-escaped, no trailing newline, arrays in
    /// Graphify's sort order. `community` is written once clustering exists.
    pub fn to_graphify_json(&self, built_at_commit: Option<&str>) -> String {
        let mut nodes: Vec<(String, String)> =
            self.nodes.par_iter().map(|n| encode(node_map(n))).collect();
        let mut links: Vec<(String, String)> =
            self.edges.par_iter().map(|e| encode(edge_map(e))).collect();
        nodes.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
        links.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));

        let mut out = String::with_capacity(
            nodes
                .iter()
                .chain(&links)
                .map(|(_, s)| s.len() + 1)
                .sum::<usize>()
                + 256,
        );
        out.push_str("{\"directed\":false,\"multigraph\":false,\"graph\":{\"schema_version\":");
        out.push_str(&SCHEMA_VERSION.to_string());
        out.push_str(",\"graphify_version\":\"");
        out.push_str(COMPAT_VERSION);
        out.push_str("\"},\"nodes\":[");
        push_joined(&mut out, &nodes);
        out.push_str("],\"links\":[");
        push_joined(&mut out, &links);
        out.push_str("],\"hyperedges\":[]");
        if let Some(c) = built_at_commit {
            out.push_str(",\"built_at_commit\":");
            out.push_str(&serde_json::to_string(c).unwrap());
        }
        out.push('}');
        ascii_escape(out)
    }
}

fn push_joined(out: &mut String, items: &[(String, String)]) {
    for (i, (_, s)) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(s);
    }
}

/// `lower(strip_combining(NFKD(label)))` (§5.4).
pub fn norm_label(label: &str) -> String {
    let stripped: String = label.nfkd().filter(|c| !is_combining_mark(*c)).collect();
    stripped.to_lowercase()
}

fn attr_value(a: &Attr) -> Value {
    match a {
        Attr::Str(s) => Value::String(s.clone()),
        Attr::Int(i) => Value::from(*i),
        Attr::Float(f) => Value::from(*f),
        Attr::Bool(b) => Value::Bool(*b),
        Attr::Null => Value::Null,
        Attr::Json(raw) => serde_json::from_str(raw).unwrap_or(Value::Null),
    }
}

fn loc_value(loc: Loc) -> Option<Value> {
    match loc {
        Loc::Line(n) => Some(Value::String(format!("L{n}"))),
        Loc::Empty => Some(Value::String(String::new())),
        Loc::Null => Some(Value::Null),
        Loc::Absent => None,
    }
}

/// Fields in Graphify's key order: `id`, `label`, then the rest sorted.
fn node_map(n: &Node) -> (Vec<(&'static str, Value)>, BTreeMap<String, Value>) {
    let head = vec![
        ("id", Value::String(n.id.clone())),
        ("label", Value::String(n.label.clone())),
    ];
    let mut rest = BTreeMap::new();
    rest.insert("file_type".into(), Value::String(n.file_type.clone()));
    rest.insert("source_file".into(), Value::String(n.source_file.clone()));
    if let Some(v) = loc_value(n.loc) {
        rest.insert("source_location".into(), v);
    }
    rest.insert("norm_label".into(), Value::String(norm_label(&n.label)));
    for (k, v) in &n.attrs {
        rest.insert(k.clone(), attr_value(v));
    }
    (head, rest)
}

/// Fields in Graphify's key order: `source`, `target`, `relation`, then the rest sorted.
fn edge_map(e: &Edge) -> (Vec<(&'static str, Value)>, BTreeMap<String, Value>) {
    let head = vec![
        ("source", Value::String(e.source.clone())),
        ("target", Value::String(e.target.clone())),
        ("relation", Value::String(e.relation.clone())),
    ];
    let mut rest = BTreeMap::new();
    rest.insert(
        "confidence".into(),
        Value::String(e.confidence.as_str().into()),
    );
    let score = e.score.unwrap_or_else(|| e.confidence.default_score());
    rest.insert("confidence_score".into(), Value::from(score));
    if let Some(c) = &e.context {
        rest.insert("context".into(), Value::String(c.clone()));
    }
    rest.insert("source_file".into(), Value::String(e.source_file.clone()));
    if let Some(v) = loc_value(e.loc) {
        rest.insert("source_location".into(), v);
    }
    if let Some(w) = e.weight {
        rest.insert("weight".into(), Value::from(w));
    }
    for (k, v) in &e.attrs {
        rest.insert(k.clone(), attr_value(v));
    }
    (head, rest)
}

/// Returns (sort key, output text). The sort key is Python's
/// `json.dumps(obj, sort_keys=True, ensure_ascii=False)` with default separators (§2.2); the
/// output keeps the head keys first and is compact.
fn encode((head, rest): (Vec<(&'static str, Value)>, BTreeMap<String, Value>)) -> (String, String) {
    let mut sorted: BTreeMap<String, Value> = rest.clone();
    for (k, v) in &head {
        sorted.insert((*k).to_string(), v.clone());
    }
    let key = python_dumps(&Value::Object(sorted.into_iter().collect()));

    let mut map = Map::new();
    for (k, v) in head {
        map.insert(k.to_string(), v);
    }
    for (k, v) in rest {
        map.insert(k, v);
    }
    (key, serde_json::to_string(&Value::Object(map)).unwrap())
}

/// `json.dumps(v, sort_keys=True, ensure_ascii=False)`: `", "` and `": "` separators.
fn python_dumps(v: &Value) -> String {
    let mut out = String::new();
    dump(v, &mut out);
    out
}

fn dump(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&serde_json::to_string(k).unwrap());
                out.push_str(": ");
                dump(&m[*k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                dump(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap()),
    }
}

/// Python's `json.dumps(ensure_ascii=True)`: every non-ASCII character as `\uXXXX`.
fn ascii_escape(s: String) -> String {
    if s.is_ascii() {
        return s;
    }
    let mut out = String::with_capacity(s.len() + s.len() / 8);
    let mut buf = [0u16; 2];
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            for unit in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Confidence, NodeKind};

    fn node(id: &str) -> Node {
        Node {
            id: id.into(),
            label: id.into(),
            kind: NodeKind::Def,
            file_type: "code".into(),
            source_file: "f.py".into(),
            loc: Loc::Line(1),
            attrs: vec![],
        }
    }

    fn edge(s: &str, t: &str, rel: &str, line: u32) -> Edge {
        Edge {
            source: s.into(),
            target: t.into(),
            relation: rel.into(),
            confidence: Confidence::Extracted,
            score: None,
            context: None,
            source_file: "f.py".into(),
            loc: Loc::Line(line),
            weight: Some(1.0),
            attrs: vec![],
        }
    }

    fn one(nodes: &[&str], edges: Vec<Edge>) -> Edge {
        let g = Graph::assemble(nodes.iter().map(|n| node(n)).collect(), edges);
        assert_eq!(g.edges.len(), 1);
        g.edges.into_iter().next().unwrap()
    }

    #[test]
    fn contains_absorbs_a_later_call_as_context() {
        let e = one(
            &["a", "b"],
            vec![edge("a", "b", "contains", 52), edge("a", "b", "calls", 55)],
        );
        assert_eq!(
            (e.relation.as_str(), e.context.as_deref(), e.loc),
            ("contains", Some("call"), Loc::Line(52))
        );
    }

    #[test]
    fn higher_rank_same_direction_merges_keys() {
        let mut r = edge("a", "b", "references", 3);
        r.context = Some("parameter_type".into());
        let mut u = edge("a", "b", "uses", 4);
        u.confidence = Confidence::Inferred;
        u.score = Some(0.95);
        u.weight = Some(0.8);
        let e = one(&["a", "b"], vec![r, u]);
        assert_eq!(e.relation, "uses");
        assert_eq!(e.context.as_deref(), Some("parameter_type"));
        assert_eq!(e.score, Some(0.95));
    }

    #[test]
    fn inferred_then_extracted_call_keeps_the_score() {
        let mut first = edge("a", "b", "calls", 1);
        first.confidence = Confidence::Inferred;
        first.score = Some(0.85);
        let e = one(&["a", "b"], vec![first, edge("a", "b", "calls", 2)]);
        assert_eq!(
            (e.confidence, e.score, e.loc),
            (Confidence::Extracted, Some(0.85), Loc::Line(2))
        );
    }

    #[test]
    fn opposite_direction_tie_keeps_smaller_source() {
        let e = one(
            &["x_alpha", "x_zeta"],
            vec![
                edge("x_zeta", "x_alpha", "calls", 1),
                edge("x_alpha", "x_zeta", "calls", 2),
            ],
        );
        assert_eq!(e.source, "x_alpha");
        let e = one(
            &["a", "b"],
            vec![edge("b", "a", "calls", 1), edge("a", "b", "method", 2)],
        );
        assert_eq!(e.relation, "method");
    }

    #[test]
    fn self_loops_kept_dangling_dropped_first_node_wins() {
        let mut dup = node("a");
        dup.loc = Loc::Line(11);
        let g = Graph::assemble(
            vec![node("a"), dup],
            vec![edge("a", "a", "calls", 1), edge("a", "missing", "calls", 1)],
        );
        assert_eq!(g.nodes.len(), 1);
        assert_eq!(g.nodes[0].loc, Loc::Line(1));
        assert_eq!((g.edges.len(), g.dangling), (1, 1));
    }

    #[test]
    fn json_shape_order_and_escaping() {
        let mut n = node("z");
        n.label = "Über".into();
        n.attrs.push(("_origin".into(), Attr::Str("ast".into())));
        let g = Graph::assemble(vec![n, node("a")], vec![edge("a", "z", "calls", 1)]);
        let json = g.to_graphify_json(None);
        assert!(json.starts_with(
            "{\"directed\":false,\"multigraph\":false,\"graph\":{\"schema_version\":1,\"graphify_version\":\"0.9.86\"},\"nodes\":[{\"id\":\"z\",\"label\":\"\\u00dcber\",\"_origin\":\"ast\""
        ));
        assert!(json.contains("\"norm_label\":\"uber\""));
        assert!(json.contains("{\"source\":\"a\",\"target\":\"z\",\"relation\":\"calls\",\"confidence\":\"EXTRACTED\",\"confidence_score\":1.0,"));
        assert!(json.ends_with("\"hyperedges\":[]}"));
        assert_eq!(norm_label("función…"), "funcion...");
    }
}

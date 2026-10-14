//! The assembled graph and its Graphify-compatible `graph.json` encoding (`docs/graphify-format.md`
//! §2, §8).
//!
//! Graphify stores a simple undirected graph: one link per unordered node pair. Edges are folded in
//! emission order (per-file edges in path order, then the cross-file pass) by the rank rules of
//! §8.2. Direction is kept as written by the surviving edge.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufWriter, Write};

use rayon::prelude::*;
use serde_json::{Map, Value};
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;

use crate::facts::{Attr, Confidence, Edge, Loc, Node, NodeKind};

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
        Graph::assemble_batches(nodes, vec![edges])
    }

    /// `assemble` over edge batches (one per file, then the cross-file passes), folded in order.
    /// A counting pass sizes the result exactly and each batch is freed once folded, so a large
    /// build never holds every raw edge next to a doubling result vector.
    pub fn assemble_batches(nodes: Vec<Node>, batches: Vec<Vec<Edge>>) -> Graph {
        let nodes: Vec<Node> = {
            let mut seen = HashSet::with_capacity(nodes.len());
            let keep: Vec<bool> = nodes.iter().map(|n| seen.insert(n.id.as_str())).collect();
            drop(seen);
            nodes
                .into_iter()
                .zip(keep)
                .filter_map(|(n, k)| k.then_some(n))
                .collect()
        };

        // Fold on integer node indices: no per-edge key strings are allocated.
        let index: HashMap<&str, u32> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i as u32))
            .collect();
        let pair_of = |e: &Edge| -> Option<(u32, u32)> {
            let s = *index.get(e.source.as_str())?;
            let t = *index.get(e.target.as_str())?;
            Some(if s <= t { (s, t) } else { (t, s) })
        };

        let mut slot: HashMap<(u32, u32), u32> = HashMap::new();
        for e in batches.iter().flatten() {
            if let Some(pair) = pair_of(e) {
                let next = slot.len() as u32;
                slot.entry(pair).or_insert(next);
            }
        }
        let mut kept: Vec<Option<Edge>> = Vec::with_capacity(slot.len());
        kept.resize_with(slot.len(), || None);
        let mut dangling = 0;
        for batch in batches {
            for e in batch {
                let Some(pair) = pair_of(&e) else {
                    dangling += 1;
                    continue;
                };
                let i = slot[&pair] as usize;
                match &mut kept[i] {
                    None => kept[i] = Some(e),
                    Some(old) => fold(old, e),
                }
            }
        }
        drop(slot);
        drop(index);
        Graph {
            nodes,
            // Every counted pair received at least one edge; `map` collects in place.
            edges: kept
                .into_iter()
                .map(|e| e.expect("every counted pair has an edge"))
                .collect(),
            dangling,
        }
    }

    /// Clustered-shape `graph.json` (§2.1): compact, ASCII-escaped, no trailing newline, arrays in
    /// Graphify's sort order. `community` holds every node's community (by node index) when the
    /// build clustered; the key is absent otherwise.
    ///
    /// Memory stays bounded on large graphs: only the per-item sort keys are held while sorting,
    /// and items are rendered and written in batches rather than built into one document.
    pub fn write_graphify_json<W: Write>(
        &self,
        w: W,
        built_at_commit: Option<&str>,
        community: Option<&[u32]>,
    ) -> io::Result<()> {
        self.write_graphify_json_ordered(w, built_at_commit, community)
            .map(drop)
    }

    /// [`Graph::write_graphify_json`], returning the order it wrote: node and edge indices in
    /// `graph.json` array order, for exporters that must follow the file.
    pub fn write_graphify_json_ordered<W: Write>(
        &self,
        w: W,
        built_at_commit: Option<&str>,
        community: Option<&[u32]>,
    ) -> io::Result<(Vec<u32>, Vec<u32>)> {
        let comm = |i: usize| community.map(|c| c[i]);
        let node_order = sorted_order(&self.nodes, |i, n| sort_key(node_map(n, comm(i))));
        let link_order = sorted_order(&self.edges, |_, e| sort_key(edge_map(e)));

        let mut w = BufWriter::with_capacity(1 << 20, w);
        w.write_all(b"{\"directed\":false,\"multigraph\":false,\"graph\":{\"schema_version\":")?;
        w.write_all(SCHEMA_VERSION.to_string().as_bytes())?;
        w.write_all(b",\"graphify_version\":\"")?;
        w.write_all(COMPAT_VERSION.as_bytes())?;
        w.write_all(b"\"},\"nodes\":[")?;
        write_items(&mut w, &self.nodes, &node_order, |i, n| {
            compact(node_map(n, comm(i)))
        })?;
        w.write_all(b"],\"links\":[")?;
        write_items(&mut w, &self.edges, &link_order, |_, e| {
            compact(edge_map(e))
        })?;
        w.write_all(b"],\"hyperedges\":[]")?;
        if let Some(c) = built_at_commit {
            w.write_all(b",\"built_at_commit\":")?;
            w.write_all(ascii_escape(serde_json::to_string(c).unwrap()).as_bytes())?;
        }
        w.write_all(b"}")?;
        w.flush()?;
        Ok((node_order, link_order))
    }

    /// Load a clustered-shape `graph.json` (Graphify's or ours) with nodes and links in file order,
    /// as Graphify's `cluster-only` does. Derived fields (`community`, `community_name`,
    /// `norm_label`) are dropped; they are recomputed on output.
    pub fn from_graphify_json(doc: &Value) -> Result<Graph, String> {
        let nodes = doc["nodes"]
            .as_array()
            .ok_or("graph.json has no nodes array")?
            .iter()
            .map(|n| {
                let obj = n.as_object().ok_or("node is not an object")?;
                let s = |k: &str| obj.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let mut node = Node {
                    id: s("id"),
                    label: s("label"),
                    kind: NodeKind::Def,
                    file_type: s("file_type"),
                    source_file: s("source_file"),
                    loc: loc_of(obj.get("source_location")),
                    attrs: Vec::new(),
                };
                for (k, v) in obj {
                    if !matches!(
                        k.as_str(),
                        "id" | "label"
                            | "file_type"
                            | "source_file"
                            | "source_location"
                            | "community"
                            | "community_name"
                            | "norm_label"
                    ) {
                        node.attrs.push((k.clone(), attr_of(v)));
                    }
                }
                Ok(node)
            })
            .collect::<Result<Vec<Node>, String>>()?;
        let links = doc
            .get("links")
            .or_else(|| doc.get("edges"))
            .and_then(Value::as_array)
            .ok_or("graph.json has no links array")?;
        let edges = links
            .iter()
            .map(|e| {
                let obj = e.as_object().ok_or("link is not an object")?;
                let s = |k: &str| obj.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let confidence = match obj.get("confidence").and_then(Value::as_str) {
                    Some("INFERRED") => Confidence::Inferred,
                    Some("AMBIGUOUS") => Confidence::Ambiguous,
                    _ => Confidence::Extracted,
                };
                let mut edge = Edge {
                    source: s("source"),
                    target: s("target"),
                    relation: s("relation"),
                    confidence,
                    score: obj.get("confidence_score").and_then(Value::as_f64),
                    context: obj
                        .get("context")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    source_file: s("source_file"),
                    loc: loc_of(obj.get("source_location")),
                    weight: obj.get("weight").and_then(Value::as_f64),
                    attrs: Vec::new(),
                };
                for (k, v) in obj {
                    if !matches!(
                        k.as_str(),
                        "source"
                            | "target"
                            | "relation"
                            | "confidence"
                            | "confidence_score"
                            | "context"
                            | "source_file"
                            | "source_location"
                            | "weight"
                    ) {
                        edge.attrs.push((k.clone(), attr_of(v)));
                    }
                }
                Ok(edge)
            })
            .collect::<Result<Vec<Edge>, String>>()?;
        let known: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        let (edges, dropped): (Vec<Edge>, Vec<Edge>) = edges
            .into_iter()
            .partition(|e| known.contains(e.source.as_str()) && known.contains(e.target.as_str()));
        drop(known);
        Ok(Graph {
            nodes,
            edges,
            dangling: dropped.len(),
        })
    }

    /// The whole document as a string (tests and small graphs).
    pub fn to_graphify_json(
        &self,
        built_at_commit: Option<&str>,
        community: Option<&[u32]>,
    ) -> String {
        let mut out = Vec::new();
        self.write_graphify_json(&mut out, built_at_commit, community)
            .unwrap();
        String::from_utf8(out).unwrap()
    }
}

/// Indices of `items` ordered by `key`, ties broken by position (keys are unique in practice).
fn sorted_order<T: Sync>(items: &[T], key: impl Fn(usize, &T) -> String + Sync) -> Vec<u32> {
    let mut keyed: Vec<(Box<str>, u32)> = items
        .par_iter()
        .enumerate()
        .map(|(i, x)| (key(i, x).into_boxed_str(), i as u32))
        .collect();
    keyed.par_sort_unstable();
    keyed.into_iter().map(|(_, i)| i).collect()
}

/// Render items in `order`, comma-separated and ASCII-escaped, a bounded batch at a time.
fn write_items<T: Sync, W: Write>(
    w: &mut W,
    items: &[T],
    order: &[u32],
    render: impl Fn(usize, &T) -> String + Sync,
) -> io::Result<()> {
    const BATCH: usize = 1 << 16;
    const CHUNK: usize = 1 << 10;
    for (b, batch) in order.chunks(BATCH).enumerate() {
        let parts: Vec<String> = batch
            .par_chunks(CHUNK)
            .map(|chunk| {
                let mut s = String::new();
                for (i, &idx) in chunk.iter().enumerate() {
                    if i > 0 {
                        s.push(',');
                    }
                    s.push_str(&render(idx as usize, &items[idx as usize]));
                }
                ascii_escape(s)
            })
            .collect();
        for (i, part) in parts.iter().enumerate() {
            if b > 0 || i > 0 {
                w.write_all(b",")?;
            }
            w.write_all(part.as_bytes())?;
        }
    }
    Ok(())
}

/// `lower(strip_combining(NFKD(label)))` (§5.4): marks with a non-zero canonical combining class are
/// dropped, like Python's `unicodedata.combining`; spacing vowel signs (class 0) stay.
pub fn norm_label(label: &str) -> String {
    let stripped: String = label
        .nfkd()
        .filter(|&c| canonical_combining_class(c) == 0)
        .collect();
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

fn loc_of(v: Option<&Value>) -> Loc {
    match v {
        None => Loc::Absent,
        Some(Value::Null) => Loc::Null,
        Some(Value::String(s)) if s.is_empty() => Loc::Empty,
        Some(Value::String(s)) => s
            .strip_prefix('L')
            .and_then(|n| n.parse().ok())
            .map_or(Loc::Empty, Loc::Line),
        Some(_) => Loc::Empty,
    }
}

fn attr_of(v: &Value) -> Attr {
    match v {
        Value::String(s) => Attr::Str(s.clone()),
        Value::Bool(b) => Attr::Bool(*b),
        Value::Null => Attr::Null,
        Value::Number(n) => match n.as_i64() {
            Some(i) if !n.is_f64() => Attr::Int(i),
            _ => Attr::Float(n.as_f64().unwrap_or(0.0)),
        },
        other => Attr::Json(other.to_string()),
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
fn node_map(n: &Node, community: Option<u32>) -> Fields {
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
    if let Some(c) = community {
        rest.insert("community".into(), Value::from(c));
    }
    for (k, v) in &n.attrs {
        rest.insert(k.clone(), attr_value(v));
    }
    (head, rest)
}

/// Fields in Graphify's key order: `source`, `target`, `relation`, then the rest sorted.
fn edge_map(e: &Edge) -> Fields {
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

type Fields = (Vec<(&'static str, Value)>, BTreeMap<String, Value>);

/// Python's `json.dumps(obj, sort_keys=True, ensure_ascii=False)` with default separators: the
/// string Graphify sorts arrays by (§2.2).
fn sort_key((head, rest): Fields) -> String {
    let mut sorted: Map<String, Value> = rest.into_iter().collect();
    for (k, v) in head {
        sorted.insert(k.to_string(), v);
    }
    python_dumps(&Value::Object(sorted))
}

/// The compact output object: head keys first, then the rest sorted (§2.3). Not yet escaped.
fn compact((head, rest): Fields) -> String {
    let mut map = Map::new();
    for (k, v) in head {
        map.insert(k.to_string(), v);
    }
    for (k, v) in rest {
        map.insert(k, v);
    }
    serde_json::to_string(&Value::Object(map)).unwrap()
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
        let json = g.to_graphify_json(None, None);
        assert!(json.starts_with(
            "{\"directed\":false,\"multigraph\":false,\"graph\":{\"schema_version\":1,\"graphify_version\":\"0.9.86\"},\"nodes\":[{\"id\":\"z\",\"label\":\"\\u00dcber\",\"_origin\":\"ast\""
        ));
        assert!(json.contains("\"norm_label\":\"uber\""));
        assert!(json.contains("{\"source\":\"a\",\"target\":\"z\",\"relation\":\"calls\",\"confidence\":\"EXTRACTED\",\"confidence_score\":1.0,"));
        assert!(json.ends_with("\"hyperedges\":[]}"));
        assert_eq!(norm_label("función…"), "funcion...");
    }
}

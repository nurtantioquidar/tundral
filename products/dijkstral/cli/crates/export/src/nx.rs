//! A `graph.json` as NetworkX holds it after Graphify loads it (`docs/graphify-exports.md` §9),
//! with every attribute kept raw: the loader of the export-only formats (Cypher, Obsidian, SVG,
//! call-flow, tree).
//!
//! - The graph kind follows `directed` (default false) and `multigraph` (default **true**, as in
//!   NetworkX's `node_link_graph`): Graphify's own `graph.json` says false to both, while an
//!   extraction fragment (`nodes` + `edges`, no flags) loads as an undirected multigraph.
//! - Nodes in file order; a node id repeated in `nodes` merges its attributes into the first.
//! - Links in file order. In a simple graph a repeated pair (ordered when directed) keeps its
//!   first position, merges the attributes of every link (later keys win) and takes the
//!   direction of the last link; in a multigraph every link is its own edge.
//! - A link endpoint missing from `nodes` becomes a node without attributes, appended in the order
//!   the links name them (source before target).
//! - Adjacency lists neighbours in the order of the links that introduced them, a pair's
//!   parallel edges together; a self-loop appears once per edge and counts twice in the degree.
//!   For a directed graph `adj` holds every incident edge and `G.edges()` walks successors.
//!
//! The file is streamed into compact attribute lists (interned keys, scalar values unboxed)
//! rather than a `serde_json::Value` tree, which takes about three times the memory.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::view::Names;

/// One attribute value. Numbers keep whether they were written as integers or floats.
#[derive(Debug, Clone, PartialEq)]
pub enum Attr {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Box<str>),
    /// Lists, objects and integers beyond `i64`.
    Json(Box<Value>),
}

impl Attr {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Attr::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Attr::Int(i) => Some(*i),
            Attr::Json(v) => v.as_i64(),
            _ => None,
        }
    }

    /// Any number as a float (as `Value::as_f64`).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Attr::Int(i) => Some(*i as f64),
            Attr::Float(f) => Some(*f),
            Attr::Json(v) => v.as_f64(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Attr::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Attr::Null)
    }

    /// The value as JSON.
    pub fn to_value(&self) -> Value {
        match self {
            Attr::Null => Value::Null,
            Attr::Bool(b) => Value::Bool(*b),
            Attr::Int(i) => Value::from(*i),
            Attr::Float(f) => serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number),
            Attr::Str(s) => Value::String(s.to_string()),
            Attr::Json(v) => (**v).clone(),
        }
    }

    /// The value's JSON text (`null`, `1.5`, `"s"`, …).
    pub fn json_text(&self) -> String {
        self.to_value().to_string()
    }
}

impl<'de> Deserialize<'de> for Attr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Attr, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Attr;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_bool<E>(self, b: bool) -> Result<Attr, E> {
                Ok(Attr::Bool(b))
            }
            fn visit_i64<E>(self, i: i64) -> Result<Attr, E> {
                Ok(Attr::Int(i))
            }
            fn visit_u64<E>(self, u: u64) -> Result<Attr, E> {
                Ok(i64::try_from(u)
                    .map_or_else(|_| Attr::Json(Box::new(Value::from(u))), Attr::Int))
            }
            fn visit_f64<E>(self, f: f64) -> Result<Attr, E> {
                Ok(Attr::Float(f))
            }
            fn visit_str<E>(self, s: &str) -> Result<Attr, E> {
                Ok(Attr::Str(s.into()))
            }
            fn visit_string<E>(self, s: String) -> Result<Attr, E> {
                Ok(Attr::Str(s.into_boxed_str()))
            }
            fn visit_unit<E>(self) -> Result<Attr, E> {
                Ok(Attr::Null)
            }
            fn visit_none<E>(self) -> Result<Attr, E> {
                Ok(Attr::Null)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Attr, A::Error> {
                let v = Value::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
                Ok(Attr::Json(Box::new(v)))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Attr, A::Error> {
                let v = Value::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(Attr::Json(Box::new(v)))
            }
        }
        d.deserialize_any(V)
    }
}

/// A node's or edge's attributes in file order; keys are shared between items.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attrs(Vec<(Arc<str>, Attr)>);

impl Attrs {
    pub fn get(&self, key: &str) -> Option<&Attr> {
        self.0.iter().find(|(k, _)| &**k == key).map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Attr)> {
        self.0.iter().map(|(k, v)| (&**k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| &**k)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Set `key`, keeping an existing key's position (a Python dict update).
    pub fn insert(&mut self, key: Arc<str>, value: Attr) {
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key, value)),
        }
    }

    pub fn extend(&mut self, other: Attrs) {
        for (k, v) in other.0 {
            self.insert(k, v);
        }
    }

    /// The attributes as a JSON object.
    pub fn to_map(&self) -> Map<String, Value> {
        self.iter()
            .map(|(k, v)| (k.to_string(), v.to_value()))
            .collect()
    }

    fn remove(&mut self, key: &str) -> Option<Attr> {
        let i = self.0.iter().position(|(k, _)| &**k == key)?;
        Some(self.0.remove(i).1)
    }
}

pub struct NxNode {
    pub id: String,
    /// Every field but `id`, in file order.
    pub attrs: Attrs,
}

pub struct NxEdge {
    /// The link's direction (in a simple graph, the last link's for the pair).
    pub source: u32,
    pub target: u32,
    /// Every field but `source` and `target` (merged in link order in a simple graph).
    pub attrs: Attrs,
}

pub struct NxGraph {
    pub nodes: Vec<NxNode>,
    pub edges: Vec<NxEdge>,
    /// Per node, `(neighbour, edge)` in NetworkX adjacency order, every incident edge.
    pub adj: Vec<Vec<(u32, u32)>>,
    /// Per node, `(successor, edge)`: directed graphs only, empty otherwise.
    pub succ: Vec<Vec<(u32, u32)>>,
    pub directed: bool,
    pub multigraph: bool,
    /// `graph.json`'s `graph` object.
    pub graph_attrs: Map<String, Value>,
    pub index: HashMap<String, u32>,
}

/// A JSON id as text: strings as they are, anything else as its JSON form.
fn id_text(v: Attr) -> String {
    match v {
        Attr::Str(s) => s.into_string(),
        other => other.json_text(),
    }
}

// ---------------------------------------------------------------------------------------------
// Streaming parse

#[derive(Default)]
struct Interner(HashMap<Box<str>, Arc<str>>);

impl Interner {
    fn get(&mut self, k: &str) -> Arc<str> {
        if let Some(a) = self.0.get(k) {
            return a.clone();
        }
        let a: Arc<str> = Arc::from(k);
        self.0.insert(k.into(), a.clone());
        a
    }
}

/// One JSON object into `Attrs` with interned keys.
struct ObjectSeed<'a>(&'a mut Interner);

impl<'de> DeserializeSeed<'de> for ObjectSeed<'_> {
    type Value = Attrs;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Attrs, D::Error> {
        d.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for ObjectSeed<'_> {
    type Value = Attrs;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Attrs, A::Error> {
        let mut out = Attrs(Vec::with_capacity(map.size_hint().unwrap_or(8)));
        while let Some(k) = map.next_key::<std::borrow::Cow<'de, str>>()? {
            let v: Attr = map.next_value()?;
            out.insert(self.0.get(&k), v);
        }
        Ok(out)
    }
}

/// A JSON array of objects.
struct ArraySeed<'a>(&'a mut Interner);

impl<'de> DeserializeSeed<'de> for ArraySeed<'_> {
    type Value = Vec<Attrs>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Vec<Attrs>, D::Error> {
        d.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for ArraySeed<'_> {
    type Value = Vec<Attrs>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an array of objects")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<Attrs>, A::Error> {
        let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(o) = seq.next_element_seed(ObjectSeed(self.0))? {
            out.push(o);
        }
        Ok(out)
    }
}

#[derive(Default)]
struct Doc {
    directed: Option<bool>,
    multigraph: Option<bool>,
    graph: Map<String, Value>,
    nodes: Option<Vec<Attrs>>,
    links: Option<Vec<Attrs>>,
    edges: Option<Vec<Attrs>>,
}

struct DocVisitor;

impl<'de> Visitor<'de> for DocVisitor {
    type Value = Doc;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a graph.json object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Doc, A::Error> {
        let mut doc = Doc::default();
        let mut keys = Interner::default();
        while let Some(k) = map.next_key::<std::borrow::Cow<'de, str>>()? {
            match &*k {
                "directed" => doc.directed = map.next_value::<Value>()?.as_bool(),
                "multigraph" => doc.multigraph = map.next_value::<Value>()?.as_bool(),
                "graph" => {
                    if let Value::Object(m) = map.next_value::<Value>()? {
                        doc.graph = m;
                    }
                }
                "nodes" => doc.nodes = Some(map.next_value_seed(ArraySeed(&mut keys))?),
                "links" => doc.links = Some(map.next_value_seed(ArraySeed(&mut keys))?),
                "edges" => doc.edges = Some(map.next_value_seed(ArraySeed(&mut keys))?),
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(doc)
    }
}

impl<'de> Deserialize<'de> for Doc {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Doc, D::Error> {
        d.deserialize_map(DocVisitor)
    }
}

/// Neighbour lists with parallel edges grouped under the neighbour's first appearance.
struct Grouped {
    lists: Vec<Vec<(u32, Vec<u32>)>>,
    slots: Vec<HashMap<u32, usize>>,
}

impl Grouped {
    fn new(n: usize) -> Grouped {
        Grouped {
            lists: vec![Vec::new(); n],
            slots: vec![HashMap::new(); n],
        }
    }

    fn add(&mut self, u: u32, v: u32, e: u32) {
        let (list, slot) = (&mut self.lists[u as usize], &mut self.slots[u as usize]);
        match slot.get(&v) {
            Some(&k) => list[k].1.push(e),
            None => {
                slot.insert(v, list.len());
                list.push((v, vec![e]));
            }
        }
    }

    fn flatten(self) -> Vec<Vec<(u32, u32)>> {
        self.lists
            .into_iter()
            .map(|l| {
                l.into_iter()
                    .flat_map(|(v, es)| es.into_iter().map(move |e| (v, e)))
                    .collect()
            })
            .collect()
    }
}

impl NxGraph {
    pub fn read(path: &Path) -> Result<NxGraph> {
        let file = fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
        let mut de = serde_json::Deserializer::from_reader(BufReader::with_capacity(1 << 20, file));
        let doc = Doc::deserialize(&mut de)
            .and_then(|doc| de.end().map(|()| doc))
            .with_context(|| format!("parsing {}", path.display()))?;
        NxGraph::from_doc(doc)
    }

    pub fn from_json(doc: Value) -> Result<NxGraph> {
        if !doc.is_object() {
            bail!("graph.json is not a JSON object");
        }
        NxGraph::from_doc(Doc::deserialize(doc)?)
    }

    fn from_doc(doc: Doc) -> Result<NxGraph> {
        let (directed, multigraph) = (
            doc.directed.unwrap_or(false),
            doc.multigraph.unwrap_or(true),
        );
        let Some(raw_nodes) = doc.nodes else {
            bail!("graph.json has no nodes array");
        };
        let Some(raw_links) = doc.links.or(doc.edges) else {
            bail!("graph.json has no links array");
        };
        let mut g = NxGraph {
            nodes: Vec::with_capacity(raw_nodes.len()),
            edges: Vec::with_capacity(raw_links.len()),
            adj: Vec::new(),
            succ: Vec::new(),
            directed,
            multigraph,
            graph_attrs: doc.graph,
            index: HashMap::with_capacity(raw_nodes.len()),
        };
        for mut attrs in raw_nodes {
            let id = attrs.remove("id").map(id_text).unwrap_or_default();
            match g.index.get(&id) {
                Some(&i) => g.nodes[i as usize].attrs.extend(attrs),
                None => {
                    g.add_node(id, attrs);
                }
            }
        }
        let mut pair: HashMap<(u32, u32), u32> = HashMap::new();
        for mut attrs in raw_links {
            let s = attrs.remove("source").map(id_text).unwrap_or_default();
            let t = attrs.remove("target").map(id_text).unwrap_or_default();
            let s = g.node_or_add(s);
            let t = g.node_or_add(t);
            if !multigraph {
                let key = if directed {
                    (s, t)
                } else {
                    (s.min(t), s.max(t))
                };
                if let Some(&e) = pair.get(&key) {
                    let edge = &mut g.edges[e as usize];
                    edge.source = s;
                    edge.target = t;
                    edge.attrs.extend(attrs);
                    continue;
                }
                pair.insert(key, g.edges.len() as u32);
            }
            attrs.0.shrink_to_fit();
            g.edges.push(NxEdge {
                source: s,
                target: t,
                attrs,
            });
        }
        drop(pair);
        let n = g.nodes.len();
        let mut all = Grouped::new(n);
        let mut out = Grouped::new(if directed { n } else { 0 });
        for (e, edge) in g.edges.iter().enumerate() {
            let (s, t, e) = (edge.source, edge.target, e as u32);
            all.add(s, t, e);
            if s != t {
                all.add(t, s, e);
            }
            if directed {
                out.add(s, t, e);
            }
        }
        g.adj = all.flatten();
        g.succ = out.flatten();
        Ok(g)
    }

    fn add_node(&mut self, id: String, mut attrs: Attrs) -> u32 {
        let i = self.nodes.len() as u32;
        attrs.0.shrink_to_fit();
        self.index.insert(id.clone(), i);
        self.nodes.push(NxNode { id, attrs });
        i
    }

    fn node_or_add(&mut self, id: String) -> u32 {
        match self.index.get(&id) {
            Some(&i) => i,
            None => self.add_node(id, Attrs::default()),
        }
    }

    /// NetworkX degree: a self-loop counts twice.
    pub fn degree(&self, i: u32) -> usize {
        self.adj[i as usize]
            .iter()
            .map(|&(v, _)| if v == i { 2 } else { 1 })
            .sum()
    }

    /// A node's string attribute, `None` when absent or not a string.
    pub fn node_str(&self, i: u32, key: &str) -> Option<&str> {
        self.nodes[i as usize].attrs.get(key).and_then(Attr::as_str)
    }

    /// An edge's string attribute, `None` when absent or not a string.
    pub fn edge_str(&self, e: u32, key: &str) -> Option<&str> {
        self.edges[e as usize].attrs.get(key).and_then(Attr::as_str)
    }

    /// The node's `label`, else its id (`data.get("label", id)`).
    pub fn label(&self, i: u32) -> &str {
        self.node_str(i, "label")
            .unwrap_or(&self.nodes[i as usize].id)
    }

    /// The node's integer `community`, `None` when absent or not an integer.
    pub fn community(&self, i: u32) -> Option<i64> {
        self.nodes[i as usize]
            .attrs
            .get("community")
            .and_then(Attr::as_i64)
    }

    /// `G.edges()`: `(u, v, edge)` with `u` the endpoint met first in node order, a pair's
    /// parallel edges in order. For a directed graph, each node's successors in order.
    pub fn edge_walk(&self) -> Vec<(u32, u32, u32)> {
        if self.directed {
            return (0..self.nodes.len() as u32)
                .flat_map(|u| self.succ[u as usize].iter().map(move |&(v, e)| (u, v, e)))
                .collect();
        }
        let mut seen = vec![false; self.nodes.len()];
        let mut out = Vec::with_capacity(self.edges.len());
        for u in 0..self.nodes.len() as u32 {
            for &(v, e) in &self.adj[u as usize] {
                if !seen[v as usize] {
                    out.push((u, v, e));
                }
            }
            seen[u as usize] = true;
        }
        out
    }

    /// Community ids in the order their first member appears, each with its members in node
    /// order.
    pub fn communities(&self) -> Vec<(i64, Vec<u32>)> {
        let mut slot: HashMap<i64, usize> = HashMap::new();
        let mut out: Vec<(i64, Vec<u32>)> = Vec::new();
        for i in 0..self.nodes.len() as u32 {
            if let Some(c) = self.community(i) {
                let k = *slot.entry(c).or_insert_with(|| {
                    out.push((c, Vec::new()));
                    out.len() - 1
                });
                out[k].1.push(i);
            }
        }
        out
    }

    /// dijkstral's default community names when no labels file exists: each community's
    /// best-connected member (ties: the smaller id) without a trailing `()`, as in
    /// `GRAPH_REPORT.md` and `View::hub_names`.
    pub fn hub_names(&self) -> Names {
        let mut best: HashMap<i64, u32> = HashMap::new();
        for i in 0..self.nodes.len() as u32 {
            let Some(c) = self.community(i) else {
                continue;
            };
            let slot = best.entry(c).or_insert(i);
            let (d, bd) = (self.degree(i), self.degree(*slot));
            if d > bd || (d == bd && self.nodes[i as usize].id < self.nodes[*slot as usize].id) {
                *slot = i;
            }
        }
        Names::Named(
            best.into_iter()
                .filter_map(|(c, i)| {
                    let label = self.label(i);
                    let name = label.strip_suffix("()").unwrap_or(label).to_string();
                    Some((u32::try_from(c).ok()?, name))
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loads_like_networkx() {
        let g = NxGraph::from_json(json!({
            "directed": false, "multigraph": false,
            "nodes": [{"id": "p", "label": "P"}, {"id": "q"}, {"id": "p", "x": 1}],
            "links": [
                {"source": "p", "target": "q", "relation": "first", "confidence": "INFERRED"},
                {"source": "q", "target": "p", "relation": "second"},
                {"source": "q", "target": "q", "relation": "self"},
                {"source": "ghost2", "target": "ghost1"},
            ],
        }))
        .unwrap();
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["p", "q", "ghost2", "ghost1"]);
        assert_eq!(g.nodes[0].attrs.get("x"), Some(&Attr::Int(1)));
        assert_eq!(g.edges.len(), 3);
        let e = &g.edges[0];
        assert_eq!((e.source, e.target), (1, 0));
        assert_eq!(
            e.attrs.get("relation").and_then(Attr::as_str),
            Some("second")
        );
        assert_eq!(
            e.attrs.get("confidence").and_then(Attr::as_str),
            Some("INFERRED")
        );
        assert_eq!(g.degree(1), 3);
        assert_eq!(g.label(1), "q");
        let walk: Vec<(u32, u32)> = g.edge_walk().iter().map(|&(u, v, _)| (u, v)).collect();
        assert_eq!(walk, [(0, 1), (1, 1), (2, 3)]);
    }

    #[test]
    fn attributes_keep_order_and_types() {
        let text = r#"{"links": [{"r": 1, "source": "x", "s": 2.0, "target": "x", "t": null}],
            "nodes": [{"a": [1, {"k": "v"}], "id": "x", "b": 18446744073709551615, "c": "s",
                       "a": true}],
            "unknown": {"deep": [1, 2]}}"#;
        let tmp = std::env::temp_dir().join(format!("nx-test-{}.json", std::process::id()));
        fs::write(&tmp, text).unwrap();
        for g in [
            NxGraph::read(&tmp).unwrap(),
            NxGraph::from_json(serde_json::from_str(text).unwrap()).unwrap(),
        ] {
            let keys = |a: &Attrs| a.keys().map(str::to_string).collect::<Vec<_>>();
            // A repeated key keeps its first position and takes the last value.
            assert_eq!(keys(&g.nodes[0].attrs), ["a", "b", "c"]);
            assert_eq!(g.nodes[0].attrs.get("a"), Some(&Attr::Bool(true)));
            assert_eq!(
                g.nodes[0].attrs.get("b").unwrap().json_text(),
                "18446744073709551615"
            );
            assert_eq!(keys(&g.edges[0].attrs), ["r", "s", "t"]);
            assert_eq!(g.edges[0].attrs.get("s"), Some(&Attr::Float(2.0)));
            assert_eq!(g.edges[0].attrs.get("s").unwrap().json_text(), "2.0");
            assert!(g.edges[0].attrs.get("t").unwrap().is_null());
            assert!(g.multigraph && !g.directed);
        }
        fs::remove_file(&tmp).ok();
    }

    fn rel_walk(g: &NxGraph) -> Vec<String> {
        g.edge_walk()
            .iter()
            .map(|&(_, _, e)| {
                let edge = &g.edges[e as usize];
                format!(
                    "{}>{}:{}",
                    g.nodes[edge.source as usize].id,
                    g.nodes[edge.target as usize].id,
                    g.edge_str(e, "relation").unwrap_or("")
                )
            })
            .collect()
    }

    /// The four graph kinds, each against what Graphify's `cypher.txt` lists for these links.
    #[test]
    fn graph_kinds_follow_the_flags() {
        let links = json!([
            {"source": "b", "target": "a", "relation": "r1"},
            {"source": "a", "target": "b", "relation": "r2"},
            {"source": "c", "target": "a", "relation": "r3"},
            {"source": "a", "target": "b", "relation": "r4"},
        ]);
        let nodes = json!([{"id": "a"}, {"id": "b"}, {"id": "c"}]);
        let multi = NxGraph::from_json(json!({"nodes": nodes, "edges": links})).unwrap();
        assert_eq!(rel_walk(&multi), ["b>a:r1", "a>b:r2", "a>b:r4", "c>a:r3"]);
        assert_eq!(multi.degree(0), 4);
        let di =
            NxGraph::from_json(json!({"directed": true, "nodes": nodes, "links": links})).unwrap();
        assert_eq!(rel_walk(&di), ["a>b:r2", "a>b:r4", "b>a:r1", "c>a:r3"]);
        let simple_di = NxGraph::from_json(
            json!({"directed": true, "multigraph": false, "nodes": nodes, "links": links}),
        )
        .unwrap();
        assert_eq!(rel_walk(&simple_di), ["a>b:r4", "b>a:r1", "c>a:r3"]);
        let simple = NxGraph::from_json(
            json!({"directed": false, "multigraph": false, "nodes": nodes, "links": links}),
        )
        .unwrap();
        assert_eq!(rel_walk(&simple), ["a>b:r4", "c>a:r3"]);
    }
}

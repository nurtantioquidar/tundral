//! NetworkX node-link graphs as Graphify's cross-repo commands read and write them
//! (`docs/graphify-multirepo.md` §2): insertion-ordered nodes and adjacency, attribute updates
//! that keep a key's first position, and NetworkX's edge iteration order. Any `graph.json` shape
//! loads (clustered `links`, raw `edges`, directed or multigraph); [`NxGraph`] is the undirected
//! simple graph the merged and global graphs are built in.

use std::collections::HashMap;

use anyhow::{bail, Result};
use serde_json::{Map, Value};

use crate::pyjson::{float_repr, write_str};

/// Attributes in insertion order (`serde_json` is built with `preserve_order`).
pub type Attrs = Map<String, Value>;

/// The hash key Python would use for a node id: strings and other JSON values never collide.
pub fn node_key(id: &Value) -> String {
    match id {
        Value::String(s) => format!("s{s}"),
        other => format!("j{other}"),
    }
}

/// Python's `str()` of a node id, as Graphify writes it into namespaced ids and `local_id`.
pub fn py_str(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) if n.is_f64() => float_repr(n.as_f64().unwrap_or(0.0)),
        other => other.to_string(),
    }
}

/// Python truthiness of a JSON value (a missing value is falsy).
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// An integer community as Python's `isinstance(c, int)` sees it (`true`/`false` count as 1/0).
pub fn int_community(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) if !n.is_f64() => n.as_i64(),
        _ => None,
    }
}

/// One graph file as NetworkX loads it: nodes in order and edges in `G.edges()` order, each
/// with the direction its last writer gave it.
#[derive(Debug, Default)]
pub struct InputGraph {
    pub directed: bool,
    pub multigraph: bool,
    /// The `graph` object, without `hyperedges`.
    pub graph: Attrs,
    /// `graph.hyperedges` when that key exists, else the top-level `hyperedges`.
    pub hyperedges: Vec<Value>,
    pub nodes: Vec<(Value, Attrs)>,
    /// `(source, target, attributes)` by node position.
    pub edges: Vec<(usize, usize, Attrs)>,
}

impl InputGraph {
    /// NetworkX's `number_of_edges()`: one per pair (or per key in a multigraph), self-loops
    /// included.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Whether node `i` has a truthy `external` attribute (shared across repositories).
    pub fn is_external(&self, i: usize) -> bool {
        truthy(self.nodes[i].1.get("external"))
    }

    /// Load a node-link document. Endpoints missing from `nodes` become attribute-less nodes; a
    /// node without `id` is keyed by its array position; `_src`/`_tgt` on a link, when they name
    /// its two endpoints, give its direction and are dropped from its attributes.
    pub fn load(doc: &Value) -> Result<InputGraph> {
        let Some(obj) = doc.as_object() else {
            bail!("not a graph: the top level is not a JSON object");
        };
        let Some(nodes) = obj.get("nodes").and_then(Value::as_array) else {
            bail!("not a graph: no `nodes` array");
        };
        let directed = truthy(obj.get("directed"));
        let multigraph = truthy(obj.get("multigraph"));
        let mut graph = obj
            .get("graph")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let hyperedges = match graph.shift_remove("hyperedges") {
            Some(h) => h,
            None => obj.get("hyperedges").cloned().unwrap_or(Value::Null),
        };
        let hyperedges = match hyperedges {
            Value::Array(a) => a,
            _ => Vec::new(),
        };
        let mut g = NxGraph::with_kind(directed, multigraph);
        for (i, n) in nodes.iter().enumerate() {
            let Some(n) = n.as_object() else {
                bail!("not a graph: node {i} is not an object");
            };
            let id = match n.get("id") {
                Some(id) => id.clone(),
                None => Value::from(i),
            };
            let mut attrs = n.clone();
            attrs.shift_remove("id");
            g.add_node(&id, attrs);
        }
        let links = obj
            .get("links")
            .or_else(|| obj.get("edges"))
            .and_then(Value::as_array);
        for (i, l) in links.into_iter().flatten().enumerate() {
            let Some(l) = l.as_object() else {
                bail!("not a graph: link {i} is not an object");
            };
            let (Some(s), Some(t)) = (l.get("source"), l.get("target")) else {
                bail!("not a graph: link {i} has no source or target");
            };
            let mut attrs = l.clone();
            attrs.shift_remove("source");
            attrs.shift_remove("target");
            let key = if multigraph {
                attrs.shift_remove("key")
            } else {
                None
            };
            let src = attrs.shift_remove("_src");
            let tgt = attrs.shift_remove("_tgt");
            let (s, t) = match (src, tgt) {
                (Some(a), Some(b))
                    if (node_key(&a) == node_key(s) && node_key(&b) == node_key(t))
                        || (node_key(&a) == node_key(t) && node_key(&b) == node_key(s)) =>
                {
                    (a, b)
                }
                _ => (s.clone(), t.clone()),
            };
            let u = g.add_node(&s, Attrs::new());
            let v = g.add_node(&t, Attrs::new());
            g.add_edge_keyed(u, v, key, attrs);
        }
        let order: HashMap<usize, usize> = g.live().enumerate().map(|(i, s)| (s, i)).collect();
        let edges = g
            .edge_order()
            .into_iter()
            .map(|e| {
                let e = &g.edges[e];
                (order[&e.src], order[&e.tgt], e.attrs.clone())
            })
            .collect();
        let nodes = g
            .live()
            .map(|s| (g.slots[s].id.clone(), g.slots[s].attrs.clone()))
            .collect();
        Ok(InputGraph {
            directed,
            multigraph,
            graph,
            hyperedges,
            nodes,
            edges,
        })
    }
}

#[derive(Debug, Clone)]
struct Slot {
    id: Value,
    attrs: Attrs,
    alive: bool,
}

/// An edge record: its endpoints in the direction last written, and its attributes.
#[derive(Debug, Clone)]
pub struct EdgeData {
    pub src: usize,
    pub tgt: usize,
    pub attrs: Attrs,
}

/// A NetworkX graph with node removal. Node slots are never reused: a removed node that is added
/// again goes to the end, as in a Python dict.
#[derive(Debug, Default)]
pub struct NxGraph {
    directed: bool,
    multigraph: bool,
    pub graph: Attrs,
    slots: Vec<Slot>,
    by_key: HashMap<String, usize>,
    /// Neighbours (successors when directed) in first-insertion order.
    adj: Vec<Vec<usize>>,
    /// Pair → keyed edges, in key insertion order. Undirected pairs are stored low slot first.
    pairs: HashMap<(usize, usize), Vec<(String, usize)>>,
    edges: Vec<EdgeData>,
}

impl NxGraph {
    /// An empty undirected simple graph (`nx.Graph`).
    pub fn new() -> NxGraph {
        NxGraph::default()
    }

    fn with_kind(directed: bool, multigraph: bool) -> NxGraph {
        NxGraph {
            directed,
            multigraph,
            ..NxGraph::default()
        }
    }

    /// Load a document into an undirected simple graph exactly as written: links in file order,
    /// every attribute kept (`_src`/`_tgt` included). Used for the global graph.
    pub fn load_simple(doc: &Value) -> Result<NxGraph> {
        let Some(obj) = doc.as_object() else {
            bail!("not a graph: the top level is not a JSON object");
        };
        let mut g = NxGraph::new();
        g.graph = obj
            .get("graph")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (i, n) in obj
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(n) = n.as_object() else {
                bail!("not a graph: node {i} is not an object");
            };
            let id = n.get("id").cloned().unwrap_or_else(|| Value::from(i));
            let mut attrs = n.clone();
            attrs.shift_remove("id");
            g.add_node(&id, attrs);
        }
        let links = obj
            .get("links")
            .or_else(|| obj.get("edges"))
            .and_then(Value::as_array);
        for (i, l) in links.into_iter().flatten().enumerate() {
            let Some(l) = l.as_object() else {
                bail!("not a graph: link {i} is not an object");
            };
            let (Some(s), Some(t)) = (l.get("source"), l.get("target")) else {
                bail!("not a graph: link {i} has no source or target");
            };
            let mut attrs = l.clone();
            attrs.shift_remove("source");
            attrs.shift_remove("target");
            let u = g.add_node(&s.clone(), Attrs::new());
            let v = g.add_node(&t.clone(), Attrs::new());
            g.add_edge(u, v, attrs);
        }
        Ok(g)
    }

    /// The slot of a live node.
    pub fn slot(&self, id: &Value) -> Option<usize> {
        self.by_key.get(&node_key(id)).copied()
    }

    /// `G.add_node(id, **attrs)`: a new node goes last; an existing one has its attributes
    /// updated in place. Returns the slot.
    pub fn add_node(&mut self, id: &Value, attrs: Attrs) -> usize {
        match self.by_key.get(&node_key(id)) {
            Some(&s) => {
                let target = &mut self.slots[s].attrs;
                for (k, v) in attrs {
                    target.insert(k, v);
                }
                s
            }
            None => {
                let s = self.slots.len();
                self.slots.push(Slot {
                    id: id.clone(),
                    attrs,
                    alive: true,
                });
                self.adj.push(Vec::new());
                self.by_key.insert(node_key(id), s);
                s
            }
        }
    }

    pub fn id(&self, slot: usize) -> &Value {
        &self.slots[slot].id
    }

    pub fn attrs(&self, slot: usize) -> &Attrs {
        &self.slots[slot].attrs
    }

    pub fn attrs_mut(&mut self, slot: usize) -> &mut Attrs {
        &mut self.slots[slot].attrs
    }

    /// Live node slots in insertion order.
    pub fn live(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.slots.len()).filter(|&s| self.slots[s].alive)
    }

    pub fn node_count(&self) -> usize {
        self.live().count()
    }

    pub fn edge_count(&self) -> usize {
        self.edge_order().len()
    }

    fn pair(&self, u: usize, v: usize) -> (usize, usize) {
        if self.directed || u <= v {
            (u, v)
        } else {
            (v, u)
        }
    }

    /// `G.add_edge(u, v, **attrs)` on a simple graph: an existing edge has its attributes
    /// updated and takes this direction.
    pub fn add_edge(&mut self, u: usize, v: usize, attrs: Attrs) {
        self.add_edge_keyed(u, v, None, attrs);
    }

    fn add_edge_keyed(&mut self, u: usize, v: usize, key: Option<Value>, attrs: Attrs) {
        let pair = self.pair(u, v);
        if !self.pairs.contains_key(&pair) {
            self.adj[u].push(v);
            if !self.directed && u != v {
                self.adj[v].push(u);
            }
        }
        let keys = self.pairs.entry(pair).or_default();
        let key = if !self.multigraph {
            String::new()
        } else {
            match key {
                Some(k) if !k.is_null() => node_key(&k),
                // NetworkX's new_edge_key: the key count, raised past any key in use.
                _ => {
                    let mut n = keys.len();
                    while keys.iter().any(|(k, _)| *k == node_key(&Value::from(n))) {
                        n += 1;
                    }
                    node_key(&Value::from(n))
                }
            }
        };
        match keys.iter().find(|(k, _)| *k == key) {
            Some(&(_, e)) => {
                let edge = &mut self.edges[e];
                edge.src = u;
                edge.tgt = v;
                for (k, val) in attrs {
                    edge.attrs.insert(k, val);
                }
            }
            None => {
                keys.push((key, self.edges.len()));
                self.edges.push(EdgeData {
                    src: u,
                    tgt: v,
                    attrs,
                });
            }
        }
    }

    /// `G.remove_node(n)`: the node and its edges.
    pub fn remove_node(&mut self, slot: usize) {
        if !self.slots[slot].alive {
            return;
        }
        self.slots[slot].alive = false;
        let key = node_key(&self.slots[slot].id);
        self.by_key.remove(&key);
        let nbrs = std::mem::take(&mut self.adj[slot]);
        for n in nbrs {
            let p = self.pair(slot, n);
            self.pairs.remove(&p);
        }
        if self.directed {
            self.pairs.retain(|&(_, t), _| t != slot);
        }
    }

    /// Edge records in `G.edges()` order, with the node each is reached from.
    fn ordered(&self) -> Vec<(usize, usize, usize)> {
        let mut seen = vec![false; self.slots.len()];
        let mut out = Vec::new();
        for n in self.live() {
            for &m in &self.adj[n] {
                if !self.slots[m].alive || (!self.directed && seen[m]) {
                    continue;
                }
                if let Some(keys) = self.pairs.get(&self.pair(n, m)) {
                    out.extend(keys.iter().map(|&(_, e)| (n, m, e)));
                }
            }
            seen[n] = true;
        }
        out
    }

    fn edge_order(&self) -> Vec<usize> {
        self.ordered().into_iter().map(|(_, _, e)| e).collect()
    }

    /// Edges in `G.edges()` order as `(n, nbr, edge)`: `n` is the node the iteration reached the
    /// edge from, not necessarily its direction.
    pub fn iter_edges(&self) -> impl Iterator<Item = (usize, usize, &EdgeData)> + '_ {
        self.ordered()
            .into_iter()
            .map(move |(n, m, e)| (n, m, &self.edges[e]))
    }

    /// The node object NetworkX's `node_link_data` writes: attributes, then `id`.
    pub fn node_json(&self, slot: usize) -> Value {
        let mut m = self.slots[slot].attrs.clone();
        m.insert("id".into(), self.slots[slot].id.clone());
        Value::Object(m)
    }
}

/// Compact `json.dumps(v, separators=(",", ":"), ensure_ascii=True)`: Graphify's `graph.json`
/// encoding. No trailing newline.
pub fn dumps_compact(v: &Value) -> String {
    let mut out = String::new();
    dump(v, &mut out);
    out
}

fn dump(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            _ if n.is_f64() => out.push_str(&float_repr(n.as_f64().unwrap_or(0.0))),
            (Some(i), _) => out.push_str(&i.to_string()),
            (_, Some(u)) => out.push_str(&u.to_string()),
            _ => out.push_str(&n.to_string()),
        },
        Value::String(s) => write_str(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                dump(x, out);
            }
            out.push(']');
        }
        Value::Object(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(k, out);
                out.push(':');
                dump(x, out);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rels(g: &InputGraph) -> Vec<String> {
        g.edges
            .iter()
            .map(|(s, t, a)| {
                format!(
                    "{}-{}-{}",
                    py_str(&g.nodes[*s].0),
                    a["relation"].as_str().unwrap(),
                    py_str(&g.nodes[*t].0)
                )
            })
            .collect()
    }

    #[test]
    fn undirected_pairs_collapse_last_writer_wins() {
        let g = InputGraph::load(&json!({
            "nodes": [{"id": "a"}, {"id": "b"}],
            "links": [
                {"source": "a", "target": "b", "relation": "r1", "only": 1, "shared": "first"},
                {"source": "b", "target": "a", "relation": "r2", "shared": "second"}
            ]
        }))
        .unwrap();
        assert_eq!(rels(&g), ["b-r2-a"]);
        assert_eq!(
            dumps_compact(&Value::Object(g.edges[0].2.clone())),
            r#"{"relation":"r2","only":1,"shared":"second"}"#
        );
    }

    #[test]
    fn directed_and_multigraph_iterate_like_networkx() {
        let d = InputGraph::load(&json!({
            "directed": true,
            "nodes": [{"id": "a"}, {"id": "b"}],
            "links": [
                {"source": "b", "target": "a", "relation": "r2"},
                {"source": "a", "target": "b", "relation": "r1"}
            ]
        }))
        .unwrap();
        assert_eq!(rels(&d), ["a-r1-b", "b-r2-a"]);
        let m = InputGraph::load(&json!({
            "multigraph": true,
            "nodes": [{"id": "a"}, {"id": "b"}],
            "links": [
                {"source": "a", "target": "b", "relation": "r1", "key": 0},
                {"source": "b", "target": "a", "relation": "r2"}
            ]
        }))
        .unwrap();
        assert_eq!(rels(&m), ["a-r1-b", "b-r2-a"]);
        assert!(!m.edges[0].2.contains_key("key"));
    }

    #[test]
    fn missing_endpoints_and_ids_become_nodes() {
        let g = InputGraph::load(&json!({
            "nodes": [{"id": "a"}, {"label": "no id"}],
            "edges": [{"source": "a", "target": "ghost", "relation": "r"}]
        }))
        .unwrap();
        let ids: Vec<String> = g.nodes.iter().map(|(id, _)| py_str(id)).collect();
        assert_eq!(ids, ["a", "1", "ghost"]);
    }

    #[test]
    fn src_tgt_give_the_direction() {
        let g = InputGraph::load(&json!({
            "nodes": [{"id": "b"}, {"id": "a"}],
            "links": [{"relation": "calls", "_src": "a", "_tgt": "b", "source": "b", "target": "a"}]
        }))
        .unwrap();
        assert_eq!(rels(&g), ["a-calls-b"]);
        assert_eq!(g.edges[0].2.len(), 1);
    }

    #[test]
    fn removed_nodes_drop_their_edges_and_return_last() {
        let mut g = NxGraph::new();
        let a = g.add_node(&json!("a"), Attrs::new());
        let b = g.add_node(&json!("b"), Attrs::new());
        let c = g.add_node(&json!("c"), Attrs::new());
        g.add_edge(a, b, Attrs::new());
        g.add_edge(c, b, Attrs::new());
        g.remove_node(a);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        let a2 = g.add_node(&json!("a"), Attrs::new());
        let order: Vec<String> = g.live().map(|s| py_str(g.id(s))).collect();
        assert_eq!(order, ["b", "c", "a"]);
        assert!(a2 > c);
    }

    #[test]
    fn compact_dump_matches_python() {
        let v = json!({"a": [1, 1.0, 0.85, "Café—", null, true], "b": {}});
        assert_eq!(
            dumps_compact(&v),
            r#"{"a":[1,1.0,0.85,"Caf\u00e9\u2014",null,true],"b":{}}"#
        );
    }
}

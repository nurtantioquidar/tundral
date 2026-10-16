//! The graph as Graphify's exporters see it (`docs/graphify-exports.md` §1): nodes and links in
//! `graph.json` order, NetworkX degrees and adjacency, and a community and name per node.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use dijkstral_core::facts::Attr;
use dijkstral_core::graph::Graph;
use dijkstral_core::pipeline::Built;
use serde_json::Value;

/// Graphify's community-labels sidecar, read from next to the graph when present.
pub const LABELS_FILE: &str = ".graphify_labels.json";

/// The link attributes holding an edge's true direction, written by the global graph, whose
/// `source`/`target` follow NetworkX iteration order (`docs/graphify-multirepo.md` §5.2). Every
/// exporter but GraphML draws an edge from `_src` to `_tgt`, each taken separately when it is a
/// string naming a node, else the link's own `source` or `target` (`docs/graphify-exports.md` §1).
pub const TRUE_SOURCE: &str = "_src";
pub const TRUE_TARGET: &str = "_tgt";

/// A `graph.json` loaded for export, in file order.
pub struct Loaded {
    pub graph: Graph,
    /// Each node's `community`, `None` when the key is absent.
    pub community: Vec<Option<u32>>,
    /// The top-level `hyperedges` array as JSON text (`[]` when absent).
    pub hyperedges: String,
    /// Edges whose link has no `confidence` (the graph reads them as `EXTRACTED`).
    pub no_confidence: HashSet<u32>,
}

impl Loaded {
    pub fn from_json(doc: &Value) -> Result<Loaded> {
        let graph = Graph::from_graphify_json(doc).map_err(anyhow::Error::msg)?;
        // The graph keeps the links whose endpoints exist, in file order.
        let ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
        let links = doc
            .get("links")
            .or_else(|| doc.get("edges"))
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let mut no_confidence = HashSet::new();
        let kept = links.iter().filter(|l| {
            let end = |k: &str| l.get(k).and_then(Value::as_str).unwrap_or("");
            ids.contains(end("source")) && ids.contains(end("target"))
        });
        for (k, l) in kept.enumerate() {
            if l.get("confidence").and_then(Value::as_str).is_none() {
                no_confidence.insert(k as u32);
            }
        }
        drop(ids);
        let community = doc["nodes"]
            .as_array()
            .map(|nodes| {
                nodes
                    .iter()
                    .map(|n| n.get("community").and_then(Value::as_u64).map(|c| c as u32))
                    .collect()
            })
            .unwrap_or_default();
        let hyperedges = doc
            .get("hyperedges")
            .map_or_else(|| "[]".to_string(), Value::to_string);
        Ok(Loaded {
            graph,
            community,
            hyperedges,
            no_confidence,
        })
    }

    pub fn read(path: &Path) -> Result<(Loaded, Value)> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let doc: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok((Loaded::from_json(&doc)?, doc))
    }

    pub fn view(&self, names: Names) -> View<'_> {
        let n = self.graph.nodes.len() as u32;
        let m = self.graph.edges.len() as u32;
        let mut view = View::new(
            &self.graph,
            (0..n).collect(),
            (0..m).collect(),
            self.community.clone(),
            names,
            self.hyperedges.clone(),
        );
        view.no_confidence = self.no_confidence.clone();
        view
    }
}

/// Community names.
#[derive(Debug, Clone, Default)]
pub enum Names {
    /// `Community N`, as Graphify writes without a labels file.
    #[default]
    Placeholder,
    /// Names by community id; ids without one fall back to `Community N`.
    Named(HashMap<u32, String>),
}

impl Names {
    /// Read a Graphify labels file: `{"0": "name", …}`.
    pub fn read(path: &Path) -> Result<Names> {
        let doc: Value = serde_json::from_slice(
            &fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("parsing {}", path.display()))?;
        let Some(obj) = doc.as_object() else {
            bail!("{} is not a JSON object", path.display());
        };
        Ok(Names::Named(
            obj.iter()
                .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_string())))
                .collect(),
        ))
    }

    pub fn from_vec(labels: &[String]) -> Names {
        Names::Named(
            labels
                .iter()
                .enumerate()
                .map(|(i, l)| (i as u32, l.clone()))
                .collect(),
        )
    }

    pub fn is_named(&self) -> bool {
        matches!(self, Names::Named(_))
    }

    pub fn get(&self, community: u32) -> String {
        match self {
            Names::Named(m) => m
                .get(&community)
                .cloned()
                .unwrap_or_else(|| format!("Community {community}")),
            Names::Placeholder => format!("Community {community}"),
        }
    }
}

/// Everything the exporters read. Node and edge numbers are indices into `graph`; `nodes` and
/// `edges` give `graph.json` order.
pub struct View<'a> {
    pub graph: &'a Graph,
    pub nodes: Vec<u32>,
    pub edges: Vec<u32>,
    pub community: Vec<Option<u32>>,
    pub names: Names,
    pub hyperedges: String,
    /// Endpoints of every edge as the link stores them: `(source, target)`.
    pub ends: Vec<(u32, u32)>,
    /// The direction `graph.html` draws every edge in (`docs/graphify-exports.md` §1, link
    /// direction): `(from, to)`.
    pub dir: Vec<(u32, u32)>,
    /// NetworkX degree: a self-loop counts twice.
    pub degree: Vec<u32>,
    /// Per node, `(neighbour, edge)` in the order NetworkX's adjacency holds them after loading
    /// `graph.json` (links in file order); a self-loop appears once.
    pub adj: Vec<Vec<(u32, u32)>>,
    /// Edges whose link carried no `confidence` (only for a loaded `graph.json`).
    pub no_confidence: HashSet<u32>,
}

impl<'a> View<'a> {
    pub fn new(
        graph: &'a Graph,
        nodes: Vec<u32>,
        edges: Vec<u32>,
        community: Vec<Option<u32>>,
        names: Names,
        hyperedges: String,
    ) -> View<'a> {
        let index: HashMap<&str, u32> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i as u32))
            .collect();
        let ends: Vec<(u32, u32)> = graph
            .edges
            .iter()
            .map(|e| (index[e.source.as_str()], index[e.target.as_str()]))
            .collect();
        let dir = graph
            .edges
            .iter()
            .zip(&ends)
            .map(|(e, &(s, t))| {
                let end = |key: &str, stored: u32| {
                    e.attrs
                        .iter()
                        .find(|(k, _)| k == key)
                        .and_then(|(_, v)| match v {
                            Attr::Str(id) => index.get(id.as_str()).copied(),
                            _ => None,
                        })
                        .unwrap_or(stored)
                };
                (end(TRUE_SOURCE, s), end(TRUE_TARGET, t))
            })
            .collect();
        drop(index);
        let mut degree = vec![0u32; graph.nodes.len()];
        let mut adj: Vec<Vec<(u32, u32)>> = vec![Vec::new(); graph.nodes.len()];
        for &e in &edges {
            let (s, t) = ends[e as usize];
            degree[s as usize] += 1;
            degree[t as usize] += 1;
            adj[s as usize].push((t, e));
            if s != t {
                adj[t as usize].push((s, e));
            }
        }
        View {
            graph,
            nodes,
            edges,
            community,
            names,
            hyperedges,
            ends,
            dir,
            degree,
            adj,
            no_confidence: HashSet::new(),
        }
    }

    /// The build's graph, in the order `graph.json` was written, with the report's community
    /// names; nodes of an unclustered build have no community.
    pub fn of_build(built: &Built<'a>) -> View<'a> {
        let (community, names) = match built.communities {
            Some((c, labels)) => (
                c.of_node.iter().map(|&c| Some(c)).collect(),
                Names::from_vec(labels),
            ),
            None => (vec![None; built.graph.nodes.len()], Names::Placeholder),
        };
        View::new(
            built.graph,
            built.node_order.to_vec(),
            built.edge_order.to_vec(),
            community,
            names,
            Value::Array(built.graph.hyperedges.clone()).to_string(),
        )
    }

    pub fn has_communities(&self) -> bool {
        self.community.iter().any(Option::is_some)
    }

    /// `G.edges()`: `(u, v, edge)` with `u` the endpoint met first in node order.
    pub fn edge_walk(&self) -> Vec<(u32, u32, u32)> {
        let mut seen = vec![false; self.graph.nodes.len()];
        let mut out = Vec::with_capacity(self.edges.len());
        for &u in &self.nodes {
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
    /// order (the dict Graphify builds while reading the nodes).
    pub fn communities(&self) -> Vec<(u32, Vec<u32>)> {
        let mut slot: HashMap<u32, usize> = HashMap::new();
        let mut out: Vec<(u32, Vec<u32>)> = Vec::new();
        for &i in &self.nodes {
            if let Some(c) = self.community[i as usize] {
                let k = *slot.entry(c).or_insert_with(|| {
                    out.push((c, Vec::new()));
                    out.len() - 1
                });
                out[k].1.push(i);
            }
        }
        out
    }

    /// The names `graph.html` and the wiki use when the caller gives none and no labels file
    /// exists: each community's best-connected member (ties: the smaller id), without a trailing
    /// `()` — the names of `GRAPH_REPORT.md`.
    pub fn hub_names(&self) -> Names {
        let mut best: HashMap<u32, u32> = HashMap::new();
        for &i in &self.nodes {
            let Some(c) = self.community[i as usize] else {
                continue;
            };
            let slot = best.entry(c).or_insert(i);
            let (d, bd) = (self.degree[i as usize], self.degree[*slot as usize]);
            let id = &self.graph.nodes[i as usize].id;
            if d > bd || (d == bd && id < &self.graph.nodes[*slot as usize].id) {
                *slot = i;
            }
        }
        Names::Named(
            best.into_iter()
                .map(|(c, i)| {
                    let label = &self.graph.nodes[i as usize].label;
                    (c, label.strip_suffix("()").unwrap_or(label).to_string())
                })
                .collect(),
        )
    }
}

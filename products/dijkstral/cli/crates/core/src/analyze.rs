//! Graph analysis after clustering (`docs/graphify-format.md` §17): degree, cohesion, god nodes,
//! surprising connections, betweenness, isolated nodes, import cycles and suggested questions.
//!
//! Graphify iterates a NetworkX graph whose nodes and adjacency follow build order. `Adjacency`
//! reproduces that order (nodes as in `Graph::nodes`, each node's neighbours in the order its
//! links first appeared), because ties in every ranking below are broken by it.

use std::collections::HashSet;

use rayon::prelude::*;

use crate::cluster::{cohesion_of, internal_edges, labels, non_self, Communities, Topology};
use crate::facts::{Confidence, Node};
use crate::graph::Graph;
use crate::pyjson::{self, Json};

/// Sidecar file names, as Graphify writes them next to `graph.json`.
pub const ANALYSIS_FILE: &str = ".graphify_analysis.json";
pub const REPORT_FILE: &str = "GRAPH_REPORT.md";

const GOD_COUNT: usize = 10;
const SURPRISE_COUNT: usize = 5;
/// Labels too generic to be an architectural hub (found by probing `graphify god-nodes`).
const NOISE_LABELS: &[&str] = &[
    "Any",
    "AsyncMock",
    "Bool",
    "Callable",
    "ClassVar",
    "Color",
    "Combine",
    "Comparable",
    "Counter",
    "Data",
    "Date",
    "Dict",
    "Double",
    "Enum",
    "Equatable",
    "Error",
    "False",
    "Final",
    "Float",
    "Font",
    "Foundation",
    "Hashable",
    "Identifiable",
    "Int",
    "List",
    "Literal",
    "MagicMock",
    "Mock",
    "Optional",
    "OrderedDict",
    "Path",
    "Protocol",
    "Sendable",
    "Set",
    "String",
    "True",
    "Tuple",
    "Type",
    "URL",
    "UUID",
    "Union",
    "View",
    "abc",
    "bool",
    "bytearray",
    "bytes",
    "complex",
    "datetime",
    "defaultdict",
    "float",
    "int",
    "io",
    "json",
    "object",
    "os",
    "patch",
    "re",
    "sentinel",
    "str",
    "sys",
    "typing",
];

/// Keys too generic to be hubs in JSON files, compared case-insensitively (probed the same way).
const JSON_NOISE_LABELS: &[&str] = &[
    "bundleddependencies",
    "bundledependencies",
    "data",
    "dependencies",
    "description",
    "devdependencies",
    "end",
    "id",
    "items",
    "key",
    "name",
    "optionaldependencies",
    "peerdependencies",
    "properties",
    "start",
    "title",
    "type",
    "value",
    "version",
];

/// Python's `PurePath(path).suffix != ""`.
pub fn has_suffix(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) => i > 0 && i < name.len() - 1,
        None => false,
    }
}

/// A file's own node: its label is a `/`-suffix of its source file (§5.1).
pub fn is_file_node(n: &Node) -> bool {
    is_file_label(&n.label, &n.source_file)
}

/// `label` is a `/`-suffix of `source_file`.
pub fn is_file_label(label: &str, sf: &str) -> bool {
    !sf.is_empty()
        && (sf == label
            || (sf.len() > label.len()
                && sf.ends_with(label)
                && sf.as_bytes()[sf.len() - label.len() - 1] == b'/'))
}

/// A symbol defined in a real source file: not a file node, stub, external or extensionless
/// script.
pub fn is_symbol(n: &Node) -> bool {
    has_suffix(&n.source_file) && !is_file_node(n)
}

pub fn is_method_label(label: &str) -> bool {
    label.starts_with('.') && label.ends_with("()")
}

/// A function (`…()`) with at most one link: never a god node, surprise endpoint or bridge.
pub fn is_minor_function(label: &str, degree: u32) -> bool {
    label.ends_with("()") && degree <= 1
}

/// Not a file node, not a method and not a minor function: what every ranking requires. The
/// community-bridge fallback of the surprises asks nothing more (stubs, rationale and files
/// without a suffix qualify there).
fn is_ranked_parts(label: &str, source_file: &str, degree: u32) -> bool {
    !is_file_label(label, source_file)
        && !is_method_label(label)
        && !is_minor_function(label, degree)
}

/// Eligible as a surprise endpoint or a bridge (§17.4): a ranked node in a file with a suffix.
/// Noise labels (`String`), concepts and rationale qualify; only god nodes exclude them.
pub fn is_hub_candidate(n: &Node, degree: u32) -> bool {
    has_suffix(&n.source_file) && is_ranked_parts(&n.label, &n.source_file, degree)
}

/// A symbol whose label is not in the noise list, on its label and source file.
pub fn is_hub_candidate_parts(label: &str, source_file: &str) -> bool {
    has_suffix(source_file)
        && !is_file_label(label, source_file)
        && !is_method_label(label)
        && !NOISE_LABELS.contains(&label)
}

/// [`is_hub_candidate_parts`] that is not a generic JSON key either. JSON keys are compared
/// stripped and case-insensitively (`" name"` is excluded too).
pub fn is_god_candidate_parts(label: &str, source_file: &str) -> bool {
    is_hub_candidate_parts(label, source_file)
        && !(source_file.to_ascii_lowercase().ends_with(".json")
            && JSON_NOISE_LABELS.contains(&label.trim().to_lowercase().as_str()))
}

/// Eligible as a god node: [`is_god_candidate_parts`] and not a minor function. The report, the
/// wiki, `god-nodes` and the MCP server share this rule.
pub fn is_god_node(label: &str, source_file: &str, degree: u32) -> bool {
    is_god_candidate_parts(label, source_file) && !is_minor_function(label, degree)
}

/// The NetworkX graph Graphify analyses: nodes in build (or loaded) order, links added sorted by
/// `(source id, target id)` as written, so each node's neighbours follow that sort (§17.4).
pub struct Adjacency {
    /// Per node: `(neighbour, link index)` in insertion order; a self-loop appears once.
    pub neighbors: Vec<Vec<(u32, u32)>>,
    /// NetworkX degree: a self-loop counts twice.
    pub degree: Vec<u32>,
}

impl Adjacency {
    pub fn of(topo: &Topology) -> Adjacency {
        let mut neighbors: Vec<Vec<(u32, u32)>> = vec![Vec::new(); topo.n];
        let mut degree = vec![0u32; topo.n];
        // `topo` keeps each link's stored direction; `rank` orders nodes by id.
        let mut order: Vec<u32> = (0..topo.edges.len() as u32).collect();
        order.sort_by_key(|&i| {
            let (s, t, _) = topo.edges[i as usize];
            (topo.rank[s as usize], topo.rank[t as usize])
        });
        for i in order {
            let (s, t, _) = topo.edges[i as usize];
            neighbors[s as usize].push((t, i));
            degree[s as usize] += 1;
            degree[t as usize] += 1;
            if s != t {
                neighbors[t as usize].push((s, i));
            }
        }
        Adjacency { neighbors, degree }
    }

    /// Link indices in `G.edges()` order.
    pub fn edge_order(&self) -> Vec<u32> {
        self.edges_uv().into_iter().map(|(e, _, _)| e).collect()
    }

    /// `G.edges()` as `(link, u, v)`, `u` being the node whose neighbours yielded the link.
    pub fn edges_uv(&self) -> Vec<(u32, u32, u32)> {
        let mut seen = vec![false; self.neighbors.len()];
        let mut out = Vec::new();
        for (u, list) in self.neighbors.iter().enumerate() {
            for &(v, e) in list {
                if !seen[v as usize] {
                    out.push((e, u as u32, v));
                }
            }
            seen[u] = true;
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Surprise {
    pub edge: u32,
    pub score: u32,
    /// Why the link is surprising; empty for a community-bridge fallback, which has `note`.
    pub why: String,
    /// `Bridges community a → community b` when no cross-file link qualified (§17.4).
    pub note: Option<String>,
}

/// Everything `.graphify_analysis.json` holds, plus what the report reuses.
pub struct Analysis {
    pub adjacency: Adjacency,
    pub cohesion: Vec<f64>,
    pub labels: Vec<String>,
    pub gods: Vec<u32>,
    pub surprises: Vec<Surprise>,
    /// Links inside each community.
    pub internal: Vec<usize>,
    /// Model usage of the command writing the sidecar: `(input, output)` tokens (§8.2 of
    /// `docs/graphify-semantic.md`; 0/0 without a semantic pass or when it was fully cached).
    pub tokens: (u64, u64),
}

pub fn analyze(graph: &Graph, topo: &Topology, communities: &Communities) -> Analysis {
    let adjacency = Adjacency::of(topo);
    let internal_lists = internal_edges(topo, &communities.of_node, communities.members.len());
    let internal: Vec<usize> = internal_lists.iter().map(|l| non_self(topo, l)).collect();
    let cohesion = communities
        .members
        .iter()
        .zip(&internal)
        .map(|(m, &i)| cohesion_of(m.len(), i))
        .collect();
    let labels = labels(graph, communities, &adjacency.degree);
    let gods = god_nodes(graph, &adjacency, GOD_COUNT);
    let surprises = surprises(graph, topo, &adjacency, communities);
    Analysis {
        adjacency,
        cohesion,
        labels,
        gods,
        surprises,
        internal,
        tokens: (0, 0),
    }
}

/// The `n` best-connected god-node candidates, ties in build order.
pub fn god_nodes(graph: &Graph, adj: &Adjacency, n: usize) -> Vec<u32> {
    let mut cands: Vec<u32> = (0..graph.nodes.len() as u32)
        .filter(|&i| {
            let node = &graph.nodes[i as usize];
            is_god_node(&node.label, &node.source_file, adj.degree[i as usize])
        })
        .collect();
    cands.sort_by_key(|&i| std::cmp::Reverse(adj.degree[i as usize]));
    cands.truncate(n);
    cands
}

fn top_dir(path: &str) -> &str {
    path.split_once('/').map_or("", |(d, _)| d)
}

/// What a link between two kinds of file adds to a surprise's score
/// (`docs/graphify-semantic.md` §8.2).
const CROSS_TYPE_SCORE: u32 = 2;

/// A source file's kind for surprises: `paper` (PDF), `image`, `doc` (the documents the semantic
/// pass reads, office files included), otherwise `code`.
pub fn file_category(source_file: &str) -> &'static str {
    let name = source_file.rsplit('/').next().unwrap_or(source_file);
    let ext = name
        .rfind('.')
        .filter(|&i| i > 0)
        .map_or(String::new(), |i| name[i + 1..].to_ascii_lowercase());
    match ext.as_str() {
        "pdf" => "paper",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => "image",
        "md" | "mdx" | "rst" | "txt" | "html" | "yaml" | "yml" | "docx" | "xlsx" => "doc",
        _ => "code",
    }
}

/// Relations never listed as surprising: structure and imports.
fn unsurprising(relation: &str) -> bool {
    matches!(relation, "contains" | "method" | "imports" | "imports_from")
}

fn confidence_score(c: Confidence) -> u32 {
    match c {
        Confidence::Extracted => 1,
        Confidence::Inferred => 2,
        Confidence::Ambiguous => 3,
    }
}

/// Links between hub candidates in different files, other than structure and imports, scored by
/// how unexpected they are: confidence (EXTRACTED 1, INFERRED 2, AMBIGUOUS 3), +2 across top-level
/// directories, +1 across communities, +1 when a node with at most 2 links reaches one with at
/// least 5. Best five, ties in `G.edges()` order. Without any, the community-bridge fallback.
fn surprises(
    graph: &Graph,
    topo: &Topology,
    adj: &Adjacency,
    communities: &Communities,
) -> Vec<Surprise> {
    let order = adj.edges_uv();
    let mut cands: Vec<Surprise> = Vec::new();
    for &(e, _, _) in &order {
        let link = &graph.edges[e as usize];
        if unsurprising(&link.relation) {
            continue;
        }
        let (s, t, _) = topo.edges[e as usize];
        let (u, v) = (&graph.nodes[s as usize], &graph.nodes[t as usize]);
        let (ds, dt) = (adj.degree[s as usize], adj.degree[t as usize]);
        if !is_hub_candidate(u, ds) || !is_hub_candidate(v, dt) || u.source_file == v.source_file {
            continue;
        }
        let mut score = confidence_score(link.confidence);
        let mut why: Vec<String> = Vec::new();
        match link.confidence {
            Confidence::Extracted => {}
            Confidence::Inferred => {
                why.push("inferred connection - not explicitly stated in source".into());
            }
            Confidence::Ambiguous => {
                why.push("ambiguous connection - not explicitly stated in source".into());
            }
        }
        let (cu, cv) = (file_category(&u.source_file), file_category(&v.source_file));
        if cu != cv {
            score += CROSS_TYPE_SCORE;
            // Graphify prints the pair in Python set order, which varies from run to run; the
            // sorted order is one of the two it prints.
            let (a, b) = if cu <= cv { (cu, cv) } else { (cv, cu) };
            why.push(format!("crosses file types ({a} \u{2194} {b})"));
        }
        if top_dir(&u.source_file) != top_dir(&v.source_file) {
            score += 2;
            why.push("connects across different repos/directories".into());
        }
        if communities.of_node[s as usize] != communities.of_node[t as usize] {
            score += 1;
            why.push("bridges separate communities".into());
        }
        if ds.min(dt) <= 2 && ds.max(dt) >= 5 {
            score += 1;
            let (p, h) = if ds <= dt { (u, v) } else { (v, u) };
            why.push(format!(
                "peripheral node `{}` unexpectedly reaches hub `{}`",
                p.label, h.label
            ));
        }
        let why = if why.is_empty() {
            "cross-file semantic connection".to_string()
        } else {
            why.join("; ")
        };
        cands.push(Surprise {
            edge: e,
            score,
            why,
            note: None,
        });
    }
    if cands.is_empty() {
        return community_bridges(graph, adj, communities, &order);
    }
    cands.sort_by_key(|c| std::cmp::Reverse(c.score));
    cands.truncate(SURPRISE_COUNT);
    cands
}

/// The fallback when no cross-file link qualifies: links between communities, other than
/// structure and imports, whose endpoints are ranked nodes (stubs, rationale and files without a
/// suffix qualify); by confidence, then `G.edges()` order; the first for each pair of
/// communities, at most five. The note names the communities in `G.edges()`'s `(u, v)` order.
fn community_bridges(
    graph: &Graph,
    adj: &Adjacency,
    communities: &Communities,
    order: &[(u32, u32, u32)],
) -> Vec<Surprise> {
    let comm = |i: u32| communities.of_node[i as usize];
    let ranked = |i: u32| {
        let n = &graph.nodes[i as usize];
        is_ranked_parts(&n.label, &n.source_file, adj.degree[i as usize])
    };
    let mut cands: Vec<(u32, u32, u32, u32)> = order
        .iter()
        .filter(|&&(e, u, v)| {
            !unsurprising(&graph.edges[e as usize].relation)
                && comm(u) != comm(v)
                && ranked(u)
                && ranked(v)
        })
        .map(|&(e, u, v)| {
            let score = confidence_score(graph.edges[e as usize].confidence);
            (score, e, comm(u), comm(v))
        })
        .collect();
    cands.sort_by_key(|c| std::cmp::Reverse(c.0));
    let mut pairs = HashSet::new();
    cands
        .into_iter()
        .filter(|&(_, _, a, b)| pairs.insert((a.min(b), a.max(b))))
        .take(SURPRISE_COUNT)
        .map(|(score, edge, a, b)| Surprise {
            edge,
            score,
            why: String::new(),
            note: Some(format!("Bridges community {a} \u{2192} community {b}")),
        })
        .collect()
}

impl Analysis {
    /// `.graphify_analysis.json` as Graphify writes it after `extract`: `json.dump(indent=2)`,
    /// ASCII-escaped, no trailing newline.
    pub fn to_json(&self, graph: &Graph, communities: &Communities) -> String {
        self.to_json_with(graph, communities, None)
    }

    /// With `questions`, the `cluster-only` shape: `questions` (type, question, why) replaces
    /// `tokens`.
    pub fn to_json_with(
        &self,
        graph: &Graph,
        communities: &Communities,
        questions: Option<Vec<(&str, Option<&str>, &str)>>,
    ) -> String {
        let node = |i: u32| &graph.nodes[i as usize];
        let comms = Json::Object(
            communities
                .members
                .iter()
                .enumerate()
                .map(|(c, m)| {
                    (
                        c.to_string(),
                        Json::Array(m.iter().map(|&i| Json::str(&node(i).id)).collect()),
                    )
                })
                .collect(),
        );
        let cohesion = Json::Object(
            self.cohesion
                .iter()
                .enumerate()
                .map(|(c, &v)| (c.to_string(), Json::Float(v)))
                .collect(),
        );
        let gods = Json::Array(
            self.gods
                .iter()
                .map(|&i| {
                    Json::Object(vec![
                        ("id".into(), Json::str(&node(i).id)),
                        ("label".into(), Json::str(&node(i).label)),
                        (
                            "degree".into(),
                            Json::Int(self.adjacency.degree[i as usize] as i64),
                        ),
                    ])
                })
                .collect(),
        );
        let surprises = Json::Array(
            self.surprises
                .iter()
                .map(|s| {
                    let e = &graph.edges[s.edge as usize];
                    let (u, v) = (node_by_id(graph, &e.source), node_by_id(graph, &e.target));
                    let tail = match &s.note {
                        Some(note) => ("note".to_string(), Json::str(note)),
                        None => ("why".to_string(), Json::str(&s.why)),
                    };
                    Json::Object(vec![
                        ("source".into(), Json::str(&u.label)),
                        ("target".into(), Json::str(&v.label)),
                        (
                            "source_files".into(),
                            Json::Array(vec![Json::str(&u.source_file), Json::str(&v.source_file)]),
                        ),
                        ("confidence".into(), Json::str(e.confidence.as_str())),
                        ("relation".into(), Json::str(&e.relation)),
                        tail,
                    ])
                })
                .collect(),
        );
        // `extract` escapes non-ASCII text, `cluster-only` writes it as UTF-8.
        let dumps = if questions.is_some() {
            pyjson::dumps_indent_utf8
        } else {
            pyjson::dumps_indent
        };
        let tail = match questions {
            None => (
                "tokens".to_string(),
                Json::Object(vec![
                    ("input".into(), Json::Int(self.tokens.0 as i64)),
                    ("output".into(), Json::Int(self.tokens.1 as i64)),
                ]),
            ),
            Some(qs) => (
                "questions".to_string(),
                Json::Array(
                    qs.into_iter()
                        .map(|(kind, q, why)| {
                            let mut fields = vec![
                                ("type".into(), Json::str(kind)),
                                ("question".into(), q.map_or(Json::Null, Json::str)),
                                ("why".into(), Json::str(why)),
                            ];
                            if kind == "ambiguous_edge" {
                                fields.push(("low_confidence".into(), Json::Bool(true)));
                            }
                            Json::Object(fields)
                        })
                        .collect(),
                ),
            ),
        };
        dumps(&Json::Object(vec![
            ("communities".into(), comms),
            ("cohesion".into(), cohesion),
            ("gods".into(), gods),
            ("surprises".into(), surprises),
            tail,
        ]))
    }
}

fn node_by_id<'a>(graph: &'a Graph, id: &str) -> &'a Node {
    graph
        .nodes
        .iter()
        .find(|n| n.id == id)
        .expect("link endpoint exists")
}

// ---------------------------------------------------------------------------------------------
// Betweenness

/// Graphify's betweenness: NetworkX `betweenness_centrality` (normalized, unweighted), exact up to
/// 1,000 nodes, otherwise sampled from `k = 100` sources drawn by `random.Random(42).sample` over
/// the nodes in build order.
pub fn betweenness(adj: &Adjacency) -> Vec<f64> {
    let n = adj.neighbors.len();
    let (sources, sampled): (Vec<u32>, bool) = if n > 1000 {
        (PyRandom::new(42).sample(n, 100), true)
    } else {
        ((0..n as u32).collect(), false)
    };
    let mut bc = vec![0.0f64; n];
    // Sources are processed in parallel batches but summed in sample order, so the result does not
    // depend on the thread count.
    const BATCH: usize = 16;
    for chunk in sources.chunks(BATCH) {
        let deltas: Vec<Vec<(u32, f64)>> =
            chunk.par_iter().map(|&s| single_source(adj, s)).collect();
        for d in deltas {
            for (v, x) in d {
                bc[v as usize] += x;
            }
        }
    }
    // NetworkX `_rescale` with endpoints=False.
    let nn = n.saturating_sub(1) as f64;
    if n < 3 {
        return bc;
    }
    if sampled {
        let k = sources.len() as f64;
        let src: HashSet<u32> = sources.iter().copied().collect();
        let (s_scale, o_scale) = (1.0 / ((k - 1.0) * (nn - 1.0)), 1.0 / (k * (nn - 1.0)));
        for (v, x) in bc.iter_mut().enumerate() {
            *x *= if src.contains(&(v as u32)) {
                s_scale
            } else {
                o_scale
            };
        }
    } else {
        let scale = 1.0 / (nn * (nn - 1.0));
        for x in &mut bc {
            *x *= scale;
        }
    }
    bc
}

/// Brandes' accumulation from one source, as `_single_source_shortest_path_basic` and
/// `_accumulate_basic` do it; returns `(node, dependency)` for every reached node but the source,
/// in the order NetworkX adds them.
fn single_source(adj: &Adjacency, s: u32) -> Vec<(u32, f64)> {
    let n = adj.neighbors.len();
    let mut dist = vec![u32::MAX; n];
    let mut sigma = vec![0.0f64; n];
    let mut preds: Vec<Vec<u32>> = Vec::new();
    let mut slot = vec![u32::MAX; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    dist[s as usize] = 0;
    sigma[s as usize] = 1.0;
    queue.push_back(s);
    let mut slot_of = |v: u32, preds: &mut Vec<Vec<u32>>| -> usize {
        if slot[v as usize] == u32::MAX {
            slot[v as usize] = preds.len() as u32;
            preds.push(Vec::new());
        }
        slot[v as usize] as usize
    };
    while let Some(v) = queue.pop_front() {
        stack.push(v);
        let dv = dist[v as usize];
        let sv = sigma[v as usize];
        for &(w, _) in &adj.neighbors[v as usize] {
            if dist[w as usize] == u32::MAX {
                queue.push_back(w);
                dist[w as usize] = dv + 1;
            }
            if dist[w as usize] == dv + 1 {
                sigma[w as usize] += sv;
                let i = slot_of(w, &mut preds);
                preds[i].push(v);
            }
        }
    }
    let mut delta = vec![0.0f64; n];
    let mut out = Vec::with_capacity(stack.len());
    while let Some(w) = stack.pop() {
        let coeff = (1.0 + delta[w as usize]) / sigma[w as usize];
        if slot[w as usize] != u32::MAX {
            for &v in &preds[slot[w as usize] as usize] {
                delta[v as usize] += sigma[v as usize] * coeff;
            }
        }
        if w != s {
            out.push((w, delta[w as usize]));
        }
    }
    out
}

/// CPython's `random.Random` (MT19937) — just enough for `sample(range(n), k)`.
pub struct PyRandom {
    mt: [u32; 624],
    index: usize,
}

impl PyRandom {
    /// `random.Random(seed)` for a non-negative integer seed below 2³².
    pub fn new(seed: u32) -> PyRandom {
        let mut r = PyRandom {
            mt: [0; 624],
            index: 624,
        };
        r.init_genrand(19_650_218);
        let key = [seed];
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..624.max(key.len()) {
            let prev = r.mt[i - 1] ^ (r.mt[i - 1] >> 30);
            r.mt[i] = (r.mt[i] ^ prev.wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                r.mt[0] = r.mt[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            let prev = r.mt[i - 1] ^ (r.mt[i - 1] >> 30);
            r.mt[i] = (r.mt[i] ^ prev.wrapping_mul(1_566_083_941)).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                r.mt[0] = r.mt[623];
                i = 1;
            }
        }
        r.mt[0] = 0x8000_0000;
        r
    }

    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..624 {
            let prev = self.mt[i - 1] ^ (self.mt[i - 1] >> 30);
            self.mt[i] = 1_812_433_253u32.wrapping_mul(prev).wrapping_add(i as u32);
        }
        self.index = 624;
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for k in 0..624 {
                let y = (self.mt[k] & 0x8000_0000) | (self.mt[(k + 1) % 624] & 0x7fff_ffff);
                let mut v = self.mt[(k + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 {
                    v ^= 0x9908_b0df;
                }
                self.mt[k] = v;
            }
            self.index = 0;
        }
        let mut y = self.mt[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// `_randbelow_with_getrandbits(n)` for `0 < n < 2³²`.
    fn randbelow(&mut self, n: u32) -> u32 {
        let k = 32 - n.leading_zeros();
        loop {
            let r = self.next_u32() >> (32 - k);
            if r < n {
                return r;
            }
        }
    }

    /// `sample(population, k)` over `0..n`, returning positions.
    pub fn sample(&mut self, n: usize, k: usize) -> Vec<u32> {
        let mut setsize = 21usize;
        if k > 5 {
            let e = ((k * 3) as f64).ln() / 4f64.ln();
            setsize += 4usize.pow(e.ceil() as u32);
        }
        let mut out = Vec::with_capacity(k);
        if n <= setsize {
            let mut pool: Vec<u32> = (0..n as u32).collect();
            for i in 0..k {
                let j = self.randbelow((n - i) as u32) as usize;
                out.push(pool[j]);
                pool[j] = pool[n - i - 1];
            }
        } else {
            let mut selected = HashSet::new();
            for _ in 0..k {
                let mut j = self.randbelow(n as u32);
                while selected.contains(&j) {
                    j = self.randbelow(n as u32);
                }
                selected.insert(j);
                out.push(j);
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Report inputs

/// Graphify's "isolated nodes" (symbols only), in build order: symbols other than functions with at
/// most one link, unless that link is a `contains`.
pub fn isolated(graph: &Graph, adj: &Adjacency) -> Vec<u32> {
    (0..graph.nodes.len() as u32)
        .filter(|&i| {
            let n = &graph.nodes[i as usize];
            if !is_symbol(n)
                || n.file_type == "rationale"
                || n.label.ends_with("()")
                || adj.degree[i as usize] > 1
            {
                return false;
            }
            adj.neighbors[i as usize]
                .first()
                .is_none_or(|&(_, e)| graph.edges[e as usize].relation != "contains")
        })
        .collect()
}

/// Members a community section lists: symbols, no rationale, no methods, and no functions with at
/// most one link.
pub fn listed(n: &Node, degree: u32) -> bool {
    if !is_symbol(n) || n.file_type == "rationale" {
        return false;
    }
    let l = n.label.as_str();
    !(l.ends_with("()") && (l.starts_with('.') || degree <= 1))
}

/// File-level import cycles: shortest first, each rotated to start at its smallest path, at most
/// `limit`. The file graph has one arc per `imports_from` link between the source files of
/// its endpoints.
pub fn import_cycles(
    graph: &Graph,
    topo: &Topology,
    adj: &Adjacency,
    limit: usize,
) -> Vec<Vec<String>> {
    // Files and arcs in `G.edges()` order.
    let mut file_ix: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut files: Vec<&str> = Vec::new();
    let mut arcs: Vec<(usize, usize)> = Vec::new();
    let mut named: Vec<(&str, &str)> = Vec::new();
    for e in adj.edge_order() {
        let link = &graph.edges[e as usize];
        if link.relation != "imports_from" && link.relation != "re_exports" {
            continue;
        }
        // `topo` keeps each link's stored direction: `s` is its source.
        let (s, t, _) = topo.edges[e as usize];
        let (src, dst) = (&graph.nodes[s as usize], &graph.nodes[t as usize]);
        // Type-only (`if TYPE_CHECKING:`, `import type`) and deferred (inside a function) imports
        // cannot form a load-time cycle.
        let lazy = link.attrs.iter().any(|(k, v)| {
            (k == "type_only" || k == "deferred") && *v == crate::facts::Attr::Bool(true)
        });
        // A re-export counts when it re-exports a whole module (Python `from .m import X` in a
        // package), not a single symbol (TS `export { a } from './x'`).
        let symbol_reexport = link.relation == "re_exports" && !is_file_node(dst);
        if src.source_file.is_empty() || dst.source_file.is_empty() || lazy || symbol_reexport {
            continue;
        }
        named.push((src.source_file.as_str(), dst.source_file.as_str()));
    }
    for (a, b) in named {
        let mut ix = |f| {
            *file_ix.entry(f).or_insert_with(|| {
                files.push(f);
                files.len() - 1
            })
        };
        let (ia, ib) = (ix(a), ix(b));
        arcs.push((ia, ib));
    }
    let n = files.len();
    // Successors sorted by path; nodes ranked by path.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&x, &y| files[x].cmp(files[y]));
    let mut rank = vec![0usize; n];
    for (r, &x) in order.iter().enumerate() {
        rank[x] = r;
    }
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in &arcs {
        if !succ[a].contains(&b) {
            succ[a].push(b);
        }
    }
    for s in &mut succ {
        s.sort_by_key(|&x| rank[x]);
    }
    // Iterative deepening: every cycle of length L is found from its smallest file, through files
    // that sort after it. Stop after the first length that fills the quota.
    const MAX_LEN: usize = 5;
    const STEP_BUDGET: usize = 2_000_000;
    let mut steps = 0usize;
    let mut found: Vec<Vec<usize>> = Vec::new();
    for len in 1..=MAX_LEN {
        for &start in &order {
            let mut path = vec![start];
            cycles_of_len(&succ, &rank, start, len, &mut path, &mut found, &mut steps);
            if steps > STEP_BUDGET {
                break;
            }
        }
        if found.len() >= limit || steps > STEP_BUDGET {
            break;
        }
    }
    found.truncate(limit);
    found
        .into_iter()
        .map(|c| c.into_iter().map(|f| files[f].to_string()).collect())
        .collect()
}

fn cycles_of_len(
    succ: &[Vec<usize>],
    rank: &[usize],
    start: usize,
    len: usize,
    path: &mut Vec<usize>,
    found: &mut Vec<Vec<usize>>,
    steps: &mut usize,
) {
    *steps += 1;
    if *steps > 2_000_000 {
        return;
    }
    let last = *path.last().unwrap();
    for &nx in &succ[last] {
        if path.len() == len {
            if nx == start {
                found.push(path.clone());
            }
            continue;
        }
        if rank[nx] <= rank[start] || path.contains(&nx) {
            continue;
        }
        path.push(nx);
        cycles_of_len(succ, rank, start, len, path, found, steps);
        path.pop();
    }
}

/// CPython's iteration order of a `set` of small non-negative ints built by adding `items` in order.
pub fn py_int_set_order(items: impl IntoIterator<Item = u32>) -> Vec<u32> {
    const PERTURB_SHIFT: u32 = 5;
    const LINEAR_PROBES: usize = 9;
    let mut table: Vec<Option<u32>> = vec![None; 8];
    let mut used = 0usize;
    fn insert(table: &mut [Option<u32>], key: u32) -> bool {
        let mask = table.len() - 1;
        let mut perturb = key as usize;
        let mut i = key as usize & mask;
        loop {
            let probes = if i + LINEAR_PROBES <= mask {
                LINEAR_PROBES
            } else {
                0
            };
            for slot in &mut table[i..=i + probes] {
                match *slot {
                    None => {
                        *slot = Some(key);
                        return true;
                    }
                    Some(k) if k == key => return false,
                    _ => {}
                }
            }
            perturb >>= PERTURB_SHIFT;
            i = (i * 5 + 1 + perturb) & mask;
        }
    }
    for key in items {
        if insert(&mut table, key) {
            used += 1;
            let mask = table.len() - 1;
            if used * 5 >= mask * 3 {
                let minused = if used > 50_000 { used * 2 } else { used * 4 };
                let mut size = 8;
                while size <= minused {
                    size <<= 1;
                }
                let old = std::mem::replace(&mut table, vec![None; size]);
                for k in old.into_iter().flatten() {
                    insert(&mut table, k);
                }
            }
        }
    }
    table.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_random_sample_matches_cpython() {
        // CPython: random.Random(42).sample(range(2000), 5) (set-based path) and
        // random.Random(42).sample(range(1001), 100)[:5] (pool-based path).
        assert_eq!(
            PyRandom::new(42).sample(2000, 5),
            vec![1309, 228, 51, 1518, 563]
        );
        assert_eq!(
            PyRandom::new(42).sample(1001, 100)[..5],
            [654, 114, 25, 759, 281]
        );
    }

    #[test]
    fn python_set_order() {
        // CPython: s = set(); [s.add(x) for x in seq]; list(s)
        assert_eq!(py_int_set_order([0, 4, 100, 8]), vec![0, 100, 4, 8]);
        assert_eq!(py_int_set_order([3, 1, 2]), vec![1, 2, 3]);
        let seq = [
            0, 1, 2, 131, 128, 4, 5, 6, 7, 261, 10, 395, 140, 15, 200, 300, 400, 500, 600, 700, 64,
            65,
        ];
        assert_eq!(
            py_int_set_order(seq),
            vec![
                0, 1, 2, 131, 128, 4, 5, 6, 7, 261, 10, 395, 140, 15, 400, 300, 700, 64, 65, 200,
                600, 500
            ]
        );
    }

    #[test]
    fn suffix_follows_purepath() {
        assert!(has_suffix("a/b.py"));
        assert!(!has_suffix("bench/runner"));
        assert!(!has_suffix(".bashrc"));
        assert!(!has_suffix(""));
        assert!(has_suffix("x/.eslintrc.json"));
    }
}

//! Community detection with Graphify's observable behaviour (`docs/graphify-format.md` §17).
//!
//! Leiden runs through Microsoft's `network_partitions` (graspologic-native 1.3.1, the engine
//! Graphify calls) with resolution 1.0, randomness 0.001, one iteration, modularity and seed 42.
//! Its input reproduces what Graphify feeds it: every link as `(lo, hi, weight)` with `lo < hi` by
//! id, the list sorted, nodes numbered in order of first appearance in that list. Post-processing:
//!
//! 1. Nodes without links become singleton communities.
//! 2. A community larger than a quarter of the graph and than 10 nodes is re-partitioned on its own
//!    induced subgraph.
//! 3. Then, once, every community of at least 50 nodes with cohesion (self-loops excluded) below
//!    0.05 is re-partitioned the same way.
//! 4. Communities are numbered by size, descending; ties by their sorted member ids.

use network_partitions::clustering::Clustering;
use network_partitions::leiden;
use network_partitions::network::prelude::CompactNetwork;
use rand::rngs::SmallRng;
use rand::SeedableRng;

use crate::facts::Confidence;
use crate::graph::Graph;

const SEED: u64 = 42;
const RESOLUTION: f64 = 1.0;
const RANDOMNESS: f64 = 0.001;
const SPLIT_FRACTION: f64 = 0.25;
const SPLIT_MIN: usize = 10;
const RESPLIT_MIN: usize = 50;
const RESPLIT_COHESION: f64 = 0.05;

/// A partition of the graph's nodes (indices into `Graph::nodes`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Communities {
    /// Community id of every node.
    pub of_node: Vec<u32>,
    /// Members of every community, ordered by node id.
    pub members: Vec<Vec<u32>>,
}

/// The simple undirected graph Leiden and the analysis see: integer endpoints and Leiden weights.
pub struct Topology {
    pub n: usize,
    /// One entry per link, in `Graph::edges` order: `(source, target, weight)`.
    pub edges: Vec<(u32, u32, f64)>,
    /// Position of every node when nodes are sorted by id (Python string order).
    pub rank: Vec<u32>,
}

impl Topology {
    pub fn of(graph: &Graph) -> Topology {
        let index: std::collections::HashMap<&str, u32> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i as u32))
            .collect();
        let edges = graph
            .edges
            .iter()
            .map(|e| {
                let mut w = e.weight.unwrap_or(1.0);
                if e.confidence == Confidence::Ambiguous {
                    w *= 0.5;
                }
                (index[e.source.as_str()], index[e.target.as_str()], w)
            })
            .collect();
        let mut by_id: Vec<u32> = (0..graph.nodes.len() as u32).collect();
        by_id
            .sort_unstable_by(|&a, &b| graph.nodes[a as usize].id.cmp(&graph.nodes[b as usize].id));
        let mut rank = vec![0u32; graph.nodes.len()];
        for (r, &i) in by_id.iter().enumerate() {
            rank[i as usize] = r as u32;
        }
        Topology {
            n: graph.nodes.len(),
            edges,
            rank,
        }
    }
}

/// Partition the graph (see the module docs).
pub fn cluster(topo: &Topology) -> Communities {
    let all: Vec<u32> = (0..topo.n as u32).collect();
    let all_edges: Vec<usize> = (0..topo.edges.len()).collect();
    let mut parts = leiden_parts(topo, &all, &all_edges);

    let mut community = vec![0u32; topo.n];
    let assign = |parts: &[Vec<u32>], community: &mut [u32]| {
        for (c, p) in parts.iter().enumerate() {
            for &m in p {
                community[m as usize] = c as u32;
            }
        }
    };

    // Oversized communities are partitioned again on their own.
    let limit = SPLIT_FRACTION * topo.n as f64;
    if parts
        .iter()
        .any(|p| p.len() as f64 > limit && p.len() > SPLIT_MIN)
    {
        assign(&parts, &mut community);
        let internal = internal_edges(topo, &community, parts.len());
        let mut out = Vec::with_capacity(parts.len());
        for (c, p) in parts.into_iter().enumerate() {
            if p.len() as f64 > limit && p.len() > SPLIT_MIN {
                out.extend(leiden_parts(topo, &p, &internal[c]));
            } else {
                out.push(p);
            }
        }
        parts = out;
    }

    // Then, once, large communities that hang together loosely.
    assign(&parts, &mut community);
    let internal = internal_edges(topo, &community, parts.len());
    let loose = |c: usize, p: &[u32]| {
        p.len() >= RESPLIT_MIN
            && cohesion_of(p.len(), non_self(topo, &internal[c])) < RESPLIT_COHESION
    };
    if parts.iter().enumerate().any(|(c, p)| loose(c, p)) {
        let mut out = Vec::with_capacity(parts.len());
        for (c, p) in parts.into_iter().enumerate() {
            if loose(c, &p) {
                out.extend(leiden_parts(topo, &p, &internal[c]));
            } else {
                out.push(p);
            }
        }
        parts = out;
    }

    // Size descending, ties by the sorted member ids.
    for p in &mut parts {
        p.sort_unstable_by_key(|&m| topo.rank[m as usize]);
    }
    parts.sort_by(|a, b| {
        b.len().cmp(&a.len()).then_with(|| {
            a.iter()
                .map(|&m| topo.rank[m as usize])
                .cmp(b.iter().map(|&m| topo.rank[m as usize]))
        })
    });
    assign(&parts, &mut community);
    Communities {
        of_node: community,
        members: parts,
    }
}

/// Links among `edges` that are not self-loops.
pub fn non_self(topo: &Topology, edges: &[usize]) -> usize {
    edges
        .iter()
        .filter(|&&i| topo.edges[i].0 != topo.edges[i].1)
        .count()
}

/// `internal links (self-loops excluded) / (n·(n−1)/2)`, `1.0` for fewer than two members.
pub fn cohesion_of(members: usize, internal_links: usize) -> f64 {
    if members < 2 {
        return 1.0;
    }
    let possible = (members * (members - 1)) as f64 / 2.0;
    internal_links as f64 / possible
}

/// Indices of the links inside each community (self-loops included).
pub fn internal_edges(topo: &Topology, community: &[u32], count: usize) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new(); count];
    for (i, &(s, t, _)) in topo.edges.iter().enumerate() {
        let c = community[s as usize];
        if c == community[t as usize] {
            out[c as usize].push(i);
        }
    }
    out
}

/// One Leiden run over `nodes` and the links `edges` (all inside `nodes`), plus singletons for
/// nodes without links. Parts come back in no particular order.
fn leiden_parts(topo: &Topology, nodes: &[u32], edges: &[usize]) -> Vec<Vec<u32>> {
    // `(lo, hi, w)` with `lo` the smaller id, sorted like Python sorts the tuples.
    let mut sorted: Vec<(u32, u32, f64)> = edges
        .iter()
        .map(|&i| {
            let (s, t, w) = topo.edges[i];
            if topo.rank[s as usize] <= topo.rank[t as usize] {
                (s, t, w)
            } else {
                (t, s, w)
            }
        })
        .collect();
    sorted.sort_by(|a, b| {
        (topo.rank[a.0 as usize], topo.rank[a.1 as usize])
            .cmp(&(topo.rank[b.0 as usize], topo.rank[b.1 as usize]))
            .then(a.2.total_cmp(&b.2))
    });

    let mut parts: Vec<Vec<u32>> = Vec::new();
    if !sorted.is_empty() {
        let (labels, network) = compact_network(&sorted);
        let mut rng = SmallRng::seed_from_u64(SEED);
        let (_, clustering): (bool, Clustering) = leiden::leiden(
            &network,
            None,
            Some(1),
            Some(RESOLUTION),
            Some(RANDOMNESS),
            &mut rng,
            true,
            None,
        )
        .expect("leiden on a non-empty network");
        let mut by_cluster: Vec<Vec<u32>> = vec![Vec::new(); clustering.next_cluster_id()];
        for (compact, &node) in labels.iter().enumerate() {
            by_cluster[clustering.cluster_at(compact).expect("clustered node")].push(node);
        }
        parts.extend(by_cluster.into_iter().filter(|p| !p.is_empty()));
    }
    let mut seen: std::collections::HashSet<u32> = parts.iter().flatten().copied().collect();
    for &n in nodes {
        if seen.insert(n) {
            parts.push(vec![n]);
        }
    }
    parts
}

/// What graspologic-native's `LabeledNetworkBuilder` makes of an edge list: nodes numbered by first
/// appearance, neighbours sorted by number, the first weight of a repeated pair kept, self-loops
/// only counted in the total self-link weight, node weight = summed non-self link weight.
fn compact_network(edges: &[(u32, u32, f64)]) -> (Vec<u32>, CompactNetwork) {
    let mut local: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut labels: Vec<u32> = Vec::new();
    let mut id = |n: u32, labels: &mut Vec<u32>| -> usize {
        *local.entry(n).or_insert_with(|| {
            labels.push(n);
            labels.len() - 1
        })
    };
    let mut adj: Vec<Vec<(usize, f64)>> = Vec::new();
    for &(s, t, w) in edges {
        let a = id(s, &mut labels);
        let b = id(t, &mut labels);
        if adj.len() < labels.len() {
            adj.resize_with(labels.len(), Vec::new);
        }
        // Pairs are unique in a simple graph; a self-loop is recorded once.
        adj[a].push((b, w));
        if a != b {
            adj[b].push((a, w));
        }
    }
    let mut nodes = Vec::with_capacity(labels.len());
    let mut neighbors = Vec::new();
    let mut self_links = 0.0;
    for list in &mut adj {
        list.sort_by_key(|&(n, _)| n);
        list.dedup_by_key(|&mut (n, _)| n);
        let start = neighbors.len();
        let mut weight = 0.0;
        let me = nodes.len();
        for &(n, w) in list.iter() {
            if n == me {
                self_links += w;
            } else {
                weight += w;
                neighbors.push((n, w));
            }
        }
        nodes.push((weight, start));
    }
    (labels, CompactNetwork::from(nodes, neighbors, self_links))
}

/// The community label Graphify derives without a model: the label of the member with the most
/// links (ties: the smaller id), without a trailing `()`.
pub fn labels(graph: &Graph, communities: &Communities, degree: &[u32]) -> Vec<String> {
    communities
        .members
        .iter()
        .map(|members| {
            // Members are in id order, so the first maximum is the smallest id.
            let mut best = members[0];
            for &m in members {
                if degree[m as usize] > degree[best as usize] {
                    best = m;
                }
            }
            let label = &graph.nodes[best as usize].label;
            label.strip_suffix("()").unwrap_or(label).to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Edge, Loc, Node, NodeKind};

    fn node(id: &str) -> Node {
        Node {
            id: id.into(),
            label: id.into(),
            kind: NodeKind::Def,
            file_type: "code".into(),
            source_file: "a.py".into(),
            loc: Loc::Line(1),
            attrs: vec![],
        }
    }

    fn edge(s: &str, t: &str) -> Edge {
        Edge {
            source: s.into(),
            target: t.into(),
            relation: "calls".into(),
            confidence: Confidence::Extracted,
            score: None,
            context: None,
            source_file: "a.py".into(),
            loc: Loc::Line(1),
            weight: Some(1.0),
            attrs: vec![],
        }
    }

    #[test]
    fn two_triangles_and_an_isolate() {
        let ids = ["a", "b", "c", "x", "y", "z", "lone"];
        let mut edges = vec![];
        for (s, t) in [
            ("a", "b"),
            ("b", "c"),
            ("a", "c"),
            ("x", "y"),
            ("y", "z"),
            ("x", "z"),
            ("c", "x"),
        ] {
            edges.push(edge(s, t));
        }
        let g = Graph::assemble(ids.iter().map(|i| node(i)).collect(), edges);
        let c = cluster(&Topology::of(&g));
        assert_eq!(c.members.len(), 3);
        assert_eq!(c.members[2], vec![6]);
        assert_eq!(c.of_node[0], c.of_node[1]);
        assert_ne!(c.of_node[0], c.of_node[3]);
        // Equal sizes: the community whose sorted ids come first gets the smaller number.
        assert_eq!(c.of_node[0], 0);
        assert_eq!(cohesion_of(3, 3), 1.0);
        assert_eq!(cohesion_of(1, 0), 1.0);
    }
}

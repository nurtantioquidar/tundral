//! Graph-wide summaries the MCP server exposes as tools and resources (`docs/graphify-query.md`
//! §10.6–§10.7): statistics, the confidence audit, surprising connections and suggested
//! questions.
//!
//! Unlike `GRAPH_REPORT.md` (`graphify-format.md` §17), these are computed from the stored graph
//! alone: the communities are the stored `community` values (named `Community N`) and the graph is
//! the directed multigraph of `links`. Iteration follows NetworkX's `MultiDiGraph`: nodes in file
//! order; a node's successors in the order their first link appears, each successor's links in
//! file order. Every rule below was fitted on the python and rust fixtures and the fastapi
//! reference by driving `graphify-mcp` as a black box.

use std::collections::HashMap;

use dijkstral_core::analyze::{
    betweenness, has_suffix, is_file_label, is_hub_candidate_parts, py_int_set_order, Adjacency,
};
use dijkstral_core::cluster::cohesion_of;
use dijkstral_core::index::{Community, Index};

use crate::god_nodes;

const SURPRISE_LIMIT: usize = 10;
const QUESTION_LIMIT: usize = 10;
const BRIDGE_CANDIDATES: usize = 3;
const VERIFY_CANDIDATES: usize = 5;
const ISOLATED_SHOWN: usize = 3;
const LOW_COHESION: f64 = 0.15;
const CONFIDENCES: [&str; 3] = ["EXTRACTED", "INFERRED", "AMBIGUOUS"];

/// Links per confidence tier, in [`CONFIDENCES`] order.
fn confidence_counts(ix: &Index) -> [usize; 3] {
    let mut counts = [0usize; 3];
    for e in 0..ix.edge_count() as u32 {
        if let Some(c) = ix.edge(e).confidence {
            if let Some(k) = CONFIDENCES.iter().position(|x| *x == c) {
                counts[k] += 1;
            }
        }
    }
    counts
}

/// Python's `round(count / total * 100)` (ties to even).
fn pct(count: usize, total: usize) -> i64 {
    if total == 0 {
        return 0;
    }
    (count as f64 / total as f64 * 100.0).round_ties_even() as i64
}

/// The `graph_stats` text, which (alone among the tools) ends with a newline. `commit` is the
/// graph's `built_at_commit`.
pub fn stats(ix: &Index, commit: Option<&str>) -> String {
    let m = ix.edge_count();
    let mut communities: Vec<u32> = (0..ix.node_count() as u32)
        .filter_map(|i| ix.node(i).community.id())
        .collect();
    communities.sort_unstable();
    communities.dedup();
    let counts = confidence_counts(ix);
    let mut out = format!(
        "Nodes: {}\nEdges: {m}\nCommunities: {}\n",
        ix.node_count(),
        communities.len()
    );
    for (name, c) in CONFIDENCES.iter().zip(counts) {
        out.push_str(&format!("{name}: {}%\n", pct(c, m)));
    }
    if let Some(c) = commit {
        out.push_str(&format!("Built at commit: {c}\n"));
    }
    out
}

/// The `graphify://audit` text (ends with a newline).
pub fn audit(ix: &Index) -> String {
    let m = ix.edge_count();
    let mut out = format!("Total edges: {m}\n");
    for (name, c) in CONFIDENCES.iter().zip(confidence_counts(ix)) {
        out.push_str(&format!("{name}: {c} ({}%)\n", pct(c, m)));
    }
    out
}

/// Per node, its outgoing links grouped by target: targets in first-link order, each target's
/// links in file order (`MultiDiGraph.edges()` order).
fn out_links(ix: &Index, u: u32) -> Vec<(u32, Vec<u32>)> {
    let mut groups: Vec<(u32, Vec<u32>)> = Vec::new();
    let mut pos: HashMap<u32, usize> = HashMap::new();
    for e in ix.incident(u) {
        let (s, t) = ix.endpoints(e);
        if s != u {
            continue;
        }
        match pos.get(&t) {
            Some(&k) => groups[k].1.push(e),
            None => {
                pos.insert(t, groups.len());
                groups.push((t, vec![e]));
            }
        }
    }
    groups
}

fn eligible(ix: &Index, i: u32) -> bool {
    let n = ix.node(i);
    is_hub_candidate_parts(n.label, n.source_file)
        && n.file_type != "concept"
        && n.file_type != "rationale"
}

fn top_dir(path: &str) -> &str {
    path.split_once('/').map_or("", |(d, _)| d)
}

/// Surprising connections: links between hub candidates in different files, scored by
/// confidence (EXTRACTED 1, INFERRED 2, AMBIGUOUS 3), +2 across top-level directories, +1 across
/// stored communities and +1 when a node with at most two links reaches one with at least five;
/// the best ten, ties in `MultiDiGraph.edges()` order. Returns link indices.
pub fn surprises(ix: &Index) -> Vec<u32> {
    let mut scored: Vec<(u32, u32)> = Vec::new();
    for u in 0..ix.node_count() as u32 {
        if !eligible(ix, u) {
            continue;
        }
        for (v, links) in out_links(ix, u) {
            if !eligible(ix, v) || ix.source_file(u) == ix.source_file(v) {
                continue;
            }
            let (a, b) = (ix.node(u), ix.node(v));
            for e in links {
                let mut score = match ix.edge(e).confidence {
                    Some("INFERRED") => 2,
                    Some("AMBIGUOUS") => 3,
                    _ => 1,
                };
                if top_dir(a.source_file) != top_dir(b.source_file) {
                    score += 2;
                }
                if a.community != b.community {
                    score += 1;
                }
                let (lo, hi) = (a.degree.min(b.degree), a.degree.max(b.degree));
                if lo <= 2 && hi >= 5 {
                    score += 1;
                }
                scored.push((e, score));
            }
        }
    }
    scored.sort_by_key(|&(_, s)| std::cmp::Reverse(s));
    scored.truncate(SURPRISE_LIMIT);
    scored.into_iter().map(|(e, _)| e).collect()
}

/// The `graphify://surprises` text.
pub fn render_surprises(ix: &Index, links: &[u32]) -> String {
    let mut out = vec!["Surprising cross-community connections:".to_string()];
    for &e in links {
        let e = ix.edge(e);
        out.push(format!(
            "  {} <-> {} [{}]",
            ix.label(e.source),
            ix.label(e.target),
            e.relation
        ));
    }
    out.join("\n")
}

/// Suggested questions, interleaved one of each kind per round (bridges, INFERRED-heavy god
/// nodes, isolated nodes, loosely connected communities), at most ten.
pub fn questions(ix: &Index) -> Vec<String> {
    let n = ix.node_count();
    let community = |i: u32| ix.node(i).community;
    // Successor lists for betweenness (unique successors, first-link order).
    let succ: Vec<Vec<(u32, Vec<u32>)>> = (0..n as u32).map(|u| out_links(ix, u)).collect();

    // Bridges: of the three hub candidates with the highest directed betweenness, those whose
    // successors lie in other communities.
    let adj = Adjacency {
        neighbors: succ
            .iter()
            .map(|l| l.iter().map(|(v, links)| (*v, links[0])).collect())
            .collect(),
        degree: (0..n as u32).map(|i| ix.degree(i)).collect(),
    };
    let bc = betweenness(&adj);
    let mut ranked: Vec<u32> = (0..n as u32)
        .filter(|&i| is_hub_candidate_parts(ix.label(i), ix.source_file(i)))
        .collect();
    ranked.sort_by(|&a, &b| bc[b as usize].total_cmp(&bc[a as usize]));
    let mut bridges = Vec::new();
    for &x in ranked.iter().take(BRIDGE_CANDIDATES) {
        let Community::Id(own) = community(x) else {
            continue;
        };
        let others = py_int_set_order(
            succ[x as usize]
                .iter()
                .map(|(v, _)| *v)
                .filter_map(|v| community(v).id())
                .filter(|&c| c != own),
        );
        if others.is_empty() {
            continue;
        }
        let to: Vec<String> = others.iter().map(|c| format!("`Community {c}`")).collect();
        bridges.push(format!(
            "Why does `{}` connect `Community {own}` to {}?",
            ix.label(x),
            to.join(", ")
        ));
    }

    // Of the five god-node candidates with the most incoming links, those with at least two
    // outgoing INFERRED links, named in link order.
    let mut in_degree = vec![0u32; n];
    for e in 0..ix.edge_count() as u32 {
        in_degree[ix.endpoints(e).1 as usize] += 1;
    }
    let mut gods = god_nodes::god_nodes(ix, n as i64);
    gods.sort_by_key(|&g| (std::cmp::Reverse(in_degree[g as usize]), g));
    let mut verify = Vec::new();
    for &g in gods.iter().take(VERIFY_CANDIDATES) {
        let inferred: Vec<u32> = succ[g as usize]
            .iter()
            .flat_map(|(v, links)| {
                links
                    .iter()
                    .filter(|&&e| ix.edge(e).confidence == Some("INFERRED"))
                    .map(move |_| *v)
            })
            .collect();
        if inferred.len() < 2 {
            continue;
        }
        verify.push(format!(
            "Are the {} inferred relationships involving `{}` (e.g. with `{}` and `{}`) actually correct?",
            inferred.len(),
            ix.label(g),
            ix.label(inferred[0]),
            ix.label(inferred[1])
        ));
    }

    // Isolated symbols: not functions, at most one link, and that link (when it is outgoing) is
    // not a `contains`.
    let lonely: Vec<u32> = (0..n as u32)
        .filter(|&i| {
            let node = ix.node(i);
            if !has_suffix(node.source_file)
                || is_file_label(node.label, node.source_file)
                || node.file_type == "rationale"
                || node.label.ends_with("()")
                || node.degree > 1
            {
                return false;
            }
            succ[i as usize]
                .first()
                .is_none_or(|(_, links)| ix.relation(links[0]) != "contains")
        })
        .collect();
    let mut isolated = Vec::new();
    if !lonely.is_empty() {
        let names: Vec<String> = lonely
            .iter()
            .take(ISOLATED_SHOWN)
            .map(|&i| format!("`{}`", ix.label(i)))
            .collect();
        isolated.push(format!(
            "What connects {} to the rest of the system?",
            names.join(", ")
        ));
    }

    // Loosely connected communities, in order of first appearance among the nodes.
    let mut order: Vec<u32> = Vec::new();
    let mut members: HashMap<u32, usize> = HashMap::new();
    for i in 0..n as u32 {
        if let Some(c) = community(i).id() {
            let m = members.entry(c).or_insert(0);
            if *m == 0 {
                order.push(c);
            }
            *m += 1;
        }
    }
    let mut internal: HashMap<u32, usize> = HashMap::new();
    for e in 0..ix.edge_count() as u32 {
        let (s, t) = ix.endpoints(e);
        if let (Some(a), Some(b)) = (community(s).id(), community(t).id()) {
            if a == b {
                *internal.entry(a).or_insert(0) += 1;
            }
        }
    }
    let loose: Vec<String> = order
        .iter()
        .filter(|c| cohesion_of(members[c], internal.get(c).copied().unwrap_or(0)) < LOW_COHESION)
        .map(|c| format!("Should `Community {c}` be split into smaller, more focused modules?"))
        .collect();

    let lists = [bridges, verify, isolated, loose];
    let rounds = lists.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = Vec::new();
    for r in 0..rounds {
        for l in &lists {
            if let Some(q) = l.get(r) {
                out.push(q.clone());
            }
        }
    }
    out.truncate(QUESTION_LIMIT);
    out
}

/// The `graphify://questions` text.
pub fn render_questions(questions: &[String]) -> String {
    let mut out = vec!["Suggested questions:".to_string()];
    out.extend(questions.iter().map(|q| format!("  - {q}")));
    out.join("\n")
}

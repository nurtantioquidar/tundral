//! `path`: the shortest path between two nodes (`docs/graphify-query.md` §5).
//!
//! Directed by default: a link is walkable from its source to its target, and a `contains` link
//! also backwards. Which of several equal-length paths Graphify prints was fitted on 340 probed
//! paths (fixtures and fastapi, both modes) and is reproduced exactly by a bidirectional
//! breadth-first search that always expands the smaller fringe (the forward one on a tie) and
//! stops at the first node both sides have reached, where a node's successors are its forward
//! neighbours in id order followed by its backward-walkable neighbours in id order (predecessors
//! likewise), and undirected neighbours are in id order.

use dijkstral_core::index::Index;

/// One step of a path: the link used and whether it was followed in its stored direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hop {
    pub link: u32,
    pub forward: bool,
    pub to: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub from: u32,
    pub hops: Vec<Hop>,
}

/// Neighbours of `v` a search may step to: `out` for successors, otherwise predecessors.
/// `rank[i]` is node `i`'s position in id order.
fn steps(ix: &Index, rank: &[u32], v: u32, out: bool, undirected: bool) -> Vec<u32> {
    let by_id = |xs: &mut Vec<u32>| {
        xs.sort_unstable_by_key(|&a| rank[a as usize]);
        xs.dedup();
    };
    let mut fwd = Vec::new();
    let mut back = Vec::new();
    for e in ix.incident(v) {
        let (s, t) = ix.endpoints(e);
        let (near, far) = if out { (s, t) } else { (t, s) };
        if undirected {
            fwd.push(if s == v { t } else { s });
            if s == t {
                continue;
            }
        } else {
            if near == v {
                fwd.push(far);
            }
            if far == v && ix.relation(e) == "contains" {
                back.push(near);
            }
        }
    }
    by_id(&mut fwd);
    if !back.is_empty() {
        by_id(&mut back);
        back.retain(|x| {
            fwd.binary_search_by_key(&rank[*x as usize], |y| rank[*y as usize])
                .is_err()
        });
        fwd.extend(back);
    }
    fwd
}

/// The node sequence of the shortest path from `a` to `b`, or `None`.
pub fn shortest_nodes(ix: &Index, a: u32, b: u32, undirected: bool) -> Option<Vec<u32>> {
    if a == b {
        return Some(vec![a]);
    }
    const NONE: u32 = u32::MAX;
    const ROOT: u32 = u32::MAX - 1;
    let n = ix.node_count();
    let mut pred = vec![NONE; n];
    let mut succ = vec![NONE; n];
    pred[a as usize] = ROOT;
    succ[b as usize] = ROOT;
    let mut rank = vec![0u32; n];
    for (r, i) in ix.ids_sorted().enumerate() {
        rank[i as usize] = r as u32;
    }
    let mut forward = vec![a];
    let mut reverse = vec![b];
    let meet = 'search: loop {
        if forward.is_empty() || reverse.is_empty() {
            return None;
        }
        if forward.len() <= reverse.len() {
            let level = std::mem::take(&mut forward);
            for v in level {
                for w in steps(ix, &rank, v, true, undirected) {
                    if pred[w as usize] == NONE {
                        pred[w as usize] = v;
                        forward.push(w);
                    }
                    if succ[w as usize] != NONE {
                        break 'search w;
                    }
                }
            }
        } else {
            let level = std::mem::take(&mut reverse);
            for v in level {
                for w in steps(ix, &rank, v, false, undirected) {
                    if succ[w as usize] == NONE {
                        succ[w as usize] = v;
                        reverse.push(w);
                    }
                    if pred[w as usize] != NONE {
                        break 'search w;
                    }
                }
            }
        }
    };
    let mut path = Vec::new();
    let mut x = meet;
    while x != ROOT {
        path.push(x);
        x = pred[x as usize];
    }
    path.reverse();
    let mut x = succ[meet as usize];
    while x != ROOT {
        path.push(x);
        x = succ[x as usize];
    }
    Some(path)
}

/// The shortest path from `a` to `b` with the link shown for each hop: the first link from the
/// hop's start to its end in graph.json order, else the first link back.
pub fn shortest_path(ix: &Index, a: u32, b: u32, undirected: bool) -> Option<Path> {
    let nodes = shortest_nodes(ix, a, b, undirected)?;
    let hops = nodes
        .windows(2)
        .map(|w| {
            let (u, v) = (w[0], w[1]);
            let mut back = None;
            for e in ix.incident(u) {
                let (s, t) = ix.endpoints(e);
                if s == u && t == v {
                    return Hop {
                        link: e,
                        forward: true,
                        to: v,
                    };
                }
                if back.is_none() && s == v && t == u {
                    back = Some(e);
                }
            }
            Hop {
                link: back.expect("consecutive path nodes are linked"),
                forward: false,
                to: v,
            }
        })
        .collect();
    Some(Path { from: a, hops })
}

/// `Shortest path (<h> hops):` and the arrow line, without a trailing newline.
pub fn render(ix: &Index, p: &Path) -> String {
    let mut line = format!("  {}", ix.label(p.from));
    for h in &p.hops {
        let e = ix.edge(h.link);
        let mid = match e.confidence {
            Some(c) => format!("{} [{c}]", e.relation),
            None => e.relation.to_string(),
        };
        if h.forward {
            line.push_str(&format!(" --{mid}--> {}", ix.label(h.to)));
        } else {
            line.push_str(&format!(" <--{mid}-- {}", ix.label(h.to)));
        }
    }
    format!("Shortest path ({} hops):\n{line}", p.hops.len())
}

pub fn no_path(a: &str, b: &str, undirected: bool) -> String {
    if undirected {
        format!("No path found between '{a}' and '{b}'.")
    } else {
        format!(
            "No directed path found between '{a}' and '{b}'. Re-run with --undirected to search ignoring edge direction."
        )
    }
}

pub fn same_node(a: &str, b: &str, id: &str) -> String {
    format!(
        "'{a}' and '{b}' both resolved to the same node '{id}'. Use a more specific label or the exact node ID."
    )
}

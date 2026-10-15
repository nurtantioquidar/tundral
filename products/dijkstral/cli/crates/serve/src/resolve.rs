//! Name resolution for `path` and `explain` (and MCP `get_node`, `get_neighbors`,
//! `shortest_path`): `docs/graphify-query.md` §4.

use dijkstral_core::index::{label_words, py_lower, Index};
use memchr::memmem::Finder;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Found(u32),
    /// Candidates of the winning tier, in node order.
    Ambiguous(Vec<u32>),
    NotFound,
}

/// Resolve a user-supplied node name (§4, refined by probes). Labels and the query are compared
/// as their lower-cased alphanumeric words joined by single spaces (`{ Store }`, `.store()`,
/// `_store_` and `store` are equal; `foo_bar` equals `foo bar`); ids are compared lower-cased with
/// one trailing `()` dropped from the query (`main()` matches the id `main`). Tiers, first
/// non-empty wins: exact (words or id), prefix (words or id), substring (words). `path::symbol`
/// narrows to nodes whose `source_file` ends with `path`.
pub fn resolve(ix: &Index, query: &str) -> Resolved {
    // A cross-repo id (`<repo>::<id>`, `docs/graphify-multirepo.md` §3.3) is an exact id before
    // it is a `path::symbol`.
    if query.contains("::") {
        let exact: Vec<u32> = ix
            .ids_with_prefix(query)
            .filter(|&i| ix.id(i) == query)
            .collect();
        if let [one] = exact[..] {
            return Resolved::Found(one);
        }
    }
    let (path, symbol) = match query.split_once("::") {
        Some((p, s)) => (Some(p), s),
        None => (None, query),
    };
    let lower = symbol.to_lowercase();
    let q = label_words(&lower);
    let q = q.as_ref();
    let qid = lower.strip_suffix("()").unwrap_or(&lower);
    let finder = Finder::new(q.as_bytes());
    let mut tiers: [Vec<u32>; 3] = Default::default();
    let all: Vec<u32>;
    let nodes: &[u32] = match candidates(ix, q, qid) {
        Some(c) => {
            all = c;
            &all
        }
        None => {
            all = (0..ix.node_count() as u32).collect();
            &all
        }
    };
    for &i in nodes {
        if let Some(p) = path {
            if !ix.source_file(i).ends_with(p) {
                continue;
            }
        }
        let label = ix.label_words(i);
        let id = py_lower(ix.id(i));
        if label == q || id == qid {
            tiers[0].push(i);
        } else if !tiers[0].is_empty() {
            continue;
        } else if label.starts_with(q) || id.starts_with(qid) {
            tiers[1].push(i);
        } else if tiers[1].is_empty() && finder.find(label.as_bytes()).is_some() {
            tiers[2].push(i);
        }
    }
    match tiers.into_iter().find(|t| !t.is_empty()) {
        None => Resolved::NotFound,
        Some(t) if t.len() == 1 => Resolved::Found(t[0]),
        Some(t) => Resolved::Ambiguous(t),
    }
}

/// A superset of the nodes any tier can match, ascending, from the trigram index (every word of an
/// ASCII query is a substring of a matching label's `norm_label`) and the id-prefix range; `None`
/// when no query word has three characters, so every node is scanned.
fn candidates(ix: &Index, q: &str, qid: &str) -> Option<Vec<u32>> {
    if !q.is_ascii() {
        return None;
    }
    let mut best: Option<Vec<u32>> = None;
    for w in q.split(' ') {
        if let Some(c) = ix.substring_candidates(w) {
            best = Some(match best {
                None => c,
                Some(b) => b
                    .into_iter()
                    .filter(|x| c.binary_search(x).is_ok())
                    .collect(),
            });
        }
    }
    let mut out = best?;
    if !qid.is_empty() {
        out.extend(ix.ids_with_prefix(qid));
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// `No node matching '<q>' found.`
pub fn not_found(query: &str) -> String {
    format!("No node matching '{query}' found.")
}

/// The ambiguity text (§4), without a trailing newline.
pub fn ambiguous(ix: &Index, query: &str, nodes: &[u32]) -> String {
    let files: Vec<&str> = nodes.iter().map(|&i| ix.source_file(i)).collect();
    let same_file = files.iter().all(|f| *f == files[0]);
    let mut out = Vec::with_capacity(nodes.len() * 2 + 2);
    if same_file {
        let place = if files[0].is_empty() {
            "with no source file".to_string()
        } else {
            format!("in {}", files[0])
        };
        out.push(format!(
            "Ambiguous: '{query}' matches {} nodes {place}.",
            nodes.len()
        ));
        for &i in nodes {
            let n = ix.node(i);
            // Probed on fastapi: nodes without a location are listed by their label.
            let shown = if n.source_location.is_empty() {
                n.label
            } else {
                n.source_location
            };
            out.push(format!("  {shown}"));
            out.push(format!("    id: {}", n.id));
        }
        out.push("Retry with the full node id.".into());
    } else {
        // One candidate per file, the first in node order (probed on fastapi: 393 `get`
        // candidates in 11 files list as "matches 11 nodes").
        let mut seen = std::collections::HashSet::new();
        let nodes: Vec<u32> = nodes
            .iter()
            .copied()
            .filter(|&i| seen.insert(ix.source_file(i)))
            .collect();
        out.push(format!(
            "Ambiguous: '{query}' matches {} nodes in different files.",
            nodes.len()
        ));
        for &i in &nodes {
            let n = ix.node(i);
            // A node without a source file is listed by its id (probed).
            let shown = if n.source_file.is_empty() {
                n.id
            } else {
                n.source_file
            };
            out.push(format!("  {shown}"));
            out.push(format!("    id: {}", n.id));
        }
        out.push(format!(
            "Retry with path::symbol using one of the paths above (e.g. <path>::{}) or the full node id.",
            ix.label(nodes[0])
        ));
    }
    out.join("\n")
}

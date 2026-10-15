//! `god-nodes`: the most connected symbols (`docs/graphify-query.md` §8).

use dijkstral_core::analyze::is_god_node;
use dijkstral_core::index::Index;
use dijkstral_core::pyjson::{dumps_indent, Json};

pub const DEFAULT_TOP: i64 = 10;

/// The same candidates as the report's god nodes (`docs/graphify-format.md` §17.4): symbols in
/// files with a suffix, not file nodes or methods, not noise labels or generic JSON keys, and no
/// functions with at most one link.
fn candidate(ix: &Index, i: u32) -> bool {
    is_god_node(ix.label(i), ix.source_file(i), ix.degree(i))
}

/// The top `n` candidates by degree, descending, stable over node order (`n` below 1 is 1).
pub fn god_nodes(ix: &Index, n: i64) -> Vec<u32> {
    let n = n.max(1) as usize;
    let mut cands: Vec<u32> = (0..ix.node_count() as u32)
        .filter(|&i| candidate(ix, i))
        .collect();
    cands.sort_by_key(|&i| std::cmp::Reverse(ix.degree(i)));
    cands.truncate(n);
    cands
}

pub fn render_text(ix: &Index, nodes: &[u32]) -> String {
    let mut out = vec!["God nodes (most connected):".to_string()];
    for (k, &i) in nodes.iter().enumerate() {
        out.push(format!(
            "  {}. {} - {} edges",
            k + 1,
            ix.label(i),
            ix.degree(i)
        ));
    }
    out.join("\n")
}

pub fn render_json(ix: &Index, nodes: &[u32]) -> String {
    let items = nodes
        .iter()
        .map(|&i| {
            Json::Object(vec![
                ("id".into(), Json::str(ix.id(i))),
                ("label".into(), Json::str(ix.label(i))),
                ("degree".into(), Json::Int(ix.degree(i) as i64)),
            ])
        })
        .collect();
    dumps_indent(&Json::Array(items))
}

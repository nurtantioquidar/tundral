//! `affected`: what depends on a node, by reverse traversal (`docs/graphify-query.md` §7).

use std::collections::HashSet;

use dijkstral_core::index::{py_lower, Index};

/// Relations followed by default, in the order the header prints them.
pub const DEFAULT_RELATIONS: &[&str] = &[
    "calls",
    "indirect_call",
    "references",
    "imports",
    "imports_from",
    "dynamic_import",
    "re_exports",
    "inherits",
    "extends",
    "implements",
    "uses",
    "mixes_in",
    "embeds",
    "defines",
    "requires",
];

pub const DEFAULT_DEPTH: i64 = 2;

/// `affected`'s own resolution: id exact, else a unique case-insensitive exact label, else a
/// unique case-insensitive label substring.
pub fn resolve(ix: &Index, name: &str) -> Option<u32> {
    if let Some(i) = ix.by_id(name) {
        return Some(i);
    }
    let q = py_lower(name);
    let mut exact = Vec::new();
    let mut sub = Vec::new();
    for i in 0..ix.node_count() as u32 {
        let l = ix.lower_label(i);
        if l == q {
            exact.push(i);
        }
        if l.contains(q.as_ref()) {
            sub.push(i);
            if sub.len() > 1 && !exact.is_empty() {
                break;
            }
        }
    }
    match (exact.len(), sub.len()) {
        (1, _) => Some(exact[0]),
        (0, 1) => Some(sub[0]),
        _ => None,
    }
}

/// One affected node: the relation and link that first reached it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Affected {
    pub node: u32,
    pub link: u32,
}

/// Targets of `node`'s outgoing links with one of `relations`, in link order.
fn out_targets<'a>(
    ix: &'a Index,
    node: u32,
    relations: &'a [&str],
) -> impl Iterator<Item = u32> + 'a {
    ix.incident(node).filter_map(move |e| {
        let (s, t) = ix.endpoints(e);
        (s == node && t != node && relations.contains(&ix.relation(e))).then_some(t)
    })
}

/// Ancestors of a class, breadth-first over outgoing `inherits`, `extends` and `implements` links
/// in link order.
fn ancestors(ix: &Index, class: u32) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    let mut seen: HashSet<u32> = HashSet::from([class]);
    let mut k = 0;
    let mut cur = class;
    loop {
        for p in out_targets(ix, cur, &["inherits", "extends", "implements"]) {
            if seen.insert(p) {
                out.push(p);
            }
        }
        if k == out.len() {
            break;
        }
        cur = out[k];
        k += 1;
    }
    out
}

/// The methods `m` overrides: for each class owning `m` (source of a `method` link to it), the
/// `method` targets with `m`'s label of that class's ancestors, visited in reverse breadth-first
/// order (probed: with `A(B, D)` and `B(C)`, `A.run` overrides `C.run`, `D.run`, `B.run` in that
/// order).
pub fn overridden(ix: &Index, m: u32) -> Vec<u32> {
    let label = ix.label(m);
    let mut out = Vec::new();
    for e in ix.incident(m) {
        let (s, t) = ix.endpoints(e);
        if t != m || s == m || ix.relation(e) != "method" {
            continue;
        }
        for &a in ancestors(ix, s).iter().rev() {
            for t in out_targets(ix, a, &["method"]) {
                if ix.label(t) == label && !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    out
}

/// Reverse breadth-first search over incoming links whose relation is in `relations` (probed).
///
/// The start layer is the node and its direct members (targets of its outgoing `contains` and
/// `method` links, in link order); grandchildren are not included. Each frontier node is
/// processed together with the methods it overrides ([`overridden`]) that are not yet seen at
/// that moment, so callers of a base method that a reached subclass method overrides are affected
/// too. Overridden methods are not marked as seen: one can be listed later when it calls another
/// (erpnext: `StockController.validate` calls `AccountsController.validate` and is listed for
/// `StockEntry`), and once listed its own override entry is skipped.
pub fn affected<S: AsRef<str>>(
    ix: &Index,
    node: u32,
    relations: &[S],
    depth: i64,
) -> Vec<Affected> {
    let rels: HashSet<&str> = relations.iter().map(AsRef::as_ref).collect();
    let mut seen: HashSet<u32> = HashSet::from([node]);
    let mut frontier = vec![node];
    for m in out_targets(ix, node, &["contains", "method"]).collect::<Vec<_>>() {
        if seen.insert(m) {
            frontier.push(m);
        }
    }
    let mut out = Vec::new();
    for _ in 0..depth.max(0) {
        let mut next = Vec::new();
        for &u in &frontier {
            let mut group = vec![u];
            group.extend(overridden(ix, u).into_iter().filter(|o| !seen.contains(o)));
            for x in group {
                for e in ix.incident(x) {
                    let (s, t) = ix.endpoints(e);
                    if t != x || !rels.contains(ix.relation(e)) {
                        continue;
                    }
                    if seen.insert(s) {
                        out.push(Affected { node: s, link: e });
                        next.push(s);
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out
}

/// Graphify's `affected` text (no trailing newline).
pub fn render<S: AsRef<str>>(
    ix: &Index,
    node: u32,
    relations: &[S],
    depth: i64,
    found: &[Affected],
) -> String {
    let rels: Vec<&str> = relations.iter().map(AsRef::as_ref).collect();
    let mut out = vec![
        format!("Affected nodes for {}", ix.label(node)),
        format!("Relations: {}", rels.join(", ")),
        format!("Depth: {depth}"),
    ];
    if found.is_empty() {
        out.push("No affected nodes found.".into());
    }
    for a in found {
        // A link without a location falls back to the affected node's own (probed on erpnext's
        // `dynamic_import` links, which carry no `source_location`).
        let e = ix.edge(a.link);
        let n = ix.node(a.node);
        out.push(format!(
            "- {} [{}] {}:{}",
            n.label,
            e.relation,
            e.source_file.unwrap_or(n.source_file),
            e.source_location.unwrap_or(n.source_location)
        ));
    }
    out.join("\n")
}

pub fn no_unique(name: &str) -> String {
    format!("No unique node match for {name}")
}

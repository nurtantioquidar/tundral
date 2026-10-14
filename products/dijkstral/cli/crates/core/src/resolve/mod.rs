//! The build stage after per-file extraction (`docs/graphify-format.md` §4.5, §5.1, §8):
//!
//! 1. Rename ids emitted by more than one file (§4.5).
//! 2. Index every node by its final id.
//! 3. Run each language's cross-file pass (imports, calls, stub resolution…).
//! 4. Apply redirects (deleted stubs → their resolved node) and merge duplicate rationale.
//! 5. Assign unique-suffix file labels and fold edges into one link per pair (`Graph::assemble`).
//!
//! Language passes see the renamed per-file facts through `View`; any id stored in their
//! `LangFacts` was recorded before renaming, so translate it with `View::id`.

pub mod data;
pub mod go;
pub mod java;
pub mod python;
pub mod rust;
pub mod script;
pub mod swift;

use std::collections::{HashMap, HashSet};

use crate::discover::SourceFile;
use crate::extract::slug;
use crate::facts::{Attr, Edge, FileFacts, Node, NodeKind};
use crate::graph::Graph;
use crate::lang::Family;

/// One file's facts after collision renaming.
pub struct View<'a> {
    pub file: &'a SourceFile,
    pub facts: FileFacts,
    renames: HashMap<String, String>,
}

impl View<'_> {
    /// The final id for an id this file emitted during extraction.
    pub fn id<'b>(&'b self, local: &'b str) -> &'b str {
        self.renames.get(local).map_or(local, String::as_str)
    }

    pub fn family(&self) -> Family {
        self.file.lang.family()
    }
}

/// Every node after renaming, by final id (first occurrence wins, like the build).
pub struct Index {
    nodes: HashMap<String, (usize, usize)>,
    /// File node id by relative path.
    pub file_ids: HashMap<String, String>,
}

impl Index {
    pub fn get<'a>(&self, views: &'a [View], id: &str) -> Option<&'a Node> {
        let &(f, n) = self.nodes.get(id)?;
        views.get(f)?.facts.nodes.get(n)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }
}

/// What a language's cross-file pass contributes.
#[derive(Debug, Default)]
pub struct Resolution {
    /// New nodes (externals, stubs, phantoms) appended after all per-file nodes.
    pub nodes: Vec<Node>,
    /// Cross-file edges, appended after all per-file edges in this order.
    pub edges: Vec<Edge>,
    /// `(from, to)`: delete node `from` and re-point every edge endpoint `from` to `to`.
    pub redirects: Vec<(String, String)>,
    /// `(id, key, value)`: set a field on an existing node (first node with that id) when the
    /// value depends on other files, e.g. Java `metadata.unresolved_calls`.
    pub attrs: Vec<(String, String, Attr)>,
}

pub fn resolve(files: Vec<(&SourceFile, FileFacts)>) -> Graph {
    let views = rename_collisions(files);

    let mut nodes_by_id = HashMap::new();
    let mut file_ids = HashMap::new();
    for (fi, v) in views.iter().enumerate() {
        for (ni, n) in v.facts.nodes.iter().enumerate() {
            nodes_by_id.entry(n.id.clone()).or_insert((fi, ni));
            if n.kind == NodeKind::File {
                file_ids.insert(v.file.rel.clone(), n.id.clone());
            }
        }
    }
    let index = Index {
        nodes: nodes_by_id,
        file_ids,
    };

    let passes: [fn(&[View], &Index) -> Resolution; 7] = [
        python::resolve,
        script::resolve,
        rust::resolve,
        go::resolve,
        java::resolve,
        swift::resolve,
        data::resolve,
    ];
    let results: Vec<Resolution> = std::thread::scope(|s| {
        let handles: Vec<_> = passes
            .iter()
            .map(|p| s.spawn(|| p(&views, &index)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("resolver panicked"))
            .collect()
    });

    drop(index);

    // Keep each file's edges in its own batch and fold them in order: the build never holds
    // one vector of every raw edge next to the folded result. Language facts are dropped here.
    let node_count = views.iter().map(|v| v.facts.nodes.len()).sum::<usize>()
        + results.iter().map(|r| r.nodes.len()).sum::<usize>();
    let mut nodes: Vec<Node> = Vec::with_capacity(node_count);
    let mut batches: Vec<Vec<Edge>> = Vec::with_capacity(views.len() + results.len());
    for v in views {
        nodes.extend(v.facts.nodes);
        batches.push(v.facts.edges);
    }
    let mut redirects: HashMap<String, String> = HashMap::new();
    let mut attrs: HashMap<String, Vec<(String, Attr)>> = HashMap::new();
    for r in results {
        nodes.extend(r.nodes);
        batches.push(r.edges);
        redirects.extend(r.redirects);
        for (id, key, value) in r.attrs {
            attrs.entry(id).or_default().push((key, value));
        }
    }
    // Each patch applies to the first node with its id (the one the build keeps).
    for n in &mut nodes {
        if attrs.is_empty() {
            break;
        }
        if let Some(extra) = attrs.remove(&n.id) {
            n.attrs.extend(extra);
        }
    }
    redirects.extend(merge_rationale(&nodes));

    if !redirects.is_empty() {
        nodes.retain(|n| !redirects.contains_key(&n.id));
    }
    assign_file_labels(&mut nodes);

    let target = |id: &str| -> Option<String> {
        // Follow chains (a stub redirected to a node that was itself redirected).
        let mut cur = redirects.get(id)?;
        for _ in 0..8 {
            match redirects.get(cur) {
                Some(next) => cur = next,
                None => break,
            }
        }
        Some(cur.clone())
    };
    if !redirects.is_empty() {
        for e in batches.iter_mut().flatten() {
            if let Some(t) = target(&e.source) {
                e.source = t;
            }
            if let Some(t) = target(&e.target) {
                e.target = t;
            }
        }
    }
    Graph::assemble_batches(nodes, batches)
}

/// §4.5: an id emitted by two or more files is rewritten for every emitter to
/// `slug(path with extension)_id`, plus `_sha1(path)[:6]` if that still collides. Externals are
/// shared and exempt.
fn rename_collisions(files: Vec<(&SourceFile, FileFacts)>) -> Vec<View<'_>> {
    let mut emitters: HashMap<&str, Vec<usize>> = HashMap::new();
    for (fi, (_, facts)) in files.iter().enumerate() {
        let mut local = HashSet::new();
        for n in &facts.nodes {
            if n.kind != NodeKind::External && local.insert(n.id.as_str()) {
                emitters.entry(n.id.as_str()).or_default().push(fi);
            }
        }
    }
    let mut per_file: Vec<HashMap<String, String>> = vec![HashMap::new(); files.len()];
    let mut rewritten: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    for (id, fs) in &emitters {
        if fs.len() < 2 {
            continue;
        }
        for &fi in fs {
            let new = format!("{}_{}", slug(&files[fi].0.rel), id);
            rewritten
                .entry(new)
                .or_default()
                .push((fi, (*id).to_string()));
        }
    }
    for (new, owners) in rewritten {
        let clash = owners.len() > 1 || emitters.contains_key(new.as_str());
        for (fi, old) in owners {
            let id = if clash {
                let digest = sha1_smol::Sha1::from(files[fi].0.rel.as_bytes())
                    .digest()
                    .to_string();
                format!("{new}_{}", &digest[..6])
            } else {
                new.clone()
            };
            per_file[fi].insert(old, id);
        }
    }

    files
        .into_iter()
        .zip(per_file)
        .map(|((file, mut facts), renames)| {
            if !renames.is_empty() {
                for n in &mut facts.nodes {
                    if n.kind != NodeKind::External {
                        if let Some(new) = renames.get(&n.id) {
                            n.id = new.clone();
                        }
                    }
                }
                for e in &mut facts.edges {
                    if let Some(new) = renames.get(&e.source) {
                        e.source = new.clone();
                    }
                    if let Some(new) = renames.get(&e.target) {
                        e.target = new.clone();
                    }
                }
            }
            View {
                file,
                facts,
                renames,
            }
        })
        .collect()
}

/// §8.1: rationale nodes with the same `source_file` and `label` merge into the first.
fn merge_rationale(nodes: &[Node]) -> Vec<(String, String)> {
    let mut first: HashMap<(&str, &str), &str> = HashMap::new();
    let mut out = Vec::new();
    for n in nodes.iter().filter(|n| n.kind == NodeKind::Rationale) {
        match first.get(&(n.source_file.as_str(), n.label.as_str())) {
            Some(keep) if *keep != n.id => out.push((n.id.clone(), (*keep).to_string())),
            Some(_) => {}
            None => {
                first.insert((n.source_file.as_str(), n.label.as_str()), n.id.as_str());
            }
        }
    }
    out
}

/// §5.1: a file's label is the shortest `/`-suffix of its path unique among all file nodes, or
/// the full path when no suffix is unique.
fn assign_file_labels(nodes: &mut [Node]) {
    let paths: Vec<String> = {
        let mut seen = HashSet::new();
        nodes
            .iter()
            .filter(|n| n.kind == NodeKind::File && seen.insert(n.source_file.clone()))
            .map(|n| n.source_file.clone())
            .collect()
    };
    let mut suffix_count: HashMap<&str, usize> = HashMap::new();
    for p in &paths {
        for s in suffixes(p) {
            *suffix_count.entry(s).or_default() += 1;
        }
    }
    let labels: HashMap<&str, &str> = paths
        .iter()
        .map(|p| {
            let label = suffixes(p).find(|s| suffix_count[s] == 1).unwrap_or(p);
            (p.as_str(), label)
        })
        .collect();
    let labels: HashMap<String, String> = labels
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for n in nodes.iter_mut().filter(|n| n.kind == NodeKind::File) {
        if let Some(l) = labels.get(&n.source_file) {
            n.label = l.clone();
        }
    }
}

/// `a/b/c.rs` → `c.rs`, `b/c.rs`, `a/b/c.rs` (shortest first).
fn suffixes(path: &str) -> impl Iterator<Item = &str> {
    let mut starts: Vec<usize> = path.match_indices('/').map(|(i, _)| i + 1).collect();
    starts.reverse();
    starts.push(0);
    starts.into_iter().map(move |i| &path[i..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::Loc;

    fn file_node(path: &str) -> Node {
        Node {
            id: crate::ids::file_id(path),
            label: String::new(),
            kind: NodeKind::File,
            file_type: "code".into(),
            source_file: path.into(),
            loc: Loc::Line(1),
            attrs: vec![],
        }
    }

    #[test]
    fn suffixes_shortest_first() {
        assert_eq!(
            suffixes("a/b/c.rs").collect::<Vec<_>>(),
            ["c.rs", "b/c.rs", "a/b/c.rs"]
        );
        assert_eq!(suffixes("c.rs").collect::<Vec<_>>(), ["c.rs"]);
    }

    #[test]
    fn file_labels_are_shortest_unique_suffix() {
        let mut nodes: Vec<Node> = [
            "crates/globset/src/pathutil.rs",
            "crates/ignore/src/pathutil.rs",
            "pkg/models.py",
            "b/c.py",
            "x/b/c.py",
        ]
        .iter()
        .map(|p| file_node(p))
        .collect();
        assign_file_labels(&mut nodes);
        let labels: Vec<_> = nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "globset/src/pathutil.rs",
                "ignore/src/pathutil.rs",
                "models.py",
                "b/c.py",
                "x/b/c.py"
            ]
        );
    }

    #[test]
    fn colliding_ids_are_prefixed_per_emitter() {
        use crate::lang::Lang;
        let sf = |rel: &str| SourceFile {
            rel: rel.into(),
            abs: rel.into(),
            lang: Lang::Python,
        };
        let (a, b, c, d) = (
            sf("a/util.py"),
            sf("a/util.ts"),
            sf("a-b/c.py"),
            sf("a_b/c.py"),
        );
        let facts = |rel: &str| FileFacts {
            nodes: vec![file_node(rel)],
            ..Default::default()
        };
        let views = rename_collisions(vec![
            (&a, facts("a/util.py")),
            (&b, facts("a/util.ts")),
            (&c, facts("a-b/c.py")),
            (&d, facts("a_b/c.py")),
        ]);
        let ids: Vec<_> = views.iter().map(|v| v.facts.nodes[0].id.clone()).collect();
        assert_eq!(ids[0], "a_util_py_a_util");
        assert_eq!(ids[1], "a_util_ts_a_util");
        assert!(
            ids[2].starts_with("a_b_c_py_a_b_c_") && ids[2].len() == "a_b_c_py_a_b_c_".len() + 6
        );
        assert_ne!(ids[2], ids[3]);
        assert_eq!(views[0].id("a_util"), "a_util_py_a_util");
    }
}

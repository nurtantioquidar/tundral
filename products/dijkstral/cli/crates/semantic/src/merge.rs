//! Replies into the graph, hyperedges, and the outputs that change (§7, §8 of
//! `docs/graphify-semantic.md`).
//!
//! Merge works on each reply's JSON object as the model wrote it (after `run`'s repair), not on
//! the typed [`crate::types::Reply`]: Graphify keeps a key the model wrote as `null` and omits one
//! it left out, and passes unknown keys through, so presence matters. [`MergeInput::from_reply`]
//! adapts a typed reply.
//!
//! What `merge` does, in order:
//!
//! 1. Nodes: drop those without an `id` and those attributed to a file not dispatched in this run
//!    (§7.1); of two with the same id, keep the first.
//! 2. Ids: rewrite `<stem>_<entity>` to `<file id>_<entity>` (§7.2); a rewrite that meets another
//!    reply node merges into it, later keys winning; a node whose id is a code node's is dropped
//!    in favour of the code node.
//! 3. Attributes: `file_type` outside the six types becomes `concept`, `community` is removed,
//!    `code` nodes whose label does not occur in their file get `verification: "unverified"`,
//!    nodes and edges without an `_origin` get `_origin: "semantic"`, fresh or cached.
//! 4. Edges: drop those naming an out-of-scope node, those with an endpoint that is neither a kept
//!    reply node nor a graph node, and those touching the code graph under a `source_file` not
//!    sent in this run; fill a missing `source_file` from the source node, else the target;
//!    normalise `confidence` and its score; fold the rest into the graph
//!    ([`Graph::fold_semantic`]).
//! 5. Hyperedges: members renamed, unknown members removed, sorted like nodes and links.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use dijkstral_core::facts::{Attr, Confidence, Edge, Loc, Node, NodeKind};
use dijkstral_core::graph::{attr_of, loc_of, Graph};
use dijkstral_core::ids;
use serde_json::{Map, Value};

use crate::types::{Reply, ReplyEdge, ReplyHyperedge, ReplyNode};

/// The `file_type`s kept as written (§7.2).
pub const FILE_TYPES: &[&str] = &["code", "document", "paper", "image", "concept", "rationale"];

/// One reply to merge.
#[derive(Debug, Clone, Default)]
pub struct MergeInput {
    /// The reply object: `nodes`, `edges`, `hyperedges` (other keys are ignored).
    pub reply: Map<String, Value>,
}

impl MergeInput {
    pub fn new(reply: Map<String, Value>) -> MergeInput {
        MergeInput { reply }
    }

    /// Parse a reply object from JSON text.
    pub fn parse(text: &str) -> anyhow::Result<MergeInput> {
        match serde_json::from_str(text)? {
            Value::Object(reply) => Ok(MergeInput { reply }),
            _ => anyhow::bail!("reply is not a JSON object"),
        }
    }

    /// A typed reply: every `Some` field becomes a key (a typed reply cannot say `null`).
    pub fn from_reply(reply: &Reply) -> MergeInput {
        fn put<T: serde::Serialize>(m: &mut Map<String, Value>, k: &str, v: &Option<T>) {
            if let Some(v) = v {
                m.insert(k.into(), serde_json::to_value(v).unwrap_or(Value::Null));
            }
        }
        let node = |n: &ReplyNode| {
            let mut m = Map::new();
            put(&mut m, "id", &n.id);
            put(&mut m, "label", &n.label);
            put(&mut m, "file_type", &n.file_type);
            put(&mut m, "source_file", &n.source_file);
            put(&mut m, "source_location", &n.source_location);
            put(&mut m, "source_url", &n.source_url);
            put(&mut m, "captured_at", &n.captured_at);
            put(&mut m, "author", &n.author);
            put(&mut m, "contributor", &n.contributor);
            put(&mut m, "rationale", &n.rationale);
            Value::Object(m)
        };
        let edge = |e: &ReplyEdge| {
            let mut m = Map::new();
            put(&mut m, "source", &e.source);
            put(&mut m, "target", &e.target);
            put(&mut m, "relation", &e.relation);
            put(&mut m, "confidence", &e.confidence);
            put(&mut m, "confidence_score", &e.confidence_score);
            put(&mut m, "source_file", &e.source_file);
            put(&mut m, "source_location", &e.source_location);
            put(&mut m, "weight", &e.weight);
            Value::Object(m)
        };
        let hyper = |h: &ReplyHyperedge| {
            let mut m = Map::new();
            put(&mut m, "id", &h.id);
            put(&mut m, "label", &h.label);
            m.insert(
                "nodes".into(),
                Value::Array(h.nodes.iter().cloned().map(Value::String).collect()),
            );
            put(&mut m, "relation", &h.relation);
            put(&mut m, "confidence", &h.confidence);
            put(&mut m, "confidence_score", &h.confidence_score);
            put(&mut m, "source_file", &h.source_file);
            Value::Object(m)
        };
        let mut m = Map::new();
        m.insert(
            "nodes".into(),
            Value::Array(reply.nodes.iter().map(node).collect()),
        );
        m.insert(
            "edges".into(),
            Value::Array(reply.edges.iter().map(edge).collect()),
        );
        m.insert(
            "hyperedges".into(),
            Value::Array(reply.hyperedges.iter().map(hyper).collect()),
        );
        MergeInput { reply: m }
    }

    fn items(&self, key: &str) -> impl Iterator<Item = &Map<String, Value>> {
        self.reply
            .get(key)
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter_map(Value::as_object)
    }
}

/// A file dispatched to the model in this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatched {
    /// The path replies must name as `source_file` (repo-relative; for a converted office file,
    /// the converted file's path).
    pub source_file: String,
    /// The text the evidence check searches (§7.2); `None` skips the check (images, papers).
    pub text: Option<String>,
}

impl Dispatched {
    pub fn new(source_file: impl Into<String>, text: Option<String>) -> Dispatched {
        Dispatched {
            source_file: source_file.into(),
            text,
        }
    }

    /// A dispatched file whose text is read from `path` for the evidence check.
    pub fn read(source_file: impl Into<String>, path: PathBuf) -> Dispatched {
        let text = std::fs::read(&path)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        Dispatched::new(source_file, text)
    }
}

/// What a merge did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergeStats {
    pub nodes_added: usize,
    pub edges_added: usize,
    pub hyperedges: usize,
    /// Distinct `source_file`s of nodes dropped as out of scope, in first-seen order.
    pub out_of_scope: Vec<String>,
    pub out_of_scope_nodes: usize,
    pub nodes_without_id: usize,
    /// Nodes whose id was already taken by a code node (the code node is kept).
    pub code_collisions: usize,
    /// Same id twice in the replies: `(id, kept label, dropped label)`.
    pub duplicates: Vec<(String, String, String)>,
    /// Edges dropped for an unknown endpoint.
    pub edges_dropped: usize,
    /// `code` nodes flagged `verification: "unverified"`.
    pub unverified: usize,
    /// Dispatched files that produced at least one node.
    pub files_with_nodes: usize,
}

impl MergeStats {
    /// Lines for the user, in Graphify's wording where it has one.
    pub fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, kept, dropped) in &self.duplicates {
            out.push(format!(
                "note: node '{id}' was extracted twice under different labels — keeping '{kept}', dropping '{dropped}'"
            ));
        }
        if self.out_of_scope_nodes > 0 {
            out.push(format!(
                "dropped {} out-of-scope node(s) attributed to file(s) not dispatched for extraction: {}",
                self.out_of_scope_nodes,
                self.out_of_scope.join(", ")
            ));
        }
        if self.unverified > 0 {
            out.push(format!(
                "{} semantic node(s) had no evidence in the source and were flagged verification=unverified",
                self.unverified
            ));
        }
        out
    }
}

/// `source_file` as a dispatched path: `./README.md` names `README.md`.
fn canonical(source_file: &str) -> &str {
    let mut s = source_file;
    while let Some(rest) = s.strip_prefix("./") {
        s = rest;
    }
    s
}

/// §7.2: `<stem>` and `<stem>_<entity>` (stem compared lower-cased) become the file id and
/// `<file id>_<slug(entity)>`; anything else is kept verbatim.
pub fn rewrite_id(id: &str, source_file: &str) -> String {
    let path = canonical(source_file);
    let file = ids::file_id(path);
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    let stem = ids::normalize(stem);
    if stem.is_empty() || id == file || id.starts_with(&format!("{file}_")) {
        return id.to_string();
    }
    if id == stem {
        return file;
    }
    let lower = id.to_lowercase();
    match lower.strip_prefix(&format!("{stem}_")) {
        // Lower-casing can change byte lengths; take the rest from the lower-cased id, which
        // `normalize` folds the same way.
        Some(rest) => {
            let rest = ids::normalize(rest);
            if rest.is_empty() {
                file
            } else {
                format!("{file}_{rest}")
            }
        }
        None => id.to_string(),
    }
}

/// Merge `inputs` (in order) into `graph`, whose nodes are the code graph's. `dispatched` lists
/// the files sent in this run, which bounds the nodes kept (§7.1).
pub fn merge(graph: &mut Graph, dispatched: &[Dispatched], inputs: &[MergeInput]) -> MergeStats {
    let mut stats = MergeStats::default();
    let scope: HashMap<&str, &Dispatched> = dispatched
        .iter()
        .map(|d| (d.source_file.as_str(), d))
        .collect();
    let code_ids: HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();

    // 1. Nodes in reply order: first of a raw id wins, out-of-scope ones dropped.
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut kept: Vec<&Map<String, Value>> = Vec::new();
    let mut out_of_scope: HashSet<String> = HashSet::new();
    // Raw ids of the out-of-scope nodes: edges naming them are dropped too (§7.1).
    let mut dropped_ids: HashSet<&str> = HashSet::new();
    for input in inputs {
        for n in input.items("nodes") {
            let Some(id) = n.get("id").and_then(Value::as_str) else {
                stats.nodes_without_id += 1;
                continue;
            };
            if let Some(sf) = n.get("source_file").and_then(Value::as_str) {
                if !scope.contains_key(canonical(sf)) {
                    stats.out_of_scope_nodes += 1;
                    dropped_ids.insert(id);
                    if out_of_scope.insert(sf.to_string()) {
                        stats.out_of_scope.push(sf.to_string());
                    }
                    continue;
                }
            }
            let label = n
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            match seen.get(id) {
                Some(first) => {
                    if *first != label {
                        stats
                            .duplicates
                            .push((id.to_string(), first.clone(), label));
                    }
                }
                None => {
                    seen.insert(id.to_string(), label);
                    kept.push(n);
                }
            }
        }
    }

    // 2–3. Rename, convert, and collapse rename collisions (later keys win).
    let mut rename: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut nodes: HashMap<String, Node> = HashMap::new();
    let mut files_with_nodes: HashSet<String> = HashSet::new();
    for n in kept {
        let raw = n["id"].as_str().unwrap_or_default();
        let sf = n.get("source_file").and_then(Value::as_str);
        let id = sf.map_or_else(|| raw.to_string(), |sf| rewrite_id(raw, sf));
        rename.insert(raw.to_string(), id.clone());
        if let Some(sf) = sf {
            files_with_nodes.insert(canonical(sf).to_string());
        }
        if code_ids.contains(&id) {
            stats.code_collisions += 1;
            continue;
        }
        let mut node = to_node(&id, n);
        if node.file_type == "code" {
            let evidence = sf
                .and_then(|sf| scope.get(canonical(sf)))
                .and_then(|d| d.text.as_deref());
            if let Some(text) = evidence {
                if !has_evidence(&node.label, text) {
                    node.attrs
                        .push(("verification".into(), Attr::Str("unverified".into())));
                    stats.unverified += 1;
                }
            }
        }
        match nodes.get_mut(&id) {
            Some(existing) => merge_node(existing, node),
            None => {
                order.push(id.clone());
                nodes.insert(id, node);
            }
        }
    }
    stats.files_with_nodes = files_with_nodes.len();
    stats.nodes_added = order.len();
    let source_files: HashMap<String, Option<String>> = graph
        .nodes
        .iter()
        .map(|n| (n.id.clone(), node_source_file(n)))
        .chain(
            nodes
                .iter()
                .map(|(id, n)| (id.clone(), node_source_file(n))),
        )
        .collect();
    for id in order {
        if let Some(n) = nodes.remove(&id) {
            graph.nodes.push(n);
        }
    }

    // 4. Edges.
    let renamed = |id: &str| rename.get(id).cloned().unwrap_or_else(|| id.to_string());
    let mut edges = Vec::new();
    for input in inputs {
        for e in input.items("edges") {
            let (Some(s), Some(t)) = (
                e.get("source").and_then(Value::as_str),
                e.get("target").and_then(Value::as_str),
            ) else {
                stats.edges_dropped += 1;
                continue;
            };
            if dropped_ids.contains(s) || dropped_ids.contains(t) {
                stats.edges_dropped += 1;
                continue;
            }
            let (s, t) = (renamed(s), renamed(t));
            let (Some(source_sf), Some(target_sf)) = (source_files.get(&s), source_files.get(&t))
            else {
                stats.edges_dropped += 1;
                continue;
            };
            // An edge the reply attributes to a file not sent in this run may not touch the code
            // graph.
            let given = e.get("source_file").and_then(Value::as_str);
            if given.is_some_and(|sf| !scope.contains_key(canonical(sf)))
                && (code_ids.contains(&s) || code_ids.contains(&t))
            {
                stats.edges_dropped += 1;
                continue;
            }
            // A missing `source_file` comes from the source node, else from the target.
            let fallback = source_sf.as_deref().or(target_sf.as_deref());
            edges.push(to_edge(s, t, e, fallback));
        }
    }
    let before = graph.edges.len();
    graph.fold_semantic(edges);
    stats.edges_added = graph.edges.len() - before;

    // 5. Hyperedges.
    let known: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut with_id = Vec::new();
    let mut without_id = Vec::new();
    for input in inputs {
        for h in input.items("hyperedges") {
            // Like nodes, a hyperedge attributed to a file not dispatched is dropped.
            if let Some(sf) = h.get("source_file").and_then(Value::as_str) {
                if !scope.contains_key(canonical(sf)) {
                    continue;
                }
            }
            let mut h = h.clone();
            if let Some(Value::Array(members)) = h.get("nodes") {
                let members: Vec<Value> = members
                    .iter()
                    .filter_map(Value::as_str)
                    .map(renamed)
                    .filter(|m| known.contains(m.as_str()))
                    .map(Value::String)
                    .collect();
                // One member left is kept; none drops the hyperedge.
                if members.is_empty() {
                    continue;
                }
                h.insert("nodes".into(), Value::Array(members));
            }
            if h.contains_key("id") {
                with_id.push(Value::Object(h));
            } else {
                without_id.push(Value::Object(h));
            }
        }
    }
    stats.hyperedges = with_id.len() + without_id.len();
    graph.hyperedges.extend(with_id);
    graph.hyperedges.extend(without_id);
    graph.sort_hyperedges();
    stats
}

/// The label occurs in the file (case-insensitively; the exact rule is open, §12.2 item 5).
fn has_evidence(label: &str, text: &str) -> bool {
    let label = label.trim();
    !label.is_empty() && text.to_lowercase().contains(&label.to_lowercase())
}

fn node_source_file(n: &Node) -> Option<String> {
    if n.attrs
        .iter()
        .any(|(k, v)| k == "source_file" && *v == Attr::Absent)
    {
        None
    } else {
        Some(n.source_file.clone())
    }
}

/// `{**existing, **new}` on two reply nodes that ended with the same id.
fn merge_node(existing: &mut Node, new: Node) {
    let mut attrs = std::mem::take(&mut existing.attrs);
    for (k, v) in new.attrs {
        match attrs.iter_mut().find(|(ek, _)| *ek == k) {
            Some(slot) => slot.1 = v,
            None => attrs.push((k, v)),
        }
    }
    *existing = Node { attrs, ..new };
}

fn to_node(id: &str, n: &Map<String, Value>) -> Node {
    let mut attrs: Vec<(String, Attr)> = Vec::new();
    let text = |k: &str| n.get(k).and_then(Value::as_str).map(str::to_string);
    // Required strings: absent → not written; present but not a string → written verbatim.
    let field = |k: &str, attrs: &mut Vec<(String, Attr)>| -> String {
        match n.get(k) {
            None => {
                attrs.push((k.to_string(), Attr::Absent));
                String::new()
            }
            Some(Value::String(s)) => s.clone(),
            Some(other) => {
                attrs.push((k.to_string(), attr_of(other)));
                String::new()
            }
        }
    };
    let label = field("label", &mut attrs);
    let source_file = field("source_file", &mut attrs);
    let file_type = text("file_type")
        .filter(|t| FILE_TYPES.contains(&t.as_str()))
        .unwrap_or_else(|| "concept".into());
    let loc = match n.get("source_location") {
        None => Loc::Absent,
        Some(v) => {
            let (loc, raw) = loc_of(Some(v));
            if let Some(raw) = raw {
                attrs.push(("source_location".into(), raw));
            }
            loc
        }
    };
    for (k, v) in n {
        if !matches!(
            k.as_str(),
            "id" | "label" | "file_type" | "source_file" | "source_location" | "community"
        ) {
            attrs.push((k.clone(), attr_of(v)));
        }
    }
    // 0.9.97 fills `_origin` on every reply node, as 0.9.86 did on cached ones (§7.2).
    if !n.contains_key("_origin") {
        attrs.push(("_origin".into(), Attr::Str("semantic".into())));
    }
    Node {
        id: id.to_string(),
        label,
        kind: NodeKind::Def,
        file_type,
        source_file,
        loc,
        attrs,
    }
}

/// A given `confidence_score` read as Python's `float()` would (numbers, booleans, numeric
/// strings); anything unreadable, negative or not finite scores 1.0 in the clustered graph (§7.2).
fn score_of(v: &Value) -> f64 {
    let x = match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    x.map_or(1.0, usable_score)
}

fn usable_score(x: f64) -> f64 {
    if x.is_finite() && x >= 0.0 {
        x
    } else {
        1.0
    }
}

fn to_edge(
    source: String,
    target: String,
    e: &Map<String, Value>,
    fallback_source_file: Option<&str>,
) -> Edge {
    let mut attrs: Vec<(String, Attr)> = Vec::new();
    let relation = match e.get("relation") {
        None => {
            attrs.push(("relation".into(), Attr::Absent));
            String::new()
        }
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            attrs.push(("relation".into(), attr_of(other)));
            String::new()
        }
    };
    // §7.2: a number is the score the model meant; a string is matched without case and
    // anything else `INFERRED`; a falsy value given with a score is `INFERRED` too. What is left
    // (`true`, or `null`/`false` with no score) is kept and scored 1.0.
    let has_score = e.contains_key("confidence_score");
    let mut moved_score = None;
    let confidence = match e.get("confidence") {
        None => {
            attrs.push(("confidence".into(), Attr::Absent));
            Confidence::Extracted
        }
        Some(Value::Number(n)) => {
            if !has_score {
                moved_score = n.as_f64();
            }
            Confidence::Inferred
        }
        Some(Value::String(s)) => match s.to_uppercase().as_str() {
            "EXTRACTED" => Confidence::Extracted,
            "AMBIGUOUS" => Confidence::Ambiguous,
            _ => Confidence::Inferred,
        },
        Some(Value::Null | Value::Bool(false)) if has_score => Confidence::Inferred,
        Some(other) => {
            attrs.push(("confidence".into(), attr_of(other)));
            Confidence::Extracted
        }
    };
    let source_file = match e.get("source_file") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            attrs.push(("source_file".into(), attr_of(other)));
            String::new()
        }
        None => match fallback_source_file {
            Some(sf) => sf.to_string(),
            None => {
                attrs.push(("source_file".into(), Attr::Absent));
                String::new()
            }
        },
    };
    let loc = match e.get("source_location") {
        None => Loc::Absent,
        Some(v) => {
            let (loc, raw) = loc_of(Some(v));
            if let Some(raw) = raw {
                attrs.push(("source_location".into(), raw));
            }
            loc
        }
    };
    let context = match e.get("context") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => {
            attrs.push(("context".into(), attr_of(other)));
            None
        }
        None => None,
    };
    for (k, v) in e {
        if !matches!(
            k.as_str(),
            "source"
                | "target"
                | "relation"
                | "confidence"
                | "confidence_score"
                | "source_file"
                | "source_location"
                | "weight"
                | "context"
        ) {
            attrs.push((k.clone(), attr_of(v)));
        }
    }
    if !e.contains_key("_origin") {
        attrs.push(("_origin".into(), Attr::Str("semantic".into())));
    }
    let number = |k: &str| e.get(k).filter(|v| v.is_number()).and_then(Value::as_f64);
    let score = match e.get("confidence_score") {
        Some(v) => Some(score_of(v)),
        None => moved_score.map(usable_score),
    };
    Edge {
        source,
        target,
        relation,
        confidence,
        score,
        context,
        source_file,
        loc,
        weight: number("weight"),
        attrs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_rewritten_to_the_file_id() {
        let g = "docs/glossary.txt";
        assert_eq!(rewrite_id("glossary", g), "docs_glossary");
        assert_eq!(rewrite_id("glossary_x", g), "docs_glossary_x");
        assert_eq!(rewrite_id("Glossary_Z", g), "docs_glossary_z");
        assert_eq!(rewrite_id("docs_glossary_y", g), "docs_glossary_y");
        assert_eq!(rewrite_id("x_glossary", g), "x_glossary");
        assert_eq!(rewrite_id("glossaryx", g), "glossaryx");
        assert_eq!(rewrite_id("guide_slug", g), "guide_slug");
        assert_eq!(
            rewrite_id("Docs Guide/Operator Guide!", "docs/guide.md"),
            "Docs Guide/Operator Guide!"
        );
        assert_eq!(
            rewrite_id("queue_cfg", "config/queue.yaml"),
            "config_queue_cfg"
        );
        assert_eq!(
            rewrite_id(
                "capacity_e7fd8f6a_sheet",
                "graphify-out/converted/capacity_e7fd8f6a.md"
            ),
            "graphify_out_converted_capacity_e7fd8f6a_sheet"
        );
        assert_eq!(
            rewrite_id(
                "capacity_sheet",
                "graphify-out/converted/capacity_e7fd8f6a.md"
            ),
            "capacity_sheet"
        );
        assert_eq!(rewrite_id("readme_job", "README.md"), "readme_job");
        assert_eq!(rewrite_id("dot_readme", "./README.md"), "dot_readme");
    }
}

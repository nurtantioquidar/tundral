//! The assembled graph and its Graphify-compatible `graph.json` encoding (`docs/graphify-format.md`
//! §2, §8).
//!
//! Graphify stores a simple undirected graph: one link per unordered node pair. Edges are folded in
//! emission order (per-file edges in path order, then the cross-file pass) by the rank rules of
//! §8.2. Direction is kept as written by the surviving edge.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufWriter, Write};

use rayon::prelude::*;
use serde_json::{Map, Value};
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;

use crate::facts::{Attr, Confidence, Edge, Loc, Node, NodeKind};

pub const SCHEMA_VERSION: u32 = 1;
/// The Graphify version whose output format this crate reproduces.
pub const COMPAT_VERSION: &str = "0.9.86";

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Edges dropped because an endpoint does not exist.
    pub dangling: usize,
    /// Group relationships from the semantic pass (`docs/graphify-semantic.md` §7.4), written
    /// as the top-level `hyperedges` and, when there are any, as `graph.hyperedges`.
    pub hyperedges: Vec<Value>,
}

/// Collapse precedence (§8.2). Unobserved relations rank with the structural ones.
fn rank(relation: &str) -> u8 {
    match relation {
        "references" => 10,
        "uses" => 20,
        "calls" | "case_of" | "dynamic_import" => 30,
        "references_constant" => 35,
        "uses_static_prop" => 36,
        // SQL (§45.5): a procedure's later read of a table it writes leaves `writes_to`.
        "reads_from" => 37,
        "instantiates" => 41,
        "imports" => 45,
        "imports_from" | "indirect_call" | "re_exports" => 50,
        _ => 40, // contains, method, inherits, implements, rationale_for, embeds, defines, …
    }
}

/// Relations that, surviving, replace a lower edge whole and take nothing from one they absorb
/// but a call's `context` (§8.2).
fn is_structural(relation: &str) -> bool {
    matches!(
        relation,
        "contains" | "method" | "inherits" | "implements" | "defines" | "embeds"
    )
}

/// `{**base, **over}`: every field `over` carries wins; fields only `base` has survive.
fn merge_over(base: &Edge, over: &Edge) -> Edge {
    let mut attrs = base.attrs.clone();
    for (k, v) in &over.attrs {
        match attrs.iter_mut().find(|(bk, _)| bk == k) {
            Some(slot) => slot.1 = v.clone(),
            None => attrs.push((k.clone(), v.clone())),
        }
    }
    Edge {
        source: over.source.clone(),
        target: over.target.clone(),
        relation: over.relation.clone(),
        confidence: over.confidence,
        score: over.score.or(base.score),
        context: over.context.clone().or_else(|| base.context.clone()),
        source_file: over.source_file.clone(),
        loc: if over.loc == Loc::Absent {
            base.loc
        } else {
            over.loc
        },
        weight: over.weight.or(base.weight),
        attrs,
    }
}

/// Fold `new` into the link `old` already holding the same unordered pair, whose first edge ran
/// from `first_source`.
///
/// Orientation is judged against that first edge, not against the current survivor: after a
/// reversed edge won the pair, a later edge in the first edge's direction still merges into it
/// (Kotlin probe: `contains A→Z`, `implements Z→A`, `calls A→Z` gives `implements` with
/// `context: "call"`). A self-loop always counts as reversed, so a repeated one keeps the first.
fn fold(old: &mut Edge, new: Edge, first_source: &str) {
    let (ro, rn) = (rank(&old.relation), rank(&new.relation));
    if new.source == first_source && new.source != new.target {
        if rn >= ro {
            *old = if is_structural(&new.relation) {
                new
            } else {
                merge_over(old, &new)
            };
        } else if is_structural(&old.relation) {
            // A structural link absorbs only a call's context (none when the call has none).
            if new.relation == "calls" && old.context.is_none() {
                old.context = new.context;
            }
        } else {
            *old = merge_over(&new, old);
        }
    } else if rn > ro
        || (rn == ro
            && if new.relation == old.relation {
                new.source < old.source
            } else {
                new.source > old.source
            })
    {
        // Equal ranks: of two links of one relation the smaller source survives, of two
        // relations the larger one (a nested class inheriting its outer class keeps `contains`
        // or `inherits` by id order; `re_exports` and `imports_from` likewise).
        *old = new;
    }
}

/// The semantic fold's tier (see [`Graph::fold_semantic`]).
fn semantic_tier(relation: &str) -> u8 {
    match relation {
        "mentions" => 0,
        r => rank(r).min(40),
    }
}

/// `{**base, **over}` for edges that may lack `relation`, `confidence` or `source_file`
/// ([`Attr::Absent`]): a key `over` lacks keeps `base`'s value.
fn merge_dict(base: &Edge, over: &Edge) -> Edge {
    let mut e = merge_over(base, over);
    if over.loc != Loc::Absent {
        // A located `over` replaces a free-text location `base` carried as an attribute.
        if !over.attrs.iter().any(|(k, _)| k == "source_location") {
            e.attrs.retain(|(k, _)| k != "source_location");
        }
    }
    fn attr<'a>(x: &'a Edge, key: &str) -> Option<&'a Attr> {
        x.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    for key in ["relation", "confidence", "source_file"] {
        match attr(over, key) {
            // `over`'s field is authoritative: drop a raw value `base` carried.
            None => {
                e.attrs.retain(|(k, _)| k != key);
                continue;
            }
            Some(Attr::Absent) if attr(base, key) != Some(&Attr::Absent) => {}
            Some(_) => continue,
        }
        match key {
            "relation" => e.relation = base.relation.clone(),
            "confidence" => e.confidence = base.confidence,
            _ => e.source_file = base.source_file.clone(),
        }
        e.attrs.retain(|(k, _)| k != key);
        if let Some(kept) = base.attrs.iter().find(|(k, _)| k == key) {
            e.attrs.push(kept.clone());
        }
    }
    e
}

fn fold_semantic_edge(old: &mut Edge, new: Edge, first_source: &str) {
    let (to, tn) = (semantic_tier(&old.relation), semantic_tier(&new.relation));
    if new.source == first_source && new.source != new.target {
        if tn >= to {
            *old = merge_dict(old, &new);
        } else if is_structural(&old.relation) {
            if new.relation == "calls" && old.context.is_none() {
                old.context = new.context;
            }
        } else {
            *old = merge_dict(&new, old);
        }
    } else if tn > to
        || (tn == to
            && if new.relation == old.relation {
                new.source < old.source
            } else {
                new.source > old.source
            })
    {
        *old = merge_dict(old, &new);
    }
}

impl Graph {
    /// Merge duplicate node ids (first wins), drop dangling edges, and fold parallel edges to one
    /// link per unordered pair in the given order.
    pub fn assemble(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
        Graph::assemble_batches(nodes, vec![edges])
    }

    /// `assemble` over edge batches (one per file, then the cross-file passes), folded in order.
    /// A counting pass sizes the result exactly and each batch is freed once folded, so a large
    /// build never holds every raw edge next to a doubling result vector.
    pub fn assemble_batches(nodes: Vec<Node>, batches: Vec<Vec<Edge>>) -> Graph {
        let nodes: Vec<Node> = {
            let mut seen = HashSet::with_capacity(nodes.len());
            let keep: Vec<bool> = nodes.iter().map(|n| seen.insert(n.id.as_str())).collect();
            drop(seen);
            nodes
                .into_iter()
                .zip(keep)
                .filter_map(|(n, k)| k.then_some(n))
                .collect()
        };

        // Fold on integer node indices: no per-edge key strings are allocated.
        let index: HashMap<&str, u32> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i as u32))
            .collect();
        let pair_of = |e: &Edge| -> Option<(u32, u32)> {
            let s = *index.get(e.source.as_str())?;
            let t = *index.get(e.target.as_str())?;
            Some(if s <= t { (s, t) } else { (t, s) })
        };
        // The source of each pair's first edge, which fixes the pair's orientation (`fold`).
        let mut first: Vec<u32> = Vec::new();

        let mut slot: HashMap<(u32, u32), u32> = HashMap::new();
        for e in batches.iter().flatten() {
            if let Some(pair) = pair_of(e) {
                let next = slot.len() as u32;
                slot.entry(pair).or_insert(next);
            }
        }
        let mut kept: Vec<Option<Edge>> = Vec::with_capacity(slot.len());
        kept.resize_with(slot.len(), || None);
        first.resize(slot.len(), 0);
        let mut dangling = 0;
        for batch in batches {
            for e in batch {
                let Some(pair) = pair_of(&e) else {
                    dangling += 1;
                    continue;
                };
                let i = slot[&pair] as usize;
                match &mut kept[i] {
                    None => {
                        first[i] = index[e.source.as_str()];
                        kept[i] = Some(e);
                    }
                    Some(old) => fold(old, e, &nodes[first[i] as usize].id),
                }
            }
        }
        drop(first);
        drop(slot);
        drop(index);
        Graph {
            nodes,
            // Every counted pair received at least one edge; `map` collects in place.
            edges: kept
                .into_iter()
                .map(|e| e.expect("every counted pair has an edge"))
                .collect(),
            dangling,
            hyperedges: Vec::new(),
        }
    }

    /// Fold the semantic pass's edges, in reply order, into the built graph
    /// (`docs/graphify-semantic.md` §7.3). Edges with an unknown endpoint are counted as dangling.
    ///
    /// Probed against Graphify (`extract` with crafted replies on reply and code pairs,
    /// `tests/fixtures/semantic/fold_probe*`), in two steps:
    ///
    /// 1. Reply edges with the same source and target are reduced to one: taken in relation-name
    ///    order (stable), each replaces the survivor when its tier is at least the survivor's,
    ///    merged as `{**survivor, **edge}`, and is dropped otherwise. Tiers: `mentions` lowest,
    ///    then `references`, `uses`, the `calls` family, the PHP relations, and one tier for
    ///    every other relation, model-invented ones included (so `semantically_similar_to` keeps
    ///    a later `implements`, and `related_to` beats `imports`).
    /// 2. The survivors fold into the code graph's links like code edges, on the same tiers:
    ///    in the pair's orientation a new edge of at least the link's tier wins merged
    ///    (`{**link, **edge}`), a structural link takes only a call's context, any other link
    ///    takes the edge's keys it lacks (`{**edge, **link}`); against the orientation the higher
    ///    tier wins, equal tiers keeping the smaller source for one relation and the larger for
    ///    two, the winner merged over the loser.
    ///
    /// The pair's orientation is the current link's, which is the first edge's unless a reversed
    /// code edge won the pair.
    pub fn fold_semantic(&mut self, edges: Vec<Edge>) {
        // Step 1: one edge per (source, target), in order of first appearance.
        let mut slots: HashMap<(String, String), usize> = HashMap::new();
        let mut groups: Vec<Vec<Edge>> = Vec::new();
        for e in edges {
            let key = (e.source.clone(), e.target.clone());
            match slots.get(&key) {
                Some(&i) => groups[i].push(e),
                None => {
                    slots.insert(key, groups.len());
                    groups.push(vec![e]);
                }
            }
        }
        drop(slots);
        let edges = groups.into_iter().map(|mut group| {
            group.sort_by(|a, b| a.relation.cmp(&b.relation));
            let mut it = group.into_iter();
            let mut cur = it.next().expect("groups are never empty");
            for e in it {
                if semantic_tier(&e.relation) >= semantic_tier(&cur.relation) {
                    cur = merge_dict(&cur, &e);
                }
            }
            cur
        });

        // Step 2: into the links.
        let index: HashMap<String, u32> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i as u32))
            .collect();
        let pair_of = |e: &Edge| -> Option<(u32, u32)> {
            let s = *index.get(e.source.as_str())?;
            let t = *index.get(e.target.as_str())?;
            Some(if s <= t { (s, t) } else { (t, s) })
        };
        let mut slot: HashMap<(u32, u32), usize> = HashMap::with_capacity(self.edges.len());
        for (i, e) in self.edges.iter().enumerate() {
            if let Some(pair) = pair_of(e) {
                slot.entry(pair).or_insert(i);
            }
        }
        for e in edges {
            let Some(pair) = pair_of(&e) else {
                self.dangling += 1;
                continue;
            };
            match slot.get(&pair) {
                None => {
                    slot.insert(pair, self.edges.len());
                    self.edges.push(e);
                }
                Some(&i) => {
                    let old = &mut self.edges[i];
                    let first = old.source.clone();
                    fold_semantic_edge(old, e, &first);
                }
            }
        }
    }

    /// Order hyperedges as Graphify writes them: by `json.dumps(h, sort_keys=True)`, like nodes
    /// and links (`docs/graphify-semantic.md` §7.4; this is also why ones without an `id`
    /// usually come last).
    pub fn sort_hyperedges(&mut self) {
        self.hyperedges.sort_by_cached_key(python_dumps);
    }

    /// Clustered-shape `graph.json` (§2.1): compact, ASCII-escaped, no trailing newline, arrays in
    /// Graphify's sort order. `community` holds every node's community (by node index) when the
    /// build clustered; the key is absent otherwise.
    ///
    /// Memory stays bounded on large graphs: only the per-item sort keys are held while sorting,
    /// and items are rendered and written in batches rather than built into one document.
    pub fn write_graphify_json<W: Write>(
        &self,
        w: W,
        built_at_commit: Option<&str>,
        community: Option<&[u32]>,
    ) -> io::Result<()> {
        self.write_graphify_json_ordered(w, built_at_commit, community)
            .map(drop)
    }

    /// [`Graph::write_graphify_json`], returning the order it wrote: node and edge indices in
    /// `graph.json` array order, for exporters that must follow the file.
    pub fn write_graphify_json_ordered<W: Write>(
        &self,
        w: W,
        built_at_commit: Option<&str>,
        community: Option<&[u32]>,
    ) -> io::Result<(Vec<u32>, Vec<u32>)> {
        let comm = |i: usize| community.map(|c| c[i]);
        let node_order = sorted_order(&self.nodes, |i, n| sort_key(node_map(n, comm(i))));
        let link_order = sorted_order(&self.edges, |_, e| sort_key(edge_map(e)));

        // Hyperedges keep their reply key order (§7.4); `graph` lists them first when present.
        let hyperedges = ascii_escape(serde_json::to_string(&self.hyperedges).unwrap());
        let mut w = BufWriter::with_capacity(1 << 20, w);
        w.write_all(b"{\"directed\":false,\"multigraph\":false,\"graph\":{")?;
        if !self.hyperedges.is_empty() {
            w.write_all(b"\"hyperedges\":")?;
            w.write_all(hyperedges.as_bytes())?;
            w.write_all(b",")?;
        }
        w.write_all(b"\"schema_version\":")?;
        w.write_all(SCHEMA_VERSION.to_string().as_bytes())?;
        w.write_all(b",\"graphify_version\":\"")?;
        w.write_all(COMPAT_VERSION.as_bytes())?;
        w.write_all(b"\"},\"nodes\":[")?;
        write_items(&mut w, &self.nodes, &node_order, |i, n| {
            compact(node_map(n, comm(i)))
        })?;
        w.write_all(b"],\"links\":[")?;
        write_items(&mut w, &self.edges, &link_order, |_, e| {
            compact(edge_map(e))
        })?;
        w.write_all(b"],\"hyperedges\":")?;
        w.write_all(hyperedges.as_bytes())?;
        if let Some(c) = built_at_commit {
            w.write_all(b",\"built_at_commit\":")?;
            w.write_all(ascii_escape(serde_json::to_string(c).unwrap()).as_bytes())?;
        }
        w.write_all(b"}")?;
        w.flush()?;
        Ok((node_order, link_order))
    }

    /// Load a clustered-shape `graph.json` (Graphify's or ours) with nodes and links in file order,
    /// as Graphify's `cluster-only` does. Derived fields (`community`, `community_name`,
    /// `norm_label`) are dropped; they are recomputed on output.
    pub fn from_graphify_json(doc: &Value) -> Result<Graph, String> {
        let nodes = doc["nodes"]
            .as_array()
            .ok_or("graph.json has no nodes array")?
            .iter()
            .map(|n| {
                let obj = n.as_object().ok_or("node is not an object")?;
                let s = |k: &str| obj.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let (loc, raw_loc) = loc_of(obj.get("source_location"));
                let mut node = Node {
                    id: s("id"),
                    label: s("label"),
                    kind: NodeKind::Def,
                    file_type: s("file_type"),
                    source_file: s("source_file"),
                    loc,
                    attrs: Vec::new(),
                };
                if let Some(raw) = raw_loc {
                    node.attrs.push(("source_location".into(), raw));
                }
                keep_non_strings(obj, &["file_type", "source_file"], &mut node.attrs);
                mark_absent(obj, &["label", "source_file"], &mut node.attrs);
                for (k, v) in obj {
                    if !matches!(
                        k.as_str(),
                        "id" | "label"
                            | "file_type"
                            | "source_file"
                            | "source_location"
                            | "community"
                            | "community_name"
                            | "norm_label"
                    ) {
                        node.attrs.push((k.clone(), attr_of(v)));
                    }
                }
                Ok(node)
            })
            .collect::<Result<Vec<Node>, String>>()?;
        let links = doc
            .get("links")
            .or_else(|| doc.get("edges"))
            .and_then(Value::as_array)
            .ok_or("graph.json has no links array")?;
        let edges = links
            .iter()
            .map(|e| {
                let obj = e.as_object().ok_or("link is not an object")?;
                let s = |k: &str| obj.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let confidence = match obj.get("confidence").and_then(Value::as_str) {
                    Some("INFERRED") => Confidence::Inferred,
                    Some("AMBIGUOUS") => Confidence::Ambiguous,
                    _ => Confidence::Extracted,
                };
                let (loc, raw_loc) = loc_of(obj.get("source_location"));
                let mut edge = Edge {
                    source: s("source"),
                    target: s("target"),
                    relation: s("relation"),
                    confidence,
                    score: obj.get("confidence_score").and_then(Value::as_f64),
                    context: obj
                        .get("context")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    source_file: s("source_file"),
                    loc,
                    weight: obj.get("weight").and_then(Value::as_f64),
                    attrs: Vec::new(),
                };
                if let Some(raw) = raw_loc {
                    edge.attrs.push(("source_location".into(), raw));
                }
                keep_non_strings(obj, &["source_file"], &mut edge.attrs);
                mark_absent(
                    obj,
                    &["relation", "confidence", "source_file"],
                    &mut edge.attrs,
                );
                // A semantic edge's confidence outside the three values is kept verbatim (§7.3).
                if let Some(c) = obj.get("confidence").filter(|c| {
                    !matches!(
                        c.as_str(),
                        Some("EXTRACTED") | Some("INFERRED") | Some("AMBIGUOUS")
                    )
                }) {
                    edge.attrs.push(("confidence".into(), attr_of(c)));
                }
                for (k, v) in obj {
                    if !matches!(
                        k.as_str(),
                        "source"
                            | "target"
                            | "relation"
                            | "confidence"
                            | "confidence_score"
                            | "context"
                            | "source_file"
                            | "source_location"
                            | "weight"
                    ) {
                        edge.attrs.push((k.clone(), attr_of(v)));
                    }
                }
                Ok(edge)
            })
            .collect::<Result<Vec<Edge>, String>>()?;
        let known: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        let (edges, dropped): (Vec<Edge>, Vec<Edge>) = edges
            .into_iter()
            .partition(|e| known.contains(e.source.as_str()) && known.contains(e.target.as_str()));
        drop(known);
        let hyperedges = doc
            .get("hyperedges")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(Graph {
            nodes,
            edges,
            dangling: dropped.len(),
            hyperedges,
        })
    }

    /// The whole document as a string (tests and small graphs).
    pub fn to_graphify_json(
        &self,
        built_at_commit: Option<&str>,
        community: Option<&[u32]>,
    ) -> String {
        let mut out = Vec::new();
        self.write_graphify_json(&mut out, built_at_commit, community)
            .unwrap();
        String::from_utf8(out).unwrap()
    }
}

/// Indices of `items` ordered by `key`, ties broken by position (keys are unique in practice).
fn sorted_order<T: Sync>(items: &[T], key: impl Fn(usize, &T) -> String + Sync) -> Vec<u32> {
    let mut keyed: Vec<(Box<str>, u32)> = items
        .par_iter()
        .enumerate()
        .map(|(i, x)| (key(i, x).into_boxed_str(), i as u32))
        .collect();
    keyed.par_sort_unstable();
    keyed.into_iter().map(|(_, i)| i).collect()
}

/// Render items in `order`, comma-separated and ASCII-escaped, a bounded batch at a time.
fn write_items<T: Sync, W: Write>(
    w: &mut W,
    items: &[T],
    order: &[u32],
    render: impl Fn(usize, &T) -> String + Sync,
) -> io::Result<()> {
    const BATCH: usize = 1 << 16;
    const CHUNK: usize = 1 << 10;
    for (b, batch) in order.chunks(BATCH).enumerate() {
        let parts: Vec<String> = batch
            .par_chunks(CHUNK)
            .map(|chunk| {
                let mut s = String::new();
                for (i, &idx) in chunk.iter().enumerate() {
                    if i > 0 {
                        s.push(',');
                    }
                    s.push_str(&render(idx as usize, &items[idx as usize]));
                }
                ascii_escape(s)
            })
            .collect();
        for (i, part) in parts.iter().enumerate() {
            if b > 0 || i > 0 {
                w.write_all(b",")?;
            }
            w.write_all(part.as_bytes())?;
        }
    }
    Ok(())
}

/// `lower(strip_combining(NFKD(label)))` (§5.4): marks with a non-zero canonical combining class are
/// dropped, like Python's `unicodedata.combining`; spacing vowel signs (class 0) stay.
pub fn norm_label(label: &str) -> String {
    let stripped: String = label
        .nfkd()
        .filter(|&c| canonical_combining_class(c) == 0)
        .collect();
    stripped.to_lowercase()
}

fn attr_value(a: &Attr) -> Value {
    match a {
        Attr::Str(s) => Value::String(s.clone()),
        Attr::Int(i) => Value::from(*i),
        Attr::Float(f) => Value::from(*f),
        Attr::Bool(b) => Value::Bool(*b),
        Attr::Null => Value::Null,
        Attr::Json(raw) => serde_json::from_str(raw).unwrap_or(Value::Null),
        // Never written: `apply_attrs` removes the key instead.
        Attr::Absent => Value::Null,
    }
}

/// Insert `attrs` over the fields already in `head` and `rest`; an [`Attr::Absent`] removes its
/// key from either.
fn apply_attrs(
    head: &mut Vec<(&'static str, Value)>,
    rest: &mut BTreeMap<String, Value>,
    attrs: &[(String, Attr)],
) {
    for (k, v) in attrs {
        if *v == Attr::Absent {
            rest.remove(k);
            head.retain(|(h, _)| h != k);
        } else {
            rest.insert(k.clone(), attr_value(v));
        }
    }
}

/// A loaded `source_location`: one of [`Loc`]'s shapes, or any other value (Lua's bare `"3"`,
/// quirk LU1) kept verbatim as an attribute, as the build writes it.
pub fn loc_of(v: Option<&Value>) -> (Loc, Option<Attr>) {
    match v {
        None => (Loc::Absent, None),
        Some(Value::Null) => (Loc::Null, None),
        Some(Value::String(s)) if s.is_empty() => (Loc::Empty, None),
        Some(Value::String(s)) => match s
            .strip_prefix('L')
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|n| n.parse().ok())
        {
            Some(n) if format!("L{n}") == *s => (Loc::Line(n), None),
            _ => (Loc::Absent, Some(Attr::Str(s.clone()))),
        },
        Some(other) => (Loc::Absent, Some(attr_of(other))),
    }
}

/// String fields the model reads as `""` when absent or not a string; a present non-string value
/// (Dart's shared nodes have `"source_file": null`) is kept as an attribute, which the writer
/// emits in the field's place.
fn keep_non_strings(obj: &Map<String, Value>, keys: &[&str], attrs: &mut Vec<(String, Attr)>) {
    for &k in keys {
        if let Some(v) = obj.get(k).filter(|v| !v.is_string()) {
            attrs.push((k.to_string(), attr_of(v)));
        }
    }
}

/// Fields the writer always emits but this object lacks (semantic nodes and edges, §7).
fn mark_absent(obj: &Map<String, Value>, keys: &[&str], attrs: &mut Vec<(String, Attr)>) {
    for &k in keys {
        if !obj.contains_key(k) {
            attrs.push((k.to_string(), Attr::Absent));
        }
    }
}

/// A JSON value as an attribute.
pub fn attr_of(v: &Value) -> Attr {
    match v {
        Value::String(s) => Attr::Str(s.clone()),
        Value::Bool(b) => Attr::Bool(*b),
        Value::Null => Attr::Null,
        Value::Number(n) => match n.as_i64() {
            Some(i) if !n.is_f64() => Attr::Int(i),
            _ => Attr::Float(n.as_f64().unwrap_or(0.0)),
        },
        other => Attr::Json(other.to_string()),
    }
}

fn loc_value(loc: Loc) -> Option<Value> {
    match loc {
        Loc::Line(n) => Some(Value::String(format!("L{n}"))),
        Loc::Empty => Some(Value::String(String::new())),
        Loc::Null => Some(Value::Null),
        Loc::Absent => None,
    }
}

/// Fields in Graphify's key order: `id`, `label`, then the rest sorted.
fn node_map(n: &Node, community: Option<u32>) -> Fields {
    let mut head = vec![
        ("id", Value::String(n.id.clone())),
        ("label", Value::String(n.label.clone())),
    ];
    let mut rest = BTreeMap::new();
    rest.insert("file_type".into(), Value::String(n.file_type.clone()));
    rest.insert("source_file".into(), Value::String(n.source_file.clone()));
    if let Some(v) = loc_value(n.loc) {
        rest.insert("source_location".into(), v);
    }
    rest.insert("norm_label".into(), Value::String(norm_label(&n.label)));
    if let Some(c) = community {
        rest.insert("community".into(), Value::from(c));
    }
    apply_attrs(&mut head, &mut rest, &n.attrs);
    (head, rest)
}

/// Fields in Graphify's key order: `source`, `target`, `relation`, then the rest sorted.
fn edge_map(e: &Edge) -> Fields {
    let mut head = vec![
        ("source", Value::String(e.source.clone())),
        ("target", Value::String(e.target.clone())),
        ("relation", Value::String(e.relation.clone())),
    ];
    let mut rest = BTreeMap::new();
    rest.insert(
        "confidence".into(),
        Value::String(e.confidence.as_str().into()),
    );
    let score = e.score.unwrap_or_else(|| e.confidence.default_score());
    rest.insert("confidence_score".into(), Value::from(score));
    if let Some(c) = &e.context {
        rest.insert("context".into(), Value::String(c.clone()));
    }
    rest.insert("source_file".into(), Value::String(e.source_file.clone()));
    if let Some(v) = loc_value(e.loc) {
        rest.insert("source_location".into(), v);
    }
    if let Some(w) = e.weight {
        rest.insert("weight".into(), Value::from(w));
    }
    apply_attrs(&mut head, &mut rest, &e.attrs);
    (head, rest)
}

type Fields = (Vec<(&'static str, Value)>, BTreeMap<String, Value>);

/// Python's `json.dumps(obj, sort_keys=True, ensure_ascii=False)` with default separators: the
/// string Graphify sorts arrays by (§2.2).
fn sort_key((head, rest): Fields) -> String {
    let mut sorted: Map<String, Value> = rest.into_iter().collect();
    for (k, v) in head {
        sorted.insert(k.to_string(), v);
    }
    python_dumps(&Value::Object(sorted))
}

/// The compact output object: head keys first, then the rest sorted (§2.3). Not yet escaped.
fn compact((head, rest): Fields) -> String {
    let mut map = Map::new();
    for (k, v) in head {
        map.insert(k.to_string(), v);
    }
    for (k, v) in rest {
        map.insert(k, v);
    }
    serde_json::to_string(&Value::Object(map)).unwrap()
}

/// `json.dumps(v, sort_keys=True, ensure_ascii=False)`: `", "` and `": "` separators.
fn python_dumps(v: &Value) -> String {
    let mut out = String::new();
    dump(v, &mut out);
    out
}

fn dump(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&serde_json::to_string(k).unwrap());
                out.push_str(": ");
                dump(&m[*k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                dump(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap()),
    }
}

/// Python's `json.dumps(ensure_ascii=True)`: every non-ASCII character as `\uXXXX`.
fn ascii_escape(s: String) -> String {
    if s.is_ascii() {
        return s;
    }
    let mut out = String::with_capacity(s.len() + s.len() / 8);
    let mut buf = [0u16; 2];
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            for unit in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str) -> Node {
        Node {
            id: id.into(),
            label: id.into(),
            kind: NodeKind::Def,
            file_type: "code".into(),
            source_file: "f.py".into(),
            loc: Loc::Line(1),
            attrs: vec![],
        }
    }

    fn edge(s: &str, t: &str, rel: &str, line: u32) -> Edge {
        Edge {
            source: s.into(),
            target: t.into(),
            relation: rel.into(),
            confidence: Confidence::Extracted,
            score: None,
            context: None,
            source_file: "f.py".into(),
            loc: Loc::Line(line),
            weight: Some(1.0),
            attrs: vec![],
        }
    }

    fn one(nodes: &[&str], edges: Vec<Edge>) -> Edge {
        let g = Graph::assemble(nodes.iter().map(|n| node(n)).collect(), edges);
        assert_eq!(g.edges.len(), 1);
        g.edges.into_iter().next().unwrap()
    }

    fn with_context(mut e: Edge, context: &str) -> Edge {
        e.context = Some(context.into());
        e
    }

    #[test]
    fn structural_links_absorb_a_later_calls_context_only() {
        let e = one(
            &["a", "b"],
            vec![
                edge("a", "b", "contains", 52),
                with_context(edge("a", "b", "calls", 55), "call"),
            ],
        );
        assert_eq!(
            (e.relation.as_str(), e.context.as_deref(), e.loc),
            ("contains", Some("call"), Loc::Line(52))
        );
        // A call without context (Zig, quirk ZG3) leaves none.
        let e = one(
            &["a", "b"],
            vec![edge("a", "b", "method", 1), edge("a", "b", "calls", 2)],
        );
        assert_eq!(e.context, None);
        // `defines`, `inherits`, `implements` and `embeds` take nothing from a reference
        // (Dart, C#, Go probes), and replace an earlier one whole.
        for rel in ["defines", "inherits", "implements", "embeds"] {
            let e = one(
                &["a", "b"],
                vec![
                    edge("a", "b", rel, 1),
                    with_context(edge("a", "b", "references", 2), "field"),
                ],
            );
            assert_eq!((e.relation.as_str(), e.context), (rel, None));
            let e = one(
                &["a", "b"],
                vec![
                    with_context(edge("a", "b", "references", 1), "field"),
                    edge("a", "b", rel, 2),
                ],
            );
            assert_eq!((e.relation.as_str(), e.context), (rel, None));
        }
    }

    #[test]
    fn orientation_is_the_first_edges() {
        // Kotlin probe: the reversed `implements` wins the tie (larger source), and a later
        // call in the first edge's direction still lends it its context.
        let e = one(
            &["f_aa", "f_zz"],
            vec![
                edge("f_aa", "f_zz", "contains", 1),
                edge("f_zz", "f_aa", "implements", 1),
                with_context(edge("f_aa", "f_zz", "calls", 3), "call"),
            ],
        );
        assert_eq!(
            (e.source.as_str(), e.relation.as_str(), e.context.as_deref()),
            ("f_zz", "implements", Some("call"))
        );
        // A call in the survivor's own direction is reversed relative to the pair: no trace.
        let e = one(
            &["f_aa", "f_zz"],
            vec![
                edge("f_aa", "f_zz", "contains", 1),
                edge("f_zz", "f_aa", "inherits", 1),
                with_context(edge("f_zz", "f_aa", "calls", 3), "call"),
            ],
        );
        assert_eq!((e.relation.as_str(), e.context), ("inherits", None));
    }

    #[test]
    fn repeated_self_loops_keep_the_first() {
        let e = one(
            &["a"],
            vec![edge("a", "a", "case_of", 1), edge("a", "a", "case_of", 2)],
        );
        assert_eq!(e.loc, Loc::Line(1));
        let e = one(
            &["a"],
            vec![
                edge("a", "a", "references", 1),
                with_context(edge("a", "a", "references", 2), "field"),
            ],
        );
        assert_eq!((e.loc, e.context), (Loc::Line(1), None));
    }

    #[test]
    fn higher_rank_same_direction_merges_keys() {
        let mut r = edge("a", "b", "references", 3);
        r.context = Some("parameter_type".into());
        let mut u = edge("a", "b", "uses", 4);
        u.confidence = Confidence::Inferred;
        u.score = Some(0.95);
        u.weight = Some(0.8);
        let e = one(&["a", "b"], vec![r, u]);
        assert_eq!(e.relation, "uses");
        assert_eq!(e.context.as_deref(), Some("parameter_type"));
        assert_eq!(e.score, Some(0.95));
    }

    #[test]
    fn inferred_then_extracted_call_keeps_the_score() {
        let mut first = edge("a", "b", "calls", 1);
        first.confidence = Confidence::Inferred;
        first.score = Some(0.85);
        let e = one(&["a", "b"], vec![first, edge("a", "b", "calls", 2)]);
        assert_eq!(
            (e.confidence, e.score, e.loc),
            (Confidence::Extracted, Some(0.85), Loc::Line(2))
        );
    }

    #[test]
    fn opposite_direction_tie_keeps_smaller_source() {
        let e = one(
            &["x_alpha", "x_zeta"],
            vec![
                edge("x_zeta", "x_alpha", "calls", 1),
                edge("x_alpha", "x_zeta", "calls", 2),
            ],
        );
        assert_eq!(e.source, "x_alpha");
        let e = one(
            &["a", "b"],
            vec![edge("b", "a", "calls", 1), edge("a", "b", "method", 2)],
        );
        assert_eq!(e.relation, "method");
    }

    #[test]
    fn opposite_direction_tie_of_one_relation_keeps_smaller_source() {
        // Python and Java probes: `class Aa(Bb)` / `class Bb(Aa)` keep `Aa -inherits-> Bb` in
        // either order; Julia's `file -defines-> module` beats `module -defines-> file`.
        for (first, second) in [
            (("m_aa", "m_bb"), ("m_bb", "m_aa")),
            (("m_bb", "m_aa"), ("m_aa", "m_bb")),
        ] {
            for rel in ["inherits", "implements", "defines", "references"] {
                let e = one(
                    &["m_aa", "m_bb"],
                    vec![
                        edge(first.0, first.1, rel, 1),
                        edge(second.0, second.1, rel, 2),
                    ],
                );
                assert_eq!(e.source, "m_aa", "{rel}");
            }
        }
        // `re_exports` and `imports_from` share a rank: the larger source wins (fastapi's
        // package import cycles, quirk tri_a / tri_z).
        let e = one(
            &["pkg_init", "pkg_z"],
            vec![
                edge("pkg_init", "pkg_z", "re_exports", 2),
                edge("pkg_z", "pkg_init", "imports_from", 1),
            ],
        );
        assert_eq!(
            (e.source.as_str(), e.relation.as_str()),
            ("pkg_z", "imports_from")
        );
    }

    #[test]
    fn opposite_direction_structural_tie_keeps_larger_source() {
        let e = one(
            &["f_result", "f_ok"],
            vec![
                edge("f_result", "f_ok", "contains", 1),
                edge("f_ok", "f_result", "inherits", 2),
            ],
        );
        assert_eq!(e.relation, "contains");
        let e = one(
            &["f_event", "f_start"],
            vec![
                edge("f_event", "f_start", "contains", 1),
                edge("f_start", "f_event", "inherits", 2),
            ],
        );
        assert_eq!(e.relation, "inherits");
    }

    #[test]
    fn self_loops_kept_dangling_dropped_first_node_wins() {
        let mut dup = node("a");
        dup.loc = Loc::Line(11);
        let g = Graph::assemble(
            vec![node("a"), dup],
            vec![edge("a", "a", "calls", 1), edge("a", "missing", "calls", 1)],
        );
        assert_eq!(g.nodes.len(), 1);
        assert_eq!(g.nodes[0].loc, Loc::Line(1));
        assert_eq!((g.edges.len(), g.dangling), (1, 1));
    }

    #[test]
    fn json_shape_order_and_escaping() {
        let mut n = node("z");
        n.label = "Über".into();
        n.attrs.push(("_origin".into(), Attr::Str("ast".into())));
        let g = Graph::assemble(vec![n, node("a")], vec![edge("a", "z", "calls", 1)]);
        let json = g.to_graphify_json(None, None);
        assert!(json.starts_with(
            "{\"directed\":false,\"multigraph\":false,\"graph\":{\"schema_version\":1,\"graphify_version\":\"0.9.86\"},\"nodes\":[{\"id\":\"z\",\"label\":\"\\u00dcber\",\"_origin\":\"ast\""
        ));
        assert!(json.contains("\"norm_label\":\"uber\""));
        assert!(json.contains("{\"source\":\"a\",\"target\":\"z\",\"relation\":\"calls\",\"confidence\":\"EXTRACTED\",\"confidence_score\":1.0,"));
        assert!(json.ends_with("\"hyperedges\":[]}"));
        assert_eq!(norm_label("función…"), "funcion...");
    }
}

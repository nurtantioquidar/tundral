//! `dijkstral-out/graph.dij`: the graph in a form queries read in place from a memory map.
//!
//! `graph.json` is the compatibility contract; this file is dijkstral's own and exists so a query,
//! a hook or an MCP call never parses JSON. Everything is little-endian and every section starts
//! on an 8-byte boundary.
//!
//! ```text
//! header   magic "DJKIDX02", u32 node and link counts, then u64 offset/length pairs per section
//! strings  u32 count, u32 offsets (count + 1) and UTF-8 bytes: every id, label, path, relation…
//! nodes    fixed records (`NODE_SIZE` bytes) in graph node order, then one kind byte per node
//! edges    fixed records (`EDGE_SIZE` bytes), one per link, in graph.json link order
//! adj      CSR over undirected incidence: u32 offsets (nodes + 1), u32 edge indices in link order
//!          (a self-loop is listed once)
//! by_id    node indices sorted by id (binary search)
//! by_norm  node indices sorted by (norm_label, id) (exact and prefix lookup)
//! trigram  sorted u32 trigram keys, u32 offsets, u32 posting node indices (norm_label substrings)
//! lower    per node: u32 strings of `label.lower()`, `source_file.lower()` and the label words
//! ```
//!
//! Text fields are stored as Graphify prints them (`str(value)` of the JSON field: a missing field
//! is `""`, `null` is `"None"`), so the query surface renders Graphify-built and dijkstral-built
//! graphs identically. Where presence matters (an edge's `context`, `confidence`, `source_file`
//! and `source_location`), a missing field is stored as `NONE`.
//!
//! Records never point outside their file, and every read is bounds-checked; a truncated or
//! foreign file is rejected at open. The same bytes can be built in memory from any Graphify
//! `graph.json` ([`Index::from_graph_json`]), so Graphify's own graphs are served without a build.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::io::Write;
use std::ops::Deref;
use std::path::Path;

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;

use crate::facts::{Loc, NodeKind};
use crate::graph::{norm_label, Graph};
use crate::pyjson::float_repr;

pub const INDEX_FILE: &str = "graph.dij";
const MAGIC: &[u8; 8] = b"DJKIDX02";
const NONE: u32 = u32::MAX;
/// `community: null`.
const COMMUNITY_NULL: u32 = u32::MAX - 1;
const NODE_SIZE: usize = 32;
const EDGE_SIZE: usize = 32;
/// Per node in the `lower` section: lower-cased label, lower-cased source file, label words.
const LOWER_SIZE: usize = 12;

#[derive(Clone, Copy)]
enum Section {
    Strings = 0,
    Nodes,
    Edges,
    Adj,
    ById,
    ByNorm,
    Trigram,
    Lower,
}
const SECTIONS: usize = 8;
const HEADER_SIZE: usize = 8 + 8 + SECTIONS * 16;

/// Stable node kind numbering.
pub mod kind {
    pub const FILE: u8 = 0;
    pub const DEF: u8 = 1;
    pub const STUB: u8 = 2;
    pub const EXTERNAL: u8 = 3;
    pub const RATIONALE: u8 = 4;
}

fn kind_code(k: NodeKind) -> u8 {
    match k {
        NodeKind::File => kind::FILE,
        NodeKind::Def => kind::DEF,
        NodeKind::Stub => kind::STUB,
        NodeKind::External => kind::EXTERNAL,
        NodeKind::Rationale => kind::RATIONALE,
    }
}

/// A node's `community` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Community {
    Absent,
    Null,
    Id(u32),
}

impl Community {
    pub fn id(self) -> Option<u32> {
        match self {
            Community::Id(c) => Some(c),
            _ => None,
        }
    }
}

/// As Python prints `node.get("community", "")`.
impl fmt::Display for Community {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Community::Absent => Ok(()),
            Community::Null => f.write_str("None"),
            Community::Id(c) => write!(f, "{c}"),
        }
    }
}

/// One node to encode. Text fields are display strings (see the module docs).
pub struct NodeRec<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub source_file: &'a str,
    pub file_type: &'a str,
    pub source_location: &'a str,
    pub community: Community,
    pub kind: u8,
}

/// One link to encode, endpoints as node positions. `None` is a missing field.
pub struct EdgeRec<'a> {
    pub source: u32,
    pub target: u32,
    pub relation: &'a str,
    pub context: Option<&'a str>,
    pub confidence: Option<&'a str>,
    pub source_file: Option<&'a str>,
    pub source_location: Option<&'a str>,
    pub score: Option<f32>,
}

/// Trigrams of a string, by Unicode scalar, packed into u32 keys (an FNV hash of the three chars;
/// collisions only widen a candidate set, callers verify).
fn trigrams(s: &str) -> Vec<u32> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<u32> = chars
        .windows(3)
        .map(|w| {
            let mut h: u32 = 2166136261;
            for c in w {
                h = (h ^ *c as u32).wrapping_mul(16777619);
            }
            h
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[derive(Default)]
struct Strings<'a> {
    ids: HashMap<Cow<'a, str>, u32>,
    list: Vec<Cow<'a, str>>,
}

impl<'a> Strings<'a> {
    fn intern(&mut self, s: Cow<'a, str>) -> u32 {
        if let Some(&i) = self.ids.get(s.as_ref()) {
            return i;
        }
        let i = self.list.len() as u32;
        self.ids.insert(s.clone(), i);
        self.list.push(s);
        i
    }
    fn borrowed(&mut self, s: &'a str) -> u32 {
        self.intern(Cow::Borrowed(s))
    }
    fn opt(&mut self, s: Option<&'a str>) -> u32 {
        s.map_or(NONE, |s| self.borrowed(s))
    }
}

fn align(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(8) {
        buf.push(0);
    }
}

fn put_u32s(buf: &mut Vec<u8>, xs: impl IntoIterator<Item = u32>) {
    for x in xs {
        buf.extend_from_slice(&x.to_le_bytes());
    }
}

/// Lower-case a string as Python's `str.lower()` does (ASCII fast path).
pub fn py_lower(s: &str) -> Cow<'_, str> {
    if s.bytes().any(|b| b.is_ascii_uppercase() || b >= 0x80) {
        Cow::Owned(s.to_lowercase())
    } else {
        Cow::Borrowed(s)
    }
}

/// The alphanumeric runs (Unicode, `_` separates) of an already lower-cased label joined by single
/// spaces: the form names are resolved in (`docs/graphify-query.md` §4).
pub fn label_words(lower: &str) -> Cow<'_, str> {
    let simple = lower
        .split(' ')
        .all(|w| !w.is_empty() && w.chars().all(char::is_alphanumeric));
    if simple {
        return Cow::Borrowed(lower);
    }
    let mut out = String::with_capacity(lower.len());
    for w in lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    Cow::Owned(out)
}

/// Encode nodes and links into index bytes.
pub fn encode<'a>(nodes: &'a [NodeRec<'a>], edges: &'a [EdgeRec<'a>]) -> Vec<u8> {
    let n = nodes.len();
    let mut strings = Strings::default();
    let norms: Vec<String> = nodes.iter().map(|nd| norm_label(nd.label)).collect();

    let mut degree = vec![0u32; n];
    let mut counts = vec![0u32; n + 1];
    let mut edge_bytes = Vec::with_capacity(edges.len() * EDGE_SIZE);
    for e in edges {
        let (s, t) = (e.source as usize, e.target as usize);
        degree[s] += 1;
        degree[t] += 1;
        counts[s] += 1;
        if t != s {
            counts[t] += 1;
        }
        let rel = strings.borrowed(e.relation);
        let ctx = strings.opt(e.context);
        let file = strings.opt(e.source_file);
        let loc = strings.opt(e.source_location);
        let conf = strings.opt(e.confidence);
        put_u32s(
            &mut edge_bytes,
            [e.source, e.target, rel, ctx, file, loc, conf],
        );
        edge_bytes.extend_from_slice(&e.score.unwrap_or(f32::NAN).to_le_bytes());
    }
    // CSR incidence in link order.
    let mut offsets = Vec::with_capacity(n + 1);
    let mut acc = 0u32;
    for c in &counts[..n] {
        offsets.push(acc);
        acc += c;
    }
    offsets.push(acc);
    let mut fill = offsets.clone();
    let mut incident = vec![0u32; acc as usize];
    for (ei, e) in edges.iter().enumerate() {
        let (s, t) = (e.source as usize, e.target as usize);
        incident[fill[s] as usize] = ei as u32;
        fill[s] += 1;
        if t != s {
            incident[fill[t] as usize] = ei as u32;
            fill[t] += 1;
        }
    }
    let mut adj = Vec::with_capacity((n + 1 + incident.len()) * 4);
    put_u32s(&mut adj, offsets);
    put_u32s(&mut adj, incident);

    let mut node_bytes = Vec::with_capacity(n * NODE_SIZE + n);
    let mut lower = Vec::with_capacity(n * LOWER_SIZE);
    for (i, nd) in nodes.iter().enumerate() {
        let id = strings.borrowed(nd.id);
        let label = strings.borrowed(nd.label);
        let norm = strings.intern(Cow::Owned(norms[i].clone()));
        let file = strings.borrowed(nd.source_file);
        let file_type = strings.borrowed(nd.file_type);
        let loc = strings.borrowed(nd.source_location);
        let community = match nd.community {
            Community::Absent => NONE,
            Community::Null => COMMUNITY_NULL,
            Community::Id(c) => c.min(COMMUNITY_NULL - 1),
        };
        put_u32s(
            &mut node_bytes,
            [id, label, norm, file, file_type, loc, community, degree[i]],
        );
        let ll = strings.intern(py_lower(nd.label));
        let lf = strings.intern(py_lower(nd.source_file));
        let lw = strings.intern(Cow::Owned(label_words(&py_lower(nd.label)).into_owned()));
        put_u32s(&mut lower, [ll, lf, lw]);
    }
    node_bytes.extend(nodes.iter().map(|nd| nd.kind));

    let mut by_id: Vec<u32> = (0..n as u32).collect();
    by_id.sort_by(|&a, &b| nodes[a as usize].id.cmp(nodes[b as usize].id));
    let mut by_norm: Vec<u32> = (0..n as u32).collect();
    by_norm.sort_by(|&a, &b| {
        (norms[a as usize].as_str(), nodes[a as usize].id)
            .cmp(&(norms[b as usize].as_str(), nodes[b as usize].id))
    });

    let mut postings: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (i, norm) in norms.iter().enumerate() {
        for t in trigrams(norm) {
            postings.entry(t).or_default().push(i as u32);
        }
    }
    let mut tri = Vec::new();
    put_u32s(&mut tri, [postings.len() as u32]);
    put_u32s(&mut tri, postings.keys().copied());
    let mut off = 0u32;
    let mut tri_offsets = Vec::with_capacity(postings.len() + 1);
    for v in postings.values() {
        tri_offsets.push(off);
        off += v.len() as u32;
    }
    tri_offsets.push(off);
    put_u32s(&mut tri, tri_offsets);
    put_u32s(&mut tri, postings.values().flatten().copied());

    let mut str_bytes = Vec::new();
    let mut str_offsets = Vec::with_capacity(strings.list.len() + 1);
    let mut data = Vec::new();
    for s in &strings.list {
        str_offsets.push(data.len() as u32);
        data.extend_from_slice(s.as_bytes());
    }
    str_offsets.push(data.len() as u32);
    put_u32s(&mut str_bytes, [strings.list.len() as u32]);
    put_u32s(&mut str_bytes, str_offsets);
    str_bytes.extend_from_slice(&data);

    let u32_section = |xs: Vec<u32>| {
        let mut b = Vec::with_capacity(xs.len() * 4);
        put_u32s(&mut b, xs);
        b
    };
    let sections: [Vec<u8>; SECTIONS] = [
        str_bytes,
        node_bytes,
        edge_bytes,
        adj,
        u32_section(by_id),
        u32_section(by_norm),
        tri,
        lower,
    ];
    let mut out =
        Vec::with_capacity(HEADER_SIZE + sections.iter().map(|s| s.len() + 8).sum::<usize>());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&(edges.len() as u32).to_le_bytes());
    let table_at = out.len();
    out.resize(HEADER_SIZE, 0);
    let mut table = Vec::with_capacity(SECTIONS * 16);
    for s in &sections {
        align(&mut out);
        table.extend_from_slice(&(out.len() as u64).to_le_bytes());
        table.extend_from_slice(&(s.len() as u64).to_le_bytes());
        out.extend_from_slice(s);
    }
    out[table_at..table_at + table.len()].copy_from_slice(&table);
    out
}

fn loc_display(loc: Loc) -> Option<String> {
    match loc {
        Loc::Line(l) => Some(format!("L{l}")),
        Loc::Empty => Some(String::new()),
        Loc::Null => Some("None".into()),
        Loc::Absent => None,
    }
}

/// Index bytes for an assembled graph (with `community` per node when clustered).
pub fn encode_graph(graph: &Graph, communities: Option<&[u32]>) -> Vec<u8> {
    let index_of: HashMap<&str, u32> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| (nd.id.as_str(), i as u32))
        .collect();
    let node_locs: Vec<String> = graph
        .nodes
        .iter()
        .map(|nd| loc_display(nd.loc).unwrap_or_default())
        .collect();
    let nodes: Vec<NodeRec<'_>> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| NodeRec {
            id: &nd.id,
            label: &nd.label,
            source_file: &nd.source_file,
            file_type: &nd.file_type,
            source_location: &node_locs[i],
            community: communities.map_or(Community::Absent, |c| Community::Id(c[i])),
            kind: kind_code(nd.kind),
        })
        .collect();
    let edge_locs: Vec<Option<String>> = graph.edges.iter().map(|e| loc_display(e.loc)).collect();
    let edges: Vec<EdgeRec<'_>> = graph
        .edges
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            Some(EdgeRec {
                source: *index_of.get(e.source.as_str())?,
                target: *index_of.get(e.target.as_str())?,
                relation: &e.relation,
                context: e.context.as_deref(),
                confidence: Some(e.confidence.as_str()),
                source_file: Some(&e.source_file),
                source_location: edge_locs[i].as_deref(),
                score: Some(e.score.unwrap_or_else(|| e.confidence.default_score()) as f32),
            })
        })
        .collect();
    encode(&nodes, &edges)
}

/// Write `graph.dij` for `graph` (with `community` per node when clustered) atomically.
pub fn write(path: &Path, graph: &Graph, communities: Option<&[u32]>) -> Result<()> {
    let out = encode_graph(graph, communities);
    let tmp = path.with_extension("dij.tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(&out)?;
    f.sync_all().ok();
    fs::rename(&tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Loading a Graphify graph.json

/// A JSON field value as Python's `str()` prints it, borrowing strings where the JSON allows.
#[derive(Debug)]
enum PyVal<'a> {
    Str(Cow<'a, str>),
    Null,
    Int(i64),
    Other(String),
}

impl PyVal<'_> {
    fn display(&self) -> Cow<'_, str> {
        match self {
            PyVal::Str(s) => Cow::Borrowed(s.as_ref()),
            PyVal::Null => Cow::Borrowed("None"),
            PyVal::Int(i) => Cow::Owned(i.to_string()),
            PyVal::Other(s) => Cow::Borrowed(s),
        }
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for PyVal<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = PyVal<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(PyVal::Str(Cow::Borrowed(v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(PyVal::Str(Cow::Owned(v.to_string())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(PyVal::Str(Cow::Owned(v)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(PyVal::Null)
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(PyVal::Null)
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(PyVal::Other(if v { "True" } else { "False" }.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(PyVal::Int(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(i64::try_from(v).map_or_else(|_| PyVal::Other(v.to_string()), PyVal::Int))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(PyVal::Other(float_repr(v)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                while a.next_element::<IgnoredAny>()?.is_some() {}
                Ok(PyVal::Other("[...]".into()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                while a.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(PyVal::Other("{...}".into()))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// A present field, `null` included (plain `Option` would read `null` as missing).
fn present<'de: 'a, 'a, D: Deserializer<'de>>(d: D) -> Result<Option<PyVal<'a>>, D::Error> {
    PyVal::deserialize(d).map(Some)
}

#[derive(Deserialize)]
struct RawNode<'a> {
    #[serde(borrow, default, deserialize_with = "present")]
    id: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    label: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    source_file: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    file_type: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    source_location: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    community: Option<PyVal<'a>>,
    #[serde(borrow, default, rename = "type", deserialize_with = "present")]
    node_type: Option<PyVal<'a>>,
}

#[derive(Deserialize)]
struct RawEdge<'a> {
    #[serde(borrow, default, deserialize_with = "present")]
    source: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    target: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    relation: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    context: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    confidence: Option<PyVal<'a>>,
    #[serde(default)]
    confidence_score: Option<f64>,
    #[serde(borrow, default, deserialize_with = "present")]
    source_file: Option<PyVal<'a>>,
    #[serde(borrow, default, deserialize_with = "present")]
    source_location: Option<PyVal<'a>>,
}

#[derive(Deserialize)]
struct RawGraph<'a> {
    #[serde(borrow)]
    nodes: Vec<RawNode<'a>>,
    #[serde(borrow, default)]
    links: Option<Vec<RawEdge<'a>>>,
    #[serde(borrow, default)]
    edges: Option<Vec<RawEdge<'a>>>,
}

fn disp<'a>(v: &'a Option<PyVal<'_>>) -> Cow<'a, str> {
    v.as_ref().map_or(Cow::Borrowed(""), PyVal::display)
}

/// A present, truthy string field (`None`, `null` and `""` are absent).
fn opt_str<'a>(v: &'a Option<PyVal<'_>>) -> Option<Cow<'a, str>> {
    match v {
        Some(PyVal::Null) | None => None,
        Some(v) => Some(v.display()),
    }
}

/// Index bytes for a Graphify `graph.json` (clustered `links` or raw `edges` shape). Links whose
/// endpoints are not nodes are skipped; a repeated node id keeps its first record.
pub fn encode_graph_json(bytes: &[u8]) -> Result<Vec<u8>> {
    let raw: RawGraph<'_> = serde_json::from_slice(bytes).context("parsing graph.json")?;
    let ids: Vec<Cow<'_, str>> = raw.nodes.iter().map(|n| disp(&n.id)).collect();
    let mut index_of: HashMap<&str, u32> = HashMap::with_capacity(ids.len());
    let mut keep = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        let fresh = !index_of.contains_key(id.as_ref());
        if fresh {
            index_of.insert(id.as_ref(), keep.len() as u32);
            keep.push(i);
        }
    }
    struct Fields<'a> {
        label: Cow<'a, str>,
        source_file: Cow<'a, str>,
        file_type: Cow<'a, str>,
        source_location: Cow<'a, str>,
    }
    let fields: Vec<Fields<'_>> = keep
        .iter()
        .map(|&i| {
            let n = &raw.nodes[i];
            Fields {
                label: disp(&n.label),
                source_file: disp(&n.source_file),
                file_type: disp(&n.file_type),
                source_location: disp(&n.source_location),
            }
        })
        .collect();
    let nodes: Vec<NodeRec<'_>> = keep
        .iter()
        .zip(&fields)
        .map(|(&i, f)| {
            let n = &raw.nodes[i];
            let community = match &n.community {
                None => Community::Absent,
                Some(PyVal::Null) => Community::Null,
                Some(PyVal::Int(c)) if *c >= 0 => Community::Id(*c as u32),
                Some(_) => Community::Absent,
            };
            let base = f.source_file.rsplit('/').next().unwrap_or("");
            let kind = if matches!(&n.node_type, Some(PyVal::Str(t)) if t == "external") {
                kind::EXTERNAL
            } else if f.file_type == "rationale" {
                kind::RATIONALE
            } else if f.source_file.is_empty() {
                kind::STUB
            } else if f.label == base {
                kind::FILE
            } else {
                kind::DEF
            };
            NodeRec {
                id: &ids[i],
                label: &f.label,
                source_file: &f.source_file,
                file_type: &f.file_type,
                source_location: &f.source_location,
                community,
                kind,
            }
        })
        .collect();

    let links = raw.links.as_ref().or(raw.edges.as_ref());
    let links: &[RawEdge<'_>] = links.map_or(&[], |v| v.as_slice());
    struct EFields<'a> {
        source: u32,
        target: u32,
        relation: Cow<'a, str>,
        context: Option<Cow<'a, str>>,
        confidence: Option<Cow<'a, str>>,
        source_file: Option<Cow<'a, str>>,
        source_location: Option<Cow<'a, str>>,
        score: Option<f32>,
    }
    let efields: Vec<EFields<'_>> = links
        .iter()
        .filter_map(|e| {
            let s = *index_of.get(disp(&e.source).as_ref())?;
            let t = *index_of.get(disp(&e.target).as_ref())?;
            Some(EFields {
                source: s,
                target: t,
                relation: disp(&e.relation),
                context: opt_str(&e.context).filter(|c| !c.is_empty()),
                confidence: e.confidence.as_ref().map(PyVal::display),
                source_file: e.source_file.as_ref().map(PyVal::display),
                source_location: e.source_location.as_ref().map(PyVal::display),
                score: e.confidence_score.map(|s| s as f32),
            })
        })
        .collect();
    let edges: Vec<EdgeRec<'_>> = efields
        .iter()
        .map(|e| EdgeRec {
            source: e.source,
            target: e.target,
            relation: &e.relation,
            context: e.context.as_deref(),
            confidence: e.confidence.as_deref(),
            source_file: e.source_file.as_deref(),
            source_location: e.source_location.as_deref(),
            score: e.score,
        })
        .collect();
    Ok(encode(&nodes, &edges))
}

// ---------------------------------------------------------------------------------------------
// Reading

enum Backing {
    Map(Mmap),
    Owned(Vec<u8>),
}

impl Deref for Backing {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Backing::Map(m) => m,
            Backing::Owned(v) => v,
        }
    }
}

/// A `graph.dij`, memory-mapped from disk or built in memory.
pub struct Index {
    data: Backing,
    nodes: usize,
    edges: usize,
    sec: [(usize, usize); SECTIONS],
    strings: usize,
}

/// A node as read from the index. Text fields are as Graphify prints them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeRef<'a> {
    pub index: u32,
    pub id: &'a str,
    pub label: &'a str,
    pub norm_label: &'a str,
    pub source_file: &'a str,
    pub file_type: &'a str,
    /// `L<n>`, `""` (empty or missing) or `None` (JSON null).
    pub source_location: &'a str,
    pub community: Community,
    /// Incident links, a self-loop counting twice (NetworkX degree).
    pub degree: u32,
    pub kind: u8,
}

/// A link as read from the index (direction as written in graph.json). `None` is a missing field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeRef<'a> {
    pub index: u32,
    pub source: u32,
    pub target: u32,
    pub relation: &'a str,
    pub context: Option<&'a str>,
    pub confidence: Option<&'a str>,
    pub source_file: Option<&'a str>,
    pub source_location: Option<&'a str>,
    pub score: Option<f32>,
}

impl Index {
    /// Memory-map a `graph.dij`.
    pub fn open(path: &Path) -> Result<Index> {
        let file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        // SAFETY: the file is only read; dijkstral replaces it by rename, never in place.
        let map = unsafe { Mmap::map(&file)? };
        Index::from_backing(Backing::Map(map))
            .with_context(|| format!("{} is not a usable dijkstral index", path.display()))
    }

    /// An index held in memory (bytes from [`encode`]).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Index> {
        Index::from_backing(Backing::Owned(bytes))
    }

    /// Build an in-memory index from a Graphify-format `graph.json`.
    pub fn from_graph_json(bytes: &[u8]) -> Result<Index> {
        Index::from_bytes(encode_graph_json(bytes)?)
    }

    fn from_backing(data: Backing) -> Result<Index> {
        if data.len() < HEADER_SIZE || &data[..8] != MAGIC {
            bail!("not a dijkstral index (or an older format)");
        }
        let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().unwrap()) as usize;
        let nodes = u32_at(8) as usize;
        let edges = u32_at(12) as usize;
        let mut sec = [(0usize, 0usize); SECTIONS];
        for (i, s) in sec.iter_mut().enumerate() {
            let at = 16 + i * 16;
            *s = (u64_at(at), u64_at(at + 8));
            if s.0.checked_add(s.1).is_none_or(|end| end > data.len()) {
                bail!("truncated");
            }
        }
        let strings = if sec[0].1 >= 4 {
            u32_at(sec[0].0) as usize
        } else {
            0
        };
        let need = [
            (Section::Strings, 4 + (strings + 1) * 4),
            (Section::Nodes, nodes * NODE_SIZE + nodes),
            (Section::Edges, edges * EDGE_SIZE),
            (Section::Adj, (nodes + 1) * 4),
            (Section::ById, nodes * 4),
            (Section::ByNorm, nodes * 4),
            (Section::Trigram, 4),
            (Section::Lower, nodes * LOWER_SIZE),
        ];
        for (s, min) in need {
            if sec[s as usize].1 < min {
                bail!("truncated");
            }
        }
        Ok(Index {
            data,
            nodes,
            edges,
            sec,
            strings,
        })
    }

    pub fn node_count(&self) -> usize {
        self.nodes
    }

    pub fn edge_count(&self) -> usize {
        self.edges
    }

    fn sec(&self, s: Section) -> &[u8] {
        let (o, l) = self.sec[s as usize];
        &self.data[o..o + l]
    }

    #[inline]
    fn u32_in(bytes: &[u8], i: usize) -> u32 {
        u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
    }

    pub fn string(&self, i: u32) -> &str {
        if i as usize >= self.strings {
            return "";
        }
        let s = self.sec(Section::Strings);
        let a = Self::u32_in(s, 1 + i as usize) as usize;
        let b = Self::u32_in(s, 2 + i as usize) as usize;
        let data = &s[4 + (self.strings + 1) * 4..];
        std::str::from_utf8(data.get(a..b).unwrap_or(&[])).unwrap_or("")
    }

    fn opt_string(&self, i: u32) -> Option<&str> {
        (i != NONE).then(|| self.string(i))
    }

    #[inline]
    fn node_field(&self, i: u32, k: usize) -> u32 {
        let s = self.sec(Section::Nodes);
        Self::u32_in(s, i as usize * (NODE_SIZE / 4) + k)
    }

    pub fn node(&self, i: u32) -> NodeRef<'_> {
        let s = self.sec(Section::Nodes);
        let rec = &s[i as usize * NODE_SIZE..(i as usize + 1) * NODE_SIZE];
        let f = |k: usize| Self::u32_in(rec, k);
        let community = match f(6) {
            NONE => Community::Absent,
            COMMUNITY_NULL => Community::Null,
            c => Community::Id(c),
        };
        NodeRef {
            index: i,
            id: self.string(f(0)),
            label: self.string(f(1)),
            norm_label: self.string(f(2)),
            source_file: self.string(f(3)),
            file_type: self.string(f(4)),
            source_location: self.string(f(5)),
            community,
            degree: f(7),
            kind: s[self.nodes * NODE_SIZE + i as usize],
        }
    }

    pub fn id(&self, i: u32) -> &str {
        self.string(self.node_field(i, 0))
    }

    pub fn label(&self, i: u32) -> &str {
        self.string(self.node_field(i, 1))
    }

    pub fn source_file(&self, i: u32) -> &str {
        self.string(self.node_field(i, 3))
    }

    pub fn degree(&self, i: u32) -> u32 {
        self.node_field(i, 7)
    }

    /// Python's `label.lower()`.
    pub fn lower_label(&self, i: u32) -> &str {
        self.string(Self::u32_in(self.sec(Section::Lower), i as usize * 3))
    }

    /// Python's `source_file.lower()`.
    pub fn lower_file(&self, i: u32) -> &str {
        self.string(Self::u32_in(self.sec(Section::Lower), i as usize * 3 + 1))
    }

    /// [`label_words`] of the lower-cased label.
    pub fn label_words(&self, i: u32) -> &str {
        self.string(Self::u32_in(self.sec(Section::Lower), i as usize * 3 + 2))
    }

    /// The string id of a node's lower-cased source file (equal ids are equal strings).
    pub fn lower_file_key(&self, i: u32) -> u32 {
        Self::u32_in(self.sec(Section::Lower), i as usize * 3 + 1)
    }

    pub fn edge(&self, i: u32) -> EdgeRef<'_> {
        let s = self.sec(Section::Edges);
        let rec = &s[i as usize * EDGE_SIZE..(i as usize + 1) * EDGE_SIZE];
        let f = |k: usize| Self::u32_in(rec, k);
        let score = f32::from_le_bytes(rec[28..32].try_into().unwrap());
        EdgeRef {
            index: i,
            source: f(0),
            target: f(1),
            relation: self.string(f(2)),
            context: self.opt_string(f(3)),
            source_file: self.opt_string(f(4)),
            source_location: self.opt_string(f(5)),
            confidence: self.opt_string(f(6)),
            score: (!score.is_nan()).then_some(score),
        }
    }

    /// `(source, target)` of a link without decoding its strings.
    #[inline]
    pub fn endpoints(&self, e: u32) -> (u32, u32) {
        let s = self.sec(Section::Edges);
        let base = e as usize * (EDGE_SIZE / 4);
        (Self::u32_in(s, base), Self::u32_in(s, base + 1))
    }

    /// A link's relation without decoding its other fields.
    #[inline]
    pub fn relation(&self, e: u32) -> &str {
        let s = self.sec(Section::Edges);
        self.string(Self::u32_in(s, e as usize * (EDGE_SIZE / 4) + 2))
    }

    /// The raw string id of a link's `context` (`None` when missing), for cheap filtering.
    #[inline]
    pub fn context_key(&self, e: u32) -> Option<u32> {
        let s = self.sec(Section::Edges);
        let v = Self::u32_in(s, e as usize * (EDGE_SIZE / 4) + 3);
        (v != NONE).then_some(v)
    }

    /// Indices of the links incident to node `i`, in graph.json link order.
    pub fn incident(&self, i: u32) -> impl Iterator<Item = u32> + '_ {
        let s = self.sec(Section::Adj);
        let a = Self::u32_in(s, i as usize) as usize;
        let b = Self::u32_in(s, i as usize + 1) as usize;
        let base = self.nodes + 1;
        (a..b).map(move |k| Self::u32_in(s, base + k))
    }

    /// `(link, neighbour)` pairs for node `i`, in link order, without decoding strings.
    pub fn adjacent(&self, i: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.incident(i).map(move |e| {
            let (s, t) = self.endpoints(e);
            (e, if s == i { t } else { s })
        })
    }

    /// `(edge, neighbour)` pairs for node `i`.
    pub fn neighbors(&self, i: u32) -> impl Iterator<Item = (EdgeRef<'_>, u32)> + '_ {
        self.incident(i).map(move |e| {
            let e = self.edge(e);
            let other = if e.source == i { e.target } else { e.source };
            (e, other)
        })
    }

    /// The node with exactly this id.
    pub fn by_id(&self, id: &str) -> Option<u32> {
        let s = self.sec(Section::ById);
        let (mut lo, mut hi) = (0usize, self.nodes);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let n = Self::u32_in(s, mid);
            match self.id(n).cmp(id) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(n),
            }
        }
        None
    }

    /// Nodes whose id starts with `prefix`, in id order.
    pub fn ids_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = u32> + 'a {
        let s = self.sec(Section::ById);
        let (mut lo, mut hi) = (0usize, self.nodes);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.id(Self::u32_in(s, mid)) < prefix {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (lo..self.nodes)
            .map(move |k| Self::u32_in(s, k))
            .take_while(move |&n| self.id(n).starts_with(prefix))
    }

    /// Node indices in id order.
    pub fn ids_sorted(&self) -> impl Iterator<Item = u32> + '_ {
        let s = self.sec(Section::ById);
        (0..self.nodes).map(move |k| Self::u32_in(s, k))
    }

    /// Nodes whose `norm_label` starts with `prefix` (exact matches first), in (norm_label, id)
    /// order.
    pub fn by_norm_prefix(&self, prefix: &str) -> impl Iterator<Item = u32> + '_ {
        let s = self.sec(Section::ByNorm);
        let norm = |n: u32| self.string(self.node_field(n, 2));
        let (mut lo, mut hi) = (0usize, self.nodes);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if norm(Self::u32_in(s, mid)) < prefix {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let prefix = prefix.to_string();
        (lo..self.nodes)
            .map(move |k| Self::u32_in(s, k))
            .take_while(move |&n| norm(n).starts_with(prefix.as_str()))
    }

    /// Candidate nodes whose `norm_label` may contain `needle` (a superset; verify with
    /// `contains`). `None` when the needle is too short for trigrams: scan every node.
    pub fn substring_candidates(&self, needle: &str) -> Option<Vec<u32>> {
        let grams = trigrams(needle);
        if grams.is_empty() {
            return None;
        }
        let s = self.sec(Section::Trigram);
        let k = Self::u32_in(s, 0) as usize;
        let keys = |j: usize| Self::u32_in(s, 1 + j);
        let off = |j: usize| Self::u32_in(s, 1 + k + j) as usize;
        let post = |p: usize| Self::u32_in(s, 1 + k + k + 1 + p);
        let mut lists: Vec<(usize, usize)> = Vec::with_capacity(grams.len());
        for g in grams {
            let (mut lo, mut hi) = (0usize, k);
            while lo < hi {
                let mid = (lo + hi) / 2;
                if keys(mid) < g {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            if lo == k || keys(lo) != g {
                return Some(Vec::new());
            }
            lists.push((off(lo), off(lo + 1)));
        }
        lists.sort_by_key(|(a, b)| b - a);
        let (a, b) = lists[0];
        let mut out: Vec<u32> = (a..b).map(post).collect();
        for &(a, b) in &lists[1..] {
            let other: Vec<u32> = (a..b).map(post).collect();
            out.retain(|x| other.binary_search(x).is_ok());
            if out.is_empty() {
                break;
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Confidence, Edge, Node};

    fn node(id: &str, label: &str) -> Node {
        Node {
            id: id.into(),
            label: label.into(),
            kind: NodeKind::Def,
            file_type: "code".into(),
            source_file: "a.py".into(),
            loc: Loc::Line(3),
            attrs: vec![],
        }
    }

    fn edge(s: &str, t: &str, rel: &str) -> Edge {
        Edge {
            source: s.into(),
            target: t.into(),
            relation: rel.into(),
            confidence: Confidence::Inferred,
            score: Some(0.85),
            context: Some("call".into()),
            source_file: "a.py".into(),
            loc: Loc::Line(7),
            weight: Some(1.0),
            attrs: vec![],
        }
    }

    #[test]
    fn round_trips_nodes_edges_and_lookups() {
        let g = Graph::assemble(
            vec![
                node("a_run", "run()"),
                node("a_runner", "Runner"),
                node("b_über", "Über"),
            ],
            vec![
                edge("a_run", "a_runner", "calls"),
                edge("b_über", "a_run", "references"),
                edge("a_runner", "a_runner", "calls"),
            ],
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(INDEX_FILE);
        write(&path, &g, Some(&[0, 0, 1])).unwrap();
        let ix = Index::open(&path).unwrap();
        assert_eq!((ix.node_count(), ix.edge_count()), (3, 3));

        let run = ix.by_id("a_run").unwrap();
        let n = ix.node(run);
        assert_eq!(
            (n.label, n.source_location, n.degree, n.community),
            ("run()", "L3", 2, Community::Id(0))
        );
        let runner = ix.by_id("a_runner").unwrap();
        assert_eq!(ix.degree(runner), 3, "a self-loop counts twice");
        assert_eq!(ix.incident(runner).count(), 2, "a self-loop is listed once");
        assert_eq!(ix.node(ix.by_id("b_über").unwrap()).norm_label, "uber");
        assert_eq!(ix.lower_label(ix.by_id("b_über").unwrap()), "über");
        assert!(ix.by_id("missing").is_none());

        let nbrs: Vec<_> = ix
            .neighbors(run)
            .map(|(e, o)| (e.relation, ix.node(o).id))
            .collect();
        assert_eq!(nbrs, [("calls", "a_runner"), ("references", "b_über")]);
        let e = ix.edge(0);
        assert_eq!(
            (e.context, e.source_location, e.confidence),
            (Some("call"), Some("L7"), Some("INFERRED"))
        );
        assert!((e.score.unwrap() - 0.85).abs() < 1e-6);

        let pre: Vec<_> = ix.by_norm_prefix("run").map(|i| ix.node(i).id).collect();
        assert_eq!(pre, ["a_run", "a_runner"]);
        let sub = ix.substring_candidates("unn").unwrap();
        assert_eq!(
            sub.iter().map(|&i| ix.node(i).id).collect::<Vec<_>>(),
            ["a_runner"]
        );
        assert!(ix.substring_candidates("ru").is_none());
    }

    #[test]
    fn loads_graphify_json_with_missing_and_null_fields() {
        let json = r#"{"directed": false, "nodes": [
            {"id": "a", "label": "Aé", "source_file": "x.py", "source_location": null, "community": 2},
            {"id": "b", "label": "b.py", "source_file": "b.py", "community": null},
            {"id": "c"}
          ], "links": [
            {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED", "context": "call", "source_file": "x.py", "source_location": "L4"},
            {"source": "b", "target": "c", "relation": "uses"},
            {"source": "a", "target": "zz", "relation": "calls"}
          ]}"#;
        let ix = Index::from_graph_json(json.as_bytes()).unwrap();
        assert_eq!((ix.node_count(), ix.edge_count()), (3, 2));
        let a = ix.node(0);
        assert_eq!(
            (a.label, a.source_location, a.community.to_string()),
            ("Aé", "None", "2".to_string())
        );
        let b = ix.node(1);
        assert_eq!(
            (b.community.to_string(), b.kind),
            ("None".to_string(), kind::FILE)
        );
        let c = ix.node(2);
        assert_eq!(
            (c.label, c.source_file, c.community.to_string()),
            ("", "", String::new())
        );
        let e = ix.edge(1);
        assert_eq!(
            (e.relation, e.confidence, e.context, e.source_file),
            ("uses", None, None, None)
        );
        assert_eq!(ix.lower_label(0), "aé");
    }

    #[test]
    fn rejects_foreign_and_truncated_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.dij");
        fs::write(
            &p,
            b"not an index at all, just some bytes padding the header",
        )
        .unwrap();
        assert!(Index::open(&p).is_err());
        let g = Graph::assemble(vec![node("a", "A")], vec![]);
        write(&p, &g, None).unwrap();
        let bytes = fs::read(&p).unwrap();
        fs::write(&p, &bytes[..bytes.len() - 4]).unwrap();
        assert!(Index::open(&p).is_err());
    }
}

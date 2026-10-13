//! `dijkstral-out/graph.dij`: the graph in a form queries read in place from a memory map.
//!
//! `graph.json` is the compatibility contract; this file is dijkstral's own and exists so a query,
//! a hook or an MCP call never parses JSON. Everything is little-endian and every section starts
//! on an 8-byte boundary.
//!
//! ```text
//! header   magic "DJKIDX01", then u64 offset/length pairs for each section (see `Section`)
//! strings  u32 offsets (count + 1) and UTF-8 bytes: every id, label, path, relation, context…
//! nodes    fixed records (`NODE_SIZE` bytes) in graph node order, then one kind byte per node
//! edges    fixed records (`EDGE_SIZE` bytes), one per link, in graph.json link order
//! adj      CSR over undirected incidence: u32 offsets (nodes + 1), u32 edge indices
//! by_id    node indices sorted by id (binary search)
//! by_norm  node indices sorted by (norm_label, id) (exact and prefix lookup)
//! trigram  sorted u32 trigram keys, u32 offsets, u32 posting node indices (norm_label substrings)
//! ```
//!
//! Records never point outside their file, and every read is bounds-checked; a truncated or
//! foreign file is rejected at open.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use memmap2::Mmap;

use crate::facts::{Confidence, Loc, NodeKind};
use crate::graph::{norm_label, Graph};

pub const INDEX_FILE: &str = "graph.dij";
const MAGIC: &[u8; 8] = b"DJKIDX01";
const NONE: u32 = u32::MAX;
const NODE_SIZE: usize = 32;
const EDGE_SIZE: usize = 32;

#[derive(Clone, Copy)]
enum Section {
    Strings = 0,
    Nodes,
    Edges,
    Adj,
    ById,
    ByNorm,
    Trigram,
}
const SECTIONS: usize = 7;
const HEADER_SIZE: usize = 8 + 8 + SECTIONS * 16;

/// Node kinds as stored (stable numbering).
fn kind_code(k: NodeKind) -> u8 {
    match k {
        NodeKind::File => 0,
        NodeKind::Def => 1,
        NodeKind::Stub => 2,
        NodeKind::External => 3,
        NodeKind::Rationale => 4,
    }
}

fn confidence_code(c: Confidence) -> u8 {
    match c {
        Confidence::Extracted => 0,
        Confidence::Inferred => 1,
        Confidence::Ambiguous => 2,
    }
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

struct Strings {
    ids: HashMap<String, u32>,
    list: Vec<String>,
}

impl Strings {
    fn new() -> Strings {
        Strings {
            ids: HashMap::new(),
            list: Vec::new(),
        }
    }
    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.ids.get(s) {
            return i;
        }
        let i = self.list.len() as u32;
        self.ids.insert(s.to_string(), i);
        self.list.push(s.to_string());
        i
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

/// Write `graph.dij` for `graph` (with `community` per node when clustered) atomically.
pub fn write(path: &Path, graph: &Graph, communities: Option<&[u32]>) -> Result<()> {
    let n = graph.nodes.len();
    let mut strings = Strings::new();
    let index_of: HashMap<&str, u32> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| (nd.id.as_str(), i as u32))
        .collect();

    let norms: Vec<String> = graph.nodes.iter().map(|nd| norm_label(&nd.label)).collect();
    let mut node_bytes = Vec::with_capacity(n * NODE_SIZE);
    let mut degree = vec![0u32; n];
    let mut incident: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut edge_bytes = Vec::with_capacity(graph.edges.len() * EDGE_SIZE);
    for (ei, e) in graph.edges.iter().enumerate() {
        let (Some(&s), Some(&t)) = (
            index_of.get(e.source.as_str()),
            index_of.get(e.target.as_str()),
        ) else {
            continue;
        };
        degree[s as usize] += 1;
        incident[s as usize].push(ei as u32);
        if t != s {
            degree[t as usize] += 1;
            incident[t as usize].push(ei as u32);
        }
        let rel = strings.intern(&e.relation);
        let ctx = e.context.as_deref().map_or(NONE, |c| strings.intern(c));
        let file = strings.intern(&e.source_file);
        let line = match e.loc {
            Loc::Line(l) => l,
            _ => 0,
        };
        let score = e.score.unwrap_or_else(|| e.confidence.default_score()) as f32;
        put_u32s(&mut edge_bytes, [s, t, rel, ctx, file, line]);
        edge_bytes.extend_from_slice(&score.to_le_bytes());
        edge_bytes.push(confidence_code(e.confidence));
        edge_bytes.extend_from_slice(&[0; 3]);
    }
    for (i, nd) in graph.nodes.iter().enumerate() {
        let id = strings.intern(&nd.id);
        let label = strings.intern(&nd.label);
        let norm = strings.intern(&norms[i]);
        let file = strings.intern(&nd.source_file);
        let file_type = strings.intern(&nd.file_type);
        let line = match nd.loc {
            Loc::Line(l) => l,
            _ => 0,
        };
        let community = communities.map_or(NONE, |c| c[i]);
        put_u32s(
            &mut node_bytes,
            [id, label, norm, file, file_type, line, community, degree[i]],
        );
    }
    let kinds: Vec<u8> = graph.nodes.iter().map(|nd| kind_code(nd.kind)).collect();

    let mut adj = Vec::with_capacity((n + 1) * 4 + graph.edges.len() * 8);
    let mut offset = 0u32;
    let mut offsets = Vec::with_capacity(n + 1);
    for list in &incident {
        offsets.push(offset);
        offset += list.len() as u32;
    }
    offsets.push(offset);
    put_u32s(&mut adj, offsets);
    put_u32s(&mut adj, incident.iter().flatten().copied());

    let mut by_id: Vec<u32> = (0..n as u32).collect();
    by_id.sort_by(|&a, &b| graph.nodes[a as usize].id.cmp(&graph.nodes[b as usize].id));
    let mut by_norm: Vec<u32> = (0..n as u32).collect();
    by_norm.sort_by(|&a, &b| {
        (
            norms[a as usize].as_str(),
            graph.nodes[a as usize].id.as_str(),
        )
            .cmp(&(
                norms[b as usize].as_str(),
                graph.nodes[b as usize].id.as_str(),
            ))
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

    // Nodes section: records, then the kind byte per node.
    let mut nodes_section = node_bytes;
    nodes_section.extend_from_slice(&kinds);

    let sections: [Vec<u8>; SECTIONS] = [
        str_bytes,
        nodes_section,
        edge_bytes,
        adj,
        {
            let mut b = Vec::new();
            put_u32s(&mut b, by_id);
            b
        },
        {
            let mut b = Vec::new();
            put_u32s(&mut b, by_norm);
            b
        },
        tri,
    ];
    let mut out =
        Vec::with_capacity(HEADER_SIZE + sections.iter().map(|s| s.len() + 8).sum::<usize>());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&(graph.edges.len() as u32).to_le_bytes());
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

    let tmp = path.with_extension("dij.tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(&out)?;
    f.sync_all().ok();
    fs::rename(&tmp, path)?;
    Ok(())
}

/// A memory-mapped `graph.dij`.
pub struct Index {
    map: Mmap,
    nodes: usize,
    edges: usize,
    sec: [(usize, usize); SECTIONS],
    strings: usize,
}

/// A node as read from the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeRef<'a> {
    pub index: u32,
    pub id: &'a str,
    pub label: &'a str,
    pub norm_label: &'a str,
    pub source_file: &'a str,
    pub file_type: &'a str,
    /// 1-based line, or 0 when the node has none.
    pub line: u32,
    pub community: Option<u32>,
    pub degree: u32,
    pub kind: u8,
}

/// An edge as read from the index (direction as written in graph.json).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeRef<'a> {
    pub index: u32,
    pub source: u32,
    pub target: u32,
    pub relation: &'a str,
    pub context: Option<&'a str>,
    pub source_file: &'a str,
    pub line: u32,
    pub score: f32,
    pub confidence: u8,
}

impl Index {
    pub fn open(path: &Path) -> Result<Index> {
        let file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        // SAFETY: the file is only read; dijkstral replaces it by rename, never in place.
        let map = unsafe { Mmap::map(&file)? };
        if map.len() < HEADER_SIZE || &map[..8] != MAGIC {
            bail!("{} is not a dijkstral index", path.display());
        }
        let u32_at = |o: usize| u32::from_le_bytes(map[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_le_bytes(map[o..o + 8].try_into().unwrap()) as usize;
        let nodes = u32_at(8) as usize;
        let edges = u32_at(12) as usize;
        let mut sec = [(0usize, 0usize); SECTIONS];
        for (i, s) in sec.iter_mut().enumerate() {
            let at = 16 + i * 16;
            *s = (u64_at(at), u64_at(at + 8));
            if s.0.checked_add(s.1).is_none_or(|end| end > map.len()) {
                bail!("{} is truncated", path.display());
            }
        }
        let strings = if sec[0].1 >= 4 {
            u32_at(sec[0].0) as usize
        } else {
            0
        };
        let idx = Index {
            map,
            nodes,
            edges,
            sec,
            strings,
        };
        let need = [
            (Section::Strings, 4 + (strings + 1) * 4),
            (Section::Nodes, nodes * NODE_SIZE + nodes),
            (Section::Edges, edges * EDGE_SIZE),
            (Section::Adj, (nodes + 1) * 4),
            (Section::ById, nodes * 4),
            (Section::ByNorm, nodes * 4),
            (Section::Trigram, 4),
        ];
        for (s, min) in need {
            if idx.sec[s as usize].1 < min {
                bail!("{} is truncated", path.display());
            }
        }
        Ok(idx)
    }

    pub fn node_count(&self) -> usize {
        self.nodes
    }

    pub fn edge_count(&self) -> usize {
        self.edges
    }

    fn sec(&self, s: Section) -> &[u8] {
        let (o, l) = self.sec[s as usize];
        &self.map[o..o + l]
    }

    fn u32_in(bytes: &[u8], i: usize) -> u32 {
        u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
    }

    pub fn string(&self, i: u32) -> &str {
        if i == NONE || i as usize >= self.strings {
            return "";
        }
        let s = self.sec(Section::Strings);
        let a = Self::u32_in(s, 1 + i as usize) as usize;
        let b = Self::u32_in(s, 2 + i as usize) as usize;
        let data = &s[4 + (self.strings + 1) * 4..];
        std::str::from_utf8(data.get(a..b).unwrap_or(&[])).unwrap_or("")
    }

    pub fn node(&self, i: u32) -> NodeRef<'_> {
        let s = self.sec(Section::Nodes);
        let rec = &s[i as usize * NODE_SIZE..(i as usize + 1) * NODE_SIZE];
        let f = |k: usize| Self::u32_in(rec, k);
        let community = f(6);
        NodeRef {
            index: i,
            id: self.string(f(0)),
            label: self.string(f(1)),
            norm_label: self.string(f(2)),
            source_file: self.string(f(3)),
            file_type: self.string(f(4)),
            line: f(5),
            community: (community != NONE).then_some(community),
            degree: f(7),
            kind: s[self.nodes * NODE_SIZE + i as usize],
        }
    }

    pub fn edge(&self, i: u32) -> EdgeRef<'_> {
        let s = self.sec(Section::Edges);
        let rec = &s[i as usize * EDGE_SIZE..(i as usize + 1) * EDGE_SIZE];
        let f = |k: usize| Self::u32_in(rec, k);
        let ctx = f(3);
        EdgeRef {
            index: i,
            source: f(0),
            target: f(1),
            relation: self.string(f(2)),
            context: (ctx != NONE).then(|| self.string(ctx)),
            source_file: self.string(f(4)),
            line: f(5),
            score: f32::from_le_bytes(rec[24..28].try_into().unwrap()),
            confidence: rec[28],
        }
    }

    /// Indices of the edges incident to node `i`, in graph.json link order.
    pub fn incident(&self, i: u32) -> impl Iterator<Item = u32> + '_ {
        let s = self.sec(Section::Adj);
        let a = Self::u32_in(s, i as usize) as usize;
        let b = Self::u32_in(s, i as usize + 1) as usize;
        let base = self.nodes + 1;
        (a..b).map(move |k| Self::u32_in(s, base + k))
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
            match self.node(n).id.cmp(id) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(n),
            }
        }
        None
    }

    /// Nodes whose `norm_label` starts with `prefix` (exact matches first), in (norm_label, id)
    /// order.
    pub fn by_norm_prefix(&self, prefix: &str) -> impl Iterator<Item = u32> + '_ {
        let s = self.sec(Section::ByNorm);
        let (mut lo, mut hi) = (0usize, self.nodes);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.node(Self::u32_in(s, mid)).norm_label < prefix {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let prefix = prefix.to_string();
        (lo..self.nodes)
            .map(move |k| Self::u32_in(s, k))
            .take_while(move |&n| self.node(n).norm_label.starts_with(prefix.as_str()))
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
    use crate::facts::{Edge, Node};

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
            ],
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(INDEX_FILE);
        write(&path, &g, Some(&[0, 0, 1])).unwrap();
        let ix = Index::open(&path).unwrap();
        assert_eq!((ix.node_count(), ix.edge_count()), (3, 2));

        let run = ix.by_id("a_run").unwrap();
        let n = ix.node(run);
        assert_eq!(
            (n.label, n.line, n.degree, n.community),
            ("run()", 3, 2, Some(0))
        );
        assert_eq!(ix.node(ix.by_id("b_über").unwrap()).norm_label, "uber");
        assert!(ix.by_id("missing").is_none());

        let nbrs: Vec<_> = ix
            .neighbors(run)
            .map(|(e, o)| (e.relation, ix.node(o).id))
            .collect();
        assert_eq!(nbrs, [("calls", "a_runner"), ("references", "b_über")]);
        let e = ix.edge(0);
        assert_eq!((e.context, e.line, e.confidence), (Some("call"), 7, 1));
        assert!((e.score - 0.85).abs() < 1e-6);

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

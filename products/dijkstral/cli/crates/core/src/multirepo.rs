//! Cross-repo graphs (`docs/graphify-multirepo.md`): `merge-graphs` (§3) and the global graph
//! (§5), as Graphify builds them. Both namespace each repository's node ids as `<tag>::<id>`,
//! share `external` nodes by their raw id, and keep NetworkX's insertion and iteration order, so
//! the outputs match Graphify's byte for byte.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::nodelink::{
    dumps_compact, int_community, node_key, py_str, truthy, Attrs, InputGraph, NxGraph,
};
use crate::pipeline::write_atomic;
use crate::pyjson::{dumps_indent, Json};

// ---------------------------------------------------------------------------------------------
// Tags

/// Python's `PurePath(p).name` of `p`'s `up`-th lexical parent: `..` is a name, `.` and the root
/// are not.
fn ancestor_name(p: &Path, up: usize) -> String {
    let comps: Vec<Component> = p
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    let keep = comps.len().saturating_sub(up);
    match comps[..keep].last() {
        Some(Component::Normal(s)) => s.to_string_lossy().into_owned(),
        Some(Component::ParentDir) => "..".into(),
        _ => String::new(),
    }
}

/// The tags `merge-graphs` gives its inputs (§3.2): the directory holding each graph's output
/// directory (`<repo>/graphify-out/graph.json` → `repo`), read from the path as given, `repo`
/// when there is none. Names shared by several inputs are qualified with their parent
/// (`<parent>_<repo>`) where the path has one, and any name still taken gets the first free
/// `-2`, `-3`… suffix. Returns the tags and whether any name collided.
pub fn merge_tags(paths: &[PathBuf]) -> (Vec<String>, bool) {
    let base: Vec<String> = paths
        .iter()
        .map(|p| {
            let n = ancestor_name(p, 2);
            if n.is_empty() {
                "repo".into()
            } else {
                n
            }
        })
        .collect();
    let mut count: HashMap<&str, usize> = HashMap::new();
    for b in &base {
        *count.entry(b).or_default() += 1;
    }
    let collided = count.values().any(|&c| c > 1);
    if !collided {
        return (base, false);
    }
    let wanted: Vec<String> = paths
        .iter()
        .zip(&base)
        .map(|(p, b)| {
            let parent = ancestor_name(p, 3);
            if count[b.as_str()] > 1 && !parent.is_empty() {
                format!("{parent}_{b}")
            } else {
                b.clone()
            }
        })
        .collect();
    let mut taken: HashSet<String> = HashSet::new();
    let mut tags = Vec::with_capacity(wanted.len());
    for w in wanted {
        let mut tag = w.clone();
        let mut k = 2;
        while taken.contains(&tag) {
            tag = format!("{w}-{k}");
            k += 1;
        }
        taken.insert(tag.clone());
        tags.push(tag);
    }
    (tags, true)
}

/// The default tag of `global add` (§5.3): the directory that holds the graph's output
/// directory, from the resolved path.
pub fn global_default_tag(graph: &Path) -> String {
    let resolved = graph.canonicalize().unwrap_or_else(|_| graph.to_path_buf());
    let n = ancestor_name(&resolved, 2);
    if n.is_empty() {
        "repo".into()
    } else {
        n
    }
}

// ---------------------------------------------------------------------------------------------
// merge-graphs

/// One `merge-graphs` input.
pub struct MergeInput {
    pub tag: String,
    pub graph: InputGraph,
}

/// The merged graph and its counts.
pub struct Merged {
    /// `graph.json` text, compact and ASCII-escaped like every Graphify graph.
    pub json: String,
    pub nodes: usize,
    pub edges: usize,
    /// With `--previous`: how many nodes took their community from it.
    pub restored: Option<usize>,
}

/// The namespaced id of a repository node.
fn namespaced(tag: &str, id: &Value) -> Value {
    Value::String(format!("{tag}::{}", py_str(id)))
}

/// `repo` (set) and `local_id` (kept when already present), in that key order.
fn tag_attrs(attrs: &mut Attrs, tag: &str, id: &Value) {
    attrs.insert("repo".into(), Value::String(tag.into()));
    if !attrs.contains_key("local_id") {
        attrs.insert("local_id".into(), Value::String(py_str(id)));
    }
}

/// Merge graphs as `graphify merge-graphs` does (§3): repository nodes namespaced by tag, their
/// integer communities shifted past those already merged (the original kept as
/// `local_community`), `external` nodes shared, one edge per node pair, hyperedges namespaced.
/// With `previous`, every node's community is the previous merged graph's (by id) or absent.
pub fn merge(inputs: &[MergeInput], previous: Option<&Value>) -> Merged {
    let mut g = NxGraph::new();
    let mut hyperedges = Vec::new();
    for input in inputs {
        let tag = &input.tag;
        let offset = g
            .live()
            .filter_map(|s| int_community(g.attrs(s).get("community")))
            .max()
            .map_or(0, |m| m + 1);
        for (k, v) in &input.graph.graph {
            g.graph.insert(k.clone(), v.clone());
        }
        let mut slots = Vec::with_capacity(input.graph.nodes.len());
        let mut local: HashSet<String> = HashSet::new();
        for (i, (id, attrs)) in input.graph.nodes.iter().enumerate() {
            if input.graph.is_external(i) {
                slots.push(g.add_node(id, attrs.clone()));
                continue;
            }
            local.insert(node_key(id));
            let mut a = attrs.clone();
            let original = a.get("community").cloned();
            // Only a non-zero shift rewrites the value (a `true` community stays `true` at 0).
            let shifted = int_community(original.as_ref())
                .filter(|_| offset != 0)
                .map(|c| c + offset);
            if let Some(c) = shifted {
                a.insert("community".into(), Value::from(c));
            }
            tag_attrs(&mut a, tag, id);
            if let (Some(_), Some(orig)) = (shifted, original) {
                a.insert("local_community".into(), orig);
            }
            slots.push(g.add_node(&namespaced(tag, id), a));
        }
        for (s, t, attrs) in &input.graph.edges {
            g.add_edge(slots[*s], slots[*t], attrs.clone());
        }
        for h in &input.graph.hyperedges {
            let Some(h) = h.as_object() else { continue };
            let Some(id) = h.get("id").filter(|v| !v.is_null()) else {
                continue;
            };
            let mut out = h.clone();
            out.insert("id".into(), namespaced(tag, id));
            if let Some(Value::Array(members)) = h.get("nodes") {
                let members = members
                    .iter()
                    .map(|m| {
                        if local.contains(&node_key(m)) {
                            namespaced(tag, m)
                        } else {
                            m.clone()
                        }
                    })
                    .collect();
                out.insert("nodes".into(), Value::Array(members));
            }
            hyperedges.push(Value::Object(out));
        }
    }
    g.graph.shift_remove("hyperedges");
    if !hyperedges.is_empty() {
        g.graph
            .insert("hyperedges".into(), Value::Array(hyperedges.clone()));
    }
    let restored = previous.map(|prev| restore_communities(&mut g, prev));
    let nodes: Vec<Value> = g.live().map(|s| g.node_json(s)).collect();
    let links: Vec<Value> = g
        .iter_edges()
        .map(|(_, _, e)| {
            let mut m = e.attrs.clone();
            m.insert("source".into(), g.id(e.src).clone());
            m.insert("target".into(), g.id(e.tgt).clone());
            Value::Object(m)
        })
        .collect();
    let (node_count, edge_count) = (nodes.len(), links.len());
    let mut doc = Map::new();
    doc.insert("directed".into(), Value::Bool(false));
    doc.insert("multigraph".into(), Value::Bool(false));
    doc.insert("graph".into(), Value::Object(g.graph.clone()));
    doc.insert("nodes".into(), Value::Array(nodes));
    doc.insert("links".into(), Value::Array(links));
    doc.insert("hyperedges".into(), Value::Array(hyperedges));
    Merged {
        json: dumps_compact(&Value::Object(doc)),
        nodes: node_count,
        edges: edge_count,
        restored,
    }
}

/// `--previous` (§3.6): each node takes the previous graph's integer community for its id;
/// nodes it does not cover lose `community`. Returns how many were restored.
fn restore_communities(g: &mut NxGraph, prev: &Value) -> usize {
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for n in prev
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(c)) = (n.get("id"), n.get("community")) else {
            continue;
        };
        if int_community(Some(c)).is_some() {
            by_id.insert(node_key(id), c.clone());
        }
    }
    let slots: Vec<usize> = g.live().collect();
    let mut restored = 0;
    for s in slots {
        let key = node_key(g.id(s));
        let attrs = g.attrs_mut(s);
        match by_id.get(&key) {
            Some(c) => {
                attrs.insert("community".into(), c.clone());
                restored += 1;
            }
            None => {
                attrs.shift_remove("community");
            }
        }
    }
    restored
}

/// Read and parse one graph file.
pub fn read_graph(path: &Path) -> Result<(Value, Vec<u8>)> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let doc: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    Ok((doc, bytes))
}

// ---------------------------------------------------------------------------------------------
// The global graph

pub const GLOBAL_GRAPH: &str = "global-graph.json";
pub const GLOBAL_MANIFEST: &str = "global-manifest.json";

/// A repository recorded in the global manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoEntry {
    pub tag: String,
    pub added_at: String,
    pub source_path: String,
    pub node_count: i64,
    pub edge_count: i64,
    pub source_hash: String,
    /// Keys this version does not know, kept as read.
    pub extra: Vec<(String, Value)>,
}

/// The global graph and manifest under a home directory (`~/.dijkstral` by default, §5.1).
pub struct GlobalStore {
    pub dir: PathBuf,
}

/// What `global add` did.
#[derive(Debug, PartialEq)]
pub enum AddOutcome {
    /// The graph's bytes match what this tag was last added from; nothing written.
    Unchanged,
    Added {
        added: usize,
        pruned: usize,
    },
}

/// A `global add` result, with the source path the tag pointed to before when it changed.
pub struct Added {
    pub outcome: AddOutcome,
    pub moved_from: Option<String>,
}

impl GlobalStore {
    pub fn new(dir: impl Into<PathBuf>) -> GlobalStore {
        GlobalStore { dir: dir.into() }
    }

    pub fn graph_path(&self) -> PathBuf {
        self.dir.join(GLOBAL_GRAPH)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join(GLOBAL_MANIFEST)
    }

    /// Repositories in manifest order (empty when there is no manifest yet).
    pub fn repos(&self) -> Result<Vec<RepoEntry>> {
        let path = self.manifest_path();
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let doc: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not valid JSON", path.display()))?;
        let mut out = Vec::new();
        for (tag, e) in doc
            .get("repos")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let i = |k: &str| e.get(k).and_then(Value::as_i64).unwrap_or(0);
            let known = [
                "added_at",
                "source_path",
                "node_count",
                "edge_count",
                "source_hash",
            ];
            out.push(RepoEntry {
                tag: tag.clone(),
                added_at: s("added_at"),
                source_path: s("source_path"),
                node_count: i("node_count"),
                edge_count: i("edge_count"),
                source_hash: s("source_hash"),
                extra: e
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(k, _)| !known.contains(&k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            });
        }
        Ok(out)
    }

    fn load_graph(&self) -> Result<NxGraph> {
        let path = self.graph_path();
        match fs::read(&path) {
            Ok(bytes) => {
                let doc: Value = serde_json::from_slice(&bytes)
                    .with_context(|| format!("{} is not valid JSON", path.display()))?;
                NxGraph::load_simple(&doc).with_context(|| format!("loading {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(NxGraph::new()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    fn save(&self, g: &NxGraph, repos: &[RepoEntry]) -> Result<()> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        write_atomic(&self.graph_path(), global_json(g).as_bytes())?;
        write_atomic(&self.manifest_path(), manifest_json(repos).as_bytes())?;
        Ok(())
    }

    /// `global add` (§5.3): replace `tag`'s nodes with `graph`'s. `bytes` are the file's
    /// contents (their hash decides "unchanged"); `source` is its resolved path.
    pub fn add(&self, graph: &Value, bytes: &[u8], source: &Path, tag: &str) -> Result<Added> {
        let input = InputGraph::load(graph)?;
        let hash = source_hash(bytes);
        let mut repos = self.repos()?;
        let source_path = source.to_string_lossy().into_owned();
        let prior = repos.iter().position(|r| r.tag == tag);
        let moved_from = prior
            .map(|i| &repos[i])
            .filter(|r| r.source_path != source_path)
            .map(|r| r.source_path.clone());
        if prior.is_some_and(|i| repos[i].source_hash == hash) {
            return Ok(Added {
                outcome: AddOutcome::Unchanged,
                moved_from,
            });
        }
        let mut g = self.load_graph()?;
        let (added, pruned) = add_to_global(&mut g, &input, tag);
        let entry = RepoEntry {
            tag: tag.into(),
            added_at: utc_now_iso(),
            source_path,
            node_count: added as i64,
            edge_count: input.edge_count() as i64,
            source_hash: hash,
            extra: Vec::new(),
        };
        match prior {
            Some(i) => repos[i] = entry,
            None => repos.push(entry),
        }
        self.save(&g, &repos)?;
        Ok(Added {
            outcome: AddOutcome::Added { added, pruned },
            moved_from,
        })
    }

    /// `global remove` (§5.4): drop `tag`'s nodes and its manifest entry. Returns the number of
    /// nodes removed.
    pub fn remove(&self, tag: &str) -> Result<usize> {
        let mut repos = self.repos()?;
        let Some(i) = repos.iter().position(|r| r.tag == tag) else {
            bail!("no repository tagged '{tag}' in the global graph");
        };
        repos.remove(i);
        let mut g = self.load_graph()?;
        let pruned = prune(&mut g, tag);
        self.save(&g, &repos)?;
        Ok(pruned)
    }
}

/// Remove the nodes tagged `tag`.
fn prune(g: &mut NxGraph, tag: &str) -> usize {
    let doomed: Vec<usize> = g
        .live()
        .filter(|&s| g.attrs(s).get("repo").and_then(Value::as_str) == Some(tag))
        .collect();
    for &s in &doomed {
        g.remove_node(s);
    }
    doomed.len()
}

/// Merge one repository graph into the global graph (§5.3): the tag's old nodes are removed;
/// a node without a source file whose label names an existing such node (stubs and externals)
/// joins it; other nodes are namespaced (externals keep their id); self-loops are dropped and
/// every edge records its direction in `_src`/`_tgt`. Returns (nodes added, nodes pruned).
fn add_to_global(g: &mut NxGraph, input: &InputGraph, tag: &str) -> (usize, usize) {
    let pruned = prune(g, tag);
    let mut by_label: HashMap<String, usize> = HashMap::new();
    for s in g.live() {
        let a = g.attrs(s);
        if !truthy(a.get("source_file")) && truthy(a.get("label")) {
            by_label.insert(node_key(&a["label"]), s);
        }
    }
    let mut slots = Vec::with_capacity(input.nodes.len());
    let mut added = 0;
    for (i, (id, attrs)) in input.nodes.iter().enumerate() {
        if !truthy(attrs.get("source_file")) && truthy(attrs.get("label")) {
            if let Some(&s) = by_label.get(&node_key(&attrs["label"])) {
                slots.push(s);
                continue;
            }
        }
        added += 1;
        if input.is_external(i) {
            slots.push(g.add_node(id, attrs.clone()));
        } else {
            let mut a = attrs.clone();
            tag_attrs(&mut a, tag, id);
            slots.push(g.add_node(&namespaced(tag, id), a));
        }
    }
    for (s, t, attrs) in &input.edges {
        let (u, v) = (slots[*s], slots[*t]);
        if u == v {
            continue;
        }
        let mut a = attrs.clone();
        a.insert("_src".into(), g.id(u).clone());
        a.insert("_tgt".into(), g.id(v).clone());
        g.add_edge(u, v, a);
    }
    (added, pruned)
}

/// The global graph's file (§5.2): NetworkX's own node-link output, so links carry their
/// direction in `_src`/`_tgt` while `source`/`target` follow iteration order; no `hyperedges`.
fn global_json(g: &NxGraph) -> String {
    let nodes: Vec<Value> = g.live().map(|s| g.node_json(s)).collect();
    let links: Vec<Value> = g
        .iter_edges()
        .map(|(n, m, e)| {
            let mut a = e.attrs.clone();
            a.insert("source".into(), g.id(n).clone());
            a.insert("target".into(), g.id(m).clone());
            Value::Object(a)
        })
        .collect();
    let mut doc = Map::new();
    doc.insert("directed".into(), Value::Bool(false));
    doc.insert("multigraph".into(), Value::Bool(false));
    doc.insert("graph".into(), Value::Object(g.graph.clone()));
    doc.insert("nodes".into(), Value::Array(nodes));
    doc.insert("links".into(), Value::Array(links));
    dumps_compact(&Value::Object(doc))
}

fn json_of(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) if !n.is_f64() => Json::Int(i),
            _ => Json::Float(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => Json::Str(s.clone()),
        Value::Array(a) => Json::Array(a.iter().map(json_of).collect()),
        Value::Object(m) => Json::Object(m.iter().map(|(k, v)| (k.clone(), json_of(v))).collect()),
    }
}

/// `global-manifest.json`: `json.dumps(indent=2)`, no trailing newline.
fn manifest_json(repos: &[RepoEntry]) -> String {
    let repos = repos
        .iter()
        .map(|r| {
            let mut fields = vec![
                ("added_at".to_string(), Json::str(&r.added_at)),
                ("source_path".to_string(), Json::str(&r.source_path)),
                ("node_count".to_string(), Json::Int(r.node_count)),
                ("edge_count".to_string(), Json::Int(r.edge_count)),
                ("source_hash".to_string(), Json::str(&r.source_hash)),
            ];
            fields.extend(r.extra.iter().map(|(k, v)| (k.clone(), json_of(v))));
            (r.tag.clone(), Json::Object(fields))
        })
        .collect();
    dumps_indent(&Json::Object(vec![
        ("version".into(), Json::Int(1)),
        ("repos".into(), Json::Object(repos)),
    ]))
}

/// The first 16 hex digits of the file's SHA-256.
pub fn source_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Python's `datetime.now(timezone.utc).isoformat()`; `SOURCE_DATE_EPOCH` pins it.
fn utc_now_iso() -> String {
    let (secs, micros) = match std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
    {
        Some(s) => (s, 0),
        None => {
            let d = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            (d.as_secs() as i64, d.subsec_micros())
        }
    };
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let time = format!("{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60);
    let frac = if micros == 0 {
        String::new()
    } else {
        format!(".{micros:06}")
    };
    format!("{y:04}-{m:02}-{d:02}T{time}{frac}+00:00")
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The home directory for dijkstral's own state (§5.1): `$DIJKSTRAL_HOME`, else `~/.dijkstral`.
pub fn dijkstral_home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("DIJKSTRAL_HOME").filter(|h| !h.is_empty()) {
        return Ok(PathBuf::from(h));
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or_else(|| anyhow!("neither DIJKSTRAL_HOME nor HOME is set"))?;
    Ok(PathBuf::from(home).join(".dijkstral"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tags(paths: &[&str]) -> Vec<String> {
        merge_tags(&paths.iter().map(PathBuf::from).collect::<Vec<_>>()).0
    }

    #[test]
    fn tags_follow_the_repo_directory() {
        assert_eq!(
            tags(&["one/two/graph.json", "three/four/merged.json"]),
            ["one", "three"]
        );
        assert_eq!(
            tags(&["graph.json", "proj/graphify-out/graph.json"]),
            ["repo", "proj"]
        );
        assert_eq!(
            tags(&["x/proj/g/graph.json", "proj/g/graph.json"]),
            ["x_proj", "proj"]
        );
        assert_eq!(
            tags(&["proj/g/a.json", "proj/g/a.json", "proj/g/a.json"]),
            ["proj", "proj-2", "proj-3"]
        );
        assert_eq!(
            tags(&["/a/X/proj/g/graph.json", "/b/X/proj/g/graph.json"]),
            ["X_proj", "X_proj-2"]
        );
        assert_eq!(tags(&["../g/graph.json", "z/o/graph.json"]), ["..", "z"]);
        // Graphify would name the third input `proj-2` again; dijkstral takes the next free one.
        assert_eq!(
            tags(&["proj/g/a.json", "proj-2/g/a.json", "proj/g/a.json"]),
            ["proj", "proj-2", "proj-3"]
        );
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn offsets_follow_the_largest_merged_community() {
        let a = InputGraph::load(&json!({"nodes": [
            {"id": "a", "community": 0},
            {"id": "e", "community": 50, "external": true}
        ]}))
        .unwrap();
        let b = InputGraph::load(&json!({"nodes": [{"id": "a", "community": 2}]})).unwrap();
        let m = merge(
            &[
                MergeInput {
                    tag: "x".into(),
                    graph: a,
                },
                MergeInput {
                    tag: "y".into(),
                    graph: b,
                },
            ],
            None,
        );
        assert!(
            m.json.contains(
                r#"{"community":53,"repo":"y","local_id":"a","local_community":2,"id":"y::a"}"#
            ),
            "{}",
            m.json
        );
    }
}

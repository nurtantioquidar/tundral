//! The `graph.json` merge driver (`docs/graphify-git.md` §4): a three-way merge of two graphs
//! keyed by node id, by link endpoints and by hyperedge id, written in the canonical order and
//! formatting of a build, so the result reads in dijkstral and Graphify alike.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context, Result};
use dijkstral_core::pyjson::{float_repr, write_str};
use rayon::prelude::*;
use serde_json::{Map, Number, Value};

/// Node attributes a build recomputes wholesale (clustering), so a change in them alone does not
/// keep a node the other side deleted.
const DERIVED_NODE_FIELDS: &[&str] = &["community", "community_name"];

/// What [`merge`] produced.
#[derive(Debug)]
pub struct Merged {
    /// The merged `graph.json`, compact and without a trailing newline, like a build's.
    pub json: String,
    /// Links dropped because the merge removed one of their nodes.
    pub dropped_links: usize,
}

/// Merge `current` (git's `%A`) and `other` (`%B`) against their common ancestor `base` (`%O`).
///
/// An empty or unreadable base merges as if neither side had a common ancestor (a union). Fails
/// when `current` or `other` is not a graph, or when they disagree on `directed`/`multigraph`.
pub fn merge(base: &[u8], current: &[u8], other: &[u8]) -> Result<Merged> {
    let ours = Doc::parse(current).context("current version")?;
    let theirs = Doc::parse(other).context("other version")?;
    if ours.directed != theirs.directed || ours.multigraph != theirs.multigraph {
        bail!("the two versions disagree on `directed` or `multigraph`");
    }
    // Two `--no-cluster` graphs stay one: links keyed the way the raw output dedupes them.
    let kind = Kind {
        raw: ours.raw && theirs.raw,
        directed: ours.directed,
        multigraph: ours.multigraph,
    };
    let base = Doc::parse(base).ok().filter(|b| {
        b.directed == ours.directed && b.multigraph == ours.multigraph && (b.raw || !kind.raw)
    });
    let base = base.as_ref();

    let nodes = merge_items(
        base.map(|b| &b.nodes),
        &ours.nodes,
        &theirs.nodes,
        DERIVED_NODE_FIELDS,
        false,
    );
    let mut links = merge_items(
        base.map(|b| b.links(kind)).as_ref(),
        &ours.links(kind),
        &theirs.links(kind),
        &[],
        true,
    );
    // A link whose node the merge removed goes with it; links that never had a node entry on any
    // side (a raw graph's externals) are left alone.
    let kept: HashSet<&str> = nodes.iter().map(|(k, _)| k.as_str()).collect();
    let removed = |k: &str| {
        !kept.contains(k)
            && (ours.nodes.map.contains_key(k)
                || theirs.nodes.map.contains_key(k)
                || base.is_some_and(|b| b.nodes.map.contains_key(k)))
    };
    let before = links.len();
    links.retain(|(_, l)| {
        let (s, t) = endpoints(l);
        !removed(&s) && !removed(&t)
    });
    let dropped_links = before - links.len();
    let hyperedges = merge_items(
        base.map(|b| &b.hyperedges),
        &ours.hyperedges,
        &theirs.hyperedges,
        &[],
        false,
    );
    let graph = merge_fields(base.map(|b| &b.graph), &ours.graph, &theirs.graph, false);
    let mut extra = merge_fields(base.map(|b| &b.extra), &ours.extra, &theirs.extra, false);
    // The merged graph was built at neither commit.
    extra.shift_remove("built_at_commit");
    let commit = match (
        ours.extra.get("built_at_commit"),
        theirs.extra.get("built_at_commit"),
    ) {
        (Some(a), Some(b)) if a == b => Some(a.clone()),
        _ => None,
    };

    let nodes: Vec<Value> = nodes.into_iter().map(|(_, v)| v).collect();
    let links: Vec<Value> = links.into_iter().map(|(_, v)| v).collect();
    let hyperedges: Vec<Value> = hyperedges.into_iter().map(|(_, v)| v).collect();
    let json = if kind.raw {
        write_raw(&nodes, &links, &hyperedges, &extra)
    } else {
        write(
            kind,
            &graph,
            nodes,
            links,
            &hyperedges,
            &extra,
            commit.as_ref(),
        )
    };
    Ok(Merged {
        json,
        dropped_links,
    })
}

/// How links are identified, and how the result is written.
#[derive(Clone, Copy)]
struct Kind {
    /// Both sides are raw `--no-cluster` output (`edges`, no header).
    raw: bool,
    directed: bool,
    multigraph: bool,
}

/// Items in first-seen order, keyed by identity.
#[derive(Default)]
struct Items {
    order: Vec<String>,
    map: HashMap<String, Value>,
}

impl Items {
    /// A repeated identity folds into the first entry, later keys winning (as NetworkX does).
    fn push(&mut self, key: String, value: Value) {
        if let Some(old) = self.map.get_mut(&key) {
            match (old, value) {
                (Value::Object(old), Value::Object(new)) => old.extend(new),
                (old, value) => *old = value,
            }
            return;
        }
        self.order.push(key.clone());
        self.map.insert(key, value);
    }
}

struct Doc {
    directed: bool,
    multigraph: bool,
    /// Raw `--no-cluster` output: `edges` and no `directed` header.
    raw: bool,
    /// `graph` without its `hyperedges` copy.
    graph: Map<String, Value>,
    nodes: Items,
    links: Vec<Value>,
    hyperedges: Items,
    /// Other top-level keys (`built_at_commit`, a raw graph's token counts), in file order.
    extra: Map<String, Value>,
}

impl Doc {
    fn parse(bytes: &[u8]) -> Result<Doc> {
        if bytes.iter().all(u8::is_ascii_whitespace) {
            bail!("empty file");
        }
        let Value::Object(mut top) = serde_json::from_slice(bytes).context("not JSON")? else {
            bail!("not a JSON object");
        };
        let flag = |top: &Map<String, Value>, k: &str| top.get(k).and_then(Value::as_bool);
        let directed = flag(&top, "directed").unwrap_or(false);
        let multigraph = flag(&top, "multigraph").unwrap_or(false);
        let raw = !top.contains_key("links")
            && top.contains_key("edges")
            && !top.contains_key("directed");
        // `shift_remove` keeps the order of the keys that remain.
        let Some(Value::Array(nodes_in)) = top.shift_remove("nodes") else {
            bail!("no `nodes` array");
        };
        let links = match top
            .shift_remove("links")
            .or_else(|| top.shift_remove("edges"))
        {
            Some(Value::Array(a)) => a,
            None => Vec::new(),
            Some(_) => bail!("`links` is not an array"),
        };
        if links
            .iter()
            .any(|l| !l.is_object() || l.get("source").is_none() || l.get("target").is_none())
        {
            bail!("a link has no `source` or `target`");
        }
        let mut graph = match top.shift_remove("graph") {
            Some(Value::Object(g)) => g,
            None | Some(Value::Null) => Map::new(),
            Some(_) => bail!("`graph` is not an object"),
        };
        let graph_hyper = graph.shift_remove("hyperedges");
        let hyper_in = match top.shift_remove("hyperedges").or(graph_hyper) {
            Some(Value::Array(a)) => a,
            _ => Vec::new(),
        };
        top.shift_remove("directed");
        top.shift_remove("multigraph");

        let mut nodes = Items::default();
        for n in nodes_in {
            let id = n.get("id").context("a node has no `id`")?;
            nodes.push(ident(id), n);
        }
        let mut hyperedges = Items::default();
        for h in hyper_in {
            let key = match h.get("id") {
                Some(id) => ident(id),
                None => format!("\0{h}"),
            };
            hyperedges.push(key, h);
        }
        Ok(Doc {
            directed,
            multigraph,
            raw,
            graph,
            nodes,
            links,
            hyperedges,
            extra: top,
        })
    }

    fn links(&self, kind: Kind) -> Items {
        let mut items = Items::default();
        for l in &self.links {
            items.push(link_key(l, kind), l.clone());
        }
        items
    }
}

/// A node id as a map key: strings as themselves, anything else as its JSON text (never equal
/// to a string id).
fn ident(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => format!("\0{other}"),
    }
}

fn endpoints(link: &Value) -> (String, String) {
    (ident(&link["source"]), ident(&link["target"]))
}

/// A link's identity: its endpoints (unordered unless the graph is directed), plus `key` in a
/// multigraph; in raw output, source, target and relation (what the raw writer dedupes on).
fn link_key(link: &Value, kind: Kind) -> String {
    let (mut a, mut b) = endpoints(link);
    if kind.raw {
        let relation = link.get("relation").unwrap_or(&Value::Null);
        return format!("{a}\0{b}\0{relation}");
    }
    if !kind.directed && b < a {
        std::mem::swap(&mut a, &mut b);
    }
    let mut key = format!("{a}\0{b}");
    if kind.multigraph {
        key.push('\0');
        key.push_str(&link.get("key").map_or_else(String::new, Value::to_string));
    }
    key
}

/// The three-way rule for one value: a side that kept the base value yields to the side that
/// changed it; when both changed, the other side wins (git's `%B`, as in Graphify), except that a
/// removal never beats a change.
fn pick<'a>(b: Option<&'a Value>, o: Option<&'a Value>, t: Option<&'a Value>) -> Option<&'a Value> {
    if o == t {
        o
    } else if o == b {
        t
    } else if t == b {
        o
    } else {
        t.or(o)
    }
}

/// Merge three objects key by key; keys in the current side's order, then the other side's new
/// ones. For links, `source` and `target` merge as one value, so a link is never half flipped.
fn merge_fields(
    b: Option<&Map<String, Value>>,
    o: &Map<String, Value>,
    t: &Map<String, Value>,
    link: bool,
) -> Map<String, Value> {
    let mut out = Map::new();
    let keys = o.keys().chain(t.keys().filter(|k| !o.contains_key(*k)));
    for k in keys {
        if link && (k == "source" || k == "target") {
            continue;
        }
        if let Some(v) = pick(b.and_then(|b| b.get(k)), o.get(k), t.get(k)) {
            out.insert(k.clone(), v.clone());
        }
    }
    if link {
        let ends = |m: &Map<String, Value>| {
            Value::Array(vec![
                m.get("source").cloned().unwrap_or(Value::Null),
                m.get("target").cloned().unwrap_or(Value::Null),
            ])
        };
        let (eb, eo, et) = (b.map(ends), ends(o), ends(t));
        if let Some(Value::Array(st)) = pick(eb.as_ref(), Some(&eo), Some(&et)) {
            out.insert("source".into(), st[0].clone());
            out.insert("target".into(), st[1].clone());
        }
    }
    out
}

/// Equal once the `ignore`d keys are left out.
fn same_ignoring(a: &Value, b: &Value, ignore: &[&str]) -> bool {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) if !ignore.is_empty() => {
            let strip = |m: &Map<String, Value>| {
                m.iter()
                    .filter(|(k, _)| !ignore.contains(&k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<HashMap<_, _>>()
            };
            strip(a) == strip(b)
        }
        _ => a == b,
    }
}

/// Merge two item sets against a base; current side's order, then the other side's additions.
fn merge_items(
    base: Option<&Items>,
    ours: &Items,
    theirs: &Items,
    derived: &[&str],
    link: bool,
) -> Vec<(String, Value)> {
    let keys = ours.order.iter().chain(
        theirs
            .order
            .iter()
            .filter(|k| !ours.map.contains_key(k.as_str())),
    );
    let mut out = Vec::with_capacity(ours.order.len().max(theirs.order.len()));
    for k in keys {
        let b = base.and_then(|b| b.map.get(k));
        let merged =
            match (ours.map.get(k), theirs.map.get(k)) {
                (Some(o), Some(t)) if o == t => Some(o.clone()),
                (Some(Value::Object(o)), Some(Value::Object(t))) => Some(Value::Object(
                    merge_fields(b.and_then(Value::as_object), o, t, link),
                )),
                (Some(o), Some(t)) => pick(b, Some(o), Some(t)).cloned(),
                // Present on one side only: an addition, or a deletion on the other side that stands
                // unless this side changed the item since the base.
                (Some(x), None) | (None, Some(x)) => match b {
                    Some(b) if same_ignoring(b, x, derived) => None,
                    _ => Some(x.clone()),
                },
                (None, None) => None,
            };
        if let Some(v) = merged {
            out.push((k.clone(), v));
        }
    }
    out
}

/// Write the graph as a build does (`graphify-format.md` §2): header keys first, nodes and links
/// sorted by their Python `json.dumps(sort_keys=True)` text, `id`/`label` and
/// `source`/`target`/`relation` leading their objects, compact, ASCII-escaped, no final newline.
fn write(
    kind: Kind,
    graph: &Map<String, Value>,
    nodes: Vec<Value>,
    links: Vec<Value>,
    hyperedges: &[Value],
    extra: &Map<String, Value>,
    commit: Option<&Value>,
) -> String {
    let mut out = String::with_capacity(1 << 16);
    out.push_str(&format!(
        "{{\"directed\":{},\"multigraph\":{},\"graph\":{{",
        kind.directed, kind.multigraph
    ));
    let mut first = true;
    if !hyperedges.is_empty() {
        out.push_str("\"hyperedges\":");
        compact_array(hyperedges, &mut out);
        first = false;
    }
    for (k, v) in graph {
        if !first {
            out.push(',');
        }
        first = false;
        write_str(k, &mut out);
        out.push(':');
        compact(v, &mut out);
    }
    out.push_str("},\"nodes\":[");
    write_sorted(nodes, &["id", "label"], &mut out);
    out.push_str("],\"links\":[");
    write_sorted(links, &["source", "target", "relation"], &mut out);
    out.push_str("],\"hyperedges\":");
    compact_array(hyperedges, &mut out);
    for (k, v) in extra {
        out.push(',');
        write_str(k, &mut out);
        out.push(':');
        compact(v, &mut out);
    }
    if let Some(c) = commit {
        out.push_str(",\"built_at_commit\":");
        compact(c, &mut out);
    }
    out.push('}');
    out
}

/// The raw `--no-cluster` shape: `nodes`, `edges`, `hyperedges`, then the other keys; arrays in
/// merged order and objects in their own key order, as the raw writer keeps insertion order.
fn write_raw(
    nodes: &[Value],
    links: &[Value],
    hyperedges: &[Value],
    extra: &Map<String, Value>,
) -> String {
    let mut out = String::with_capacity(1 << 16);
    out.push_str("{\"nodes\":");
    compact_array(nodes, &mut out);
    out.push_str(",\"edges\":");
    compact_array(links, &mut out);
    out.push_str(",\"hyperedges\":");
    compact_array(hyperedges, &mut out);
    for (k, v) in extra {
        out.push(',');
        write_str(k, &mut out);
        out.push(':');
        compact(v, &mut out);
    }
    out.push('}');
    out
}

fn write_sorted(items: Vec<Value>, head: &[&str], out: &mut String) {
    let mut keyed: Vec<(String, String)> = items
        .into_par_iter()
        .map(|v| {
            let mut key = String::new();
            sort_dump(&v, &mut key);
            let mut text = String::new();
            match &v {
                Value::Object(m) => {
                    let mut fields: Vec<(&String, &Value)> = Vec::with_capacity(m.len());
                    for h in head {
                        if let Some((k, v)) = m.get_key_value(*h) {
                            fields.push((k, v));
                        }
                    }
                    let mut rest: Vec<(&String, &Value)> = m
                        .iter()
                        .filter(|(k, _)| !head.contains(&k.as_str()))
                        .collect();
                    rest.sort_by(|a, b| a.0.cmp(b.0));
                    fields.extend(rest);
                    compact_object(fields, &mut text);
                }
                other => compact(other, &mut text),
            }
            (key, text)
        })
        .collect();
    keyed.par_sort_unstable();
    for (i, (_, text)) in keyed.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(text);
    }
}

fn compact_array(items: &[Value], out: &mut String) {
    out.push('[');
    for (i, v) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        compact(v, out);
    }
    out.push(']');
}

fn compact_object(fields: Vec<(&String, &Value)>, out: &mut String) {
    out.push('{');
    for (i, (k, v)) in fields.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_str(k, out);
        out.push(':');
        compact(v, out);
    }
    out.push('}');
}

/// `json.dumps(v, separators=(",", ":"), ensure_ascii=True)`, keys in their stored order.
fn compact(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => compact_object(m.iter().collect(), out),
        Value::Array(a) => compact_array(a, out),
        Value::String(s) => write_str(s, out),
        Value::Number(n) => out.push_str(&number(n)),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Null => out.push_str("null"),
    }
}

/// `json.dumps(v, sort_keys=True, ensure_ascii=False)` with default separators: the sort key.
fn sort_dump(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_str_utf8(k, out);
                out.push_str(": ");
                sort_dump(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                sort_dump(x, out);
            }
            out.push(']');
        }
        Value::String(s) => write_str_utf8(s, out),
        other => compact(other, out),
    }
}

/// A string literal as Python writes it with `ensure_ascii=False`.
fn write_str_utf8(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Integers as written; floats as Python's `repr`.
fn number(n: &Number) -> String {
    if n.is_f64() {
        float_repr(n.as_f64().unwrap_or(0.0))
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn g(nodes: Value, links: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "directed": false, "multigraph": false,
            "graph": {"schema_version": 1},
            "nodes": nodes, "links": links, "hyperedges": []
        }))
        .unwrap()
    }

    fn parsed(m: &Merged) -> Value {
        serde_json::from_str(&m.json).unwrap()
    }

    #[test]
    fn three_way_keeps_one_sided_changes_and_deletions() {
        let base = g(
            json!([{"id": "a", "label": "A"}, {"id": "b", "label": "B"}, {"id": "c", "label": "C"}]),
            json!([{"source": "a", "target": "b", "relation": "calls"},
                   {"source": "b", "target": "c", "relation": "calls"}]),
        );
        // Ours relabels a and deletes c; theirs only re-clusters c.
        let ours = g(
            json!([{"id": "a", "label": "A2"}, {"id": "b", "label": "B"}]),
            json!([{"source": "a", "target": "b", "relation": "calls"}]),
        );
        let theirs = g(
            json!([{"id": "a", "label": "A"}, {"id": "b", "label": "B"},
                   {"id": "c", "label": "C", "community": 4}]),
            json!([{"source": "a", "target": "b", "relation": "calls"},
                   {"source": "c", "target": "b", "relation": "calls"}]),
        );
        let m = merge(&base, &ours, &theirs).unwrap();
        let v = parsed(&m);
        let ids: Vec<&str> = v["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(v["nodes"][0]["label"], "A2");
        // theirs flipped b-c, which ours deleted with c: gone, and counted.
        assert_eq!(v["links"].as_array().unwrap().len(), 1);
        assert_eq!(m.dropped_links, 1);
    }

    #[test]
    fn conflicting_values_take_the_other_side_and_links_flip_whole() {
        let base = g(
            json!([{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]),
            json!([{"source": "a", "target": "b", "relation": "calls", "weight": 1.0}]),
        );
        let ours = g(
            json!([{"id": "a", "label": "ours"}, {"id": "b", "label": "B"}]),
            json!([{"source": "a", "target": "b", "relation": "calls", "weight": 2.0}]),
        );
        let theirs = g(
            json!([{"id": "a", "label": "theirs", "x": 1}, {"id": "b", "label": "B"}]),
            json!([{"source": "b", "target": "a", "relation": "uses", "weight": 1.0}]),
        );
        let v = parsed(&merge(&base, &ours, &theirs).unwrap());
        assert_eq!(v["nodes"][0], json!({"id": "a", "label": "theirs", "x": 1}));
        assert_eq!(
            v["links"][0],
            json!({"source": "b", "target": "a", "relation": "uses", "weight": 2.0})
        );
    }

    #[test]
    fn empty_base_is_a_union_and_bad_input_fails() {
        let a = g(json!([{"id": "a"}]), json!([]));
        let b = g(json!([{"id": "b"}]), json!([]));
        let v = parsed(&merge(b"", &a, &b).unwrap());
        assert_eq!(v["nodes"].as_array().unwrap().len(), 2);
        assert!(merge(b"", &a, b"{not json").is_err());
        assert!(merge(b"", b"[]", &a).is_err());
        let mut d: Value = serde_json::from_slice(&a).unwrap();
        d["directed"] = json!(true);
        assert!(merge(b"", &a, &serde_json::to_vec(&d).unwrap()).is_err());
    }

    #[test]
    fn output_is_python_formatted() {
        let a = g(
            json!([{"id": "é", "label": "Café", "score": 1e-7, "n": 3, "z": {"b": 1, "a": 2}}]),
            json!([]),
        );
        let m = merge(b"", &a, &a).unwrap();
        assert_eq!(
            m.json,
            "{\"directed\":false,\"multigraph\":false,\"graph\":{\"schema_version\":1},\"nodes\":\
             [{\"id\":\"\\u00e9\",\"label\":\"Caf\\u00e9\",\"n\":3,\"score\":1e-07,\"z\":{\"b\":1,\
             \"a\":2}}],\"links\":[],\"hyperedges\":[]}"
        );
    }
}

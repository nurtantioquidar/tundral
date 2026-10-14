//! GraphML (`docs/graphify-exports.md` §4): what NetworkX's `write_graphml` produces for the
//! graph Graphify loads from `graph.json`, written straight from the document so every field and
//! its key order survive.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use dijkstral_core::pyjson::{float_repr, write_str};
use serde_json::{Number, Value};

pub const GRAPHML_FILE: &str = "graph.graphml";

const HEADER: &str = concat!(
    "<?xml version='1.0' encoding='utf-8'?>\n",
    "<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\" ",
    "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
    "xsi:schemaLocation=\"http://graphml.graphdrawing.org/xmlns ",
    "http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd\">\n"
);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Scope {
    Graph,
    Node,
    Edge,
}

impl Scope {
    fn name(self) -> &'static str {
        match self {
            Scope::Graph => "graph",
            Scope::Node => "node",
            Scope::Edge => "edge",
        }
    }
}

/// A value as NetworkX writes it: its GraphML type and text (`None`: an empty element).
fn typed(v: &Value) -> (&'static str, Option<String>) {
    match v {
        Value::String(s) => ("string", Some(s.clone())),
        Value::Bool(b) => ("boolean", Some(if *b { "True" } else { "False" }.into())),
        Value::Number(n) => number(n),
        Value::Null => ("string", None),
        other => ("string", Some(py_dumps(other))),
    }
}

fn number(n: &Number) -> (&'static str, Option<String>) {
    if let Some(i) = n.as_i64() {
        ("long", Some(i.to_string()))
    } else if let Some(u) = n.as_u64() {
        ("long", Some(u.to_string()))
    } else {
        ("double", Some(float_repr(n.as_f64().unwrap_or(0.0))))
    }
}

/// Python's `json.dumps(v, sort_keys=True)`: Graphify stringifies lists and dicts this way.
fn py_dumps(v: &Value) -> String {
    let mut out = String::new();
    dump(v, &mut out);
    out
}

fn dump(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(n).1.unwrap_or_default()),
        Value::String(s) => write_str(s, out),
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
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_str(k, out);
                out.push_str(": ");
                dump(&m[*k], out);
            }
            out.push('}');
        }
    }
}

/// Keys in creation order; written newest first, as NetworkX's writer inserts each at the top.
#[derive(Default)]
struct Keys {
    list: Vec<(Scope, String, &'static str)>,
    ids: HashMap<(Scope, String, &'static str), usize>,
}

impl Keys {
    fn id(&mut self, scope: Scope, name: &str, ty: &'static str) -> usize {
        let key = (scope, name.to_string(), ty);
        if let Some(&id) = self.ids.get(&key) {
            return id;
        }
        let id = self.list.len();
        self.list.push(key.clone());
        self.ids.insert(key, id);
        id
    }
}

/// Characters XML 1.0 cannot carry are dropped.
fn xml_ok(c: char) -> bool {
    !matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}')
}

fn text(s: &str, out: &mut String) {
    for c in s.chars().filter(|&c| xml_ok(c)) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

fn attr(s: &str, out: &mut String) {
    for c in s.chars().filter(|&c| xml_ok(c)) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#09;"),
            c => out.push(c),
        }
    }
}

fn id_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// One element's `<data>` children, registering keys as they are met.
fn data(keys: &mut Keys, scope: Scope, fields: &[(&str, &Value)], indent: &str, out: &mut String) {
    for (name, v) in fields {
        let (ty, t) = typed(v);
        let id = keys.id(scope, name, ty);
        let mut escaped = String::new();
        text(t.as_deref().unwrap_or(""), &mut escaped);
        if escaped.is_empty() {
            let _ = writeln!(out, "{indent}<data key=\"d{id}\" />");
        } else {
            let _ = writeln!(out, "{indent}<data key=\"d{id}\">{escaped}</data>");
        }
    }
}

/// A `<node>` or `<edge>` with its data; childless elements are written short.
fn element(keys: &mut Keys, scope: Scope, head: &str, fields: &[(&str, &Value)], out: &mut String) {
    if fields.is_empty() {
        let _ = writeln!(out, "    <{head} />");
    } else {
        let _ = writeln!(out, "    <{head}>");
        data(keys, scope, fields, "      ", out);
        let _ = writeln!(out, "    </{}>", scope.name());
    }
}

/// An object's fields in document order, without `skip` and Graphify's private `_` fields.
fn fields<'a>(v: &'a Value, skip: &[&str]) -> Vec<(&'a str, &'a Value)> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| !k.starts_with('_') && !skip.contains(&k.as_str()))
        .map(|(k, v)| (k.as_str(), v))
        .collect()
}

/// The GraphML document for a Graphify-format `graph.json`.
pub fn render(doc: &Value) -> Result<String> {
    let Some(nodes) = doc.get("nodes").and_then(Value::as_array) else {
        bail!("graph.json has no nodes array");
    };
    let links = doc
        .get("links")
        .or_else(|| doc.get("edges"))
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let directed = doc.get("directed").and_then(Value::as_bool) == Some(true);

    let mut keys = Keys::default();
    let mut body = String::new();

    // Graph attributes: `graph` then `hyperedges`; registered first, written last.
    let mut graph_fields = fields(doc.get("graph").unwrap_or(&Value::Null), &[]);
    if let Some(h) = doc.get("hyperedges") {
        graph_fields.push(("hyperedges", h));
    }
    let mut graph_data = String::new();
    data(
        &mut keys,
        Scope::Graph,
        &graph_fields,
        "    ",
        &mut graph_data,
    );

    let clustered = nodes.iter().any(|n| n.get("community").is_some());
    let minus_one = Value::from(-1);
    let mut index: HashMap<String, u32> = HashMap::with_capacity(nodes.len());
    let mut ids: Vec<String> = Vec::with_capacity(nodes.len());
    for n in nodes {
        let id = id_text(n.get("id").unwrap_or(&Value::Null));
        if index.contains_key(&id) {
            continue;
        }
        index.insert(id.clone(), ids.len() as u32);
        let mut own = fields(n, &["id"]);
        if clustered && n.get("community").is_none() {
            own.push(("community", &minus_one));
        }
        let mut head = String::from("node id=\"");
        attr(&id, &mut head);
        head.push('"');
        element(&mut keys, Scope::Node, &head, &own, &mut body);
        ids.push(id);
    }

    // NetworkX adjacency from the links in file order; one edge per (unordered) pair.
    let mut adj: Vec<Vec<(u32, usize)>> = vec![Vec::new(); ids.len()];
    let mut pair: HashMap<(u32, u32), usize> = HashMap::new();
    for (k, l) in links.iter().enumerate() {
        let (Some(s), Some(t)) = (
            l.get("source")
                .map(id_text)
                .and_then(|s| index.get(&s).copied()),
            l.get("target")
                .map(id_text)
                .and_then(|t| index.get(&t).copied()),
        ) else {
            continue;
        };
        let key = if directed || s <= t { (s, t) } else { (t, s) };
        if pair.contains_key(&key) {
            continue;
        }
        pair.insert(key, k);
        adj[s as usize].push((t, k));
        if !directed && s != t {
            adj[t as usize].push((s, k));
        }
    }
    let mut seen = vec![false; ids.len()];
    for u in 0..ids.len() {
        for &(v, k) in &adj[u] {
            if !directed && seen[v as usize] {
                continue;
            }
            let own = fields(&links[k], &["source", "target"]);
            let mut head = String::from("edge source=\"");
            attr(&ids[u], &mut head);
            head.push_str("\" target=\"");
            attr(&ids[v as usize], &mut head);
            head.push('"');
            element(&mut keys, Scope::Edge, &head, &own, &mut body);
        }
        seen[u] = true;
    }

    let mut out = String::with_capacity(body.len() + 4096);
    out.push_str(HEADER);
    for (id, (scope, name, ty)) in keys.list.iter().enumerate().rev() {
        let mut n = String::new();
        attr(name, &mut n);
        let _ = writeln!(
            out,
            "  <key id=\"d{id}\" for=\"{}\" attr.name=\"{n}\" attr.type=\"{ty}\" />",
            scope.name()
        );
    }
    let _ = writeln!(
        out,
        "  <graph edgedefault=\"{}\">",
        if directed { "directed" } else { "undirected" }
    );
    out.push_str(&body);
    out.push_str(&graph_data);
    out.push_str("  </graph>\n</graphml>\n");
    Ok(out)
}

pub fn write(doc: &Value, path: &Path) -> Result<()> {
    let xml = render(doc)?;
    fs::write(path, xml).with_context(|| format!("writing {}", path.display()))
}

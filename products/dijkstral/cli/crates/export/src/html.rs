//! `graph.html`: a self-contained interactive page (`docs/graphify-exports.md` §2).
//!
//! The embedded data has the shape of Graphify's page (`RAW_NODES`, `RAW_EDGES`, `LEGEND`,
//! `hyperedges`, the same per-item keys and values, plus each node's `source_location`). Up to
//! the node limit the page draws it with vis-network; above it, where Graphify aggregates
//! communities or refuses, the same records are embedded column-wise and drawn with sigma.js
//! (WebGL). The page itself is dijkstral's own.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use dijkstral_core::facts::{Confidence, Loc};

use crate::view::View;

pub const HTML_FILE: &str = "graph.html";
/// Graphify's default `--node-limit`: larger graphs get the WebGL view.
pub const DEFAULT_NODE_LIMIT: usize = 5000;

/// Tableau 10, by community id modulo 10 (§2.2).
pub const PALETTE: [&str; 10] = [
    "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
    "#9C755F", "#BAB0AC",
];

const TEMPLATE: &str = include_str!("page.html");

// Exact versions, with subresource integrity, from jsDelivr.
const VIS_SCRIPTS: &str = concat!(
    r#"<script src="https://cdn.jsdelivr.net/npm/vis-network@9.1.9/standalone/umd/vis-network.min.js" "#,
    r#"integrity="sha384-yxKDWWf0wwdUj/gPeuL11czrnKFQROnLgY8ll7En9NYoXibgg3C6NK/UDHNtUgWJ" crossorigin="anonymous"></script>"#
);
const WEBGL_SCRIPTS: &str = concat!(
    r#"<script src="https://cdn.jsdelivr.net/npm/graphology@0.25.4/dist/graphology.umd.min.js" "#,
    r#"integrity="sha384-643a1dipBp0OHFnm3WeZmMnuJ+ZY/MEll1vTGCjeFzkB/RySh+Almy3Ffn+sxO7y" crossorigin="anonymous"></script>"#,
    "\n",
    r#"<script src="https://cdn.jsdelivr.net/npm/sigma@3.0.2/dist/sigma.min.js" "#,
    r#"integrity="sha384-DPqVVIwLfHZ98UMPbPxfthtHyUuc0gd/jdS/udObmiaCBMi97XcvT6JU2zqv0NCj" crossorigin="anonymous"></script>"#
);

#[derive(Debug, Clone)]
pub struct HtmlOptions {
    /// Up to this many nodes the page uses vis-network; above it, WebGL.
    pub node_limit: usize,
    /// Shown in the page header (the project directory name).
    pub title: String,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        HtmlOptions {
            node_limit: DEFAULT_NODE_LIMIT,
            title: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// vis-network, Graphify's per-item records.
    Vis,
    /// sigma.js over WebGL, the same records column-wise.
    WebGl,
}

pub fn mode(view: &View, opts: &HtmlOptions) -> Mode {
    if view.nodes.len() > opts.node_limit {
        Mode::WebGl
    } else {
        Mode::Vis
    }
}

/// The whole page.
pub fn render(view: &View, opts: &HtmlOptions) -> String {
    let mode = mode(view, opts);
    let data = payload(view, opts, mode);
    let mut title = String::new();
    escape_html(
        &if opts.title.is_empty() {
            "dijkstral graph".to_string()
        } else {
            format!("{} · dijkstral graph", opts.title)
        },
        &mut title,
    );
    let scripts = match mode {
        Mode::Vis => VIS_SCRIPTS,
        Mode::WebGl => WEBGL_SCRIPTS,
    };
    fill(
        TEMPLATE,
        &[
            ("@@TITLE@@", &title),
            ("@@DATA@@", &data),
            ("@@SCRIPTS@@", scripts),
        ],
    )
}

pub fn write(view: &View, opts: &HtmlOptions, path: &Path) -> Result<()> {
    let page = render(view, opts);
    let tmp = path.with_extension("html.tmp");
    fs::write(&tmp, page).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Replace each marker once, in template order, without rescanning inserted text (a label may
/// contain a marker).
pub(crate) fn fill(template: &str, parts: &[(&str, &str)]) -> String {
    let mut out =
        String::with_capacity(template.len() + parts.iter().map(|p| p.1.len()).sum::<usize>());
    let mut rest = template;
    loop {
        let next = parts
            .iter()
            .filter_map(|(m, v)| rest.find(m).map(|at| (at, *m, *v)))
            .min_by_key(|(at, _, _)| *at);
        let Some((at, marker, value)) = next else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        out.push_str(value);
        rest = &rest[at + marker.len()..];
    }
}

/// The embedded JSON, safe inside `<script type="application/json">`: `<`, `>`, `&`, U+2028 and
/// U+2029 are written as `\u` escapes wherever they occur.
pub fn payload(view: &View, opts: &HtmlOptions, mode: Mode) -> String {
    let mut out = String::with_capacity(256 * view.nodes.len() + 160 * view.edges.len() + 1024);
    out.push_str("{\"mode\":");
    out.push_str(match mode {
        Mode::Vis => "\"vis\"",
        Mode::WebGl => "\"webgl\"",
    });
    out.push_str(",\"meta\":{\"title\":");
    json_str(&opts.title, &mut out);
    let _ = write!(
        out,
        ",\"nodes\":{},\"links\":{},\"node_limit\":{}}}",
        view.nodes.len(),
        view.edges.len(),
        opts.node_limit
    );
    match mode {
        Mode::Vis => vis_records(view, &mut out),
        Mode::WebGl => columns(view, &mut out),
    }
    out.push_str(",\"LEGEND\":");
    legend(view, &mut out);
    out.push_str(",\"hyperedges\":");
    out.push_str(&html_safe(&view.hyperedges));
    out.push('}');
    out
}

fn max_degree(view: &View) -> u32 {
    view.nodes
        .iter()
        .map(|&i| view.degree[i as usize])
        .max()
        .unwrap_or(0)
        .max(1)
}

fn community(view: &View, i: u32) -> u32 {
    view.community[i as usize].unwrap_or(0)
}

/// The label the page shows: the id when a node has none, without control characters (line
/// breaks and tabs included), as Graphify writes it.
fn label_of<'a>(view: &View<'a>, i: u32) -> Cow<'a, str> {
    let n = &view.graph.nodes[i as usize];
    let label = if n.label.is_empty() { &n.id } else { &n.label };
    if label.chars().any(|c| c < ' ') {
        Cow::Owned(label.chars().filter(|&c| c >= ' ').collect())
    } else {
        Cow::Borrowed(label)
    }
}

fn loc_str(loc: Loc) -> Option<String> {
    match loc {
        Loc::Line(n) => Some(format!("L{n}")),
        Loc::Empty => Some(String::new()),
        Loc::Null | Loc::Absent => None,
    }
}

/// `round(10 + 30·degree/max, 1)`, as Python prints it.
pub fn node_size(degree: u32, max: u32) -> String {
    format!("{:.1}", 10.0 + 30.0 * f64::from(degree) / f64::from(max))
}

/// Labels are drawn for nodes with at least 15% of the largest degree.
pub fn shows_label(degree: u32, max: u32) -> bool {
    f64::from(degree) / f64::from(max) >= 0.15
}

fn vis_records(view: &View, out: &mut String) {
    let max = max_degree(view);
    out.push_str(",\"RAW_NODES\":[");
    for (k, &i) in view.nodes.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let n = &view.graph.nodes[i as usize];
        let c = community(view, i);
        let colour = PALETTE[c as usize % PALETTE.len()];
        let label = label_of(view, i);
        let deg = view.degree[i as usize];
        out.push_str("{\"id\":");
        json_str(&n.id, out);
        out.push_str(",\"label\":");
        json_str(&label, out);
        let _ = write!(
            out,
            ",\"color\":{{\"background\":\"{colour}\",\"border\":\"{colour}\",\"highlight\":{{\"background\":\"#ffffff\",\"border\":\"{colour}\"}}}},\"size\":{},\"font\":{{\"size\":{},\"color\":\"#ffffff\"}},\"title\":",
            node_size(deg, max),
            if shows_label(deg, max) { 12 } else { 0 }
        );
        json_str(&label, out);
        let _ = write!(out, ",\"community\":{c},\"community_name\":");
        json_str(&view.names.get(c), out);
        out.push_str(",\"source_file\":");
        json_str(&n.source_file, out);
        out.push_str(",\"file_type\":");
        json_str(&n.file_type, out);
        let _ = write!(out, ",\"degree\":{deg}");
        if let Some(loc) = loc_str(n.loc) {
            out.push_str(",\"source_location\":");
            json_str(&loc, out);
        }
        out.push('}');
    }
    out.push_str("],\"RAW_EDGES\":[");
    for (k, (_, _, e)) in view.edge_walk().into_iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let edge = &view.graph.edges[e as usize];
        let conf = edge.confidence.as_str();
        let extracted = edge.confidence == Confidence::Extracted;
        out.push_str("{\"from\":");
        json_str(&edge.source, out);
        out.push_str(",\"to\":");
        json_str(&edge.target, out);
        out.push_str(",\"label\":");
        json_str(&edge.relation, out);
        out.push_str(",\"title\":");
        json_str(&format!("{} [{conf}]", edge.relation), out);
        let _ = write!(
            out,
            ",\"dashes\":{},\"width\":{},\"color\":{{\"opacity\":{}}},\"confidence\":\"{conf}\"}}",
            !extracted,
            if extracted { 2 } else { 1 },
            if extracted { "0.7" } else { "0.35" }
        );
    }
    out.push(']');
}

/// Above the node limit: the same records as arrays (fields named once), relations interned and
/// link endpoints as node positions. The page derives colour, size, label visibility and
/// community name exactly as in the per-item form.
fn columns(view: &View, out: &mut String) {
    let mut pos = vec![0u32; view.graph.nodes.len()];
    for (k, &i) in view.nodes.iter().enumerate() {
        pos[i as usize] = k as u32;
    }
    out.push_str(",\"nodeFields\":[\"id\",\"label\",\"community\",\"source_file\",\"file_type\",\"degree\",\"source_location\"],\"nodes\":[");
    for (k, &i) in view.nodes.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let n = &view.graph.nodes[i as usize];
        out.push('[');
        json_str(&n.id, out);
        out.push(',');
        json_str(&label_of(view, i), out);
        let _ = write!(out, ",{},", community(view, i));
        json_str(&n.source_file, out);
        out.push(',');
        json_str(&n.file_type, out);
        let _ = write!(out, ",{},", view.degree[i as usize]);
        match loc_str(n.loc) {
            Some(l) => json_str(&l, out),
            None => out.push_str("null"),
        }
        out.push(']');
    }
    let walk = view.edge_walk();
    let mut relations: Vec<&str> = Vec::new();
    let mut rel_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut rows = String::with_capacity(walk.len() * 24);
    for (k, &(_, _, e)) in walk.iter().enumerate() {
        let edge = &view.graph.edges[e as usize];
        let r = *rel_index.entry(edge.relation.as_str()).or_insert_with(|| {
            relations.push(edge.relation.as_str());
            relations.len() as u32 - 1
        });
        let (s, t) = view.ends[e as usize];
        let c = match edge.confidence {
            Confidence::Extracted => 0,
            Confidence::Inferred => 1,
            Confidence::Ambiguous => 2,
        };
        if k > 0 {
            rows.push(',');
        }
        let _ = write!(rows, "[{},{},{r},{c}]", pos[s as usize], pos[t as usize]);
    }
    out.push_str("],\"relations\":[");
    for (k, r) in relations.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        json_str(r, out);
    }
    out.push_str("],\"confidences\":[\"EXTRACTED\",\"INFERRED\",\"AMBIGUOUS\"],\"edgeFields\":[\"from\",\"to\",\"label\",\"confidence\"],\"edges\":[");
    out.push_str(&rows);
    out.push(']');
}

/// One entry per community, by id: Graphify's `LEGEND` shape. Graphify leaves it empty unless a
/// labels file names the communities (quirk X2); the page always lists them.
fn legend(view: &View, out: &mut String) {
    let mut entries: Vec<(u32, usize)> = view
        .communities()
        .into_iter()
        .map(|(c, m)| (c, m.len()))
        .collect();
    entries.sort_unstable();
    if entries.is_empty() && !view.nodes.is_empty() {
        entries.push((0, view.nodes.len()));
    }
    out.push('[');
    for (k, (c, count)) in entries.into_iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"cid\":{c},\"color\":\"{}\",\"label\":",
            PALETTE[c as usize % PALETTE.len()]
        );
        json_str(&view.names.get(c), out);
        let _ = write!(out, ",\"count\":{count}}}");
    }
    out.push(']');
}

/// A JSON string literal that is also inert inside a `<script>` element.
pub fn json_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Already-serialized JSON made inert inside `<script>`: the replaced characters can only occur
/// inside strings, where the `\u` escapes mean the same.
pub(crate) fn html_safe(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn escape_html(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_round_like_python() {
        // round(10 + 30·d/m, 1): exact binary ties go to even, as in Python.
        assert_eq!(node_size(1, 8), "13.8");
        assert_eq!(node_size(3, 8), "21.2");
        assert_eq!(node_size(5, 8), "28.8");
        assert_eq!(node_size(7, 8), "36.2");
        assert_eq!(node_size(1, 18), "11.7");
        assert_eq!(node_size(18, 18), "40.0");
        assert_eq!(node_size(0, 4), "10.0");
        assert!(shows_label(3, 20) && !shows_label(2, 18) && shows_label(140, 933));
    }

    #[test]
    fn fill_does_not_rescan_values() {
        assert_eq!(
            fill("a@@X@@b@@Y@@c", &[("@@X@@", "@@Y@@"), ("@@Y@@", "y")]),
            "a@@Y@@byc"
        );
    }

    #[test]
    fn strings_are_inert_in_script() {
        let mut s = String::new();
        json_str("</script><!--\u{2028}&\"\\\u{1}", &mut s);
        assert_eq!(
            s,
            "\"\\u003c/script\\u003e\\u003c!--\\u2028\\u0026\\\"\\\\\\u0001\""
        );
        assert_eq!(html_safe("[\"</x>\"]"), "[\"\\u003c/x\\u003e\"]");
    }
}

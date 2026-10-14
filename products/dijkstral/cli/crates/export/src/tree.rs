//! `GRAPH_TREE.html`: a collapsible D3 tree of the graph's source files and their symbols
//! (`docs/graphify-exports.md`, tree section).
//!
//! The hierarchy is Graphify's `graphify tree` data exactly (`{"name", "total_count",
//! "children"}`, serialized as Python's compact, ASCII-escaped `json.dumps` with `<` escaped):
//! directories and files from each node's `source_file` relative to the root, the file's symbols
//! below it, sorted, with long symbol lists cut at `max_children`. dijkstral adds a parallel
//! inspector record per leaf (location, kind, degree and the top outbound links), which Graphify's
//! `--top-k-edges` describes but its page does not carry. The page is dijkstral's own.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::html::{escape_html, fill, html_safe, json_str};
use crate::nx::NxGraph;

pub const TREE_FILE: &str = "GRAPH_TREE.html";
/// Graphify's default `--max-children`.
pub const DEFAULT_MAX_CHILDREN: usize = 200;
/// Graphify's default `--top-k-edges`.
pub const DEFAULT_TOP_K_EDGES: usize = 12;
/// The root's name when no node has a source file.
pub const EMPTY_NAME: &str = "(empty graph)";

const TEMPLATE: &str = include_str!("tree.html");

// Exact version, with subresource integrity, from jsDelivr.
const D3_SCRIPT: &str = concat!(
    r#"<script src="https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js" "#,
    r#"integrity="sha384-CjloA8y00+1SDAUkjs099PVfnY2KmDC2BZnws9kh8D/lX1s46w6EPhpXdqMfjK6i" crossorigin="anonymous"></script>"#
);

#[derive(Debug, Clone)]
pub struct TreeOptions {
    /// The hierarchy's root (`--root`); default: the common path of every source file.
    pub root: Option<String>,
    /// Symbols shown per file before a `(+N more)` entry.
    pub max_children: usize,
    /// Outbound links listed per symbol in the inspector.
    pub top_k_edges: usize,
    /// The root's name (`--label`); default: the root directory's name, else `/`.
    pub label: Option<String>,
}

impl Default for TreeOptions {
    fn default() -> Self {
        TreeOptions {
            root: None,
            max_children: DEFAULT_MAX_CHILDREN,
            top_k_edges: DEFAULT_TOP_K_EDGES,
            label: None,
        }
    }
}

/// One entry of the hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub total_count: usize,
    pub children: Vec<Entry>,
    /// The graph node a symbol leaf stands for (`None` for directories, files and `(+N more)`).
    pub node: Option<u32>,
    pub kind: Kind,
}

/// What an entry is; the page shows each kind differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Symbol,
    More,
}

impl Kind {
    fn code(self) -> char {
        match self {
            Kind::Dir => 'd',
            Kind::File => 'f',
            Kind::Symbol => 's',
            Kind::More => 'm',
        }
    }
}

impl Entry {
    fn leaf(name: String, total_count: usize, node: Option<u32>, kind: Kind) -> Entry {
        Entry {
            name,
            total_count,
            children: Vec::new(),
            node,
            kind,
        }
    }
}

/// What `build` made, for the command's summary.
#[derive(Debug, Clone, Copy, Default)]
pub struct TreeStats {
    pub files: usize,
    pub symbols: usize,
}

/// A path as `pathlib.PurePosixPath` sees it: its anchor (`""`, `/` or `//`) and its components
/// without empty and `.` ones.
fn pure_path(s: &str) -> (&str, Vec<&str>) {
    let anchor = if s.starts_with("//") && !s.starts_with("///") {
        "//"
    } else if s.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts = s[anchor.len()..]
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    (anchor, parts)
}

/// `PurePosixPath(s).name`.
fn path_name(s: &str) -> &str {
    pure_path(s).1.last().copied().unwrap_or("")
}

/// `os.path.commonpath(paths)`, `None` when absolute and relative paths are mixed.
fn common_path(paths: &[&str]) -> Option<String> {
    let absolute = paths[0].starts_with('/');
    if paths.iter().any(|p| p.starts_with('/') != absolute) {
        return None;
    }
    let split: Vec<Vec<&str>> = paths
        .iter()
        .map(|p| {
            p.split('/')
                .filter(|c| !c.is_empty() && *c != ".")
                .collect()
        })
        .collect();
    let mut common: Vec<&str> = split[0].clone();
    for parts in &split[1..] {
        let n = common.iter().zip(parts).take_while(|(a, b)| a == b).count();
        common.truncate(n);
    }
    let prefix = if absolute { "/" } else { "" };
    Some(format!("{prefix}{}", common.join("/")))
}

/// The root's display name: `PurePosixPath(root).name`, else `root` as given (`.`, `./`, `/`);
/// `/` when no root is set.
fn root_name(root: &str) -> String {
    if root.is_empty() {
        return "/".into();
    }
    match pure_path(root).1.last() {
        Some(n) => n.to_string(),
        None => root.into(),
    }
}

/// `PurePosixPath(path).relative_to(root).parts`, `None` when `root` is not a prefix.
fn relative_parts<'a>(path: &'a str, root: &(&str, Vec<&str>)) -> Option<Vec<&'a str>> {
    let (anchor, parts) = pure_path(path);
    if anchor != root.0 || parts.len() < root.1.len() || parts[..root.1.len()] != root.1[..] {
        return None;
    }
    Some(parts[root.1.len()..].to_vec())
}

/// Python's `str.lower()` ordering key.
fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Graphify's child order: entries with children first, then by lowercased name; stable.
fn sort_children(children: &mut [Entry]) {
    children.sort_by_cached_key(|e| (e.children.is_empty(), lower(&e.name)));
}

/// A directory while the hierarchy is assembled: children in the order they were met.
#[derive(Default)]
struct Dir {
    children: Vec<Slot>,
    dirs: HashMap<String, usize>,
}

enum Slot {
    Dir(String, Dir),
    File(String, Vec<u32>),
}

fn finish_dir(name: String, dir: Dir, g: &NxGraph, max_children: usize) -> Entry {
    let mut children: Vec<Entry> = dir
        .children
        .into_iter()
        .map(|slot| match slot {
            Slot::Dir(name, d) => finish_dir(name, d, g, max_children),
            Slot::File(name, symbols) => finish_file(name, &symbols, g, max_children),
        })
        .collect();
    sort_children(&mut children);
    let total_count = if children.is_empty() {
        1
    } else {
        children.iter().map(|c| c.total_count).sum()
    };
    Entry {
        name,
        total_count,
        children,
        node: None,
        kind: Kind::Dir,
    }
}

fn finish_file(name: String, symbols: &[u32], g: &NxGraph, max_children: usize) -> Entry {
    let mut children: Vec<Entry> = symbols
        .iter()
        .map(|&i| Entry::leaf(g.label(i).to_string(), 1, Some(i), Kind::Symbol))
        .collect();
    if children.len() > max_children {
        // Which symbols stay: names without a leading `_` first, then by lowercased name.
        children.sort_by_cached_key(|e| (e.name.starts_with('_'), lower(&e.name)));
        let hidden = children.len() - max_children;
        children.truncate(max_children);
        children.push(Entry::leaf(
            format!("(+{hidden} more)"),
            hidden,
            None,
            Kind::More,
        ));
    }
    sort_children(&mut children);
    let total_count = if children.is_empty() {
        1
    } else {
        children.iter().map(|c| c.total_count).sum()
    };
    Entry {
        name,
        total_count,
        children,
        node: None,
        kind: Kind::File,
    }
}

/// The hierarchy Graphify's `tree` embeds.
pub fn build(g: &NxGraph, opts: &TreeOptions) -> Result<(Entry, TreeStats)> {
    // Every node with a source file belongs to that file; a code node labelled with the file's
    // path or a trailing part of it (`x.py`, `hub/x.py` for `apps/hub/x.py`) is the file itself
    // and is not listed.
    let mut files: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    let mut symbols = 0;
    for i in 0..g.nodes.len() as u32 {
        let Some(sf) = g.node_str(i, "source_file").filter(|s| !s.is_empty()) else {
            continue;
        };
        let list = files.entry(sf).or_default();
        let label = g.label(i);
        let is_file = g.node_str(i, "file_type") == Some("code")
            && !label.is_empty()
            && (label == sf
                || sf
                    .strip_suffix(label)
                    .is_some_and(|head| head.ends_with('/')));
        if !is_file {
            list.push(i);
            symbols += 1;
        }
    }
    let stats = TreeStats {
        files: files.len(),
        symbols,
    };
    if files.is_empty() {
        return Ok((Entry::leaf(EMPTY_NAME.into(), 0, None, Kind::Dir), stats));
    }
    let paths: Vec<&str> = files.keys().copied().collect();
    let root_text = match &opts.root {
        Some(r) => r.clone(),
        None => common_path(&paths).unwrap_or_default(),
    };
    let root = pure_path(&root_text);
    let mut matched = 0;
    let mut top = Dir::default();
    for (&sf, syms) in &files {
        let parts = match relative_parts(sf, &root) {
            Some(rel) => {
                matched += 1;
                if rel.is_empty() {
                    pure_path(sf).1
                } else {
                    rel
                }
            }
            None => vec![path_name(sf)],
        };
        let (file, dirs) = parts.split_last().map_or(("", &[][..]), |(f, d)| (*f, d));
        let mut dir = &mut top;
        for &d in dirs {
            let k = match dir.dirs.get(d) {
                Some(&k) => k,
                None => {
                    dir.children.push(Slot::Dir(d.to_string(), Dir::default()));
                    dir.dirs.insert(d.to_string(), dir.children.len() - 1);
                    dir.children.len() - 1
                }
            };
            dir = match &mut dir.children[k] {
                Slot::Dir(_, sub) => sub,
                Slot::File(..) => unreachable!("dirs index only directories"),
            };
        }
        dir.children
            .push(Slot::File(file.to_string(), syms.clone()));
    }
    if opts.root.is_some() && matched == 0 {
        bail!(
            "--root {} matched 0 of {} source files (source_file paths are stored repo-relative, \
             e.g. '{}')",
            root_text,
            paths.len(),
            paths[0]
        );
    }
    let name = match &opts.label {
        Some(l) => l.clone(),
        None => root_name(&root_text),
    };
    Ok((finish_dir(name, top, g, opts.max_children), stats))
}

/// Python's `json.dumps(s)` (ASCII-escaped) with `<` written `\u003c`.
fn py_str(s: &str, out: &mut String) {
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
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7f => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The hierarchy as Graphify's page embeds it.
pub fn data_json(root: &Entry) -> String {
    fn walk(e: &Entry, out: &mut String) {
        out.push_str("{\"name\":");
        py_str(&e.name, out);
        let _ = write!(out, ",\"total_count\":{},\"children\":[", e.total_count);
        for (k, c) in e.children.iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            walk(c, out);
        }
        out.push_str("]}");
    }
    let mut out = String::new();
    walk(root, &mut out);
    out
}

/// The page's side data: `kinds`, one letter per entry in pre-order (`d`irectory, `f`ile,
/// `s`ymbol, `m`ore), and the inspector records, one per leaf in pre-order (`null` for files
/// without symbols and `(+N more)` entries): `[id, kind, source_file, location, community, degree, outbound total,
/// [[relation, target], …]]`, targets indexing `targets` (`[label, source_file, location]`).
fn inspector(g: &NxGraph, root: &Entry, top_k: usize, title: &str) -> String {
    let mut out = String::from("{\"title\":");
    json_str(title, &mut out);
    out.push_str(",\"kinds\":\"");
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        out.push(e.kind.code());
        stack.extend(e.children.iter().rev());
    }
    out.push_str("\",\"leaves\":[");
    let mut target_slot: HashMap<u32, usize> = HashMap::new();
    let mut targets: Vec<u32> = Vec::new();
    let mut first = true;
    let mut stack = vec![root];
    let str_or = |i: u32, k: &str| g.node_str(i, k).unwrap_or("").to_string();
    while let Some(e) = stack.pop() {
        if !e.children.is_empty() {
            stack.extend(e.children.iter().rev());
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        let Some(i) = e.node else {
            out.push_str("null");
            continue;
        };
        out.push('[');
        json_str(&g.nodes[i as usize].id, &mut out);
        out.push(',');
        json_str(&str_or(i, "file_type"), &mut out);
        out.push(',');
        json_str(&str_or(i, "source_file"), &mut out);
        out.push(',');
        json_str(&str_or(i, "source_location"), &mut out);
        let community = g.community(i).map_or("null".to_string(), |c| c.to_string());
        let mut outbound: Vec<(u32, u32)> = g.adj[i as usize]
            .iter()
            .filter(|&&(_, e)| g.edges[e as usize].source == i)
            .map(|&(v, e)| (v, e))
            .collect();
        let total = outbound.len();
        // Best-connected targets first; ties keep adjacency order (the sort is stable).
        outbound.sort_by_cached_key(|&(v, _)| std::cmp::Reverse(g.degree(v)));
        outbound.truncate(top_k);
        let _ = write!(out, ",{community},{},{total},[", g.degree(i));
        for (k, (v, edge)) in outbound.into_iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            let slot = *target_slot.entry(v).or_insert_with(|| {
                targets.push(v);
                targets.len() - 1
            });
            out.push('[');
            json_str(g.edge_str(edge, "relation").unwrap_or(""), &mut out);
            let _ = write!(out, ",{slot}]");
        }
        out.push_str("]]");
    }
    out.push_str("],\"targets\":[");
    for (k, &v) in targets.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        out.push('[');
        json_str(g.label(v), &mut out);
        out.push(',');
        json_str(&str_or(v, "source_file"), &mut out);
        out.push(',');
        json_str(&str_or(v, "source_location"), &mut out);
        out.push(']');
    }
    out.push_str("]}");
    out
}

/// The whole page.
pub fn render(g: &NxGraph, opts: &TreeOptions) -> Result<(String, TreeStats)> {
    let (root, stats) = build(g, opts)?;
    let title = root.name.clone();
    let data = data_json(&root);
    let extra = html_safe(&inspector(g, &root, opts.top_k_edges, &title));
    let mut title_html = String::new();
    escape_html(&title, &mut title_html);
    let page = fill(
        TEMPLATE,
        &[
            ("@@TITLE@@", &title_html),
            ("@@SCRIPTS@@", D3_SCRIPT),
            ("@@TREE@@", &data),
            ("@@INSPECTOR@@", &extra),
        ],
    );
    Ok((page, stats))
}

pub fn write(g: &NxGraph, opts: &TreeOptions, path: &Path) -> Result<TreeStats> {
    let (page, stats) = render(g, opts)?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(path, page).with_context(|| format!("writing {}", path.display()))?;
    Ok(stats)
}

/// The embedded hierarchy of a page written by `render` (or by Graphify, whose page holds it as
/// `const initialJsonData = …;`).
pub fn embedded_tree(page: &str) -> Option<&str> {
    let open = "<script id=\"tree-data\" type=\"application/json\">";
    if let Some(start) = page.find(open).map(|s| s + open.len()) {
        let end = start + page[start..].find("</script>")?;
        return Some(&page[start..end]);
    }
    let key = "const initialJsonData = ";
    let start = page.find(key)? + key.len();
    let end = start + page[start..].find('\n')?;
    page[start..end].trim_end().strip_suffix(';')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_pathlib() {
        assert_eq!(pure_path("./a//b/./c/"), ("", vec!["a", "b", "c"]));
        assert_eq!(pure_path("/x/../y"), ("/", vec!["x", "..", "y"]));
        assert_eq!(pure_path("//net/x"), ("//", vec!["net", "x"]));
        assert_eq!(pure_path("///x"), ("/", vec!["x"]));
        assert_eq!(path_name("trail/"), "trail");
        assert_eq!(path_name("back\\slash"), "back\\slash");
        assert_eq!(root_name(""), "/");
        assert_eq!(root_name("."), ".");
        assert_eq!(root_name("/"), "/");
        assert_eq!(root_name("a/b/"), "b");
        assert_eq!(root_name("a/b/.."), "..");
        assert_eq!(root_name("./"), "./");
    }

    #[test]
    fn common_path_matches_os_path() {
        assert_eq!(common_path(&["a/x.py", "a/b/y.py"]).unwrap(), "a");
        assert_eq!(common_path(&["a/x.py", "c/z.py"]).unwrap(), "");
        assert_eq!(common_path(&["a/b/x.py"]).unwrap(), "a/b/x.py");
        assert_eq!(common_path(&["/abs/p.py", "/other/r.py"]).unwrap(), "/");
        assert_eq!(common_path(&["./a/x.py", "a/../b"]).unwrap(), "a");
        assert!(common_path(&["/abs/p.py", "a/x.py"]).is_none());
    }

    #[test]
    fn strings_dump_like_python() {
        let mut s = String::new();
        py_str("</script>\u{7f}é漢\u{1F600}\u{2028}\"\\\n\u{1}", &mut s);
        assert_eq!(
            s,
            "\"\\u003c/script>\u{7f}\\u00e9\\u6f22\\ud83d\\ude00\\u2028\\\"\\\\\\n\\u0001\""
        );
    }
}

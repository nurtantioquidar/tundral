//! The LLM-free markdown structure `update` adds (`docs/graphify-semantic.md` §3): one page node
//! per `.md`/`.mdx` file, heading nodes nested by `contains`, links between pages, and backtick
//! references from headings to unique code callables.
//!
//! It runs inside the core build as a [`Documents`] pass: pages are per-file facts placed after
//! every code file (so collision renaming and file labels span both), and backtick references are
//! a cross-file pass over the renamed graph.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use dijkstral_core::discover::SourceFile;
use dijkstral_core::extract::{count_words, origin};
use dijkstral_core::facts::{Attr, Confidence, Edge, FileFacts, Loc, Node, NodeKind};
use dijkstral_core::ids::{file_id, make_id};
use dijkstral_core::lang::Family;
use dijkstral_core::resolve::{Index, Resolution, View};
use dijkstral_core::{DocumentFacts, Documents};
use sha2::{Digest, Sha256};

/// Texts that never resolve to code, even when unique (§3.4; exact case).
const DENYLIST: &[&str] = &[
    "abs",
    "all",
    "any",
    "bool",
    "bytes",
    "callable",
    "console",
    "delattr",
    "dict",
    "dir",
    "enumerate",
    "filter",
    "float",
    "getattr",
    "hasattr",
    "hash",
    "id",
    "int",
    "isinstance",
    "iter",
    "len",
    "list",
    "map",
    "max",
    "min",
    "next",
    "open",
    "print",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "sorted",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "encodeURIComponent",
    "decodeURIComponent",
    "Array",
    "Bool",
    "Boolean",
    "Color",
    "Comparable",
    "Data",
    "Date",
    "Double",
    "Equatable",
    "Error",
    "Float",
    "Font",
    "Hashable",
    "Identifiable",
    "Int",
    "JSON",
    "Map",
    "Math",
    "Number",
    "Object",
    "Promise",
    "RegExp",
    "Request",
    "Response",
    "Sendable",
    "Set",
    "String",
    "Symbol",
    "TypeError",
    "URL",
    "UUID",
    "View",
    "Character",
    "Int64",
    "UInt",
    "NSObject",
    "Codable",
    "Decodable",
    "Encodable",
    "URLRequest",
    "URLSession",
    "DispatchQueue",
    "Headers",
    "FormData",
    "Blob",
    "File",
    "WeakMap",
    "WeakSet",
    "BigInt",
    "Intl",
    "Reflect",
    "Proxy",
    "CaseIterable",
    "NSError",
    "AnyObject",
    "SyntaxError",
    "RangeError",
    "ReferenceError",
    "EvalError",
    "URIError",
];

/// Markdown that reads as a paper is a paper, not a page.
fn is_page(rel: &str, content: &[u8]) -> bool {
    is_markdown(rel) && !crate::ingest::looks_like_paper(&String::from_utf8_lossy(content))
}

/// Whether `rel` is a markdown file (`.md`/`.mdx`, any case; `.markdown` is not, §3).
pub fn is_markdown(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rfind('.') {
        Some(i) if i > 0 => {
            let ext = name[i + 1..].to_ascii_lowercase();
            ext == "md" || ext == "mdx"
        }
        _ => false,
    }
}

/// An inline code span waiting for the cross-file pass.
#[derive(Debug, Clone, PartialEq)]
struct Span {
    /// The section's id before collision renaming (page or nearest preceding heading).
    section: String,
    line: u32,
    /// The span text, stripped.
    text: String,
}

/// `update`'s markdown structure as a core build pass (`dijkstral_core::build_full`).
#[derive(Default)]
pub struct MarkdownPass {
    /// Backtick spans per file, from `extract` to `resolve`.
    spans: Mutex<HashMap<String, Vec<Span>>>,
}

impl MarkdownPass {
    pub fn new() -> MarkdownPass {
        MarkdownPass::default()
    }
}

impl Documents for MarkdownPass {
    /// The sources that shape the pages and the word totals.
    fn fingerprint(&self) -> String {
        let mut h = Sha256::new();
        h.update(include_bytes!("markdown.rs"));
        h.update(include_bytes!("ingest.rs"));
        format!("markdown:{:x}", h.finalize())
    }

    /// Markdown pages, and every other file of the semantic pass for the report's word total.
    fn accepts(&self, rel: &str) -> bool {
        is_markdown(rel) || crate::ingest::classify(rel).is_some()
    }

    fn extract(&self, root: &Path, files: &[(SourceFile, Vec<u8>)]) -> DocumentFacts {
        let pages: HashSet<String> = files
            .iter()
            .filter(|(f, content)| is_page(&f.rel, content))
            .map(|(f, _)| file_id(&f.rel))
            .collect();
        let mut spans = self.spans.lock().unwrap();
        let mut out = DocumentFacts::default();
        for (i, (f, content)) in files.iter().enumerate() {
            if is_page(&f.rel, content) {
                let page = parse_page(&f.rel, content, &|path| link_target(root, path, &pages));
                spans.insert(f.rel.clone(), page.spans);
                out.pages.push((i, page.facts));
            } else {
                out.words += document_words(&f.rel, content);
            }
        }
        out
    }

    fn resolve(&self, views: &[View], _index: &Index) -> Resolution {
        let spans = self.spans.lock().unwrap();
        resolve_references(views, &spans)
    }
}

/// The words `update`'s report counts for a non-page document (`len(text.split())`): its text
/// decoded as UTF-8 with invalid bytes dropped (raster images included), the converted Markdown
/// of an office file, the extracted text of a PDF.
fn document_words(rel: &str, content: &[u8]) -> u64 {
    use crate::ingest::{classify, convert_office, pdf_text, Class};
    let text = match classify(rel) {
        Some(Class::Office) => convert_office(rel, content).unwrap_or_default(),
        Some(Class::Paper) => pdf_text(content).unwrap_or_default(),
        _ => content.utf8_chunks().map(|c| c.valid()).collect(),
    };
    text.split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|w| !w.is_empty())
        .count() as u64
}

/// One page's facts and its backtick spans.
struct Page {
    facts: FileFacts,
    spans: Vec<Span>,
}

/// Parse one markdown file (§3.1–3.5). `resolve_link` maps a link's path, already resolved
/// against the root, to a page id.
fn parse_page(rel: &str, content: &[u8], resolve_link: &dyn Fn(&str) -> Option<String>) -> Page {
    let text = String::from_utf8_lossy(content);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let fid = file_id(rel);
    let name = rel.rsplit('/').next().unwrap_or(rel);

    let (front, body_start) = front_matter(&lines);
    let mut page_attrs = vec![("node_kind".to_string(), Attr::Str("page".into()))];
    if let Some(fm) = front {
        page_attrs.push(("frontmatter".to_string(), Attr::Json(fm)));
    }
    page_attrs.push(origin());
    let mut facts = FileFacts {
        words: count_words(content),
        ..Default::default()
    };
    facts.nodes.push(Node {
        id: fid.clone(),
        label: name.to_string(),
        kind: NodeKind::File,
        file_type: "document".into(),
        source_file: rel.to_string(),
        loc: Loc::Line(1),
        attrs: page_attrs,
    });

    let dir = rel.rfind('/').map_or("", |i| &rel[..i]);
    let mut ids: HashSet<String> = HashSet::from([fid.clone()]);
    // (level, id) of the open headings.
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut section = fid.clone();
    let mut linked: HashSet<String> = HashSet::new();
    let mut spans = Vec::new();
    let mut in_fence = false;
    for (i, line) in lines.iter().enumerate().skip(body_start) {
        let n = i as u32 + 1;
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some((level, label)) = heading(line) {
            let mut id = make_id(&[fid.as_str(), label]);
            if !ids.insert(id.clone()) {
                id = format!("{id}_{n}");
                ids.insert(id.clone());
            }
            while stack.last().is_some_and(|(l, _)| *l >= level) {
                stack.pop();
            }
            let parent = stack.last().map_or(fid.as_str(), |(_, p)| p.as_str());
            facts.edges.push(edge(parent, &id, "contains", rel, n));
            facts.nodes.push(Node {
                id: id.clone(),
                label: label.to_string(),
                kind: NodeKind::Def,
                file_type: "document".into(),
                source_file: rel.to_string(),
                loc: Loc::Line(n),
                attrs: vec![
                    ("node_kind".to_string(), Attr::Str("heading".into())),
                    origin(),
                ],
            });
            stack.push((level, id.clone()));
            section = id;
        }
        for dest in links(line) {
            let Some(path) = resolve_relative(dir, &dest) else {
                continue;
            };
            let Some(target) = resolve_link(&path) else {
                continue;
            };
            if target != fid && linked.insert(target.clone()) {
                facts.edges.push(edge(&fid, &target, "references", rel, n));
            }
        }
        for text in code_spans(line) {
            spans.push(Span {
                section: section.clone(),
                line: n,
                text: text.to_string(),
            });
        }
    }
    Page { facts, spans }
}

fn edge(source: &str, target: &str, relation: &str, rel: &str, line: u32) -> Edge {
    Edge {
        source: source.into(),
        target: target.into(),
        relation: relation.into(),
        confidence: Confidence::Extracted,
        score: None,
        context: None,
        source_file: rel.into(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

/// Front matter (§3.2): the serialized `frontmatter` object (absent when nothing usable) and the
/// index of the first line after the block.
fn front_matter(lines: &[&str]) -> (Option<String>, usize) {
    if lines.first() != Some(&"---") {
        return (None, 0);
    }
    let Some(end) = lines
        .iter()
        .skip(1)
        .position(|l| *l == "---")
        .map(|p| p + 1)
    else {
        return (None, 0);
    };
    let mut map = serde_json::Map::new();
    for line in &lines[1..end] {
        if line.starts_with([' ', '\t', '#']) {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let mut value = value.trim();
        for q in ['"', '\''] {
            if value.len() >= 2 && value.starts_with(q) && value.ends_with(q) {
                value = &value[1..value.len() - 1];
                break;
            }
        }
        if key.is_empty() || value.is_empty() {
            continue;
        }
        map.insert(
            key.to_string(),
            serde_json::Value::String(html_escape(value)),
        );
    }
    let fm = (!map.is_empty()).then(|| serde_json::Value::Object(map).to_string());
    (fm, end + 1)
}

/// Python's `html.escape(s, quote=True)`.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// `^#{1,6} +(.+)` at column 0 (§3.3): the level and the stripped label.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) || !line[level..].starts_with(' ') {
        return None;
    }
    let label = line[level..].trim();
    (!label.is_empty()).then_some((level, label))
}

/// Inline code spans (§3.4): text between a pair of backticks with no backtick inside, which also
/// reads a double-backtick span by its inner pair. Returned stripped.
fn code_spans(line: &str) -> Vec<&str> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'`' {
            i += 1;
            continue;
        }
        match line[i + 1..].find('`') {
            Some(0) => i += 1,
            Some(len) => {
                out.push(line[i + 1..i + 1 + len].trim());
                i += len + 2;
            }
            None => break,
        }
    }
    out
}

/// Link destinations on one line, in order of position (§3.5): inline links (not images),
/// reference definitions and wiki links.
fn links(line: &str) -> Vec<String> {
    let mut found: Vec<(usize, String)> = Vec::new();
    let b = line.as_bytes();
    // Inline `[text](dest)`.
    let mut i = 0;
    while let Some(off) = line[i..].find("](") {
        let close = i + off;
        let start = line[..close].rfind(['[', ']']);
        let dest_end = line[close + 2..].find(')').map(|e| close + 2 + e);
        match (start, dest_end) {
            (Some(open), Some(end)) if b[open] == b'[' => {
                if open == 0 || b[open - 1] != b'!' {
                    found.push((open, line[close + 2..end].to_string()));
                }
                i = end + 1;
            }
            _ => i = close + 2,
        }
    }
    // Reference definitions `[r]: dest`.
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() <= 3 && trimmed.starts_with('[') && !trimmed.starts_with("[[") {
        if let Some(end) = trimmed.find("]:") {
            let label = &trimmed[1..end];
            if !label.is_empty() && !label.contains(['[', ']']) {
                let at = line.len() - trimmed.len();
                found.push((at, trimmed[end + 2..].trim().to_string()));
            }
        }
    }
    // Wiki links `[[name]]`, `[[name|alias]]`.
    let mut i = 0;
    while let Some(off) = line[i..].find("[[") {
        let open = i + off;
        match line[open + 2..].find("]]") {
            Some(len) => {
                let inner = &line[open + 2..open + 2 + len];
                if !inner.is_empty() && !inner.contains(['[', ']']) {
                    let name = inner.split('|').next().unwrap_or(inner);
                    found.push((open, name.to_string()));
                }
                i = open + 2 + len + 2;
            }
            None => break,
        }
    }
    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, d)| d).collect()
}

/// Clean a destination and resolve it against the linking file's directory; `None` for external,
/// absolute or escaping links.
fn resolve_relative(dir: &str, dest: &str) -> Option<String> {
    let mut d = dest.trim();
    if let Some(rest) = d.strip_prefix('<') {
        d = &rest[..rest.find('>')?];
    } else {
        d = d.split([' ', '\t']).next().unwrap_or("");
    }
    let d = d.split(['#', '?']).next().unwrap_or("");
    let d = percent_decode(d);
    let lower = d.to_ascii_lowercase();
    if d.is_empty()
        || d.starts_with('/')
        || lower.starts_with("mailto:")
        || lower.starts_with("http:")
        || lower.starts_with("https:")
        || d.contains("://")
    {
        return None;
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for c in Path::new(&d).components() {
        match c {
            Component::Normal(p) => parts.push(p.to_str()?),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The page a resolved link path names: the path if it exists, else `path.md`; its id must be a
/// markdown page id before collision renaming (§3.5).
fn link_target(root: &Path, path: &str, pages: &HashSet<String>) -> Option<String> {
    let existing: PathBuf = if root.join(path).exists() {
        PathBuf::from(path)
    } else if root.join(format!("{path}.md")).exists() {
        PathBuf::from(format!("{path}.md"))
    } else {
        return None;
    };
    let id = file_id(existing.to_str()?);
    pages.contains(&id).then_some(id)
}

/// A code node a backtick span may name (§3.4).
struct Candidate<'a> {
    id: &'a str,
    source_file: &'a str,
    class: Option<&'a str>,
}

/// Backtick references (§3.4) from every page's spans, in file and line order.
fn resolve_references(views: &[View], spans: &HashMap<String, Vec<Span>>) -> Resolution {
    // Callables by name, one entry per distinct final id.
    let mut by_name: HashMap<&str, Vec<Candidate>> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for v in views.iter().filter(|v| v.family() != Family::Document) {
        let classes: HashSet<&str> = v
            .facts
            .nodes
            .iter()
            .filter(|n| flagged(n, "_callable_class"))
            .map(|n| n.id.as_str())
            .collect();
        let mut enclosing: HashMap<&str, &str> = HashMap::new();
        for e in &v.facts.edges {
            if (e.relation == "method" || e.relation == "contains")
                && classes.contains(e.source.as_str())
            {
                enclosing
                    .entry(e.target.as_str())
                    .or_insert(e.source.as_str());
            }
        }
        let labels: HashMap<&str, &str> = v
            .facts
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect();
        for n in &v.facts.nodes {
            if !flagged(n, "_callable") || !seen.insert(n.id.as_str()) {
                continue;
            }
            let name = n.label.strip_prefix('.').unwrap_or(&n.label);
            let name = name.strip_suffix("()").unwrap_or(name);
            by_name.entry(name).or_default().push(Candidate {
                id: &n.id,
                source_file: &n.source_file,
                class: enclosing
                    .get(n.id.as_str())
                    .and_then(|c| labels.get(c).copied()),
            });
        }
    }

    let mut out = Resolution::default();
    for v in views.iter().filter(|v| v.family() == Family::Document) {
        let Some(spans) = spans.get(&v.file.rel) else {
            continue;
        };
        let mut done: HashSet<(String, &str)> = HashSet::new();
        for s in spans {
            let Some(target) = reference_target(&s.text, &by_name) else {
                continue;
            };
            let source = v.id(&s.section).to_string();
            if !done.insert((source.clone(), target)) {
                continue;
            }
            out.edges.push(Edge {
                source,
                target: target.to_string(),
                relation: "references".into(),
                confidence: Confidence::Inferred,
                score: Some(0.95),
                context: Some(s.text.clone()),
                source_file: v.file.rel.clone(),
                loc: Loc::Line(s.line),
                weight: Some(1.0),
                attrs: vec![origin()],
            });
        }
    }
    out
}

fn flagged(n: &Node, key: &str) -> bool {
    matches!(n.attr(key), Some(Attr::Bool(true)))
}

/// The one code node a span names, if any (§3.4: shape, denylist, uniqueness, qualifiers).
fn reference_target<'a>(
    text: &str,
    by_name: &HashMap<&str, Vec<Candidate<'a>>>,
) -> Option<&'a str> {
    let key = text.strip_suffix("()").unwrap_or(text);
    let segments: Vec<&str> = key.split('.').collect();
    if !segments.iter().all(|s| is_identifier(s)) {
        return None;
    }
    let (name, qualifiers) = segments.split_last()?;
    if DENYLIST.contains(name) {
        return None;
    }
    let [c] = by_name.get(name)?.as_slice() else {
        return None;
    };
    let path = c.source_file;
    let (dirs, file) = path.rsplit_once('/').unwrap_or(("", path));
    let stem = file
        .rfind('.')
        .filter(|&i| i > 0)
        .map_or(file, |i| &file[..i]);
    let ok = qualifiers.iter().all(|q| {
        *q == stem || c.class == Some(*q) || dirs.split('/').any(|d| !d.is_empty() && d == *q)
    });
    ok.then_some(c.id)
}

/// `[A-Za-z_][A-Za-z0-9_]*`.
fn is_identifier(s: &str) -> bool {
    let mut b = s.bytes();
    matches!(b.next(), Some(c) if c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_links(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn headings_follow_the_column_zero_rule() {
        assert_eq!(heading("# A"), Some((1, "A")));
        assert_eq!(heading("###### Six"), Some((6, "Six")));
        assert_eq!(heading("####### seven"), None);
        assert_eq!(heading("#NoSpace"), None);
        assert_eq!(heading("#"), None);
        assert_eq!(heading("   ### Indented"), None);
        assert_eq!(heading("\u{feff}# Bom"), None);
        assert_eq!(
            heading("# Closing hashes ##"),
            Some((1, "Closing hashes ##"))
        );
    }

    #[test]
    fn front_matter_values_are_escaped_strings() {
        let lines = [
            "---",
            "title: \"Probe\"",
            "html: <b>&\"'</b>",
            "empty:",
            "  - nested",
            "# comment",
            "dup: first",
            "dup: second",
            "---",
            "# Heading",
        ];
        let (fm, start) = front_matter(&lines);
        assert_eq!(start, 9);
        assert_eq!(
            fm.unwrap(),
            r#"{"title":"Probe","html":"&lt;b&gt;&amp;&quot;&#x27;&lt;/b&gt;","dup":"second"}"#
        );
        assert_eq!(front_matter(&["---", "a: 1"]), (None, 0));
        assert_eq!(front_matter(&["x", "---", "a: 1", "---"]), (None, 0));
    }

    #[test]
    fn spans_read_single_and_double_backticks() {
        assert_eq!(
            code_spans("`tsFn` first, then `tsFn` again and ``Gamma`` in double"),
            ["tsFn", "tsFn", "Gamma"]
        );
        assert_eq!(code_spans("` run ` and `run()`"), ["run", "run()"]);
        assert_eq!(code_spans("no `closing"), Vec::<&str>::new());
    }

    #[test]
    fn links_skip_images_and_keep_position_order() {
        let line = "Back to the [readme](../README.md), a [[other]] wiki link and [sibling](other.md \"Title\").";
        assert_eq!(links(line), ["../README.md", "other", "other.md \"Title\""]);
        assert_eq!(
            links("image ![img](other.md), autolink <other.md>"),
            Vec::<String>::new()
        );
        assert_eq!(links("[r1]: ../notes.mdx"), ["../notes.mdx"]);
        assert_eq!(links("[[name|alias]] [[sub/name]]"), ["name", "sub/name"]);
    }

    #[test]
    fn destinations_are_cleaned_and_resolved() {
        assert_eq!(
            resolve_relative("docs", "../README.md").as_deref(),
            Some("README.md")
        );
        assert_eq!(
            resolve_relative("", "./docs/other.md#top").as_deref(),
            Some("docs/other.md")
        );
        assert_eq!(
            resolve_relative("docs", "<other.md>").as_deref(),
            Some("docs/other.md")
        );
        assert_eq!(
            resolve_relative("docs", "other.md?x=1").as_deref(),
            Some("docs/other.md")
        );
        assert_eq!(
            resolve_relative("docs", "my%20page.md").as_deref(),
            Some("docs/my page.md")
        );
        assert_eq!(resolve_relative("", "../out.md"), None);
        assert_eq!(resolve_relative("", "/abs.md"), None);
        assert_eq!(resolve_relative("", "https://example.com/x.md"), None);
        assert_eq!(resolve_relative("", "mailto:a@b.c"), None);
    }

    #[test]
    fn duplicate_headings_take_their_line_and_nest_by_level() {
        let src = "## Starts at two\n\n# Then one\n\n### Then three\n\n#### Same\n\n#### Same\n\n# Same\n";
        let page = parse_page("docs/other.md", src.as_bytes(), &no_links);
        let ids: Vec<&str> = page.facts.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "docs_other",
                "docs_other_starts_at_two",
                "docs_other_then_one",
                "docs_other_then_three",
                "docs_other_same",
                "docs_other_same_9",
                "docs_other_same_11"
            ]
        );
        let parents: Vec<(&str, &str)> = page
            .facts
            .edges
            .iter()
            .map(|e| (e.source.as_str(), e.target.as_str()))
            .collect();
        assert_eq!(parents[3], ("docs_other_then_three", "docs_other_same"));
        assert_eq!(parents[5], ("docs_other", "docs_other_same_11"));
    }

    #[test]
    fn fenced_lines_are_skipped_but_tilde_fences_are_not() {
        let src = "# Top\n```\n# Hidden `x`\n```\n~~~\n## Tilde `y`\n~~~\n";
        let page = parse_page("a.md", src.as_bytes(), &no_links);
        let labels: Vec<&str> = page.facts.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["a.md", "Top", "Tilde `y`"]);
        let spans: Vec<(&str, u32)> = page
            .spans
            .iter()
            .map(|s| (s.text.as_str(), s.line))
            .collect();
        assert_eq!(spans, [("y", 6)]);
        assert_eq!(page.spans[0].section, "a_tilde_y");
    }

    #[test]
    fn reference_shapes_and_qualifiers() {
        let mut by_name: HashMap<&str, Vec<Candidate>> = HashMap::new();
        by_name.entry("other").or_default().push(Candidate {
            id: "pkg_b_beta_other",
            source_file: "pkg/b.py",
            class: Some("Beta"),
        });
        by_name.entry("str").or_default().push(Candidate {
            id: "a_str",
            source_file: "a.py",
            class: None,
        });
        for (text, want) in [
            ("other", Some("pkg_b_beta_other")),
            ("other()", Some("pkg_b_beta_other")),
            ("Beta.other", Some("pkg_b_beta_other")),
            ("b.Beta.other", Some("pkg_b_beta_other")),
            ("Beta.pkg.other", Some("pkg_b_beta_other")),
            ("Alpha.other", None),
            ("PKG.other", None),
            ("b_beta.other", None),
            ("other(1)", None),
            ("pkg/b.other", None),
            (".other", None),
            ("str", None),
        ] {
            assert_eq!(reference_target(text, &by_name), want, "{text}");
        }
    }
}

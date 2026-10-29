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
use dijkstral_core::extract::data::yaml::{self, PyValue};
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

/// Final segments that make a qualified span a file name (`a.py`, `Alpha.go`), never a reference
/// (§3.4; exact case: `K.PY` and `K.F77` resolve, `K.F90` does not).
const EXTENSIONS: &[&str] = &[
    "py", "ts", "tsx", "mts", "cts", "ets", "js", "jsx", "mjs", "cjs", "go", "rs", "java", "kt",
    "kts", "scala", "cs", "rb", "rake", "c", "h", "cpp", "cc", "cxx", "hpp", "cu", "cuh", "metal",
    "swift", "php", "groovy", "gradle", "dart", "lua", "luau", "zig", "ex", "exs", "erl", "hrl",
    "ml", "mli", "jl", "m", "mm", "v", "sv", "svh", "vh", "sol", "f", "f90", "f95", "f03", "f08",
    "F", "F90", "F95", "F03", "F08", "vb", "vue", "svelte", "astro", "ejs", "ps1", "psm1", "psd1",
    "r", "sql", "tf", "tfvars", "hcl", "sh", "bash", "json", "csproj", "fsproj", "vbproj", "sln",
    "slnx", "razor", "cshtml", "xaml", "pas", "dpr", "pp", "inc", "lpr", "dpk", "dfm", "lfm",
    "lpk", "cls", "trigger", "cob", "cbl", "cpy", "dm", "dme", "dmf", "dmi", "dmm", "lisp", "lsp",
    "cl", "md", "mdx", "qmd", "rst", "txt", "html", "yaml", "yml",
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
    /// The directory names of the indexed root's resolved absolute path, which qualify every
    /// code node (§3.4).
    root_dirs: Mutex<Vec<String>>,
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
        *self.root_dirs.lock().unwrap() = root_dirs(root);
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
        let root_dirs = self.root_dirs.lock().unwrap();
        resolve_references(views, &spans, &root_dirs)
    }
}

/// The directory names of `root`'s resolved absolute path (`/tmp/x` on macOS gives `private`,
/// `tmp`, `x`), the root included.
fn root_dirs(root: &Path) -> Vec<String> {
    let abs = std::fs::canonicalize(root)
        .or_else(|_| std::path::absolute(root))
        .unwrap_or_else(|_| root.to_path_buf());
    abs.components()
        .filter_map(|c| match c {
            Component::Normal(p) => p.to_str().map(str::to_string),
            _ => None,
        })
        .collect()
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
    let mut fence: Option<(char, usize)> = None;
    for (i, line) in lines.iter().enumerate().skip(body_start) {
        let n = i as u32 + 1;
        let trimmed = line.trim();
        if let Some((c, len)) = fence {
            if trimmed.len() >= len && trimmed.chars().all(|x| x == c) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = fence_open(trimmed) {
            fence = Some(open);
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

/// A fence opener (§3.3), on a stripped line: three or more backticks (no backtick after them) or
/// tildes. The fence closes on a line of only that character, at least as many.
fn fence_open(trimmed: &str) -> Option<(char, usize)> {
    for c in ['`', '~'] {
        let n = trimmed.chars().take_while(|&x| x == c).count();
        if n >= 3 {
            return (c == '~' || !trimmed[n..].contains('`')).then_some((c, n));
        }
    }
    None
}

/// Front matter (§3.2): the serialized `frontmatter` object (absent when nothing usable) and the
/// index of the first line after the block. The block is read as YAML; a block a safe loader
/// rejects falls back to its `key: value` lines.
fn front_matter(lines: &[&str]) -> (Option<String>, usize) {
    if lines.first().map(|l| l.trim()) != Some("---") {
        return (None, 0);
    }
    let Some(end) = lines
        .iter()
        .skip(1)
        .position(|l| matches!(l.trim(), "---" | "..."))
        .map(|p| p + 1)
    else {
        return (None, 0);
    };
    let block = &lines[1..end];
    let map = match yaml::safe_load(&block.join("\n")) {
        Ok(PyValue::Dict(pairs)) => front_object(&pairs),
        Ok(_) => serde_json::Map::new(),
        Err(()) => front_lines(block),
    };
    let fm = (!map.is_empty()).then(|| serde_json::Value::Object(map).to_string());
    (fm, end + 1)
}

/// A loaded front matter mapping: keys as Python's `str()` of them, strings without ASCII control
/// characters and HTML-escaped, at any depth. Values JSON cannot hold (`.inf`, `.nan`, integers
/// beyond 64 bits) are written as `null` or a float (Graphify writes bare `Infinity`/`NaN`).
fn front_object(pairs: &[(PyValue, PyValue)]) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        map.insert(html_escape(&clean(&k.py_str())), front_value(v));
    }
    map
}

fn front_value(v: &PyValue) -> serde_json::Value {
    use serde_json::{Number, Value};
    match v {
        PyValue::None => Value::Null,
        PyValue::Bool(b) => Value::Bool(*b),
        PyValue::Int(d) => d
            .parse::<i64>()
            .map(Value::from)
            .or_else(|_| d.parse::<u64>().map(Value::from))
            .unwrap_or_else(|_| {
                d.parse::<f64>()
                    .ok()
                    .and_then(Number::from_f64)
                    .map_or(Value::Null, Value::Number)
            }),
        PyValue::Float(x) => Number::from_f64(*x).map_or(Value::Null, Value::Number),
        PyValue::Str(s) | PyValue::Text(s) => Value::String(html_escape(&clean(s))),
        PyValue::List(items) => Value::Array(items.iter().map(front_value).collect()),
        PyValue::Dict(pairs) => Value::Object(front_object(pairs)),
    }
}

/// Text without ASCII control characters (`\t`, `\n`, `\r`, DEL…).
fn clean(s: &str) -> String {
    s.chars().filter(|&c| c >= ' ' && c != '\u{7f}').collect()
}

/// The fallback for front matter that is not valid YAML: each non-indented `key: value` line
/// with a non-empty value, as a string.
fn front_lines(block: &[&str]) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for line in block {
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
    map
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

/// `^ {0,3}#{1,6}[ \t]` (§3.3): the level and the label, stripped and without a closing run of
/// `#` that follows a space or tab; a heading with no text is none.
fn heading(line: &str) -> Option<(usize, &str)> {
    let body = line.trim_start_matches(' ');
    if line.len() - body.len() > 3 {
        return None;
    }
    let line = body;
    let level = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) || !line[level..].starts_with([' ', '\t']) {
        return None;
    }
    let mut label = line[level..].trim();
    let bare = label.trim_end_matches('#');
    if bare.len() < label.len() {
        if bare.is_empty() {
            label = "";
        } else if bare.ends_with([' ', '\t']) {
            label = bare.trim_end();
        }
    }
    (!label.is_empty()).then_some((level, label))
}

/// Inline code spans (§3.4): a run of N backticks opens a span that the next run of exactly N
/// closes (`` `` `%||%` `` `` is one span); an opening run with no closing run is plain text.
/// Backslashes escape nothing. Returned stripped.
fn code_spans(line: &str) -> Vec<&str> {
    // Maximal backtick runs as (start, length).
    let b = line.as_bytes();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`' {
            let start = i;
            while i < b.len() && b[i] == b'`' {
                i += 1;
            }
            runs.push((start, i - start));
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    let mut r = 0;
    while r < runs.len() {
        let (start, n) = runs[r];
        match (r + 1..runs.len()).find(|&k| runs[k].1 == n) {
            Some(k) => {
                out.push(line[start + n..runs[k].0].trim());
                r = k + 1;
            }
            None => r += 1,
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
    /// Names of the enclosing definitions, innermost first (`Top.tm.tin` → `tm`, `Top`).
    scopes: Vec<&'a str>,
}

/// A label as a backtick name: no leading `.`, no trailing `()`.
fn bare_name(label: &str) -> &str {
    let name = label.strip_prefix('.').unwrap_or(label);
    name.strip_suffix("()").unwrap_or(name)
}

/// The enclosing definitions of the node `id` named `name`, read off the ids (§3.4): drop
/// `_<slug(name)>` from the id; while what is left is a code node other than a file, its name is
/// the next scope. A local class with no node of its own is skipped (`outer.meth`), and a
/// collision-merged node keeps its first label (`Box_put.take` is qualified by `put`, `Box`). An
/// id disambiguated by a hash (`k_k_go_2e0b45` for `.Go()` beside `.go()`) drops it first.
fn scopes<'a>(id: &str, name: &str, labels: &HashMap<&str, (&'a str, bool)>) -> Vec<&'a str> {
    let mut out = Vec::new();
    let (mut id, mut name) = (id.to_string(), name.to_string());
    loop {
        let suffix = format!("_{}", make_id(&[name.as_str()]));
        let Some(parent) = id
            .strip_suffix(suffix.as_str())
            .or_else(|| strip_hash(&id)?.strip_suffix(suffix.as_str()))
        else {
            break;
        };
        let Some(&(label, is_file)) = labels.get(parent) else {
            break;
        };
        if is_file {
            break;
        }
        let parent_name = bare_name(label);
        out.push(parent_name);
        id = parent.to_string();
        name = parent_name.to_string();
    }
    out
}

/// `id` without a trailing `_<6 lowercase hex>` collision hash (format spec §4.5).
fn strip_hash(id: &str) -> Option<&str> {
    let (base, hash) = id.rsplit_once('_')?;
    (hash.len() == 6 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then_some(base)
}

/// Backtick references (§3.4) from every page's spans, in file and line order.
fn resolve_references(
    views: &[View],
    spans: &HashMap<String, Vec<Span>>,
    root_dirs: &[String],
) -> Resolution {
    let code: Vec<&View> = views
        .iter()
        .filter(|v| v.family() != Family::Document)
        .collect();
    // Every file's and definition's label by id (the first of merged ids), and whether it is a
    // file.
    let mut labels: HashMap<&str, (&str, bool)> = HashMap::new();
    for v in &code {
        for n in v
            .facts
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::File | NodeKind::Def))
        {
            labels
                .entry(n.id.as_str())
                .or_insert((n.label.as_str(), n.kind == NodeKind::File));
        }
    }
    // Callables by name, one entry per distinct final id.
    let mut by_name: HashMap<&str, Vec<Candidate>> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for v in &code {
        for n in &v.facts.nodes {
            if !flagged(n, "_callable") || !seen.insert(n.id.as_str()) {
                continue;
            }
            let name = bare_name(&n.label);
            by_name.entry(name).or_default().push(Candidate {
                id: &n.id,
                source_file: &n.source_file,
                scopes: scopes(&n.id, name, &labels),
            });
        }
    }
    let root_dirs: HashSet<&str> = root_dirs.iter().map(String::as_str).collect();

    let mut out = Resolution::default();
    for v in views.iter().filter(|v| v.family() == Family::Document) {
        let Some(spans) = spans.get(&v.file.rel) else {
            continue;
        };
        let mut done: HashSet<(String, &str)> = HashSet::new();
        for s in spans {
            let Some(target) = reference_target(&s.text, &by_name, &root_dirs) else {
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

/// The one code node a span names, if any (§3.4: shape, denylist, file names, qualifiers,
/// uniqueness). `root_dirs` are the directory names of the indexed root's absolute path.
fn reference_target<'a>(
    text: &str,
    by_name: &HashMap<&str, Vec<Candidate<'a>>>,
    root_dirs: &HashSet<&str>,
) -> Option<&'a str> {
    let key = text.strip_suffix("()").unwrap_or(text);
    let segments: Vec<&str> = key.split('.').collect();
    if !segments.iter().all(|s| is_identifier(s)) {
        return None;
    }
    let (name, qualifiers) = segments.split_last()?;
    if DENYLIST.contains(name) || (!qualifiers.is_empty() && EXTENSIONS.contains(name)) {
        return None;
    }
    let mut found = by_name
        .get(name)?
        .iter()
        .filter(|c| qualifiers.iter().all(|q| qualifies(q, c, root_dirs)));
    match (found.next(), found.next()) {
        (Some(c), None) => Some(c.id),
        _ => None,
    }
}

/// Whether `q` names a directory of the candidate's absolute path, its file stem or one of its
/// enclosing definitions (case-sensitive).
fn qualifies(q: &str, c: &Candidate, root_dirs: &HashSet<&str>) -> bool {
    let (dirs, file) = c
        .source_file
        .rsplit_once('/')
        .unwrap_or(("", c.source_file));
    let stem = file
        .rfind('.')
        .filter(|&i| i > 0)
        .map_or(file, |i| &file[..i]);
    q == stem || c.scopes.contains(&q) || root_dirs.contains(q) || dirs.split('/').any(|d| d == q)
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
    fn headings_allow_three_spaces_and_drop_closing_hashes() {
        assert_eq!(heading("# A"), Some((1, "A")));
        assert_eq!(heading("###### Six"), Some((6, "Six")));
        assert_eq!(heading("####### seven"), None);
        assert_eq!(heading("#NoSpace"), None);
        assert_eq!(heading("#\tTab"), Some((1, "Tab")));
        assert_eq!(heading("#"), None);
        assert_eq!(heading("# #"), None);
        assert_eq!(heading("   ### Indented"), Some((3, "Indented")));
        assert_eq!(heading("    #### Code"), None);
        assert_eq!(heading("\t# Tab indent"), None);
        assert_eq!(heading("\u{feff}# Bom"), None);
        assert_eq!(heading("# Closing hashes ##"), Some((1, "Closing hashes")));
        assert_eq!(heading("#  Two  spaces  #"), Some((1, "Two  spaces")));
        assert_eq!(heading("# A#"), Some((1, "A#")));
        assert_eq!(heading("# Escaped \\#"), Some((1, "Escaped \\#")));
        assert_eq!(heading("# Trail ##x"), Some((1, "Trail ##x")));
    }

    #[test]
    fn front_matter_is_yaml_with_escaped_strings() {
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
            r#"{"title":"Probe","html":"&lt;b&gt;&amp;&quot;&#x27;&lt;/b&gt;","empty":["nested"],"dup":"second"}"#
        );
        // Typed values, `str()` keys, control characters dropped, `...` closing the block.
        let (fm, _) = front_matter(&[
            " --- ",
            "n: 0x1F",
            "t: yes",
            "d: 2026-01-02T03:04:05Z",
            "1: one",
            "true: two",
            "q: \"a\\tb\"",
            "...",
        ]);
        assert_eq!(
            fm.unwrap(),
            r#"{"n":31,"t":true,"d":"2026-01-02 03:04:05+00:00","1":"two","q":"ab"}"#
        );
        // Not valid YAML: the `key: value` lines, as strings; valid but not a mapping: nothing.
        let (fm, _) = front_matter(&["---", "custom: !Ref x", "n: 1", "---"]);
        assert_eq!(fm.unwrap(), r#"{"custom":"!Ref x","n":"1"}"#);
        assert_eq!(front_matter(&["---", "- a: 1", "---"]), (None, 3));
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
        // Probed against Graphify: a run closes at the next run of the same length.
        assert_eq!(code_spans("x `` `%||%` `` then `walk`"), ["`%||%`", "walk"]);
        assert_eq!(code_spans("x ``run` and `jump``"), ["run` and `jump"]);
        assert_eq!(code_spans("x ```hop``` and ``walk``"), ["hop", "walk"]);
        assert_eq!(code_spans("x `` unclosed and `hop`"), ["hop"]);
        assert_eq!(code_spans("x `a``b` `jump`"), ["a``b", "jump"]);
        assert_eq!(code_spans("x ``walk` mixed `` run `"), ["walk` mixed"]);
        assert_eq!(code_spans("x ```jump`` hop`"), Vec::<&str>::new());
        assert_eq!(code_spans("x \\`escaped` `run`"), ["escaped", "run"]);
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
    fn fenced_lines_are_skipped() {
        let src = "# Top\n```\n# Hidden `x`\n```\n~~~\n## Tilde `y`\n~~~\n    ```\n## Four\n``` x\n````\n## After `z`\n``` a`b\n## Info `w`\n";
        let page = parse_page("a.md", src.as_bytes(), &no_links);
        let labels: Vec<&str> = page.facts.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["a.md", "Top", "After `z`", "Info `w`"]);
        let spans: Vec<(&str, u32)> = page
            .spans
            .iter()
            .map(|s| (s.text.as_str(), s.line))
            .collect();
        assert_eq!(spans, [("z", 12), ("w", 14)]);
        assert_eq!(page.spans[0].section, "a_after_z");
    }

    /// Resolve `spans` against code nodes `(id, label, source_file, callable)` (a label ending in
    /// `.py`/`.ts`/`.java` is the file) under a root at `/private/tmp/qprobe/cases/c1`.
    fn resolve_all<'a>(
        nodes: &'a [(&'a str, &'a str, &'a str, bool)],
        spans: &[&str],
    ) -> Vec<Option<&'a str>> {
        let labels: HashMap<&str, (&str, bool)> = nodes
            .iter()
            .map(|(id, label, _, _)| {
                (
                    *id,
                    (
                        *label,
                        label.contains(".py") || label.contains(".ts") || label.contains(".java"),
                    ),
                )
            })
            .collect();
        let mut by_name: HashMap<&str, Vec<Candidate>> = HashMap::new();
        for (id, label, file, callable) in nodes {
            if *callable {
                let name = bare_name(label);
                by_name.entry(name).or_default().push(Candidate {
                    id,
                    source_file: file,
                    scopes: scopes(id, name, &labels),
                });
            }
        }
        let root: HashSet<&str> = ["private", "tmp", "qprobe", "cases", "c1"].into();
        spans
            .iter()
            .map(|s| reference_target(s, &by_name, &root))
            .collect()
    }

    #[test]
    fn reference_shapes_and_qualifiers() {
        let nodes = [
            ("pkg_b", "b.py", "pkg/b.py", false),
            ("pkg_b_beta", "Beta", "pkg/b.py", true),
            ("pkg_b_beta_other", ".other()", "pkg/b.py", true),
            ("a", "a.py", "a.py", false),
            ("a_str", "str()", "a.py", true),
        ];
        for (text, want) in [
            ("other", Some("pkg_b_beta_other")),
            ("other()", Some("pkg_b_beta_other")),
            ("Beta.other", Some("pkg_b_beta_other")),
            ("b.Beta.other", Some("pkg_b_beta_other")),
            ("Beta.pkg.other", Some("pkg_b_beta_other")),
            ("Beta.Beta.other", Some("pkg_b_beta_other")),
            ("Alpha.other", None),
            ("PKG.other", None),
            ("b_beta.other", None),
            ("other.other", None),
            ("other(1)", None),
            ("pkg/b.other", None),
            ("Beta::other", None),
            (".other", None),
            ("str", None),
        ] {
            assert_eq!(resolve_all(&nodes, &[text]), [want], "{text}");
        }
    }

    /// Probed against Graphify 0.9.86 `update` (§3.4): the qualifiers pick one of several
    /// callables of the same name, and the root's absolute path qualifies every node.
    #[test]
    fn qualifiers_disambiguate_and_root_directories_qualify() {
        let nodes = [
            ("pkg_a", "a.py", "pkg/a.py", false),
            ("pkg_a_alpha", "Alpha", "pkg/a.py", true),
            ("pkg_a_alpha_walk", ".walk()", "pkg/a.py", true),
            ("pkg_a_alpha_inner", "Inner", "pkg/a.py", true),
            ("pkg_a_alpha_inner_deep", ".deep()", "pkg/a.py", true),
            ("pkg_a_dup", "dup()", "pkg/a.py", true),
            ("pkg_a_x", "X", "pkg/a.py", true),
            ("pkg_a_x_dup", ".dup()", "pkg/a.py", true),
            ("pkg_b", "b.py", "pkg/b.py", false),
            ("pkg_b_beta", "Beta", "pkg/b.py", true),
            ("pkg_b_beta_walk", ".walk()", "pkg/b.py", true),
            ("pkg_b_beta_inner", "Inner", "pkg/b.py", true),
            ("pkg_b_beta_inner_deep", ".deep()", "pkg/b.py", true),
            ("pkg_b_sub", "Sub", "pkg/b.py", true),
            ("c", "c.py", "c.py", false),
            ("c_run", "run()", "c.py", true),
            ("c_shared", "shared()", "c.py", true),
            ("d", "d.py", "d.py", false),
            ("d_shared", "shared()", "d.py", true),
        ];
        let spans = [
            "walk",
            "Alpha.walk",
            "Beta.walk()",
            "pkg.a.Alpha.walk",
            "a.walk",
            "pkg.walk",
            "alpha.walk",
            "Sub.walk",
            "shared",
            "c.shared",
            "d.shared()",
            "Inner.deep",
            "Alpha.deep",
            "Beta.Inner.deep",
            "dup",
            "a.dup",
            "X.dup",
            "cases.run",
            "private.tmp.c1.c.run",
            "zzz.run",
            "pkg.run",
        ];
        let want = [
            None,
            Some("pkg_a_alpha_walk"),
            Some("pkg_b_beta_walk"),
            Some("pkg_a_alpha_walk"),
            Some("pkg_a_alpha_walk"),
            None,
            None,
            None,
            None,
            Some("c_shared"),
            Some("d_shared"),
            None,
            Some("pkg_a_alpha_inner_deep"),
            Some("pkg_b_beta_inner_deep"),
            None,
            None,
            Some("pkg_a_x_dup"),
            Some("c_run"),
            Some("c_run"),
            None,
            None,
        ];
        assert_eq!(resolve_all(&nodes, &spans), want);
    }

    /// Enclosing functions qualify nested definitions; the scopes are read off the ids, so a
    /// local class without a node is skipped and a top-level `top_level` is not inside `Top`.
    #[test]
    fn enclosing_functions_qualify_nested_definitions() {
        let nodes = [
            ("n", "n.py", "n.py", false),
            ("n_outer", "outer()", "n.py", true),
            ("n_outer_inner", "inner()", "n.py", true),
            ("n_outer_meth", "meth()", "n.py", true),
            ("n_top", "Top", "n.py", true),
            ("n_top_tm", ".tm()", "n.py", true),
            ("n_top_tm_tin", "tin()", "n.py", true),
            ("n_top_level", "top_level()", "n.py", true),
            ("n_box", "Box", "n.py", true),
            ("n_box_put", ".put()", "n.py", true),
            ("n_box_put_take", ".take()", "n.py", true),
            ("w", "w.ts", "w.ts", false),
            ("w_shell", "Shell", "w.ts", true),
            ("w_shell_open", ".open()", "w.ts", true),
            ("w_shell_open_hidden", "hidden()", "w.ts", true),
        ];
        let spans = [
            "outer.inner",
            "outer.meth",
            "Local.meth",
            "Top.tin",
            "tm.tin",
            "Top.tm.tin",
            "Top.top_level",
            "Box.take",
            "put.take",
            "Box_put.take",
            "Shell.open.hidden",
            "Shell.top_level",
        ];
        let want = [
            Some("n_outer_inner"),
            Some("n_outer_meth"),
            None,
            Some("n_top_tm_tin"),
            Some("n_top_tm_tin"),
            Some("n_top_tm_tin"),
            None,
            Some("n_box_put_take"),
            Some("n_box_put_take"),
            None,
            Some("w_shell_open_hidden"),
            None,
        ];
        assert_eq!(resolve_all(&nodes, &spans), want);
    }

    /// A qualified span ending in a file extension is a file name (`K.py`, `K.F90`), case-sensitive;
    /// a bare name never is. The denylist applies to qualified names too.
    #[test]
    fn qualified_file_names_and_denylisted_names_never_resolve() {
        let mut nodes = vec![
            ("k", "k.py", "k.py", false),
            ("k_k", "K", "k.py", true),
            ("z", "z.py", "z.py", false),
            ("z_ts", "ts()", "z.py", true),
        ];
        let methods = [
            ("k_k_go", ".go()"),
            ("k_k_f90", ".F90()"),
            ("k_k_f77", ".F77()"),
            ("k_k_py", ".PY()"),
            ("k_k_toml", ".toml()"),
            ("k_k_zip", ".zip()"),
            ("k_k_json", ".JSON()"),
            // `.Go()` beside `.go()`: the ids differ by a collision hash only.
            ("k_k_go_2e0b45", ".Go()"),
        ];
        for (id, label) in methods {
            nodes.push((id, label, "k.py", true));
        }
        let spans = [
            "ts", "z.ts", "K.go", "k.K.go()", "K.F90", "K.F77", "K.PY", "K.toml", "K.zip",
            "K.JSON", "K.Go",
        ];
        let want = [
            Some("z_ts"),
            None,
            None,
            None,
            None,
            Some("k_k_f77"),
            Some("k_k_py"),
            Some("k_k_toml"),
            None,
            None,
            Some("k_k_go_2e0b45"),
        ];
        assert_eq!(resolve_all(&nodes, &spans), want);
    }
}

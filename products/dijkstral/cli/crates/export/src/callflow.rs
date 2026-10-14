//! The Mermaid call-flow page (`docs/graphify-exports.md` §13): an architecture overview of the
//! graph's sections and, per section, a Mermaid flowchart, a call-details table, key files and a
//! relationship summary.
//!
//! Sections, names, Mermaid ids and the tables reproduce Graphify's `export callflow-html`
//! (rules observed from its output only). The page itself (`callflow.html`, styling, script) is
//! dijkstral's own; it loads Mermaid from jsDelivr pinned with subresource integrity and runs it
//! with `securityLevel: "strict"`. Every untrusted string is escaped for HTML, and node, subgraph
//! and edge labels are also made inert for Mermaid (§13.6).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::html::{escape_html, fill};
use crate::nx::{Attr, NxGraph};

/// Graphify's file name: `<project>-callflow.html`.
pub const CALLFLOW_SUFFIX: &str = "-callflow.html";
/// Sections before the rest are merged into "Other" (Graphify's default `--max-sections`).
pub const DEFAULT_MAX_SECTIONS: i64 = 15;

const TEMPLATE: &str = include_str!("callflow.html");
const MERMAID_SCRIPT: &str = concat!(
    r#"<script src="https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.min.js" "#,
    r#"integrity="sha384-rbtjAdnIQE/aQJGEgXrVUlMibdfTSa4PQju4HDhN3sR2PmaKFzhEafuePsl9H/9I" crossorigin="anonymous"></script>"#
);

/// Diagram limits (§13.5).
const MAX_DIAGRAM_NODES: usize = 18;
const MAX_DIAGRAM_EDGES: usize = 24;
const MAX_OVERVIEW_EDGES: usize = 12;
const MAX_TABLE_ROWS: usize = 30;
const MAX_LIST_ITEMS: usize = 3;
const MAX_KEY_FILES: usize = 8;
const MAX_CHAIN: usize = 7;
const ORDER_TOP: usize = 6;
const HIGH_CONFIDENCE: f64 = 0.8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhCn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangChoice {
    Auto,
    Fixed(Lang),
}

#[derive(Debug, Clone)]
pub struct CallflowOptions {
    /// The project name (title, file name).
    pub project: String,
    pub lang: LangChoice,
    /// Graphify's `--max-sections`: 0 means the default, values below 2 mean 4.
    pub max_sections: i64,
    /// Scales the diagrams' font size (Graphify's `--diagram-scale`).
    pub diagram_scale: f64,
    /// `built_at_commit` of the graph, shortened to 7 characters on the page.
    pub commit: Option<String>,
    /// Number of hyperedges in the graph.
    pub hyperedges: usize,
    /// The "Generated:" stamp (the only run-dependent text on the page).
    pub generated: String,
}

impl Default for CallflowOptions {
    fn default() -> Self {
        CallflowOptions {
            project: String::new(),
            lang: LangChoice::Auto,
            max_sections: DEFAULT_MAX_SECTIONS,
            diagram_scale: 1.0,
            commit: None,
            hyperedges: 0,
            generated: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Categories, tokens and names (§13.2)

struct Category {
    slug: &'static str,
    en: &'static str,
    zh: &'static str,
    keywords: &'static [&'static str],
}

/// In tie-break priority order.
const CATEGORIES: &[Category] = &[
    Category {
        slug: "extract-pipeline",
        en: "Extraction Pipeline",
        zh: "提取管线",
        keywords: &[
            "ast",
            "calls",
            "extract",
            "extractor",
            "go",
            "imports",
            "java",
            "javascript",
            "language",
            "multilang",
            "parser",
            "python",
            "rust",
            "sitter",
            "tree",
            "typescript",
        ],
    },
    Category {
        slug: "build-graph",
        en: "Graph Build",
        zh: "图谱构建",
        keywords: &[
            "build",
            "confidence",
            "dedup",
            "edge",
            "graph",
            "hyperedge",
            "json",
            "merge",
            "node",
            "normalize",
            "schema",
        ],
    },
    Category {
        slug: "analysis-clustering",
        en: "Analysis & Clustering",
        zh: "分析聚类",
        keywords: &[
            "analyze",
            "benchmark",
            "cluster",
            "cohesion",
            "community",
            "explain",
            "god",
            "leiden",
            "path",
            "query",
            "question",
            "surprise",
        ],
    },
    Category {
        slug: "outputs-docs",
        en: "Outputs & Docs",
        zh: "输出文档",
        keywords: &[
            "callflow",
            "canvas",
            "documentation",
            "export",
            "graphml",
            "html",
            "mermaid",
            "obsidian",
            "report",
            "svg",
            "tree",
            "wiki",
        ],
    },
    Category {
        slug: "cli-skills",
        en: "CLI & Skill Installers",
        zh: "CLI 与技能安装",
        keywords: &[
            "agent",
            "aider",
            "claude",
            "codex",
            "command",
            "copilot",
            "hook",
            "install",
            "kiro",
            "main",
            "opencode",
            "skill",
            "uninstall",
            "vscode",
        ],
    },
    Category {
        slug: "ingest-cache-update",
        en: "Ingestion & Updates",
        zh: "摄取与增量更新",
        keywords: &[
            "audio",
            "cache",
            "download",
            "fetch",
            "google",
            "html",
            "incremental",
            "ingest",
            "manifest",
            "markdown",
            "transcribe",
            "update",
            "url",
            "video",
            "watch",
        ],
    },
    Category {
        slug: "serve-api",
        en: "Serving API",
        zh: "服务 API",
        keywords: &[
            "api", "delete", "endpoint", "enrich", "handle", "request", "response", "router",
            "search", "serve", "upload",
        ],
    },
    Category {
        slug: "security-global",
        en: "Security & Global Graph",
        zh: "安全与全局图",
        keywords: &[
            "clone",
            "global",
            "path",
            "prefix",
            "prune",
            "repo",
            "safe",
            "security",
            "traversal",
            "xss",
        ],
    },
    Category {
        slug: "tests-fixtures",
        en: "Tests & Fixtures",
        zh: "测试与样例",
        keywords: &[
            "assert", "fixture", "fixtures", "mock", "pytest", "sample", "test", "tests",
        ],
    },
];

/// Tokens never used in names.
const STOPWORDS: &[&str] = &[
    "and", "class", "core", "css", "file", "for", "from", "function", "html", "index", "init",
    "java", "jsx", "lib", "main", "method", "src", "that", "the", "this", "tsx", "with",
];

/// Node fields whose words count towards a category.
const CATEGORY_FIELDS: &[&str] = &[
    "label",
    "source_file",
    "file_type",
    "kind",
    "type",
    "category",
    "node_type",
];

/// A counter that remembers first-insertion order (Python's `Counter.most_common` ties).
#[derive(Default)]
struct Tally {
    keys: Vec<String>,
    counts: Vec<usize>,
    index: HashMap<String, usize>,
}

impl Tally {
    fn add(&mut self, key: &str) {
        match self.index.get(key) {
            Some(&k) => self.counts[k] += 1,
            None => {
                self.index.insert(key.to_string(), self.keys.len());
                self.keys.push(key.to_string());
                self.counts.push(1);
            }
        }
    }

    fn most_common(&self, n: usize) -> Vec<(&str, usize)> {
        let mut order: Vec<usize> = (0..self.keys.len()).collect();
        order.sort_by_key(|&k| std::cmp::Reverse(self.counts[k]));
        order
            .into_iter()
            .take(n)
            .map(|k| (self.keys[k].as_str(), self.counts[k]))
            .collect()
    }
}

/// Name tokens: split on whitespace, `/`, `_` and `-`; keep `[a-z0-9]` of the lowercased part;
/// at least 3 characters and not a stopword.
fn name_tokens(text: &str, out: &mut Vec<String>) {
    for part in text.split(|c: char| c.is_whitespace() || c == '/' || c == '_' || c == '-') {
        let t: String = part
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            .collect();
        if t.len() >= 3 && !STOPWORDS.contains(&t.as_str()) {
            out.push(t);
        }
    }
}

/// Words for category matching: runs of `[a-z0-9]` in the lowercased text.
fn sub_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter()
}

/// Python's `str.title()` for ASCII tokens: upper-case a letter that follows a non-letter.
fn title_case(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    let mut prev_letter = false;
    for c in token.chars() {
        if c.is_alphabetic() {
            if prev_letter {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            prev_letter = true;
        } else {
            out.push(c);
            prev_letter = false;
        }
    }
    out
}

fn sha1_hex(text: &str) -> String {
    sha1_smol::Sha1::from(text).digest().to_string()
}

/// A Mermaid id: the text with every run of characters outside `[A-Za-z0-9]` as one `_`
/// (none at either end), at most
/// 48 characters, `node_` in front of a leading digit, then `_` and the first 8 hex digits of
/// the text's SHA-1.
fn mermaid_id(text: &str) -> String {
    let mut s = String::new();
    let mut gap = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !s.is_empty() {
                s.push('_');
            }
            gap = false;
            s.push(c);
        } else {
            gap = true;
        }
    }
    let mut s: String = s.chars().take(48).collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert_str(0, "node_");
    }
    format!("{s}_{}", &sha1_hex(text)[..8])
}

/// The last three components of a path.
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let start = parts.len().saturating_sub(3);
    parts[start..].join("/")
}

/// A label shortened like Graphify's: over 40 characters becomes the first 39 (trailing space
/// dropped) and `...`.
fn truncate(label: &str) -> String {
    if label.chars().count() > 40 {
        let head: String = label.chars().take(39).collect();
        format!("{}...", head.trim_end())
    } else {
        label.to_string()
    }
}

/// A label as Graphify shows it in lists and diagrams: without a leading `.`.
fn display_label(label: &str) -> &str {
    label.strip_prefix('.').unwrap_or(label)
}

/// Graphify's Mermaid label clean-up (`"` → `'`; `{`, `}`, `#`, `;`, `` ` ``, `|` removed;
/// whitespace collapsed), then dijkstral's: `<` and `>` as Mermaid entity codes so no
/// label reaches the page as markup (deliberate difference, §13.6).
fn mermaid_text(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => cleaned.push('\''),
            '{' | '}' | '#' | ';' | '`' | '|' => {}
            c if c.is_whitespace() => cleaned.push(' '),
            c => cleaned.push(c),
        }
    }
    let collapsed = cleaned
        .split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = String::with_capacity(collapsed.len());
    for c in collapsed.chars() {
        match c {
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            c => out.push(c),
        }
    }
    out
}

/// An edge label for `-->|…|`: as Graphify writes it when it is plain words, quoted otherwise so
/// brackets and other punctuation cannot end the label (Graphify's unquoted labels then fail to
/// parse; §13.6).
fn edge_label(text: &str) -> String {
    let plain = text
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.' | ','));
    if text.is_empty() {
        // `-->||` does not parse.
        "\" \"".to_string()
    } else if plain {
        text.to_string()
    } else {
        format!("\"{text}\"")
    }
}

/// `built_at_commit` and the hyperedge count, read from the end of the file where Graphify and
/// dijkstral write them, without parsing the graph twice.
pub fn graph_trailer(path: &Path) -> (Option<String>, usize) {
    let Ok(mut f) = File::open(path) else {
        return (None, 0);
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(1 << 20);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return (None, 0);
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return (None, 0);
    }
    let text = String::from_utf8_lossy(&buf);
    let commit = text.rfind("\"built_at_commit\"").and_then(|k| {
        let rest = text[k + 17..].trim_start().strip_prefix(':')?.trim_start();
        let rest = rest.strip_prefix('"')?;
        let end = rest.find('"')?;
        let c = &rest[..end];
        (!c.is_empty() && c.chars().all(|c| c.is_ascii_hexdigit())).then(|| c.to_string())
    });
    let hyper = text
        .rfind("\"hyperedges\"")
        .and_then(|k| {
            let rest = text[k + 12..].trim_start().strip_prefix(':')?.trim_start();
            let mut de = serde_json::Deserializer::from_str(rest).into_iter::<serde_json::Value>();
            de.next()?.ok()?.as_array().map(Vec::len)
        })
        .unwrap_or(0);
    (commit, hyper)
}

// ---------------------------------------------------------------------------------------------
// Text tables

struct Words {
    lang: Lang,
}

impl Words {
    fn t(&self, en: &'static str, zh: &'static str) -> &'static str {
        match self.lang {
            Lang::En => en,
            Lang::ZhCn => zh,
        }
    }

    fn relation(&self, rel: &str) -> String {
        let en = match rel {
            "imports_from" => "imports".to_string(),
            "rationale_for" => "explains".to_string(),
            "indirect_call" => "indirect call".to_string(),
            r => r.replace('_', " "),
        };
        let shown = if self.lang == Lang::ZhCn {
            match rel {
                "calls" => "调用".to_string(),
                "imports" | "imports_from" => "导入".to_string(),
                "method" => "方法".to_string(),
                "uses" => "使用".to_string(),
                "contains" => "包含".to_string(),
                "rationale_for" => "说明".to_string(),
                _ => en,
            }
        } else {
            en
        };
        shown.replace('"', "'").replace('|', " ")
    }
}

// ---------------------------------------------------------------------------------------------
// The page model

/// One entry of a caller/callee list.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A node with a file.
    Node(String, String),
    /// A node without a file.
    Bare(String),
    /// Plain text (`+3 more`, "no inbound edge").
    Text(String),
}

#[derive(Debug, Clone)]
pub struct CallRow {
    pub node: (String, String),
    pub kind: String,
    pub callers: Vec<Item>,
    pub callees: Vec<Item>,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub slug: String,
    pub title: String,
    pub communities: Vec<i64>,
    pub members: Vec<u32>,
    pub heading: String,
    pub intro: String,
    pub diagram: Vec<String>,
    pub calls: Vec<CallRow>,
    pub key_files: Vec<(String, usize)>,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub lang: Lang,
    pub title: String,
    pub subtitle: String,
    pub overview_diagram: Vec<String>,
    pub chain: String,
    pub sections: Vec<Section>,
    pub stats: Vec<(String, String)>,
    pub confidence: Vec<(String, String)>,
    pub footer: [String; 2],
    pub diagram_scale: f64,
}

// ---------------------------------------------------------------------------------------------
// Building

struct Ctx<'a> {
    g: &'a NxGraph,
    words: Words,
    /// Per node: out-links `(target, edge)` in link order, in the link's own direction.
    out: Vec<Vec<(u32, u32)>>,
    inn: Vec<Vec<(u32, u32)>>,
}

impl Ctx<'_> {
    fn label(&self, i: u32) -> &str {
        self.g.label(i)
    }

    fn file(&self, i: u32) -> &str {
        self.g.node_str(i, "source_file").unwrap_or("")
    }

    fn relation(&self, e: u32) -> &str {
        self.g.edge_str(e, "relation").unwrap_or("")
    }

    fn confidence(&self, e: u32) -> &str {
        self.g.edge_str(e, "confidence").unwrap_or("EXTRACTED")
    }

    fn score(&self, e: u32) -> f64 {
        self.g.edges[e as usize]
            .attrs
            .get("confidence_score")
            .and_then(Attr::as_f64)
            .unwrap_or(1.0)
    }

    fn high_confidence(&self, e: u32) -> bool {
        self.confidence(e) != "AMBIGUOUS" && self.score(e) >= HIGH_CONFIDENCE
    }

    fn is_module(&self, i: u32) -> bool {
        let f = self.file(i);
        !f.is_empty() && f.rsplit('/').next() == Some(self.label(i))
    }
}

/// Whether the page should be Chinese under `--lang auto`: some label holds a CJK ideograph.
pub fn detect_lang(g: &NxGraph) -> Lang {
    let cjk = (0..g.nodes.len() as u32).any(|i| {
        g.label(i)
            .chars()
            .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
    });
    if cjk {
        Lang::ZhCn
    } else {
        Lang::En
    }
}

fn classify(ctx: &Ctx, members: &[u32]) -> (Option<usize>, Tally) {
    let mut text_hits: HashMap<String, usize> = HashMap::new();
    let mut names = Tally::default();
    let mut buf = Vec::new();
    for &i in members {
        buf.clear();
        name_tokens(ctx.label(i), &mut buf);
        name_tokens(ctx.file(i), &mut buf);
        for t in &buf {
            names.add(t);
        }
        for f in CATEGORY_FIELDS {
            let v = if *f == "label" {
                Some(ctx.label(i))
            } else {
                ctx.g.node_str(i, f)
            };
            if let Some(v) = v {
                for t in sub_tokens(v) {
                    *text_hits.entry(t).or_insert(0) += 1;
                }
            }
        }
    }
    let top: Vec<&str> = names.most_common(3).into_iter().map(|(t, _)| t).collect();
    let mut best: Option<(usize, usize)> = None;
    for (k, cat) in CATEGORIES.iter().enumerate() {
        let score: usize = cat
            .keywords
            .iter()
            .map(|kw| text_hits.get(*kw).copied().unwrap_or(0))
            .sum::<usize>()
            + top.iter().filter(|t| cat.keywords.contains(t)).count();
        if score >= 2 && best.is_none_or(|(s, _)| score > s) {
            best = Some((score, k));
        }
    }
    (best.map(|(_, k)| k), names)
}

fn plan_sections(ctx: &Ctx, max_sections: i64) -> Vec<Section> {
    let mut comms = ctx.g.communities();
    comms.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then_with(|| a.0.to_string().cmp(&b.0.to_string()))
    });
    let mut by_cat: Vec<Option<Section>> = (0..CATEGORIES.len()).map(|_| None).collect();
    let mut generated = Vec::new();
    for (c, members) in comms {
        let (cat, names) = classify(ctx, &members);
        match cat {
            Some(k) => {
                let s = by_cat[k].get_or_insert_with(|| {
                    let cat = &CATEGORIES[k];
                    new_section(
                        cat.slug.to_string(),
                        ctx.words.t(cat.en, cat.zh).to_string(),
                    )
                });
                s.communities.push(c);
                s.members.extend(members);
            }
            None => {
                let toks: Vec<&str> = names.most_common(3).into_iter().map(|(t, _)| t).collect();
                let (slug, title) = if toks.is_empty() {
                    (format!("community-{c}"), format!("Community {c}"))
                } else {
                    (
                        toks.join("-"),
                        toks.iter()
                            .map(|t| title_case(t))
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                };
                let mut s = new_section(slug, title);
                s.communities.push(c);
                s.members = members;
                generated.push(s);
            }
        }
    }
    let mut secs: Vec<Section> = by_cat.into_iter().flatten().collect();
    secs.extend(generated);
    let mut seen = HashSet::new();
    for s in &mut secs {
        if !seen.insert(s.slug.clone()) {
            s.slug = format!("{}-{}", s.slug, &sha1_hex(&s.title)[..6]);
            seen.insert(s.slug.clone());
        }
    }
    let limit = match max_sections {
        0 => DEFAULT_MAX_SECTIONS,
        n if n < 2 => 4,
        n => n,
    } as usize;
    if secs.len() > limit {
        let rest = secs.split_off(limit - 1);
        let mut other = new_section("other".into(), ctx.words.t("Other", "其他").into());
        for s in rest {
            other.communities.extend(s.communities);
            other.members.extend(s.members);
        }
        secs.push(other);
    }
    secs
}

fn new_section(slug: String, title: String) -> Section {
    Section {
        slug,
        title,
        communities: Vec::new(),
        members: Vec::new(),
        heading: String::new(),
        intro: String::new(),
        diagram: Vec::new(),
        calls: Vec::new(),
        key_files: Vec::new(),
        note: String::new(),
    }
}

/// Node kind: `(table type, diagram class)` (§13.7).
fn kind(ctx: &Ctx, i: u32) -> (&'static str, &'static str) {
    let label = ctx.label(i).to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| label.contains(w));
    if ctx.g.node_str(i, "file_type") == Some("rationale") {
        return ("Concept", "concept");
    }
    if ctx.file(i).to_lowercase().contains("test") {
        return ("Test", "test");
    }
    if has(&["api", "route", "endpoint"]) {
        return ("API", "api");
    }
    if has(&["client", "command", "cli", "click", "typer"]) {
        return ("Entry", "entry");
    }
    if has(&["props", "store", "component", "hook"]) {
        return ("UI", "ui");
    }
    if has(&["async", "await", "stream", "sse"]) {
        return ("Async", "async");
    }
    let ext = file_ext(ctx, i);
    let src_ext = ctx.file(i).rsplit_once('.').map(|(_, e)| e);
    if matches!(src_ext, Some("tsx" | "jsx")) {
        return ("UI", "ui");
    }
    let attrs = &ctx.g.nodes[i as usize].attrs;
    let flag = |k: &str| attrs.get(k).and_then(Attr::as_bool).unwrap_or(false);
    let concept = ctx.g.node_str(i, "file_type") == Some("concept");
    if flag("_callable_class")
        || (!flag("_callable")
            && !concept
            && !ctx.label(i).ends_with("()")
            && ctx.label(i).starts_with(|c: char| c.is_uppercase()))
    {
        return ("Class", "klass");
    }
    if ext.is_some_and(|e| MODULE_EXTENSIONS.contains(&e)) {
        return ("Module", "module");
    }
    ("Function", "function")
}

/// File nodes with these extensions are modules (§13.7).
const MODULE_EXTENSIONS: &[&str] = &[
    "py", "ts", "js", "go", "rs", "rb", "php", "kt", "swift", "toml", "mod",
];
/// File nodes with these extensions are described as module files.
const MODULE_FILE_EXTENSIONS: &[&str] = &["py", "ts", "tsx"];

/// The extension of a file node's label (`None` for other nodes).
fn file_ext<'a>(ctx: &'a Ctx, i: u32) -> Option<&'a str> {
    if !ctx.is_module(i) {
        return None;
    }
    let label = ctx.label(i);
    label.rsplit_once('.').map(|(_, e)| e)
}

fn kind_word(words: &Words, kind: &str) -> String {
    let zh = match kind {
        "Concept" => "概念",
        "Test" => "测试",
        "API" => "API",
        "Entry" => "入口",
        "UI" => "UI",
        "Async" => "异步",
        "Module" => "模块",
        "Class" => "类",
        _ => "函数",
    };
    match words.lang {
        Lang::En => kind.to_string(),
        Lang::ZhCn => zh.to_string(),
    }
}

/// Canned descriptions by label keyword, in priority order: `(substrings, en, zh)`.
const DESCRIPTIONS: &[(&[&str], &str, &str)] = &[
    (
        &["analy"],
        "Orchestrates analysis and returns structured documentation data.",
        "编排分析流程并产出结构化文档数据。",
    ),
    (
        &["graph", "dependency"],
        "Builds dependency relationships and graph data.",
        "构建依赖关系并提供排序或图形化数据。",
    ),
    (
        &["prompt"],
        "Builds structured prompts for model calls.",
        "构造发送给 LLM 的结构化提示。",
    ),
    (
        &["config"],
        "Reads, resolves, or persists project configuration.",
        "读取、解析或持久化项目配置。",
    ),
    (
        &["cache", "hash"],
        "Caches analysis results or computes cache keys.",
        "缓存分析结果或生成缓存键。",
    ),
    (
        &["clone", "git", "ingest"],
        "Turns a local path or remote repository into analysis context.",
        "把本地目录或远程仓库转换为分析上下文。",
    ),
    (
        &["chat", "rag", "retrieve", "useragent"],
        "Supports retrieval-augmented Q&A or streaming chat.",
        "支撑检索增强问答或流式聊天。",
    ),
    (
        &["page", "sidebar", "wiki"],
        "Organizes documentation pages, navigation, or content lookup.",
        "组织文档页面、侧边栏或内容读取。",
    ),
    (
        &["export", "html", "markdown"],
        "Exports documentation data to a target format.",
        "将文档数据导出为目标格式。",
    ),
    (
        &["scan"],
        "Starts scanning or handles scan status.",
        "触发项目扫描或处理扫描状态。",
    ),
    (
        &["test"],
        "Verifies imports, entry points, or version behavior.",
        "验证导入、入口点或版本等基础行为。",
    ),
];

fn description(ctx: &Ctx, i: u32, kind: &str) -> String {
    let label = ctx.label(i);
    let file = short_path(ctx.file(i));
    let lang = ctx.words.lang;
    if kind == "Concept" {
        return match lang {
            Lang::En => format!("Design note for {label}."),
            Lang::ZhCn => format!("设计说明：{label}"),
        };
    }
    if file_ext(ctx, i).is_some_and(|e| MODULE_FILE_EXTENSIONS.contains(&e)) {
        return match lang {
            Lang::En => format!("Module file in {file}."),
            Lang::ZhCn => format!("{file} 中的模块文件，承载该层主要实现。"),
        };
    }
    let lower = label.to_lowercase();
    for (keys, en, zh) in DESCRIPTIONS {
        if keys.iter().any(|k| lower.contains(k)) {
            return ctx.words.t(en, zh).to_string();
        }
    }
    match lang {
        Lang::En => {
            let at = if file.is_empty() {
                "project".into()
            } else {
                file
            };
            format!("{label} node in {at}.")
        }
        Lang::ZhCn => {
            let at = if file.is_empty() {
                "项目".into()
            } else {
                file
            };
            format!("{at} 中的 {label} 节点。")
        }
    }
}

/// Relations that make a caller or callee (§13.8).
const CALL_RELATIONS: &[&str] = &[
    "calls",
    "imports",
    "imports_from",
    "uses",
    "method",
    "indirect_call",
];
/// Relations a diagram prefers (§13.5).
const DIAGRAM_RELATIONS: &[&str] = &["calls", "uses", "method", "imports", "imports_from"];

fn relation_rank(rel: &str) -> u8 {
    match rel {
        "method" => 0,
        "calls" | "uses" => 1,
        "imports" | "imports_from" => 2,
        _ => 3,
    }
}

/// Which relation names an overview link (lower first).
fn overview_rank(rel: &str) -> u8 {
    match rel {
        "inherits" => 0,
        "method" => 1,
        "uses" => 2,
        "calls" => 3,
        "imports" | "imports_from" => 4,
        "contains" => 6,
        _ => 5,
    }
}

fn confidence_rank(conf: &str) -> u8 {
    match conf {
        "EXTRACTED" => 0,
        "INFERRED" => 1,
        _ => 2,
    }
}

fn list_items(ctx: &Ctx, mut nodes: Vec<u32>, empty: &str) -> Vec<Item> {
    if nodes.is_empty() {
        return vec![Item::Text(empty.to_string())];
    }
    let mut seen = HashSet::new();
    nodes.retain(|n| seen.insert(*n));
    nodes.sort_by_cached_key(|&n| display_label(ctx.label(n)).to_lowercase());
    let total = nodes.len();
    let mut out: Vec<Item> = nodes
        .into_iter()
        .take(MAX_LIST_ITEMS)
        .map(|n| {
            let label = truncate(display_label(ctx.label(n)));
            let file = ctx.file(n);
            if file.is_empty() {
                Item::Bare(label)
            } else {
                Item::Node(label, short_path(file))
            }
        })
        .collect();
    if total > MAX_LIST_ITEMS {
        let more = total - MAX_LIST_ITEMS;
        out.push(Item::Text(match ctx.words.lang {
            Lang::En => format!("+{more} more"),
            Lang::ZhCn => format!("+{more} 个更多"),
        }));
    }
    out
}

fn call_rows(ctx: &Ctx, sec: &Section) -> Vec<CallRow> {
    let w = &ctx.words;
    sec.members
        .iter()
        .take(MAX_TABLE_ROWS)
        .map(|&i| {
            let (k, _) = kind(ctx, i);
            let callers: Vec<u32> = ctx.inn[i as usize]
                .iter()
                .filter(|&&(_, e)| CALL_RELATIONS.contains(&ctx.relation(e)))
                .map(|&(s, _)| s)
                .collect();
            let callees: Vec<u32> = ctx.out[i as usize]
                .iter()
                .filter(|&&(_, e)| CALL_RELATIONS.contains(&ctx.relation(e)))
                .map(|&(t, _)| t)
                .collect();
            CallRow {
                node: (ctx.label(i).to_string(), short_path(ctx.file(i))),
                kind: kind_word(w, k),
                callers: list_items(
                    ctx,
                    callers,
                    w.t("External entry / no inbound edge", "外部入口 / 无直接入边"),
                ),
                callees: list_items(ctx, callees, w.t("No direct outbound edge", "无直接出边")),
                description: description(ctx, i, k),
            }
        })
        .collect()
}

/// Internal links of a section in Graphify's enumeration order: members in section order, each
/// member's out-links in link order.
fn internal_links(ctx: &Ctx, members: &[u32], inside: &HashSet<u32>) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    for &m in members {
        for &(t, e) in &ctx.out[m as usize] {
            if inside.contains(&t) {
                out.push((m, t, e));
            }
        }
    }
    out
}

fn section_diagram(ctx: &Ctx, sec: &Section, links: &[(u32, u32, u32)]) -> Vec<String> {
    let w = &ctx.words;
    // Candidate edges: preferred relations if any, then confidence, relation, score, enumeration.
    let preferred: Vec<(u32, u32, u32)> = links
        .iter()
        .copied()
        .filter(|&(_, _, e)| DIAGRAM_RELATIONS.contains(&ctx.relation(e)) && ctx.high_confidence(e))
        .collect();
    let mut cand = if preferred.is_empty() {
        links.to_vec()
    } else {
        preferred
    };
    let pos: HashMap<u32, usize> = links.iter().enumerate().map(|(k, l)| (l.2, k)).collect();
    cand.sort_by(|a, b| {
        let key = |&(_, _, e): &(u32, u32, u32)| {
            (
                confidence_rank(ctx.confidence(e)),
                relation_rank(ctx.relation(e)),
            )
        };
        key(a)
            .cmp(&key(b))
            .then_with(|| ctx.score(b.2).total_cmp(&ctx.score(a.2)))
            .then_with(|| pos[&a.2].cmp(&pos[&b.2]))
    });
    // Nodes: every member up to the limit, else the endpoints of the best edges first.
    let selected: Vec<u32> = if sec.members.len() <= MAX_DIAGRAM_NODES {
        sec.members.clone()
    } else {
        let mut sel = Vec::new();
        let mut have = HashSet::new();
        for &(a, b, _) in &cand {
            for n in [a, b] {
                if sel.len() < MAX_DIAGRAM_NODES && have.insert(n) {
                    sel.push(n);
                }
            }
        }
        let mut rest: Vec<u32> = sec
            .members
            .iter()
            .copied()
            .filter(|n| !have.contains(n))
            .collect();
        rest.sort_by_key(|&n| std::cmp::Reverse(ctx.g.degree(n)));
        for n in rest {
            if sel.len() >= MAX_DIAGRAM_NODES {
                break;
            }
            sel.push(n);
        }
        sel
    };
    let chosen: HashSet<u32> = selected.iter().copied().collect();
    let edges: Vec<(u32, u32, u32)> = cand
        .into_iter()
        .filter(|(a, b, _)| chosen.contains(a) && chosen.contains(b))
        .take(MAX_DIAGRAM_EDGES)
        .collect();
    let order = node_order(ctx, &selected, &edges);

    let mut lines = vec![
        "flowchart LR".to_string(),
        format!(
            "    %% Section: {} ({} nodes, {} edges)",
            mermaid_text(&sec.title),
            sec.members.len(),
            count_internal(ctx, &sec.members)
        ),
    ];
    // Groups: files with two or more nodes, and "External / generated" for file-less nodes.
    let external = "External / generated";
    let group_of = |n: u32| -> String {
        let f = ctx.file(n);
        if f.is_empty() {
            external.to_string()
        } else {
            short_path(f)
        }
    };
    let mut groups: Vec<(String, Vec<u32>)> = Vec::new();
    for &n in &order {
        let gname = group_of(n);
        match groups.iter_mut().find(|(g, _)| *g == gname) {
            Some((_, v)) => v.push(n),
            None => groups.push((gname, vec![n])),
        }
    }
    let single_group = groups.len() == 1;
    let mut boxes: Vec<&(String, Vec<u32>)> = groups
        .iter()
        .filter(|(_, v)| v.len() >= 2 && !single_group)
        .collect();
    boxes.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    let boxed: HashSet<u32> = boxes.iter().flat_map(|(_, v)| v.iter().copied()).collect();
    for (gname, nodes) in &boxes {
        lines.push(format!(
            "    subgraph {}[\"{}\"]",
            mermaid_id(&format!("{}_{}", sec.slug, gname)),
            mermaid_text(gname)
        ));
        for &n in nodes {
            lines.push(format!("        {}", node_decl(ctx, n)));
        }
        lines.push("    end".to_string());
    }
    for &n in &order {
        if !boxed.contains(&n) {
            lines.push(format!("    {}", node_decl(ctx, n)));
        }
    }
    for &(a, b, e) in &edges {
        lines.push(format!(
            "    {} -->|{}| {}",
            mermaid_id(&ctx.g.nodes[a as usize].id),
            edge_label(&mermaid_text(&w.relation(ctx.relation(e)))),
            mermaid_id(&ctx.g.nodes[b as usize].id)
        ));
    }
    // Classes in declaration order.
    let mut declared: Vec<u32> = boxes.iter().flat_map(|(_, v)| v.iter().copied()).collect();
    declared.extend(order.iter().copied().filter(|n| !boxed.contains(n)));
    for n in declared {
        lines.push(format!(
            "    class {} {};",
            mermaid_id(&ctx.g.nodes[n as usize].id),
            kind(ctx, n).1
        ));
    }
    lines
}

fn node_decl(ctx: &Ctx, n: u32) -> String {
    let label = mermaid_text(&truncate(display_label(ctx.label(n))));
    let id = mermaid_id(&ctx.g.nodes[n as usize].id);
    let file = ctx.file(n);
    if file.is_empty() || ctx.is_module(n) {
        format!("{id}(\"{label}\")")
    } else {
        format!(
            "{id}(\"{label}<br/><small>{}</small>\")",
            mermaid_text(&short_path(file))
        )
    }
}

/// Diagram node order (§13.5): the six strongest sources (out-degree minus in-degree, then
/// out-degree, then id), the other linked nodes as they appear in the edge list, then unlinked
/// nodes by label.
fn node_order(ctx: &Ctx, selected: &[u32], edges: &[(u32, u32, u32)]) -> Vec<u32> {
    let mut out_d: HashMap<u32, i64> = HashMap::new();
    let mut in_d: HashMap<u32, i64> = HashMap::new();
    let mut appear: Vec<u32> = Vec::new();
    let mut seen = HashSet::new();
    let mut sources: Vec<u32> = Vec::new();
    let mut src_seen = HashSet::new();
    for &(a, b, _) in edges {
        *out_d.entry(a).or_insert(0) += 1;
        *in_d.entry(b).or_insert(0) += 1;
        for n in [a, b] {
            if seen.insert(n) {
                appear.push(n);
            }
        }
        if src_seen.insert(a) {
            sources.push(a);
        }
    }
    let od = |n: u32| out_d.get(&n).copied().unwrap_or(0);
    let id_ = |n: u32| ctx.g.nodes[n as usize].id.as_str();
    sources.sort_by(|&a, &b| {
        let ka = (-(od(a) - in_d.get(&a).copied().unwrap_or(0)), -od(a));
        let kb = (-(od(b) - in_d.get(&b).copied().unwrap_or(0)), -od(b));
        ka.cmp(&kb).then_with(|| id_(a).cmp(id_(b)))
    });
    let top: Vec<u32> = sources.into_iter().take(ORDER_TOP).collect();
    let top_set: HashSet<u32> = top.iter().copied().collect();
    let mut order = top;
    order.extend(appear.iter().copied().filter(|n| !top_set.contains(n)));
    let mut isolated: Vec<u32> = selected
        .iter()
        .copied()
        .filter(|n| !seen.contains(n))
        .collect();
    isolated.sort_by_cached_key(|&n| {
        (
            ctx.g.node_str(n, "file_type") == Some("rationale"),
            display_label(ctx.label(n)).to_string(),
        )
    });
    order.extend(isolated);
    order
}

fn count_internal(ctx: &Ctx, members: &[u32]) -> usize {
    let inside: HashSet<u32> = members.iter().copied().collect();
    ctx.g
        .edges
        .iter()
        .filter(|e| inside.contains(&e.source) && inside.contains(&e.target))
        .count()
}

fn intro(ctx: &Ctx, sec: &Section) -> String {
    let w = &ctx.words;
    let mut names = Tally::default();
    let mut buf = Vec::new();
    for &i in &sec.members {
        buf.clear();
        name_tokens(ctx.label(i), &mut buf);
        name_tokens(ctx.file(i), &mut buf);
        for t in &buf {
            names.add(t);
        }
    }
    let toks: Vec<&str> = names.most_common(4).into_iter().map(|(t, _)| t).collect();
    let files: Vec<String> = top_files(ctx, &sec.members, 3)
        .into_iter()
        .map(|(f, _)| f)
        .collect();
    let m = count_internal(ctx, &sec.members);
    let n = sec.members.len();
    match w.lang {
        Lang::En => format!(
            "{} groups implementation around {}, mostly in {}. This section covers {n} nodes and {m} internal edges; the diagram shows only representative relationships to stay readable.",
            sec.title,
            toks.join(", "),
            files.join(", ")
        ),
        Lang::ZhCn => format!(
            "{} 汇集了与 {} 相关的实现，主要分布在 {}。本节覆盖 {n} 个节点、{m} 条内部边，图中只展示最有代表性的调用关系以保持可读性。",
            sec.title,
            toks.join("、"),
            files.join("、")
        ),
    }
}

fn top_files(ctx: &Ctx, members: &[u32], n: usize) -> Vec<(String, usize)> {
    let mut files = Tally::default();
    for &i in members {
        let f = ctx.file(i);
        if !f.is_empty() {
            files.add(&short_path(f));
        }
    }
    files
        .most_common(n)
        .into_iter()
        .map(|(f, c)| (f.to_string(), c))
        .collect()
}

fn design_note(ctx: &Ctx, links: &[(u32, u32, u32)]) -> String {
    let w = &ctx.words;
    let mut rels = Tally::default();
    for &(_, _, e) in links {
        if ctx.high_confidence(e) {
            rels.add(ctx.relation(e));
        }
    }
    let summary = if rels.keys.is_empty() {
        w.t(
            "No high-confidence call edges detected",
            "未检测到高置信调用边",
        )
        .to_string()
    } else {
        rels.most_common(4)
            .into_iter()
            .map(|(r, n)| format!("{} x{n}", w.relation(r)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match w.lang {
        Lang::En => format!(
            "This section comes from dijkstral community clustering. Relationship summary: {summary}. The diagram prioritizes high-confidence calls or usage relationships; the table keeps the broader node inventory."
        ),
        Lang::ZhCn => format!(
            "本节由 dijkstral 社区聚类生成。关系概况：{summary}。图表优先展示高置信、跨节点调用或使用关系，完整节点清单位于表格中。"
        ),
    }
}

fn overview(ctx: &Ctx, secs: &[Section]) -> (Vec<String>, String) {
    let w = &ctx.words;
    let mut sec_of: HashMap<u32, usize> = HashMap::new();
    for (k, s) in secs.iter().enumerate() {
        for &m in &s.members {
            sec_of.insert(m, k);
        }
    }
    let ids: Vec<String> = secs
        .iter()
        .map(|s| mermaid_id(&s.slug).to_uppercase())
        .collect();
    let mut lines = vec!["flowchart LR".to_string()];
    for (k, s) in secs.iter().enumerate() {
        lines.push(format!(
            "    {}(\"{}<br/><small>{} nodes</small>\")",
            ids[k],
            mermaid_text(&s.title),
            s.members.len()
        ));
        lines.push(format!("    class {} module;", ids[k]));
    }
    // Cross-section links per ordered pair of sections, labelled with the pair's highest-ranked
    // relation; the busiest pairs first, ties by that rank (§13.4).
    let mut pairs: Vec<((usize, usize), usize, Vec<String>)> = Vec::new();
    let mut index: HashMap<(usize, usize), usize> = HashMap::new();
    let mut flow: HashMap<(usize, usize), usize> = HashMap::new();
    for e in &ctx.g.edges {
        let (Some(&a), Some(&b)) = (sec_of.get(&e.source), sec_of.get(&e.target)) else {
            continue;
        };
        if a == b {
            continue;
        }
        let rel = e.attrs.get("relation").and_then(Attr::as_str).unwrap_or("");
        let k = *index.entry((a, b)).or_insert_with(|| {
            pairs.push(((a, b), 0, Vec::new()));
            pairs.len() - 1
        });
        pairs[k].1 += 1;
        pairs[k].2.push(rel.to_string());
        *flow.entry((a, b)).or_insert(0) += 1;
    }
    if pairs.is_empty() {
        for k in 1..secs.len() {
            lines.push(format!("    {} -.-> {}", ids[k - 1], ids[k]));
        }
    }
    let label = |rels: &[String]| -> String {
        let mut count: HashMap<&str, usize> = HashMap::new();
        for r in rels {
            let key = if r == "imports_from" {
                "imports"
            } else {
                r.as_str()
            };
            *count.entry(key).or_insert(0) += 1;
        }
        count
            .into_iter()
            .min_by_key(|(r, n)| (std::cmp::Reverse(*n), overview_rank(r), r.to_string()))
            .map(|(r, _)| r.to_string())
            .unwrap_or_default()
    };
    let mut order: Vec<usize> = (0..pairs.len()).collect();
    order.sort_by_key(|&k| std::cmp::Reverse(pairs[k].1));
    for k in order.into_iter().take(MAX_OVERVIEW_EDGES) {
        let ((a, b), n, rels) = &pairs[k];
        lines.push(format!(
            "    {} -->|{}| {}",
            ids[*a],
            edge_label(&format!("{} x{n}", mermaid_text(&w.relation(&label(rels))))),
            ids[*b]
        ));
    }
    // Core flow: from the section that sends most, follow the strongest unvisited successor.
    let mut chain: Vec<usize> = Vec::new();
    let mut visited = vec![false; secs.len()];
    let out_total = |k: usize| -> usize {
        flow.iter()
            .filter(|((a, _), _)| *a == k)
            .map(|(_, n)| n)
            .sum()
    };
    while chain.len() < MAX_CHAIN.min(secs.len()) {
        let next = match chain.last() {
            Some(&cur) => (0..secs.len())
                .filter(|&b| !visited[b] && flow.get(&(cur, b)).copied().unwrap_or(0) > 0)
                .max_by(|&x, &y| flow[&(cur, x)].cmp(&flow[&(cur, y)]).then(y.cmp(&x))),
            None => None,
        };
        let next = next.or_else(|| {
            (0..secs.len())
                .filter(|&b| !visited[b])
                .max_by(|&x, &y| out_total(x).cmp(&out_total(y)).then(y.cmp(&x)))
        });
        let Some(n) = next else { break };
        visited[n] = true;
        chain.push(n);
    }
    let chain = chain
        .iter()
        .map(|&k| secs[k].title.clone())
        .collect::<Vec<_>>()
        .join(" -> ");
    (lines, chain)
}

/// Build the page.
pub fn build(g: &NxGraph, opts: &CallflowOptions) -> Page {
    let lang = match opts.lang {
        LangChoice::Auto => detect_lang(g),
        LangChoice::Fixed(l) => l,
    };
    let mut out: Vec<Vec<(u32, u32)>> = vec![Vec::new(); g.nodes.len()];
    let mut inn: Vec<Vec<(u32, u32)>> = vec![Vec::new(); g.nodes.len()];
    for (k, e) in g.edges.iter().enumerate() {
        out[e.source as usize].push((e.target, k as u32));
        inn[e.target as usize].push((e.source, k as u32));
    }
    let ctx = Ctx {
        g,
        words: Words { lang },
        out,
        inn,
    };
    let mut secs = plan_sections(&ctx, opts.max_sections);
    for (k, s) in secs.iter_mut().enumerate() {
        s.heading = format!("{}. {}", k + 2, s.title);
        let inside: HashSet<u32> = s.members.iter().copied().collect();
        let links = internal_links(&ctx, &s.members, &inside);
        s.intro = intro(&ctx, s);
        s.diagram = section_diagram(&ctx, s, &links);
        s.calls = call_rows(&ctx, s);
        let mut files = top_files(&ctx, &s.members, usize::MAX);
        files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        files.truncate(MAX_KEY_FILES);
        s.key_files = files;
        s.note = design_note(&ctx, &links);
    }
    let (overview_diagram, chain) = overview(&ctx, &secs);
    let communities: HashSet<i64> = (0..g.nodes.len() as u32)
        .filter_map(|i| g.community(i))
        .collect();
    let commit = opts
        .commit
        .as_deref()
        .map(|c| c.chars().take(7).collect::<String>())
        .unwrap_or_else(|| "unknown".into());
    let (n, e, c) = (g.nodes.len(), g.edges.len(), communities.len());
    let subtitle = match lang {
        Lang::En => format!(
            "Generated from dijkstral knowledge graph: {n} nodes, {e} edges, {c} communities. Commit: {commit}"
        ),
        Lang::ZhCn => format!("由 dijkstral 知识图谱生成：{n} 个节点、{e} 条边、{c} 个社区。Commit: {commit}"),
    };
    let mut conf = [0usize; 3];
    for k in 0..g.edges.len() as u32 {
        conf[confidence_rank(ctx.confidence(k)) as usize] += 1;
    }
    Page {
        lang,
        title: match lang {
            Lang::En => format!(
                "{} — Complete Call Flow & Architecture Documentation",
                opts.project
            ),
            Lang::ZhCn => format!("{} — 完整调用流程与架构文档", opts.project),
        },
        subtitle,
        overview_diagram,
        chain,
        stats: vec![
            ("Nodes".into(), n.to_string()),
            ("Edges".into(), e.to_string()),
            ("Hyperedges".into(), opts.hyperedges.to_string()),
            ("Communities".into(), c.to_string()),
            ("Documented Sections".into(), secs.len().to_string()),
        ],
        confidence: vec![
            ("EXTRACTED".into(), conf[0].to_string()),
            ("INFERRED".into(), conf[1].to_string()),
            ("AMBIGUOUS".into(), conf[2].to_string()),
        ],
        footer: [
            format!("{} — Architecture Documentation", opts.project),
            format!("Generated: {} · dijkstral callflow-html", opts.generated),
        ],
        sections: secs,
        diagram_scale: opts.diagram_scale,
    }
}

impl Page {
    fn words(&self) -> Words {
        Words { lang: self.lang }
    }

    /// The page's data in the shape `tests/fixtures/exports/<lang>/callflow/page.json` stores
    /// for Graphify (headings, prose, Mermaid text without styling lines, table cells).
    pub fn data(&self) -> Value {
        let w = self.words();
        let item = |i: &Item| match i {
            Item::Node(l, f) => json!([l, f]),
            Item::Bare(l) => json!([l]),
            Item::Text(t) => json!(t),
        };
        let mut nav = vec![json!([
            "overview",
            w.t("Architecture Overview", "架构总览")
        ])];
        nav.extend(self.sections.iter().map(|s| json!([s.slug, s.title])));
        let layers: Vec<Value> = std::iter::once(json!([
            w.t("Layer", "层"),
            w.t("Nodes", "节点"),
            w.t("Communities", "社区")
        ]))
        .chain(self.sections.iter().map(|s| {
            json!([
                s.title,
                s.members.len().to_string(),
                s.communities
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ])
        }))
        .collect();
        let mut sections = vec![json!({
            "id": "overview",
            "heading": format!("1. {}", w.t("Architecture Overview", "架构总览")),
            "diagrams": [self.overview_diagram],
            "h3": [],
            "h4": [w.t("Architecture Layers", "架构层次"), w.t("Core Flow", "核心数据流")],
            "paragraphs": [],
            "chain": [self.chain],
            "tables": [layers],
        })];
        for s in &self.sections {
            let mut calls = vec![json!([
                "#",
                w.t("Node", "节点"),
                w.t("Type", "类型"),
                w.t("Caller", "调用方"),
                w.t("Callees", "被调用/依赖"),
                w.t("Description", "说明")
            ])];
            for (k, r) in s.calls.iter().enumerate() {
                calls.push(json!([
                    (k + 1).to_string(),
                    [[r.node.0, r.node.1]],
                    r.kind,
                    r.callers.iter().map(item).collect::<Vec<_>>(),
                    r.callees.iter().map(item).collect::<Vec<_>>(),
                    r.description
                ]));
            }
            let mut files = vec![json!(["File", w.t("Coverage", "覆盖节点")])];
            files.extend(s.key_files.iter().map(|(f, n)| match w.lang {
                Lang::En => json!([f, format!("{n} nodes")]),
                Lang::ZhCn => json!([f, format!("{n} 个节点")]),
            }));
            sections.push(json!({
                "id": s.slug,
                "heading": s.heading,
                "diagrams": [s.diagram],
                "h3": [w.t("Call Details", "调用明细")],
                "h4": [w.t("Key Files", "关键文件"), w.t("Design Notes", "设计备注")],
                "paragraphs": [s.intro, s.note],
                "tables": [calls, files],
            }));
        }
        sections.push(json!({
            "id": "stats",
            "heading": "Project Statistics",
            "diagrams": [],
            "h3": [],
            "h4": ["Graph", "Edge Confidence"],
            "paragraphs": self.footer,
            "tables": [
                self.stats.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
                self.confidence.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
            ],
        }));
        json!({
            "title": self.title,
            "subtitle": self.subtitle,
            "nav": nav,
            "sections": sections,
        })
    }

    /// Mermaid source of one diagram, with dijkstral's own styling lines.
    fn mermaid_source(&self, lines: &[String]) -> String {
        let font = 15.0 * self.diagram_scale;
        let mut s = format!(
            "%%{{init: {{\"themeVariables\": {{\"fontSize\": \"{font:.1}px\"}}, \"flowchart\": {{\"htmlLabels\": true, \"curve\": \"basis\"}}}}}}%%\n"
        );
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
        for (name, style) in CLASS_STYLES {
            let _ = writeln!(s, "    classDef {name} {style};");
        }
        s
    }

    /// The whole HTML page.
    pub fn render(&self) -> String {
        let w = self.words();
        let mut body = String::new();
        let esc = |s: &str| {
            let mut o = String::new();
            escape_html(s, &mut o);
            o
        };
        let _ = write!(
            body,
            "<header class=\"top\"><h1>{}</h1><p class=\"sub\">{}</p></header>\n<nav class=\"toc\">",
            esc(&self.title),
            esc(&self.subtitle)
        );
        let _ = write!(
            body,
            "<a href=\"#overview\">{}</a>",
            esc(w.t("Architecture Overview", "架构总览"))
        );
        for s in &self.sections {
            let _ = write!(body, "<a href=\"#{}\">{}</a>", esc(&s.slug), esc(&s.title));
        }
        body.push_str("</nav>\n<main>\n");
        // Overview.
        let _ = writeln!(
            body,
            "<section class=\"block\" id=\"overview\"><h2>1. {}</h2>",
            esc(w.t("Architecture Overview", "架构总览"))
        );
        self.diagram_html(&mut body, &self.overview_diagram);
        let _ = write!(
            body,
            "<div class=\"cards\"><div class=\"card\"><h4>{}</h4><table><thead><tr><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>",
            esc(w.t("Architecture Layers", "架构层次")),
            esc(w.t("Layer", "层")),
            esc(w.t("Nodes", "节点")),
            esc(w.t("Communities", "社区"))
        );
        for s in &self.sections {
            let comms = s
                .communities
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(
                body,
                "<tr><td>{}</td><td>{}</td><td><code>{}</code></td></tr>",
                esc(&s.title),
                s.members.len(),
                esc(&comms)
            );
        }
        let _ = writeln!(
            body,
            "</tbody></table></div><div class=\"card\"><h4>{}</h4><p class=\"chain\">{}</p></div></div></section>",
            esc(w.t("Core Flow", "核心数据流")),
            esc(&self.chain)
        );
        for s in &self.sections {
            let _ = write!(
                body,
                "<section class=\"block\" id=\"{}\"><h2>{}</h2>\n<p class=\"lead\">{}</p>\n",
                esc(&s.slug),
                esc(&s.heading),
                esc(&s.intro)
            );
            self.diagram_html(&mut body, &s.diagram);
            let _ = write!(
                body,
                "<h3>{}</h3><div class=\"scroll\"><table class=\"calls\"><thead><tr><th>#</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>",
                esc(w.t("Call Details", "调用明细")),
                esc(w.t("Node", "节点")),
                esc(w.t("Type", "类型")),
                esc(w.t("Caller", "调用方")),
                esc(w.t("Callees", "被调用/依赖")),
                esc(w.t("Description", "说明"))
            );
            for (k, r) in s.calls.iter().enumerate() {
                let _ = write!(
                    body,
                    "<tr><td>{}</td><td>{}</td><td><span class=\"kind\">{}</span></td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    k + 1,
                    items_html(&[Item::Node(r.node.0.clone(), r.node.1.clone())]),
                    esc(&r.kind),
                    items_html(&r.callers),
                    items_html(&r.callees),
                    esc(&r.description)
                );
            }
            let _ = write!(
                body,
                "</tbody></table></div>\n<div class=\"cards\"><div class=\"card\"><h4>{}</h4><table><thead><tr><th>File</th><th>{}</th></tr></thead><tbody>",
                esc(w.t("Key Files", "关键文件")),
                esc(w.t("Coverage", "覆盖节点"))
            );
            for (f, n) in &s.key_files {
                let cov = match w.lang {
                    Lang::En => format!("{n} nodes"),
                    Lang::ZhCn => format!("{n} 个节点"),
                };
                let _ = write!(
                    body,
                    "<tr><td><code>{}</code></td><td>{}</td></tr>",
                    esc(f),
                    esc(&cov)
                );
            }
            let _ = writeln!(
                body,
                "</tbody></table></div><div class=\"card\"><h4>{}</h4><p>{}</p></div></div></section>",
                esc(w.t("Design Notes", "设计备注")),
                esc(&s.note)
            );
        }
        body.push_str("<section class=\"block\" id=\"stats\"><h2>Project Statistics</h2><div class=\"cards\">");
        for (h, rows) in [
            ("Graph", &self.stats),
            ("Edge Confidence", &self.confidence),
        ] {
            let _ = write!(body, "<div class=\"card\"><h4>{h}</h4><table><tbody>");
            for (a, b) in rows.iter() {
                let _ = write!(body, "<tr><td>{}</td><td>{}</td></tr>", esc(a), esc(b));
            }
            body.push_str("</tbody></table></div>");
        }
        let _ = write!(
            body,
            "</div></section>\n</main>\n<footer><p>{}</p><p>{}</p></footer>",
            esc(&self.footer[0]),
            esc(&self.footer[1])
        );
        let lang = match self.lang {
            Lang::En => "en",
            Lang::ZhCn => "zh-CN",
        };
        fill(
            TEMPLATE,
            &[
                ("@@LANG@@", lang),
                ("@@TITLE@@", &esc(&self.title)),
                ("@@SCRIPTS@@", MERMAID_SCRIPT),
                ("@@BODY@@", &body),
            ],
        )
    }

    fn diagram_html(&self, out: &mut String, lines: &[String]) {
        out.push_str("<figure class=\"diagram\"><div class=\"tools\"><button type=\"button\" data-zoom=\"-1\" aria-label=\"Zoom out\">−</button><button type=\"button\" data-zoom=\"0\" aria-label=\"Reset zoom\">100%</button><button type=\"button\" data-zoom=\"1\" aria-label=\"Zoom in\">+</button></div><pre class=\"mermaid\">");
        escape_html(&self.mermaid_source(lines), out);
        out.push_str("</pre></figure>\n");
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        fs::write(path, self.render()).with_context(|| format!("writing {}", path.display()))
    }
}

fn items_html(items: &[Item]) -> String {
    let mut o = String::new();
    for (k, i) in items.iter().enumerate() {
        if k > 0 {
            o.push_str("<br>");
        }
        match i {
            Item::Node(l, f) => {
                o.push_str("<code>");
                escape_html(l, &mut o);
                o.push_str("</code><br><small>");
                escape_html(f, &mut o);
                o.push_str("</small>");
            }
            Item::Bare(l) => {
                o.push_str("<code>");
                escape_html(l, &mut o);
                o.push_str("</code>");
            }
            Item::Text(t) => escape_html(t, &mut o),
        }
    }
    o
}

const CLASS_STYLES: &[(&str, &str)] = &[
    ("entry", "fill:#fff4d6,stroke:#b7791f,color:#3d2a00"),
    ("api", "fill:#fde2e1,stroke:#c53030,color:#3b0d0c"),
    ("async", "fill:#ece4ff,stroke:#6b46c1,color:#22104d"),
    ("klass", "fill:#dcf5e8,stroke:#2f855a,color:#0c2e1c"),
    ("ui", "fill:#ffe1ef,stroke:#b83280,color:#3d0a24"),
    ("module", "fill:#dde8ff,stroke:#2b6cb0,color:#0c2240"),
    ("test", "fill:#ececec,stroke:#6b6b6b,color:#1d1d1d"),
    (
        "concept",
        "fill:#f3efe9,stroke:#8a7f72,color:#2b2620,stroke-dasharray:4 3",
    ),
    ("function", "fill:#e3f3fb,stroke:#2c7a9b,color:#0b2733"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_names_follow_graphify() {
        assert_eq!(mermaid_id("pkg_service_run"), "pkg_service_run_677d7a66");
        assert_eq!(
            mermaid_id("cli-skills").to_uppercase(),
            "CLI_SKILLS_9964CBB8"
        );
        assert_eq!(
            mermaid_id("tests-fixtures_External / generated"),
            "tests_fixtures_External_generated_473753cb"
        );
        assert!(mermaid_id("9g").starts_with("node_9g_"));
        assert_eq!(title_case("qq15q"), "Qq15Q");
        assert_eq!(title_case("neo4j"), "Neo4J");
        let mut t = Vec::new();
        name_tokens("pkg/service.py", &mut t);
        assert_eq!(t, ["pkg", "servicepy"]);
        assert_eq!(truncate(&"a".repeat(60)), format!("{}...", "a".repeat(39)));
        assert_eq!(short_path("src/main/java/com/A.java"), "java/com/A.java");
    }

    #[test]
    fn mermaid_labels_are_inert() {
        assert_eq!(
            mermaid_text("he said \"hi\" {z} <b>&</b> #1; `x` | y"),
            "he said 'hi' z #lt;b#gt;&#lt;/b#gt; 1 x y"
        );
    }
}

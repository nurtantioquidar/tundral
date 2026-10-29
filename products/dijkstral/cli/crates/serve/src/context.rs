//! `context`: the evidence for a question in one answer (dijkstral's own command,
//! `docs/dijkstral-context.md`). An agent's cost is set by its round trips, so where `query`
//! returns a neighbourhood to explore, `context` returns what the answer needs: each symbol the
//! question names (or, failing that, the best-matching nodes) with its definition lines, its
//! callers and users with their call-site lines, what it calls, and for documents the matching
//! lines of the cited files and the other files that mention the question's terms.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};

use dijkstral_core::index::Index;

use crate::query;
use crate::resolve::{self, Resolved};

/// Default output budget, in estimated tokens (4 characters each).
pub const DEFAULT_BUDGET: usize = 1500;
const MAX_ANCHORS: usize = 4;
const MAX_DOC_ANCHORS: usize = 8;
const DEF_LINES: usize = 4;
const MAX_CALLERS: usize = 12;
const MAX_CALLEES: usize = 10;
/// For a flow question, how many of an anchor's callers also list what else they call.
const FLOW_CALLERS: usize = 2;
const MAX_IMPORTERS: usize = 8;
const LINE_CHARS: usize = 160;
const MAX_FILE_BYTES: u64 = 2 << 20;

/// Relations that make the neighbour a caller or user of the symbol.
const USE_RELATIONS: &[&str] = &[
    "calls",
    "indirect_call",
    "uses",
    "references",
    "inherits",
    "extends",
    "implements",
    "mixes_in",
    "embeds",
    "instantiates",
];
const IMPORT_RELATIONS: &[&str] = &["imports", "imports_from", "re_exports", "dynamic_import"];

/// The repository a graph describes: the parent of its `dijkstral-out/` or `graphify-out/`.
pub fn root_of(graph: &Path, cwd: &Path) -> PathBuf {
    let dir = graph.parent().unwrap_or(Path::new(""));
    match dir.file_name().and_then(|n| n.to_str()) {
        Some("dijkstral-out" | "graphify-out") => dir
            .parent()
            .map(Path::to_path_buf)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| cwd.to_path_buf()),
        _ => cwd.to_path_buf(),
    }
}

/// Source files, read once each.
struct Sources {
    root: PathBuf,
    files: HashMap<String, Option<Vec<String>>>,
}

impl Sources {
    fn lines(&mut self, rel: &str) -> Option<&[String]> {
        if rel.is_empty() {
            return None;
        }
        let root = &self.root;
        self.files
            .entry(rel.to_string())
            .or_insert_with(|| {
                let path = root.join(rel);
                let meta = std::fs::metadata(&path).ok()?;
                if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
                    return None;
                }
                let bytes = std::fs::read(&path).ok()?;
                Some(
                    String::from_utf8_lossy(&bytes)
                        .lines()
                        .map(|l| clip(l.trim_end()))
                        .collect(),
                )
            })
            .as_deref()
    }

    /// Line `n` (1-based) of `rel`.
    fn line(&mut self, rel: &str, n: usize) -> Option<String> {
        self.lines(rel)?.get(n.checked_sub(1)?).cloned()
    }
}

/// Call-site code shown after a caller: the call itself is what matters, not the rest of the line.
const CALL_SITE_CHARS: usize = 80;

fn clip_to(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push('…');
        t
    }
}

fn clip(s: &str) -> String {
    if s.chars().count() <= LINE_CHARS {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(LINE_CHARS).collect();
        t.push('…');
        t
    }
}

/// `L586` → 586.
fn line_no(loc: &str) -> Option<usize> {
    loc.strip_prefix('L')?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn is_code(ix: &Index, i: u32) -> bool {
    ix.node(i).file_type == "code"
}

/// A file's own node (its label is the file's name).
fn is_file_node(ix: &Index, i: u32) -> bool {
    let file = ix.source_file(i);
    let label = ix.label(i);
    !file.is_empty() && !label.is_empty() && (file == label || file.ends_with(&format!("/{label}")))
}

/// A test: in a test file, or a node of an inline test module (Rust `mod tests`).
fn is_test_node(ix: &Index, i: u32) -> bool {
    is_test_path(ix.source_file(i))
        || ix.id(i).contains("_tests_")
        || ix.label(i).trim_start_matches('.').starts_with("test")
}

/// Tests, and code that is not the product itself (examples, documentation code, benchmarks):
/// listed after the code a question is about.
fn is_test_path(p: &str) -> bool {
    let p = p.to_ascii_lowercase();
    p.split('/').any(|seg| {
        seg == "test"
            || seg == "tests"
            || seg.starts_with("test_")
            || seg.ends_with("_test.go")
            || matches!(seg, "examples" | "example" | "docs_src" | "samples")
            || seg.starts_with("bench")
    }) || p.contains("_test.")
        || p.contains(".test.")
        || p.contains(".spec.")
}

/// The code-like names a question mentions: backticked spans, and words with `_`, `.`, `::`,
/// `()` or an inner capital (`solve_dependencies`, `Sink`, `Override::matched`).
fn named_symbols(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim_matches(|c: char| c == '`' || c == ',' || c == '?' || c == ':');
        let s = s.trim_end_matches('.');
        if s.len() >= 3 && !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    };
    let mut rest = question;
    while let Some(a) = rest.find('`') {
        let after = &rest[a + 1..];
        let Some(b) = after.find('`') else { break };
        for name in span_names(&after[..b]) {
            push(&name);
        }
        rest = &after[b + 1..];
    }
    for t in kind_names(question) {
        push(&t);
    }
    for word in question.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let w = word.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_' || c == ')'));
        let w = w.trim_start_matches('(');
        if w.is_empty() || !w.starts_with(|c: char| c.is_alphabetic() || c == '_') {
            continue;
        }
        let ident_ok = w
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '(' | ')'));
        if !ident_ok {
            continue;
        }
        let inner_cap =
            w.chars().skip(1).any(char::is_uppercase) && w.chars().any(char::is_lowercase);
        if w.contains('_') || w.contains("::") || w.contains('.') || w.ends_with("()") || inner_cap
        {
            push(w);
        }
    }
    out
}

/// The names in one backticked span. A bare name is kept whole (`APIRouter.include_router()`),
/// a selector loses its argument labels (`responseDecodable(of:)` → `responseDecodable()`), and a
/// code expression gives its qualified names, call heads and CamelCase words, string literals
/// skipped (`AF.request(url, encoding: JSONEncoding.default)` → `AF.request()`,
/// `JSONEncoding.default`).
fn span_names(span: &str) -> Vec<String> {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let bare = |s: &str| {
        s.chars()
            .all(|c| ident(c) || matches!(c, '.' | ':' | '(' | ')'))
    };
    if bare(span) {
        return match span.split_once('(') {
            Some((head, args)) if !args.starts_with(')') && !head.is_empty() => {
                vec![format!("{head}()")]
            }
            _ => vec![span.to_string()],
        };
    }
    let mut code = String::with_capacity(span.len());
    let mut quote: Option<char> = None;
    for c in span.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                code.push(' ');
            }
            None => code.push(c),
        }
    }
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < chars.len() {
        let starts = (ident(chars[k]) && !chars[k].is_ascii_digit())
            && (k == 0 || !(ident(chars[k - 1]) || chars[k - 1] == '.'));
        if !starts {
            k += 1;
            continue;
        }
        let mut end = k;
        loop {
            while end < chars.len() && ident(chars[end]) {
                end += 1;
            }
            let sep = if chars.get(end) == Some(&'.') {
                1
            } else if chars.get(end) == Some(&':') && chars.get(end + 1) == Some(&':') {
                2
            } else {
                0
            };
            if sep == 0 || !chars.get(end + sep).is_some_and(|&c| ident(c)) {
                break;
            }
            end += sep;
        }
        let name: String = chars[k..end].iter().collect();
        let call = matches!(chars.get(end), Some('(' | '<'));
        let qualified = name.contains('.') || name.contains("::");
        let camel =
            name.chars().skip(1).any(char::is_uppercase) && name.chars().any(char::is_lowercase);
        if qualified || camel || call {
            out.push(if call { format!("{name}()") } else { name });
        }
        k = end;
    }
    out
}

/// `the Sink trait`, `struct Searcher`, `class APIRouter`: a capitalised word beside a kind word
/// names a symbol.
fn kind_names(question: &str) -> Vec<String> {
    const KINDS: &[&str] = &[
        "trait",
        "struct",
        "class",
        "interface",
        "enum",
        "type",
        "protocol",
        "module",
        "object",
    ];
    let toks: Vec<&str> = question
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .collect();
    let kind = |j: Option<&&str>| j.is_some_and(|w| KINDS.contains(&w.to_lowercase().as_str()));
    toks.iter()
        .enumerate()
        .filter(|&(k, t)| {
            t.starts_with(char::is_uppercase)
                && t.len() >= 3
                && (kind(toks.get(k + 1)) || kind(k.checked_sub(1).and_then(|p| toks.get(p))))
        })
        .map(|(_, t)| t.to_string())
        .collect()
}

/// Resolve one named symbol: exact, else its last `.`/`::` segment; ambiguous names keep up to
/// two non-test candidates.
fn resolve_name(ix: &Index, name: &str) -> Vec<u32> {
    let pick = |r: Resolved| -> Vec<u32> {
        match r {
            Resolved::Found(i) => vec![i],
            Resolved::Ambiguous(c) => {
                let mut c: Vec<u32> = c
                    .into_iter()
                    .filter(|&i| !is_test_path(ix.source_file(i)))
                    .collect();
                c.sort_by_key(|&i| std::cmp::Reverse(ix.degree(i)));
                c.truncate(2);
                c
            }
            Resolved::NotFound => Vec::new(),
        }
    };
    // `Type::method` / `Type.method`: the method whose enclosing type is `Type`.
    let bare = name.trim_end_matches("()");
    if let Some((qual, member)) = bare.rsplit_once("::").or_else(|| bare.rsplit_once('.')) {
        let qual = qual.rsplit(['.', ':']).next().unwrap_or(qual);
        if !qual.is_empty() && member.len() >= 2 {
            let mut hits: Vec<u32> = (0..ix.node_count() as u32)
                .filter(|&i| {
                    ix.label(i).trim_start_matches('.').trim_end_matches("()") == member
                        && enclosing(ix, i).is_some_and(|p| {
                            let l = ix.label(p).trim_end_matches("()");
                            l == qual || l.split(['<', ' ']).next() == Some(qual)
                        })
                })
                .collect();
            if !hits.is_empty() {
                hits.sort_by_key(|&i| (is_test_node(ix, i), std::cmp::Reverse(ix.degree(i))));
                hits.truncate(2);
                return hits;
            }
        }
    }
    let found = pick(resolve::resolve(ix, name));
    if !found.is_empty() {
        return found;
    }
    let last = name
        .trim_end_matches("()")
        .rsplit(['.', ':'])
        .next()
        .unwrap_or(name);
    // `components.schemas` is a path in a document, not a symbol: retry the last segment only
    // when it reads as an identifier.
    let ident = last.contains('_') || last.chars().skip(1).any(char::is_uppercase);
    if last != name && last.len() >= 3 && ident {
        return pick(resolve::resolve(ix, last));
    }
    Vec::new()
}

/// Named anchors as `(explicit, bare CamelCase)`: an explicit name (backticked, with `_`, `.`,
/// `::` or `()`, or beside a kind word) is what the question is about; a bare CamelCase word that
/// spells a label exactly (and is no hub) may only be context, so it does not stop the search.
fn named_anchors_split(ix: &Index, question: &str) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let explicit_marks =
        |n: &str| n.contains('_') || n.contains('.') || n.contains("::") || n.ends_with("()");
    let mut backticked: Vec<String> = named_symbols(&backticks_only(question));
    backticked.extend(kind_names(question));
    let in_examples = example_names(question);
    let hub = query::hub_threshold(ix);
    let (mut strong, mut weak, mut examples): (Vec<u32>, Vec<u32>, Vec<u32>) =
        (Vec::new(), Vec::new(), Vec::new());
    for name in named_symbols(question) {
        let explicit = explicit_marks(&name) || backticked.contains(&name);
        for i in resolve_name(ix, &name) {
            if is_file_node(ix, i) && !name.contains('/') && !name.contains(".py") {
                continue;
            }
            if in_examples.contains(&name) {
                if !examples.contains(&i) {
                    examples.push(i);
                }
                continue;
            }
            if explicit {
                if !strong.contains(&i) {
                    strong.push(i);
                }
                continue;
            }
            let label = ix.label(i).trim_start_matches('.').trim_end_matches("()");
            if label == name && f64::from(ix.degree(i)) < hub && !weak.contains(&i) {
                weak.push(i);
            }
        }
    }
    strong.truncate(MAX_ANCHORS);
    examples.truncate(EXAMPLE_ANCHORS);
    (strong, weak, examples)
}

/// The names inside backticked code expressions (`AF.request(url, …)`), not bare names: the code
/// illustrates the question, so its names join the search instead of replacing it.
fn example_names(question: &str) -> Vec<String> {
    let bare = |s: &str| {
        s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '(' | ')'))
    };
    question
        .split('`')
        .enumerate()
        .filter(|&(k, s)| k % 2 == 1 && !bare(s))
        .flat_map(|(_, s)| span_names(s))
        .collect()
}

/// Only the backticked spans of a question, so their names count as explicit.
fn backticks_only(question: &str) -> String {
    question
        .split('`')
        .enumerate()
        .filter(|(k, _)| k % 2 == 1)
        .map(|(_, s)| format!("`{s}`"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `query`'s seeds for the question. A fact node of a document costs one line, so documents
/// get more of them; but only nodes that share a subject word with the question (not just
/// "few" or "years").
fn anchors_from_seeds(ix: &Index, question: &str) -> Vec<u32> {
    let mut out = query::seeds(ix, &query::terms(question));
    let docs = !out.is_empty() && out.iter().all(|&i| !is_code(ix, i));
    if docs {
        let subject = subject_stems(question);
        out.retain(|&i| {
            let ws: Vec<String> = words(ix.label(i)).collect();
            subject.iter().any(|t| ws.iter().any(|w| matches(w, t)))
        });
    }
    out.truncate(if docs { MAX_DOC_ANCHORS } else { MAX_ANCHORS });
    out
}

/// The function or class a nested node sits in, from its id (`…_get_request_handler_app` →
/// `…_get_request_handler`) when that node exists in the same file.
fn enclosing(ix: &Index, i: u32) -> Option<u32> {
    let file = ix.source_file(i);
    let mut cur = ix.id(i);
    while let Some((p, _)) = cur.rsplit_once('_') {
        cur = p;
        if let Some(j) = ix.by_id(cur) {
            if j != i && ix.source_file(j) == file && is_code(ix, j) && !is_file_node(ix, j) {
                return Some(j);
            }
        }
    }
    None
}

fn display(ix: &Index, i: u32) -> String {
    match enclosing(ix, i) {
        Some(p) => format!("{} › {}", ix.label(p), ix.label(i)),
        None => ix.label(i).to_string(),
    }
}

fn place(ix: &Index, i: u32) -> String {
    let n = ix.node(i);
    match line_no(n.source_location) {
        Some(l) => format!("{}:{l}", n.source_file),
        None if !n.source_location.is_empty() => {
            format!("{} ({})", n.source_file, n.source_location)
        }
        None => n.source_file.to_string(),
    }
}

/// A crude stem, so `babies` finds `baby` and `walking` finds `walk`.
fn stem(w: &str) -> String {
    let n = w.len();
    if let Some(b) = w
        .strip_suffix("ly")
        .filter(|_| n > 6 && (w.ends_with("tly") || w.ends_with("ally") || w.ends_with("sly")))
    {
        b.to_string()
    } else if let Some(b) = w.strip_suffix("ies").filter(|_| n > 4) {
        format!("{b}y")
    } else if let Some(b) = w.strip_suffix("ing").filter(|_| n > 5) {
        undouble(b)
    } else if let Some(b) = w.strip_suffix("ed").filter(|_| n > 4) {
        undouble(b)
    } else if let Some(b) = w.strip_suffix("es").filter(|_| n > 4 && w.ends_with("ses")) {
        b.to_string()
    } else if let Some(b) = w.strip_suffix('s').filter(|_| n > 3 && !w.ends_with("ss")) {
        b.to_string()
    } else {
        w.to_string()
    }
}

/// `embedd` → `embed`, `runn` → `run` (a doubled final consonant left by `-ed`/`-ing`), but not
/// `pass` or `call`.
fn undouble(b: &str) -> String {
    let c: Vec<char> = b.chars().collect();
    match c.as_slice() {
        [.., x, y] if x == y && !"aeioulsz".contains(*x) => c[..c.len() - 1].iter().collect(),
        _ => b.to_string(),
    }
}

fn words(line: &str) -> impl Iterator<Item = String> + '_ {
    line.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}

/// The question's terms, stemmed, each weighted by how rare it is among the lines of the
/// documents (idf): `babies` and `born` outweigh `friends` and `family`.
struct Weights {
    terms: Vec<(String, f64)>,
}

/// Words that frame a question rather than name its subject (`how many`, `in the last few
/// months`); matching them finds every line that happens to say "many" or "last".
const SCAFFOLD: &[&str] = &[
    "many",
    "much",
    "few",
    "several",
    "number",
    "total",
    "last",
    "latest",
    "recent",
    "recently",
    "ago",
    "ever",
    "often",
    "time",
    "times",
    "current",
    "currently",
    "previous",
    "first",
    "next",
    "day",
    "days",
    "week",
    "weeks",
    "month",
    "months",
    "year",
    "years",
    "today",
    "since",
    "still",
    "mention",
    "mentioned",
    "said",
    "tell",
    "told",
    "remember",
    "know",
];

/// The question's subject words, stemmed, in question order (no scaffolding).
fn subject_stems(question: &str) -> Vec<String> {
    query::terms(question)
        .into_iter()
        .filter(|t| t.len() >= 3 && !SCAFFOLD.contains(&t.as_str()))
        .map(|t| stem(&t))
        .fold(Vec::new(), |mut v, t| {
            if !v.contains(&t) {
                v.push(t);
            }
            v
        })
}

impl Weights {
    fn new(question: &str, src: &mut Sources, files: &[String]) -> Weights {
        let stems = subject_stems(question);
        let mut df = vec![0usize; stems.len()];
        let mut lines = 0usize;
        for f in files {
            let Some(ls) = src.lines(f) else { continue };
            for l in ls {
                lines += 1;
                let ws: Vec<String> = words(l).collect();
                for (k, t) in stems.iter().enumerate() {
                    if ws.iter().any(|w| matches(w, t)) {
                        df[k] += 1;
                    }
                }
            }
        }
        let terms: Vec<(String, f64)> = stems
            .into_iter()
            .zip(df)
            .map(|(t, d)| {
                let idf = ((lines as f64 + 1.0) / (d as f64 + 1.0)).ln().max(0.0);
                (t, idf)
            })
            .collect();
        Weights { terms }
    }
}

/// A word matches a term when their stems are equal (`babies` ~ `baby`, not `friendly` ~
/// `friend`).
fn matches(word: &str, term_stem: &str) -> bool {
    word == term_stem || stem(word) == term_stem
}

/// The lines of `file` that best match a node's label: most of its content words (stems), at
/// least two (or the only one), best first; at most two lines.
fn label_lines(src: &mut Sources, file: &str, label: &str) -> Vec<(usize, String)> {
    let want: Vec<String> = query::terms(label)
        .into_iter()
        .filter(|t| t.len() >= 3)
        .map(|t| stem(&t))
        .fold(Vec::new(), |mut v, t| {
            if !v.contains(&t) {
                v.push(t);
            }
            v
        });
    if want.is_empty() {
        return Vec::new();
    }
    let Some(lines) = src.lines(file) else {
        return Vec::new();
    };
    let need = want.len().min(2).max(want.len().div_ceil(2));
    let mut scored: Vec<(usize, usize)> = lines
        .iter()
        .enumerate()
        .map(|(k, l)| {
            let ws: Vec<String> = words(l).collect();
            (
                want.iter()
                    .filter(|t| ws.iter().any(|w| matches(w, t)))
                    .count(),
                k,
            )
        })
        .filter(|&(h, _)| h >= need)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(2)
        .map(|(_, k)| (k + 1, lines[k].clone()))
        .collect()
}

/// A node's callees (calls, indirect calls, uses) in call-site order, one per label.
fn callees_in_order(ix: &Index, i: u32) -> Vec<u32> {
    let mut v: Vec<(usize, u32)> = Vec::new();
    for e in ix.incident(i) {
        let (a, b) = ix.endpoints(e);
        let rel = ix.relation(e);
        if a == i && b != i && (rel == "calls" || rel == "indirect_call" || rel == "uses") {
            let line = line_no(ix.edge(e).source_location.unwrap_or("")).unwrap_or(usize::MAX);
            v.push((line, b));
        }
    }
    v.sort();
    let mut seen = HashSet::new();
    v.into_iter()
        .map(|(_, b)| b)
        .filter(|&b| seen.insert(ix.label(b)))
        .collect()
}

/// One anchor's section. For a flow question (`flow`), the first [`FLOW_CALLERS`] callers
/// outside tests also list what else they call, in call order: the steps around the anchor.
fn section(
    ix: &Index,
    src: &mut Sources,
    i: u32,
    hits: &[(usize, String)],
    def_lines: usize,
    flow: bool,
) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "## {}  {}", display(ix, i), place(ix, i));
    let node = ix.node(i);
    if is_code(ix, i) && !is_file_node(ix, i) {
        if let Some(start) = line_no(node.source_location) {
            if let Some(lines) = src.lines(node.source_file) {
                for (k, l) in lines.iter().enumerate().skip(start - 1).take(def_lines) {
                    let _ = writeln!(s, "  {:>5}| {l}", k + 1);
                }
            }
        }
        let shown = line_no(node.source_location).map(|l| l..l + def_lines);
        let extra: Vec<&(usize, String)> = hits
            .iter()
            .filter(|(n, _)| !shown.as_ref().is_some_and(|r| r.contains(n)))
            .collect();
        if !extra.is_empty() {
            let _ = writeln!(s, "  matching lines:");
            for (n, text) in extra {
                let _ = writeln!(s, "  {n:>5}| {text}");
            }
        }
    } else if !is_file_node(ix, i) {
        // A fact or concept of a document: the source line that best matches its own label.
        for (n, text) in label_lines(src, node.source_file, ix.label(i)) {
            let _ = writeln!(s, "  {n:>5}| {text}");
        }
    }

    let mut callers: Vec<(u32, u32)> = Vec::new();
    let mut importers: Vec<u32> = Vec::new();
    let mut callees: Vec<u32> = Vec::new();
    for e in ix.incident(i) {
        let (a, b) = ix.endpoints(e);
        let rel = ix.relation(e);
        if b == i && a != i {
            if USE_RELATIONS.contains(&rel) {
                callers.push((a, e));
            } else if IMPORT_RELATIONS.contains(&rel) {
                importers.push(a);
            }
        } else if a == i && b != i && (rel == "calls" || rel == "indirect_call" || rel == "uses") {
            callees.push(b);
        }
    }
    // Callers outside tests first, then by file and line.
    callers.sort_by_key(|&(c, e)| {
        let edge = ix.edge(e);
        (
            is_test_path(ix.source_file(c)),
            edge.source_file.unwrap_or("").to_string(),
            line_no(edge.source_location.unwrap_or("")).unwrap_or(0),
        )
    });
    callers.dedup_by_key(|(c, _)| *c);
    if !callers.is_empty() {
        let tests = callers
            .iter()
            .filter(|&&(c, _)| is_test_path(ix.source_file(c)))
            .count();
        let _ = writeln!(
            s,
            "  called/used by ({}{}):",
            callers.len(),
            if tests > 0 {
                format!(", {tests} in tests")
            } else {
                String::new()
            }
        );
        let core: Vec<(u32, u32)> = callers
            .iter()
            .copied()
            .filter(|&(c, _)| !is_test_path(ix.source_file(c)))
            .collect();
        for (k, &(c, e)) in core.iter().take(MAX_CALLERS).enumerate() {
            let edge = ix.edge(e);
            let file = edge
                .source_file
                .filter(|f| !f.is_empty())
                .unwrap_or_else(|| ix.source_file(c));
            let line = line_no(edge.source_location.unwrap_or(""));
            let at = line.map_or(file.to_string(), |l| format!("{file}:{l}"));
            let code = line
                .and_then(|l| src.line(file, l))
                .map(|t| format!("  | {}", clip_to(t.trim(), CALL_SITE_CHARS)))
                .unwrap_or_default();
            let _ = writeln!(s, "    {} [{}] {at}{code}", display(ix, c), ix.relation(e));
            if flow && k < FLOW_CALLERS && !is_file_node(ix, c) {
                let steps: Vec<u32> = callees_in_order(ix, c)
                    .into_iter()
                    .filter(|&x| x != i)
                    .collect();
                if !steps.is_empty() {
                    let names: Vec<&str> = steps
                        .iter()
                        .take(MAX_CALLEES)
                        .map(|&x| ix.label(x))
                        .collect();
                    let more = steps.len().saturating_sub(MAX_CALLEES);
                    let _ = writeln!(
                        s,
                        "      which also calls: {}{}",
                        names.join(", "),
                        if more > 0 {
                            format!(" (+{more})")
                        } else {
                            String::new()
                        }
                    );
                }
            }
        }
        if core.len() > MAX_CALLERS {
            let _ = writeln!(
                s,
                "    … {} more: `dijkstral explain <symbol> --incoming --all`",
                core.len() - MAX_CALLERS
            );
        }
        if tests > 0 {
            let names: Vec<&str> = callers
                .iter()
                .filter(|&&(c, _)| is_test_path(ix.source_file(c)))
                .take(2)
                .map(|&(c, _)| ix.label(c))
                .collect();
            let _ = writeln!(
                s,
                "    + {tests} in tests, examples or benchmarks (e.g. {})",
                names.join(", ")
            );
        }
    }
    importers.sort_by_key(|&f| {
        (
            is_test_path(ix.source_file(f)),
            ix.source_file(f).to_string(),
        )
    });
    importers.dedup();
    if !importers.is_empty() {
        let names: Vec<&str> = importers
            .iter()
            .take(MAX_IMPORTERS)
            .map(|&f| ix.source_file(f))
            .collect();
        let more = importers.len().saturating_sub(MAX_IMPORTERS);
        let _ = writeln!(
            s,
            "  imported by: {}{}",
            names.join(", "),
            if more > 0 {
                format!(" (+{more})")
            } else {
                String::new()
            }
        );
    }
    let mut seen = HashSet::new();
    callees.retain(|&c| seen.insert(ix.label(c)));
    if !callees.is_empty() {
        let names: Vec<&str> = callees
            .iter()
            .take(MAX_CALLEES)
            .map(|&c| ix.label(c))
            .collect();
        let more = callees.len().saturating_sub(MAX_CALLEES);
        let _ = writeln!(
            s,
            "  calls: {}{}",
            names.join(", "),
            if more > 0 {
                format!(" (+{more})")
            } else {
                String::new()
            }
        );
    }
    s
}

/// For a question about documents: a ready search for its rarest terms, since the graph's facts
/// summarise and a "how many" or "every" question needs each occurrence in the text.
fn search_hint(w: &Weights) -> String {
    // The subject words in question order; a stem ending in `y` searches its prefix so `baby`
    // also finds `babies`.
    let pick: Vec<String> = w
        .terms
        .iter()
        .filter(|t| t.1 > 0.0)
        .take(4)
        .map(|(t, _)| {
            t.strip_suffix('y')
                .filter(|p| p.len() >= 3)
                .unwrap_or(t)
                .to_string()
        })
        .collect();
    if pick.is_empty() {
        return String::new();
    }
    format!(
        "The graph summarises. For every occurrence (how many, all of, the latest), also run: \
         rg -n -i \"{}\"\n",
        pick.join("|")
    )
}

/// Words that describe code in general, not this question's subject.
const CODE_SCAFFOLD: &[&str] = &[
    "function",
    "method",
    "code",
    "file",
    "class",
    "package",
    "module",
    "crate",
    "repo",
    "repository",
    "defin",
    "implement",
    "implementation",
    "type",
    "struct",
    "trait",
];
const TEXT_ANCHORS: usize = 4;
/// Code-text matches added after the symbols a question names.
const AFTER_NAMED: usize = 2;
/// Symbols listed by name after the anchors.
const NAME_LIST: usize = 20;
/// Names from backticked code expressions kept as anchors.
const EXAMPLE_ANCHORS: usize = 2;

/// Per node: how often each question term occurs, and its matching lines with the terms each
/// carries (a bit per term).
/// Per node: how often each term occurs in its code and in its comments, and its matching code
/// lines with the terms each carries (a bit per term).
struct NodeHits {
    code: Vec<u32>,
    doc: Vec<u32>,
    lines: Vec<(usize, u64)>,
}
const HIT_LINES: usize = 3;

/// Words of a source line: identifiers split at `_` and lower-to-upper case changes
/// (`readJSONBody` → read, json, body; `is_body_form` → is, body, form), lower-cased and stemmed,
/// plus each whole identifier.
fn ident_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in line
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        out.push(stem(&tok.to_lowercase()));
        let parts = camel_parts(tok);
        if parts.len() > 1 {
            out.extend(parts);
        }
    }
    out
}

/// A token's words at case changes, stemmed: `readJSONBody` → read, json, body.
fn camel_parts(tok: &str) -> Vec<String> {
    let cs: Vec<char> = tok.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for k in 1..cs.len() {
        // `readJSON|Body`: lower→upper, and the last capital of a run before a lower one.
        let lower_upper =
            cs[k].is_uppercase() && (cs[k - 1].is_lowercase() || cs[k - 1].is_ascii_digit());
        let acronym_end = cs[k].is_uppercase()
            && cs[k - 1].is_uppercase()
            && cs.get(k + 1).is_some_and(|c| c.is_lowercase());
        if lower_upper || acronym_end {
            out.push(stem(
                &cs[start..k].iter().collect::<String>().to_lowercase(),
            ));
            start = k;
        }
    }
    out.push(stem(&cs[start..].iter().collect::<String>().to_lowercase()));
    out
}

/// The words of a symbol's name, stemmed: `SafeAggregateSink` → safe, aggregate, sink;
/// `get_item_details` → get, item, detail.
fn name_parts(label: &str) -> Vec<String> {
    label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .flat_map(camel_parts)
        .collect()
}

/// Symbols outside tests whose names carry the question's words, grouped by file: each scores
/// the rarity (idf among those names) of the words it shares with the question, times the root
/// of the share of its name those words make up. Questions put in a reader's words what the code
/// often names outright (`SafeAggregateSink`, `FilteringSink` for "how sinks are combined and
/// filtered"), where the text of a function body does not.
fn name_list(ix: &Index, question: &str, shown: &[u32]) -> String {
    let terms: Vec<Vec<String>> = code_terms(question).iter().map(|t| spellings(t)).collect();
    if terms.is_empty() {
        return String::new();
    }
    // A label can share a term only if its lower case contains the term's spelling (less a
    // final `y`, which `ies` stems to): most labels are passed over without being split.
    let keys: Vec<memchr::memmem::Finder> = terms
        .iter()
        .flatten()
        .map(|t| t.trim_end_matches('y'))
        .filter(|k| !k.is_empty())
        .map(|k| memchr::memmem::Finder::new(k.as_bytes()).into_owned())
        .collect();
    // Test files are decided once per file, not once per symbol.
    let mut test_file: HashMap<&str, bool> = HashMap::new();
    let mut lower: Vec<u8> = Vec::new();
    let mut n = 0usize;
    let mut cands: Vec<(u32, Vec<usize>, usize)> = Vec::new();
    for i in 0..ix.node_count() as u32 {
        if !is_code(ix, i) {
            continue;
        }
        let (file, label) = (ix.source_file(i), ix.label(i));
        let test = *test_file.entry(file).or_insert_with(|| is_test_path(file));
        if test || ix.id(i).contains("_tests_") || label.trim_start_matches('.').starts_with("test")
        {
            continue;
        }
        n += 1;
        lower.clear();
        lower.extend(label.bytes().map(|b| b.to_ascii_lowercase()));
        if !keys.iter().any(|k| k.find(&lower).is_some())
            || file.is_empty()
            || is_config(file)
            || is_file_node(ix, i)
        {
            continue;
        }
        let parts = name_parts(label);
        let hit: Vec<usize> = (0..terms.len())
            .filter(|&t| parts.iter().any(|p| terms[t].contains(p)))
            .collect();
        let words = parts
            .iter()
            .filter(|p| terms.iter().any(|t| t.contains(p)))
            .count();
        if !hit.is_empty() {
            cands.push((i, hit, words * 1000 / parts.len().max(1)));
        }
    }
    let n = n.max(1) as f64;
    let mut df = vec![0usize; terms.len()];
    for (_, hit, _) in &cands {
        for &t in hit {
            df[t] += 1;
        }
    }
    let mut ranked: Vec<(f64, u32)> = cands
        .iter()
        .filter(|(i, _, _)| !shown.contains(i))
        .map(|(i, hit, share)| {
            let idf: f64 = hit.iter().map(|&t| (n / df[t] as f64).ln().max(0.0)).sum();
            (idf * (*share as f64 / 1000.0).sqrt(), *i)
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for (_, i) in ranked {
        let (file, label) = (ix.source_file(i), ix.label(i).trim_start_matches('.'));
        if !seen.insert((file, label)) {
            continue;
        }
        match groups.iter_mut().find(|(f, _)| *f == file) {
            Some((_, v)) => v.push(label),
            None => groups.push((file, vec![label])),
        }
        if seen.len() == NAME_LIST {
            break;
        }
    }
    if groups.is_empty() {
        return String::new();
    }
    let mut out = String::from("names sharing the question's words:\n");
    for (file, labels) in groups {
        let _ = writeln!(out, "  {file}: {}", labels.join(", "));
    }
    out
}

/// Configuration and data files: their keys are not the code a question asks about.
fn is_config(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "toml" | "json" | "yaml" | "yml" | "lock" | "cfg" | "ini" | "xml" | "md" | "txt" | "csv"
    )
}

/// Comment lines (`#`, `//`, `/*`, ` * `, docstring quotes) and prose inside string literals
/// (help text: six or more words and no code punctuation) repeat every word of a question.
/// A real comment line (`#`, `//`, `/*`, ` * `, docstring quotes): the code describing itself.
fn is_true_comment(line: &str) -> bool {
    let t = line.trim_start();
    ["#", "//", "/*", "*", "\"\"\"", "\'\'\'", "--", ";"]
        .iter()
        .any(|p| t.starts_with(p))
}

fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    if ["#", "//", "/*", "*", "\"\"\"", "\'\'\'", "--", ";"]
        .iter()
        .any(|p| t.starts_with(p))
    {
        return true;
    }
    let words = t.split_whitespace().count();
    words >= 6 && !t.contains(['(', ')', '=', '{', '}', ';', '[', ']'])
}

/// Code nodes of each file by start line: the spans a text hit is credited to (the innermost
/// node starting at or before the hit's line).
fn code_spans(ix: &Index) -> HashMap<String, Vec<(usize, u32)>> {
    let mut spans: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
    for i in 0..ix.node_count() as u32 {
        if !is_code(ix, i) || is_file_node(ix, i) || is_config(ix.source_file(i)) {
            continue;
        }
        let n = ix.node(i);
        if let Some(l) = line_no(n.source_location) {
            spans
                .entry(n.source_file.to_string())
                .or_default()
                .push((l, i));
        }
    }
    for v in spans.values_mut() {
        v.sort_unstable();
    }
    spans
}

/// For a code question that names no symbol: the functions and classes whose text carries the
/// most of the question's subject words (rarer words weigh more), with their matching lines.
/// The question's code-search terms: subject words without words that describe code in general.
fn code_terms(question: &str) -> Vec<String> {
    let mut terms: Vec<String> = subject_stems(question)
        .into_iter()
        .filter(|t| !CODE_SCAFFOLD.iter().any(|c| t == c))
        .collect();
    // Two-word phrases code spells as one word (`memory map` → `mmap`).
    let ws: Vec<String> = words(question).map(|w| stem(&w)).collect();
    for pair in ws.windows(2) {
        for (a, b, short) in PHRASES {
            if pair[0] == *a && pair[1] == *b && !terms.iter().any(|t| t == short) {
                terms.push((*short).to_string());
            }
        }
    }
    terms
}

/// Abbreviations programmers write for common words (stemmed word → spellings in code).
const ABBREVIATIONS: &[(&str, &[&str])] = &[
    ("configuration", &["config", "cfg", "conf"]),
    ("error", &["err"]),
    ("initialize", &["init"]),
    ("initialise", &["init"]),
    ("authentication", &["auth", "authn"]),
    ("authorization", &["authz", "auth"]),
    ("directory", &["dir"]),
    ("request", &["req"]),
    ("response", &["resp", "res"]),
    ("message", &["msg"]),
    ("argument", &["arg"]),
    ("parameter", &["param"]),
    ("temporary", &["tmp", "temp"]),
    ("buffer", &["buf"]),
    ("length", &["len"]),
    ("context", &["ctx"]),
    ("pointer", &["ptr"]),
    ("number", &["num"]),
    ("string", &["str"]),
    ("value", &["val"]),
    ("expression", &["expr"]),
    ("environment", &["env"]),
    ("specification", &["spec"]),
    ("reference", &["ref"]),
    ("information", &["info"]),
    ("position", &["pos"]),
    ("attribute", &["attr"]),
    ("document", &["doc"]),
    ("library", &["lib"]),
    ("package", &["pkg"]),
    ("executable", &["exe", "exec"]),
    ("previous", &["prev"]),
    ("index", &["idx"]),
    ("source", &["src"]),
    ("destination", &["dst", "dest"]),
    ("command", &["cmd"]),
    ("statistic", &["stat"]),
    ("connection", &["conn"]),
    ("database", &["db"]),
    ("transaction", &["tx", "txn"]),
    ("synchronize", &["sync"]),
    ("asynchronous", &["async"]),
    ("allocation", &["alloc"]),
    ("allocate", &["alloc"]),
    ("separator", &["sep"]),
    ("utility", &["util"]),
    ("manager", &["mgr"]),
    ("controller", &["ctrl", "controller"]),
];

/// Two-word phrases code writes as one word.
const PHRASES: &[(&str, &str, &str)] = &[
    ("memory", "map", "mmap"),
    ("regular", "expression", "regex"),
    ("standard", "input", "stdin"),
    ("standard", "output", "stdout"),
    ("standard", "error", "stderr"),
    ("file", "descriptor", "fd"),
    ("garbage", "collection", "gc"),
    ("time", "out", "timeout"),
];

/// A term's spellings: itself and its abbreviations.
fn spellings(term: &str) -> Vec<String> {
    let mut out = vec![term.to_string()];
    for (word, short) in ABBREVIATIONS {
        if stem(word) == term {
            out.extend(short.iter().map(|s| s.to_string()));
        }
    }
    out
}

/// One pass over the code for a set of terms: per node, how often each term occurs and its
/// matching lines (with the terms each carries); and each term's line frequency.
struct Scan {
    terms: Vec<String>,
    df: Vec<usize>,
    lines: usize,
    per: HashMap<u32, NodeHits>,
    spans: HashMap<String, Vec<(usize, u32)>>,
}

impl Scan {
    /// Files are scanned in parallel; a line is split into identifier words only when its lower
    /// case contains one of the terms (a stem is a substring of every word it matches).
    fn new(ix: &Index, root: &Path, terms: Vec<String>) -> Scan {
        use rayon::prelude::*;
        let terms: Vec<String> = terms.into_iter().take(64).collect();
        let alts: Vec<Vec<String>> = terms.iter().map(|t| spellings(t)).collect();
        let spans = code_spans(ix);
        let mut files: Vec<&String> = spans.keys().collect();
        files.sort();
        let finders: Vec<memchr::memmem::Finder> = alts
            .iter()
            .flatten()
            .map(|t| memchr::memmem::Finder::new(t.as_bytes()).into_owned())
            .collect();
        type FileHits = (usize, Vec<usize>, Vec<(u32, usize, u64, bool)>);
        let results: Vec<FileHits> = files
            .par_iter()
            .map(|f| {
                let mut df = vec![0usize; terms.len()];
                let mut hits: Vec<(u32, usize, u64, bool)> = Vec::new();
                let path = root.join(f.as_str());
                let ok = std::fs::metadata(&path)
                    .is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE_BYTES);
                let Some(bytes) = ok.then(|| std::fs::read(&path).ok()).flatten() else {
                    return (0, df, hits);
                };
                let text = String::from_utf8_lossy(&bytes);
                let starts = &spans[f.as_str()];
                let mut n_lines = 0;
                let mut in_docstring = false;
                for (k, line) in text.lines().enumerate() {
                    n_lines += 1;
                    let quotes = line.matches("\"\"\"").count() + line.matches("\'\'\'").count();
                    let was_in = in_docstring;
                    if quotes % 2 == 1 {
                        in_docstring = !in_docstring;
                    }
                    let in_docstring = was_in || quotes > 0;
                    let lower = line.to_ascii_lowercase();
                    if !finders.iter().any(|fd| fd.find(lower.as_bytes()).is_some()) {
                        continue;
                    }
                    // Comments and docstrings describe the code in the question's own words:
                    // they count as the enclosing node's description (not as matching lines).
                    // Prose in string literals (help text) does not.
                    let doc = in_docstring || is_true_comment(line);
                    if !doc && is_comment(line) {
                        continue;
                    }
                    let ws = ident_words(line);
                    let mut mask = 0u64;
                    for (t, spelled) in alts.iter().enumerate() {
                        if ws.iter().any(|w| spelled.contains(w)) {
                            mask |= 1 << t;
                            df[t] += 1;
                        }
                    }
                    if mask == 0 {
                        continue;
                    }
                    let n = k + 1;
                    let pos = starts.partition_point(|&(l, _)| l <= n);
                    if let Some(p) = pos.checked_sub(1) {
                        hits.push((starts[p].1, n, mask, doc));
                    }
                }
                (n_lines, df, hits)
            })
            .collect();
        let mut df = vec![0usize; terms.len()];
        let mut lines = 0;
        let mut per: HashMap<u32, NodeHits> = HashMap::new();
        for (n, d, hits) in results {
            lines += n;
            for (t, c) in d.into_iter().enumerate() {
                df[t] += c;
            }
            for (node, line, mask, doc) in hits {
                let e = per.entry(node).or_insert_with(|| NodeHits {
                    code: vec![0; terms.len()],
                    doc: vec![0; terms.len()],
                    lines: Vec::new(),
                });
                let counts = if doc { &mut e.doc } else { &mut e.code };
                for (t, c) in counts.iter_mut().enumerate() {
                    if mask & (1 << t) != 0 {
                        *c += 1;
                    }
                }
                if !doc {
                    e.lines.push((line, mask));
                }
            }
        }
        Scan {
            terms,
            df,
            lines,
            per,
            spans,
        }
    }

    /// The best nodes for the terms in `wanted`, each with its best matching lines.
    fn rank(
        &self,
        ix: &Index,
        src: &mut Sources,
        wanted: &[String],
    ) -> Vec<(u32, Vec<(usize, String)>)> {
        let mask: u64 = self
            .terms
            .iter()
            .enumerate()
            .filter(|(_, t)| wanted.contains(t))
            .fold(0, |m, (k, _)| m | (1 << k));
        let n_wanted = (mask.count_ones() as usize).min(2);
        if n_wanted == 0 {
            return Vec::new();
        }
        let idf: Vec<f64> = self
            .df
            .iter()
            .map(|&d| ((self.lines as f64 + 1.0) / (d as f64 + 1.0)).ln().max(0.0))
            .collect();
        let span_len = |node: u32| -> f64 {
            let file = ix.source_file(node);
            let start = line_no(ix.node(node).source_location).unwrap_or(1);
            let next = self.spans[file]
                .iter()
                .map(|&(l, _)| l)
                .find(|&l| l > start)
                .unwrap_or(start + 200);
            (next - start) as f64
        };
        // A node's own evidence: its matching words (rarer weigh more) and the question words its
        // name carries (three times over: the name is the strongest evidence).
        let strength = |node: u32, h: &NodeHits| -> (f64, usize) {
            let on = |t: usize| mask & (1 << t) != 0 && (h.code[t] > 0 || h.doc[t] > 0);
            let distinct = (0..self.terms.len()).filter(|&t| on(t)).count();
            let term = |t: usize, c: u32| idf[t] * (1.0 + (c.max(1) as f64).ln());
            let weight: f64 = (0..self.terms.len())
                .filter(|&t| mask & (1 << t) != 0)
                .map(|t| {
                    // A term in the comments counts as much as one in the code: comments are the
                    // code described in the question's own words.
                    let code = if h.code[t] > 0 {
                        term(t, h.code[t])
                    } else {
                        0.0
                    };
                    let doc = if h.doc[t] > 0 { term(t, h.doc[t]) } else { 0.0 };
                    code + doc
                })
                .sum();
            let name = ident_words(ix.label(node));
            let named: f64 = (0..self.terms.len())
                .filter(|&t| {
                    let spelled = spellings(&self.terms[t]);
                    mask & (1 << t) != 0 && name.iter().any(|w| spelled.contains(w))
                })
                .map(|t| 3.0 * idf[t])
                .sum();
            (weight + named, distinct)
        };
        let own: HashMap<u32, (f64, usize)> = self
            .per
            .iter()
            .map(|(&node, h)| (node, strength(node, h)))
            .collect();
        let mut ranked: Vec<(f64, bool, u32)> = own
            .iter()
            .filter(|(_, &(_, distinct))| distinct >= n_wanted)
            .map(|(&node, &(weight, distinct))| {
                // The graph's view: callers and callees that match the question too make a node
                // likelier to be the answer (its three strongest such neighbours count a quarter).
                let mut near: Vec<f64> = ix
                    .incident(node)
                    .filter(|&e| matches!(ix.relation(e), "calls" | "indirect_call" | "uses"))
                    .filter_map(|e| {
                        let (a, b) = ix.endpoints(e);
                        let other = if a == node { b } else { a };
                        own.get(&other).map(|o| o.0)
                    })
                    .collect();
                near.sort_by(|a, b| b.total_cmp(a));
                let neighbours: f64 = near.iter().take(3).sum::<f64>() * 0.25;
                let score =
                    (weight + neighbours) * distinct as f64 / (1.0 + span_len(node) / 40.0).sqrt();
                (score, is_test_node(ix, node), node)
            })
            .collect();
        // Tests last; then score; then node order.
        ranked.sort_by(|a, b| a.1.cmp(&b.1).then(b.0.total_cmp(&a.0)).then(a.2.cmp(&b.2)));
        ranked
            .into_iter()
            .take(TEXT_ANCHORS)
            .map(|(_, _, node)| {
                let mut hits: Vec<(usize, u32)> = self.per[&node]
                    .lines
                    .iter()
                    .map(|&(n, m)| (n, (m & mask).count_ones()))
                    .filter(|&(_, c)| c > 0)
                    .collect();
                hits.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                hits.truncate(HIT_LINES);
                hits.sort_unstable();
                let file = ix.source_file(node).to_string();
                let lines = hits
                    .into_iter()
                    .filter_map(|(n, _)| src.line(&file, n).map(|t| (n, t)))
                    .collect();
                (node, lines)
            })
            .collect()
    }
}

/// The two ends of a flow question ("trace a file from the walker to the printer", "how does the
/// annotation end up as a schema"): the text naming where it starts and where it ends.
fn flow_ends(question: &str) -> Option<(String, String)> {
    let q = question.to_lowercase();
    let cut = |at: &str| {
        q.find(at)
            .map(|k| (q[..k].to_string(), q[k + at.len()..].to_string()))
    };
    if let Some(f) = q.find(" from ") {
        let rest = &q[f + 6..];
        for to in [" to ", " until ", " into ", " through to "] {
            if let Some(t) = rest.find(to) {
                return Some((rest[..t].to_string(), rest[t + to.len()..].to_string()));
            }
        }
    }
    for at in [
        " end up as ",
        " end up in ",
        " ends up ",
        " end up ",
        " get to ",
        " gets to ",
        " reach ",
        " reaches ",
        " become ",
        " becomes ",
        " turned into ",
        " turn into ",
    ] {
        if let Some(ends) = cut(at) {
            return Some(ends);
        }
    }
    None
}

/// Relations a call chain may follow, caller to callee.
const CHAIN_RELATIONS: &[&str] = &[
    "calls",
    "indirect_call",
    "uses",
    "references",
    "instantiates",
    "contains",
    "method",
];
const MAX_CHAIN: usize = 6;

/// The shortest call chain from any of `from` to any of `to` (breadth first over
/// [`CHAIN_RELATIONS`] in their stored direction, at most [`MAX_CHAIN`] hops), with the link
/// taken at each step.
fn call_chain(ix: &Index, from: &[u32], to: &[u32]) -> Option<Vec<(u32, Option<u32>)>> {
    let goal: HashSet<u32> = to.iter().copied().collect();
    let mut prev: HashMap<u32, (u32, u32)> = HashMap::new();
    let mut seen: HashSet<u32> = from.iter().copied().collect();
    let mut frontier: Vec<u32> = from.to_vec();
    for _ in 0..MAX_CHAIN {
        let mut next = Vec::new();
        for &v in &frontier {
            for e in ix.incident(v) {
                let (a, b) = ix.endpoints(e);
                if a != v || b == v || !CHAIN_RELATIONS.contains(&ix.relation(e)) {
                    continue;
                }
                if is_file_node(ix, b) || !seen.insert(b) {
                    continue;
                }
                prev.insert(b, (v, e));
                if goal.contains(&b) {
                    let mut chain = vec![(b, Some(e))];
                    let mut cur = v;
                    while let Some(&(p, pe)) = prev.get(&cur) {
                        chain.push((cur, Some(pe)));
                        cur = p;
                    }
                    chain.push((cur, None));
                    chain.reverse();
                    return Some(chain);
                }
                next.push(b);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    None
}

fn render_chain(ix: &Index, chain: &[(u32, Option<u32>)]) -> String {
    let mut s = String::from("Call chain from where it starts to where it ends:\n");
    for (k, &(n, via)) in chain.iter().enumerate() {
        let rel = via
            .map(|e| format!("[{}] ", ix.relation(e)))
            .unwrap_or_default();
        let at = via
            .and_then(|e| {
                let edge = ix.edge(e);
                let line = line_no(edge.source_location.unwrap_or(""))?;
                Some(format!(" (at {}:{line})", edge.source_file.unwrap_or("")))
            })
            .unwrap_or_default();
        let arrow = if k == 0 { "  " } else { "  → " };
        let _ = writeln!(s, "{arrow}{rel}{}  {}{at}", display(ix, n), place(ix, n));
    }
    s
}

/// Words that ask how the code is organised rather than about one piece of it.
const STRUCTURE_WORDS: &[&str] = &[
    "organis",
    "organiz",
    "divid",
    "split",
    "structur",
    "architectur",
    "responsib",
    "layer",
    "layout",
    "overview",
    "crate",
    "package",
    "module",
    "component",
    "subsystem",
];
const AREAS: usize = 12;

/// How many other files use a symbol (incoming links other than containment): a steadier sign of
/// a symbol that represents its area than its degree, which many calls inside one file inflate.
fn user_files(ix: &Index, i: u32) -> usize {
    let own = ix.source_file(i);
    let mut files: HashSet<&str> = HashSet::new();
    for e in ix.incident(i) {
        let (a, b) = ix.endpoints(e);
        let rel = ix.relation(e);
        if b == i && a != i && rel != "contains" && rel != "method" {
            let f = ix.source_file(a);
            if f != own && !is_test_path(f) {
                files.insert(f);
            }
        }
    }
    files.len()
}

fn asks_structure(question: &str) -> bool {
    words(question).any(|w| STRUCTURE_WORDS.iter().any(|s| w.starts_with(s)))
}

/// A map of the code: its areas (a top-level directory, or its subdirectories when it holds
/// several, such as `crates/*` or `pkg/*`), areas whose path or symbols carry the question's words
/// first, each with its most connected symbols outside tests.
fn overview(ix: &Index, question: &str) -> String {
    let mut top: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut nodes_in: HashMap<String, Vec<u32>> = HashMap::new();
    let code: Vec<u32> = (0..ix.node_count() as u32)
        .filter(|&i| is_code(ix, i) && !is_file_node(ix, i) && !is_test_node(ix, i))
        .filter(|&i| !is_config(ix.source_file(i)))
        .collect();
    for &i in &code {
        let parts: Vec<&str> = ix.source_file(i).split('/').collect();
        if parts.len() >= 3 {
            top.entry(parts[0]).or_default().insert(parts[1]);
        }
    }
    for &i in &code {
        let parts: Vec<&str> = ix.source_file(i).split('/').collect();
        let area = match parts.as_slice() {
            [a, b, _, ..] if top.get(a).is_some_and(|s| s.len() >= 3) => format!("{a}/{b}"),
            [a, _, ..] => (*a).to_string(),
            _ => continue,
        };
        nodes_in.entry(area).or_default().push(i);
    }
    if nodes_in.len() < 2 {
        return String::new();
    }
    let subject = subject_stems(question);
    let mut areas: Vec<(usize, usize, String, Vec<u32>)> = nodes_in
        .into_iter()
        .map(|(area, mut ns)| {
            ns.sort_by_key(|&i| {
                (
                    std::cmp::Reverse(user_files(ix, i)),
                    std::cmp::Reverse(ix.degree(i)),
                    i,
                )
            });
            let mut words_here: HashSet<String> = ident_words(&area).into_iter().collect();
            for &i in ns.iter().take(30) {
                words_here.extend(ident_words(ix.label(i)));
            }
            let hits = subject.iter().filter(|t| words_here.contains(*t)).count();
            (hits, ns.len(), area, ns)
        })
        .collect();
    areas.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    let total = areas.len();
    let mut s = format!("Code areas ({total}), those matching the question first:\n");
    for (_, n, area, ns) in areas.iter().take(AREAS) {
        let mut seen = HashSet::new();
        let names: Vec<&str> = ns
            .iter()
            .map(|&i| ix.label(i))
            .filter(|l| seen.insert(*l))
            .take(4)
            .collect();
        let _ = writeln!(s, "  {area}/ ({n} symbols): {}", names.join(", "));
    }
    if total > AREAS {
        let _ = writeln!(s, "  … {} more areas", total - AREAS);
    }
    // Zoom into the best-matching area: its subdirectories, so a question about one subsystem
    // of a large repository sees how that subsystem is divided.
    if let Some((hits, _, area, ns)) = areas.first() {
        if *hits > 0 {
            let depth = area.split('/').count();
            let mut sub: HashMap<String, Vec<u32>> = HashMap::new();
            for &i in ns {
                let parts: Vec<&str> = ix.source_file(i).split('/').collect();
                if parts.len() > depth + 1 {
                    sub.entry(parts[..=depth].join("/")).or_default().push(i);
                }
            }
            if sub.len() >= 2 {
                let mut subs: Vec<(usize, usize, String, Vec<u32>)> = sub
                    .into_iter()
                    .map(|(dir, ns)| {
                        let mut w: HashSet<String> = ident_words(&dir).into_iter().collect();
                        for &i in ns.iter().take(30) {
                            w.extend(ident_words(ix.label(i)));
                        }
                        let h = subject.iter().filter(|t| w.contains(*t)).count();
                        (h, ns.len(), dir, ns)
                    })
                    .collect();
                subs.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
                let _ = writeln!(s, "Inside {area}/ ({} parts):", subs.len());
                for (_, n, dir, ns) in subs.iter().take(AREAS) {
                    let mut ns = ns.clone();
                    ns.sort_by_key(|&i| {
                        (
                            std::cmp::Reverse(user_files(ix, i)),
                            std::cmp::Reverse(ix.degree(i)),
                            i,
                        )
                    });
                    let mut seen = HashSet::new();
                    let names: Vec<&str> = ns
                        .iter()
                        .map(|&i| ix.label(i))
                        .filter(|l| seen.insert(*l))
                        .take(4)
                        .collect();
                    let _ = writeln!(s, "  {dir}/ ({n} symbols): {}", names.join(", "));
                }
            }
        }
    }
    s
}

/// The `context` answer for `question` within about `budget` tokens; `None` when nothing in the
/// graph matches it.
pub fn render(ix: &Index, root: &Path, question: &str, budget: usize) -> Option<String> {
    let mut src = Sources {
        root: root.to_path_buf(),
        files: HashMap::new(),
    };
    // Named symbols first; for a code question that names none, the functions whose text
    // matches it; otherwise `query`'s seeds (documents keep their facts).
    let mut hits: HashMap<u32, Vec<(usize, String)>> = HashMap::new();
    let (mut anchors, weak, examples) = named_anchors_split(ix, question);
    let named = !anchors.is_empty();
    let mut chain_text = String::new();
    let has_code = (0..ix.node_count() as u32).any(|i| is_code(ix, i));
    // A question that names symbols may still be about code it does not name ("which steps are
    // affected if X changes"): the two best code-text matches follow the named ones.
    if !anchors.is_empty() && has_code {
        let terms = code_terms(question);
        if !terms.is_empty() {
            let scan = Scan::new(ix, root, terms.clone());
            let mut added = 0;
            for (node, lines) in scan.rank(ix, &mut src, &terms) {
                if added == AFTER_NAMED {
                    break;
                }
                if !anchors.contains(&node) {
                    anchors.push(node);
                    hits.insert(node, lines);
                    added += 1;
                }
            }
        }
    }
    if anchors.is_empty() && has_code {
        // A flow question: find each end, then the call chain between them.
        let whole = code_terms(question);
        let ends = flow_ends(question).map(|(a, b)| (code_terms(&a), code_terms(&b)));
        let mut all = whole.clone();
        if let Some((a, b)) = &ends {
            for t in a.iter().chain(b) {
                if !all.contains(t) {
                    all.push(t.clone());
                }
            }
        }
        let scan = Scan::new(ix, root, all);
        if let Some((start, end)) = &ends {
            let a = scan.rank(ix, &mut src, start);
            let b = scan.rank(ix, &mut src, end);
            let froms: Vec<u32> = a.iter().take(2).map(|x| x.0).collect();
            let tos: Vec<u32> = b.iter().take(2).map(|x| x.0).collect();
            if let Some(chain) = call_chain(ix, &froms, &tos) {
                chain_text = render_chain(ix, &chain);
            }
            for (node, lines) in a.into_iter().take(1).chain(b.into_iter().take(1)) {
                if !anchors.contains(&node) {
                    anchors.push(node);
                    hits.insert(node, lines);
                }
            }
        }
        for (node, lines) in scan.rank(ix, &mut src, &whole) {
            if anchors.len() >= TEXT_ANCHORS {
                break;
            }
            if !anchors.contains(&node) {
                anchors.push(node);
                hits.insert(node, lines);
            }
        }
    }
    // Names from a code example join what the search found; they do not replace it.
    for i in examples {
        if !anchors.contains(&i) {
            anchors.push(i);
        }
    }
    // Bare CamelCase names the question mentions come after what the search found.
    for i in weak {
        if anchors.len() < MAX_ANCHORS + 1 && !anchors.contains(&i) {
            anchors.push(i);
        }
    }
    if anchors.is_empty() {
        anchors = anchors_from_seeds(ix, question);
    }
    if anchors.is_empty() {
        return None;
    }
    let limit = budget.max(200) * 4;
    let flow = flow_ends(question).is_some();
    let mut out = String::new();
    let mut cut = 0;
    for (rank, &i) in anchors.iter().enumerate() {
        let none = Vec::new();
        // The first two anchors show their definition; later ones only its first lines.
        let def = if rank < 2 { DEF_LINES } else { 2 };
        let sec = section(ix, &mut src, i, hits.get(&i).unwrap_or(&none), def, flow);
        if !out.is_empty() && out.len() + sec.len() > limit {
            cut += 1;
            continue;
        }
        out.push_str(&sec);
        out.push('\n');
    }
    if !chain_text.is_empty() {
        out.push_str(&chain_text);
        out.push('\n');
    }
    // A question that names a symbol is about that symbol, whatever words frame it.
    if !named && asks_structure(question) && anchors.iter().any(|&i| is_code(ix, i)) {
        let map = overview(ix, question);
        if out.len() + map.len() <= limit + 1200 {
            out.push_str(&map);
            out.push('\n');
        }
    }
    if has_code {
        let names = name_list(ix, question, &anchors);
        if !names.is_empty() {
            out.push_str(&names);
            out.push('\n');
        }
    }
    if anchors.iter().any(|&i| !is_code(ix, i)) {
        let docs: Vec<String> = (0..ix.node_count() as u32)
            .filter(|&i| !is_code(ix, i))
            .map(|i| ix.source_file(i).to_string())
            .filter(|f| !f.is_empty())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let w = Weights::new(question, &mut src, &docs);
        out.push_str(&search_hint(&w));
    }
    if cut > 0 {
        let _ = writeln!(
            out,
            "({cut} more matching symbol(s) left out by the budget; raise --budget or `dijkstral explain` them)"
        );
    }
    Some(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_code_like_names() {
        assert_eq!(
            named_symbols("Which functions in the fastapi package call solve_dependencies?"),
            ["solve_dependencies"]
        );
        assert_eq!(
            named_symbols("If the Sink trait changed, which implementations must be updated?"),
            ["Sink"]
        );
        assert_eq!(
            named_symbols("Which non-test code calls Override::matched from the ignore crate?"),
            ["Override::matched"]
        );
        assert_eq!(
            named_symbols(
                "How does `include_router` apply a prefix, and APIRouter.include_router()?"
            ),
            ["include_router", "APIRouter.include_router()"]
        );
        assert_eq!(
            named_symbols("Which parts call util.PatchPodStatus?"),
            ["util.PatchPodStatus"]
        );
    }

    #[test]
    fn backticked_code_gives_its_names() {
        assert_eq!(
            span_names("responseDecodable(of:)"),
            ["responseDecodable()"]
        );
        assert_eq!(
            span_names("APIRouter.include_router()"),
            ["APIRouter.include_router()"]
        );
        assert_eq!(
            span_names(
                "AF.request(url, method: .post, parameters: params, encoding: JSONEncoding.default)"
            ),
            ["AF.request()", "JSONEncoding.default"]
        );
        assert_eq!(
            span_names(r#"using (LogContext.PushProperty("OrderId", id)) { ... }"#),
            ["LogContext.PushProperty()"]
        );
        assert_eq!(span_names("ForContext<T>()"), ["ForContext()"]);
        assert_eq!(span_names("write-to:File.path"), ["File.path"]);
        assert!(span_names(r#"import "mylib" as lib;"#).is_empty());
        assert!(span_names(".a.b |= . + 1").is_empty());
    }

    #[test]
    fn stems_and_subjects() {
        assert_eq!(stem("babies"), "baby");
        assert_eq!(stem("camping"), "camp");
        assert_eq!(stem("friends"), "friend");
        assert_eq!(stem("class"), "class");
        assert!(matches("babies", "baby"));
        assert!(!matches("friendly", "friend"));
        assert_eq!(
            subject_stems("How many babies were born to friends in the last few months?"),
            ["baby", "born", "friend"]
        );
        let w = Weights {
            terms: vec![
                ("baby".into(), 3.0),
                ("born".into(), 2.0),
                ("x".into(), 0.0),
            ],
        };
        assert_eq!(
            search_hint(&w),
            "The graph summarises. For every occurrence (how many, all of, the latest), also run: \
             rg -n -i \"bab|born\"\n"
        );
    }

    #[test]
    fn label_lines_need_most_of_the_label() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("s.md"),
            "My aunt had twins.\nThe twins, Ava and Lily, were born in April.\nApril showers.\n",
        )
        .unwrap();
        let mut src = Sources {
            root: dir.path().to_path_buf(),
            files: HashMap::new(),
        };
        assert_eq!(
            label_lines(&mut src, "s.md", "Aunt's twins Ava and Lily born in April"),
            [(
                2,
                "The twins, Ava and Lily, were born in April.".to_string()
            )]
        );
        assert!(label_lines(&mut src, "missing.md", "anything here").is_empty());
    }

    #[test]
    fn identifier_words_and_kinds() {
        let w = ident_words("body_bytes = await readJSONBody(request)");
        for want in ["body", "byte", "read", "json", "request"] {
            assert!(w.iter().any(|x| x == want), "{want} in {w:?}");
        }
        assert_eq!(
            kind_names("If the Sink trait changed, which struct Searcher uses it?"),
            ["Sink", "Searcher"]
        );
        assert!(is_comment("    # a comment"));
        assert!(is_comment(
            "    This flag controls whether binary files are searched at all."
        ));
        assert!(!is_comment("    if self.binary.is_explicit() {"));
        assert!(is_config("pyproject.toml") && !is_config("src/main.rs"));
        assert!(is_true_comment(
            "    /// Memory-map the file when it is large enough."
        ));
        assert!(!is_true_comment(
            "    This flag controls whether binary files are searched."
        ));
        assert_eq!(stem("explicitly"), "explicit");
        assert_eq!(
            spellings("configuration"),
            ["configuration", "config", "cfg", "conf"]
        );
        assert!(code_terms("Where is the memory map chosen?").contains(&"mmap".to_string()));
        assert_eq!(stem("embedded"), "embed");
        assert_eq!(stem("running"), "run");
        assert_eq!(stem("called"), "call");
        assert_eq!(stem("passed"), "pass");
    }

    #[test]
    fn structure_questions_get_a_map_of_code_areas() {
        let node = |id: &str, label: &str, file: &str| {
            serde_json::json!({"id": id, "label": label, "file_type": "code",
                "source_file": file, "source_location": "L1"})
        };
        let doc = serde_json::json!({
            "directed": false, "multigraph": false, "graph": {},
            "nodes": [
                node("a", "walk()", "crates/ignore/src/walk.rs"),
                node("b", "Searcher", "crates/searcher/src/lib.rs"),
                node("c", "StandardSink", "crates/printer/src/standard.rs"),
                node("d", "main()", "crates/core/main.rs"),
            ],
            "links": []
        });
        let ix = Index::from_graph_json(doc.to_string().as_bytes()).unwrap();
        assert!(asks_structure(
            "How is the work divided among its library crates?"
        ));
        assert!(!asks_structure("Who calls walk?"));
        let map = overview(
            &ix,
            "Which crate walks directories and applies ignore rules?",
        );
        assert!(
            map.starts_with("Code areas (4), those matching the question first:\n"),
            "{map}"
        );
        assert_eq!(
            map.lines().nth(1),
            Some("  crates/ignore/ (1 symbols): walk()"),
            "{map}"
        );
        assert!(
            map.contains("  crates/printer/ (1 symbols): StandardSink\n"),
            "{map}"
        );
    }

    #[test]
    fn flow_questions_have_two_ends() {
        assert_eq!(
            flow_ends("Trace a file from the walker that yields it to the printer that writes it."),
            Some((
                "the walker that yields it".into(),
                "the printer that writes it.".into()
            ))
        );
        assert_eq!(
            flow_ends("How does a return annotation end up as a schema in the document?"),
            Some((
                "how does a return annotation".into(),
                "a schema in the document?".into()
            ))
        );
        assert_eq!(flow_ends("Who calls solve_dependencies?"), None);
    }

    #[test]
    fn flow_questions_list_what_callers_also_call() {
        let node = |id: &str, label: &str, line: u32| {
            serde_json::json!({"id": id, "label": label, "file_type": "code",
                "source_file": "src/run.py", "source_location": format!("L{line}")})
        };
        let call = |s: &str, t: &str, line: u32| {
            serde_json::json!({"source": s, "target": t, "relation": "calls",
                "source_file": "src/run.py", "source_location": format!("L{line}")})
        };
        let doc = serde_json::json!({
            "directed": false, "multigraph": false, "graph": {},
            "nodes": [
                node("run", "run()", 1),
                node("walk", "walk_entries()", 10),
                node("build", "build_haystack()", 20),
                node("search", "search_file()", 30),
            ],
            "links": [
                call("run", "search", 4),
                call("run", "walk", 2),
                call("run", "build", 3),
            ]
        });
        let ix = Index::from_graph_json(doc.to_string().as_bytes()).unwrap();
        let search = (0..ix.node_count() as u32)
            .find(|&i| ix.label(i) == "search_file()")
            .unwrap();
        let mut src = Sources {
            root: PathBuf::from("/nonexistent"),
            files: HashMap::new(),
        };
        let flow = section(&ix, &mut src, search, &[], 2, true);
        assert!(
            flow.contains("      which also calls: walk_entries(), build_haystack()\n"),
            "{flow}"
        );
        let plain = section(&ix, &mut src, search, &[], 2, false);
        assert!(!plain.contains("which also calls"), "{plain}");
    }

    #[test]
    fn code_examples_are_examples_and_bare_names_are_not() {
        assert_eq!(
            example_names(
                "How does `x: Annotated[str, Depends(get_x)]` reach `solve_dependencies`?"
            ),
            ["Depends()"]
        );
        assert!(example_names("Which code calls `solve_dependencies` or `Sink`?").is_empty());
    }

    #[test]
    fn names_sharing_the_question_words_are_listed_by_file() {
        let node = |id: &str, label: &str, file: &str| {
            serde_json::json!({"id": id, "label": label, "file_type": "code",
                "source_file": file, "source_location": "L1"})
        };
        let doc = serde_json::json!({
            "directed": false, "multigraph": false, "graph": {},
            "nodes": [
                node("agg", "SafeAggregateSink", "src/Core/Sinks/SafeAggregateSink.cs"),
                node("filt", "FilteringSink", "src/Core/Sinks/FilteringSink.cs"),
                node("log", "Logger", "src/Core/Logger.cs"),
                node("write", ".Write()", "src/Core/Logger.cs"),
                node("t", "FilteringSinkTests", "test/Core/FilteringSinkTests.cs"),
                node("cfg", "LoggerConfiguration", "src/LoggerConfiguration.cs"),
            ],
            "links": []
        });
        let ix = Index::from_graph_json(doc.to_string().as_bytes()).unwrap();
        let shown: Vec<u32> = (0..ix.node_count() as u32)
            .filter(|&i| ix.label(i) == "Logger")
            .collect();
        let list = name_list(
            &ix,
            "How are sinks filtered before a logger writes?",
            &shown,
        );
        assert_eq!(
            list,
            "names sharing the question's words:\n  \
             src/Core/Sinks/FilteringSink.cs: FilteringSink\n  \
             src/Core/Logger.cs: Write()\n  \
             src/LoggerConfiguration.cs: LoggerConfiguration\n  \
             src/Core/Sinks/SafeAggregateSink.cs: SafeAggregateSink\n"
        );
    }

    #[test]
    fn locations_and_paths() {
        assert_eq!(line_no("L586"), Some(586));
        assert_eq!(line_no("L12-L20"), Some(12));
        assert_eq!(line_no("Sheet: Capacity"), None);
        assert!(is_test_path("tests/test_routing.py"));
        assert!(is_test_path("pkg/scheduler/framework_test.go"));
        assert!(!is_test_path("fastapi/routing.py"));
        assert_eq!(
            root_of(Path::new("/r/dijkstral-out/graph.dij"), Path::new("/cwd")),
            PathBuf::from("/r")
        );
        assert_eq!(
            root_of(Path::new("elsewhere/graph.json"), Path::new("/cwd")),
            PathBuf::from("/cwd")
        );
    }
}

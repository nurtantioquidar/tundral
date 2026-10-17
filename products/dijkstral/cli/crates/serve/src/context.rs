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
const DEF_LINES: usize = 6;
const MAX_CALLERS: usize = 12;
const MAX_CALLEES: usize = 10;
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
        push(&after[..b]);
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
fn named_anchors_split(ix: &Index, question: &str) -> (Vec<u32>, Vec<u32>) {
    let explicit_marks =
        |n: &str| n.contains('_') || n.contains('.') || n.contains("::") || n.ends_with("()");
    let mut backticked: Vec<String> = named_symbols(&backticks_only(question));
    backticked.extend(kind_names(question));
    let hub = query::hub_threshold(ix);
    let (mut strong, mut weak): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
    for name in named_symbols(question) {
        let explicit = explicit_marks(&name) || backticked.contains(&name);
        for i in resolve_name(ix, &name) {
            if is_file_node(ix, i) && !name.contains('/') && !name.contains(".py") {
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
    (strong, weak)
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

/// One anchor's section.
fn section(
    ix: &Index,
    src: &mut Sources,
    i: u32,
    hits: &[(usize, String)],
    def_lines: usize,
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
        for &(c, e) in core.iter().take(MAX_CALLERS) {
            let edge = ix.edge(e);
            let file = edge
                .source_file
                .filter(|f| !f.is_empty())
                .unwrap_or_else(|| ix.source_file(c));
            let line = line_no(edge.source_location.unwrap_or(""));
            let at = line.map_or(file.to_string(), |l| format!("{file}:{l}"));
            let code = line
                .and_then(|l| src.line(file, l))
                .map(|t| format!("  | {}", t.trim()))
                .unwrap_or_default();
            let _ = writeln!(s, "    {} [{}] {at}{code}", display(ix, c), ix.relation(e));
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

/// Per node: how often each question term occurs, and its matching lines with the terms each
/// carries (a bit per term).
type NodeHits = (Vec<u32>, Vec<(usize, u64)>);
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
        let cs: Vec<char> = tok.chars().collect();
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
        if start > 0 {
            out.push(stem(&cs[start..].iter().collect::<String>().to_lowercase()));
        }
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
        type FileHits = (usize, Vec<usize>, Vec<(u32, usize, u64)>);
        let results: Vec<FileHits> = files
            .par_iter()
            .map(|f| {
                let mut df = vec![0usize; terms.len()];
                let mut hits: Vec<(u32, usize, u64)> = Vec::new();
                let path = root.join(f.as_str());
                let ok = std::fs::metadata(&path)
                    .is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE_BYTES);
                let Some(bytes) = ok.then(|| std::fs::read(&path).ok()).flatten() else {
                    return (0, df, hits);
                };
                let text = String::from_utf8_lossy(&bytes);
                let starts = &spans[f.as_str()];
                let mut n_lines = 0;
                for (k, line) in text.lines().enumerate() {
                    n_lines += 1;
                    let lower = line.to_ascii_lowercase();
                    if !finders.iter().any(|fd| fd.find(lower.as_bytes()).is_some()) {
                        continue;
                    }
                    if is_comment(line) {
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
                        hits.push((starts[p].1, n, mask));
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
            for (node, line, mask) in hits {
                let e = per
                    .entry(node)
                    .or_insert_with(|| (vec![0; terms.len()], Vec::new()));
                for (t, c) in e.0.iter_mut().enumerate() {
                    if mask & (1 << t) != 0 {
                        *c += 1;
                    }
                }
                e.1.push((line, mask));
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
        let strength = |node: u32, counts: &[u32]| -> (f64, usize) {
            let on = |t: usize| mask & (1 << t) != 0 && counts[t] > 0;
            let distinct = (0..self.terms.len()).filter(|&t| on(t)).count();
            let weight: f64 = (0..self.terms.len())
                .filter(|&t| on(t))
                .map(|t| idf[t] * (1.0 + (counts[t] as f64).ln()))
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
            .map(|(&node, (counts, _))| (node, strength(node, counts)))
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
                    .1
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
    let (mut anchors, weak) = named_anchors_split(ix, question);
    let named = !anchors.is_empty();
    let mut chain_text = String::new();
    if anchors.is_empty() && (0..ix.node_count() as u32).any(|i| is_code(ix, i)) {
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
    let mut out = String::new();
    let mut cut = 0;
    for (rank, &i) in anchors.iter().enumerate() {
        let none = Vec::new();
        // The first two anchors show their definition; later ones only its first lines.
        let def = if rank < 2 { DEF_LINES } else { 2 };
        let sec = section(ix, &mut src, i, hits.get(&i).unwrap_or(&none), def);
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

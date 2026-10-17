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

/// The nodes the question names (code-like words resolved as `explain` does).
pub fn named_anchors(ix: &Index, question: &str) -> Vec<u32> {
    let explicit_marks =
        |n: &str| n.contains('_') || n.contains('.') || n.contains("::") || n.ends_with("()");
    let mut backticked: Vec<String> = named_symbols(&backticks_only(question));
    backticked.extend(kind_names(question));
    let hub = query::hub_threshold(ix);
    let mut out: Vec<u32> = Vec::new();
    for name in named_symbols(question) {
        let explicit = explicit_marks(&name) || backticked.contains(&name);
        for i in resolve_name(ix, &name) {
            if is_file_node(ix, i) && !name.contains('/') && !name.contains(".py") {
                continue;
            }
            // A bare CamelCase word (`FastAPI`, `Sink`) is a symbol only when a label spells it
            // exactly and it is not a hub every part of the code touches.
            if !explicit {
                let label = ix.label(i).trim_start_matches('.').trim_end_matches("()");
                if label != name || f64::from(ix.degree(i)) >= hub {
                    continue;
                }
            }
            if !out.contains(&i) {
                out.push(i);
            }
        }
    }
    out.truncate(MAX_ANCHORS);
    out
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
        b.to_string()
    } else if let Some(b) = w.strip_suffix("ed").filter(|_| n > 4) {
        b.to_string()
    } else if let Some(b) = w.strip_suffix("es").filter(|_| n > 4 && w.ends_with("ses")) {
        b.to_string()
    } else if let Some(b) = w.strip_suffix('s').filter(|_| n > 3 && !w.ends_with("ss")) {
        b.to_string()
    } else {
        w.to_string()
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

/// Per node: how often each question term occurs, and its matching lines with their counts of
/// distinct terms.
type NodeHits = (Vec<u32>, Vec<(usize, usize)>);
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
fn code_search(ix: &Index, src: &mut Sources, question: &str) -> Vec<(u32, Vec<(usize, String)>)> {
    let terms: Vec<String> = subject_stems(question)
        .into_iter()
        .filter(|t| !CODE_SCAFFOLD.iter().any(|c| t == c))
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let need = terms.len().min(2);
    let spans = code_spans(ix);
    let mut files: Vec<&String> = spans.keys().collect();
    files.sort();
    let mut df = vec![0usize; terms.len()];
    let mut lines_seen = 0usize;
    // node → (per-term counts, matching lines (line, distinct terms))
    let mut per: HashMap<u32, NodeHits> = HashMap::new();
    for f in files {
        let Some(lines) = src.lines(f) else { continue };
        let starts = &spans[f];
        for (k, line) in lines.iter().enumerate() {
            lines_seen += 1;
            if is_comment(line) {
                continue;
            }
            let ws = ident_words(line);
            let hit: Vec<usize> = (0..terms.len())
                .filter(|&t| ws.iter().any(|w| *w == terms[t]))
                .collect();
            if hit.is_empty() {
                continue;
            }
            for &t in &hit {
                df[t] += 1;
            }
            let n = k + 1;
            let pos = starts.partition_point(|&(l, _)| l <= n);
            let Some(&(_, node)) = pos.checked_sub(1).map(|p| &starts[p]) else {
                continue;
            };
            let e = per
                .entry(node)
                .or_insert_with(|| (vec![0; terms.len()], Vec::new()));
            for &t in &hit {
                e.0[t] += 1;
            }
            e.1.push((n, hit.len()));
        }
    }
    let idf: Vec<f64> = df
        .iter()
        .map(|&d| ((lines_seen as f64 + 1.0) / (d as f64 + 1.0)).ln().max(0.0))
        .collect();
    // A node's span runs to the next node of its file; long spans are damped so a dense match
    // in a focused function beats scattered words in a long one.
    let span_len = |node: u32| -> f64 {
        let file = ix.source_file(node);
        let start = line_no(ix.node(node).source_location).unwrap_or(1);
        let next = spans[file]
            .iter()
            .map(|&(l, _)| l)
            .find(|&l| l > start)
            .unwrap_or(start + 200);
        (next - start) as f64
    };
    let mut ranked: Vec<(f64, bool, u32)> = per
        .iter()
        .filter_map(|(&node, (counts, _))| {
            let distinct = counts.iter().filter(|&&c| c > 0).count();
            if distinct < need {
                return None;
            }
            let weight: f64 = counts
                .iter()
                .zip(&idf)
                .filter(|(&c, _)| c > 0)
                .map(|(&c, w)| w * (1.0 + (c as f64).ln()))
                .sum();
            let score = weight * distinct as f64 / (1.0 + span_len(node) / 40.0).sqrt();
            Some((score, is_test_node(ix, node), node))
        })
        .collect();
    // Tests last; then score; then node order.
    ranked.sort_by(|a, b| a.1.cmp(&b.1).then(b.0.total_cmp(&a.0)).then(a.2.cmp(&b.2)));
    ranked
        .into_iter()
        .take(TEXT_ANCHORS)
        .map(|(_, _, node)| {
            let mut hits = per[&node].1.clone();
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
    let mut anchors: Vec<u32> = named_anchors(ix, question);
    if anchors.is_empty() && (0..ix.node_count() as u32).any(|i| is_code(ix, i)) {
        for (node, lines) in code_search(ix, &mut src, question) {
            anchors.push(node);
            hits.insert(node, lines);
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

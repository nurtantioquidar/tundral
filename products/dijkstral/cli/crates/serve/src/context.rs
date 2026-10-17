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

fn is_test_path(p: &str) -> bool {
    let p = p.to_ascii_lowercase();
    p.split('/').any(|seg| {
        seg == "test" || seg == "tests" || seg.starts_with("test_") || seg.ends_with("_test.go")
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
    let found = pick(resolve::resolve(ix, name));
    if !found.is_empty() {
        return found;
    }
    let last = name
        .trim_end_matches("()")
        .rsplit(['.', ':'])
        .next()
        .unwrap_or(name);
    if last != name && last.len() >= 3 {
        return pick(resolve::resolve(ix, last));
    }
    Vec::new()
}

/// The nodes the answer is about.
pub fn anchors(ix: &Index, question: &str) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for name in named_symbols(question) {
        for i in resolve_name(ix, &name) {
            if !out.contains(&i) {
                out.push(i);
            }
        }
    }
    if out.is_empty() {
        let terms = query::terms(question);
        out = query::seeds(ix, &terms);
    }
    // A fact node of a document costs one line, so documents get more of them; but only nodes
    // that share a subject word with the question (not just "few" or "years").
    let docs = !out.is_empty() && out.iter().all(|&i| !is_code(ix, i));
    if docs {
        let subject = subject_stems(question);
        out.retain(|&i| {
            let ws: Vec<String> = words(ix.label(i)).collect();
            subject.iter().any(|t| ws.iter().any(|w| matches(w, t)))
        });
    }
    let cap = if docs { MAX_DOC_ANCHORS } else { MAX_ANCHORS };
    out.truncate(cap);
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
    if let Some(b) = w.strip_suffix("ies").filter(|_| n > 4) {
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
fn section(ix: &Index, src: &mut Sources, i: u32) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "## {}  {}", display(ix, i), place(ix, i));
    let node = ix.node(i);
    if is_code(ix, i) && !is_file_node(ix, i) {
        if let Some(start) = line_no(node.source_location) {
            if let Some(lines) = src.lines(node.source_file) {
                for (k, l) in lines.iter().enumerate().skip(start - 1).take(DEF_LINES) {
                    let _ = writeln!(s, "  {:>5}| {l}", k + 1);
                }
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
        for &(c, e) in callers.iter().take(MAX_CALLERS) {
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
        if callers.len() > MAX_CALLERS {
            let _ = writeln!(
                s,
                "    … {} more: `dijkstral explain <symbol> --incoming --all`",
                callers.len() - MAX_CALLERS
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

/// The `context` answer for `question` within about `budget` tokens; `None` when nothing in the
/// graph matches it.
pub fn render(ix: &Index, root: &Path, question: &str, budget: usize) -> Option<String> {
    let anchors = anchors(ix, question);
    if anchors.is_empty() {
        return None;
    }
    let mut src = Sources {
        root: root.to_path_buf(),
        files: HashMap::new(),
    };
    let limit = budget.max(200) * 4;
    let mut out = String::new();
    let mut cut = 0;
    for &i in &anchors {
        let sec = section(ix, &mut src, i);
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
            Vec::<String>::new()
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

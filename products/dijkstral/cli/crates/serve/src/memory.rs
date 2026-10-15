//! Saved results, the files of the work-memory loop (`docs/graphify-memory.md` §2–§4.3): written
//! by `save-result`, read by `reflect`, indexed by `update`.
//!
//! The file format is Graphify's, so either tool reads the other's results: a front matter of
//! double-quoted values in a fixed key order, then a Markdown body repeating the values.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::clock::Instant;

/// Where results go by default, relative to the project.
pub const MEMORY_DIR: &str = dijkstral_core::discover::MEMORY_DIR;
/// The `contributor` value of results dijkstral writes.
pub const CONTRIBUTOR: &str = "dijkstral";
/// Slugs keep this many characters of the lowercased question.
const SLUG_CHARS: usize = 50;

/// The work-memory signal of a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Useful,
    DeadEnd,
    Corrected,
}

impl Outcome {
    pub const ALL: [Outcome; 3] = [Outcome::Useful, Outcome::DeadEnd, Outcome::Corrected];

    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Useful => "useful",
            Outcome::DeadEnd => "dead_end",
            Outcome::Corrected => "corrected",
        }
    }

    /// Exact, lowercase.
    pub fn parse(s: &str) -> Option<Outcome> {
        Outcome::ALL.into_iter().find(|o| o.as_str() == s)
    }
}

/// One result to save.
#[derive(Debug, Clone, PartialEq)]
pub struct NewResult {
    pub question: String,
    pub answer: String,
    /// `--type`: `query`, `path_query`, `explain` or anything else.
    pub kind: String,
    pub nodes: Vec<String>,
    pub outcome: Option<Outcome>,
    /// Dropped when empty.
    pub correction: Option<String>,
}

/// A front-matter value: double-quoted, with the escapes of §2.4.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\L"),
            '\u{2029}' => out.push_str("\\P"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The file text of a result saved at `date` (`2026-10-15T15:56:41.434286+00:00`). No trailing
/// newline, as Graphify writes it.
pub fn render(r: &NewResult, date: &str) -> String {
    let correction = r.correction.as_deref().filter(|c| !c.is_empty());
    let mut s = String::new();
    s.push_str("---\n");
    let _ = writeln!(s, "type: {}", quote(&r.kind));
    let _ = writeln!(s, "date: {}", quote(date));
    let _ = writeln!(s, "question: {}", quote(&r.question));
    let _ = writeln!(s, "contributor: {}", quote(CONTRIBUTOR));
    if let Some(o) = r.outcome {
        let _ = writeln!(s, "outcome: {}", quote(o.as_str()));
    }
    if let Some(c) = correction {
        let _ = writeln!(s, "correction: {}", quote(c));
    }
    if !r.nodes.is_empty() {
        let items: Vec<String> = r.nodes.iter().map(|n| quote(n)).collect();
        let _ = writeln!(s, "source_nodes: [{}]", items.join(", "));
    }
    s.push_str("---\n\n");
    let _ = write!(s, "# Q: {}\n\n## Answer\n\n{}", r.question, r.answer);
    if r.outcome.is_some() || correction.is_some() {
        s.push_str("\n\n## Outcome\n");
        if let Some(o) = r.outcome {
            let _ = write!(s, "\n- Signal: {}", o.as_str());
        }
        if let Some(c) = correction {
            let _ = write!(s, "\n- Correction: {c}");
        }
    }
    if !r.nodes.is_empty() {
        s.push_str("\n\n## Source Nodes\n");
        for n in &r.nodes {
            let _ = write!(s, "\n- {n}");
        }
    }
    s
}

/// The file-name slug of a question (§2.2): lowercased, every non-word character replaced by `_`,
/// the first 50 characters, `_` stripped from both ends.
pub fn slug(question: &str) -> String {
    let replaced: String = question
        .to_lowercase()
        .chars()
        .map(|c| {
            if c == '_' || c.is_alphanumeric() {
                c
            } else {
                '_'
            }
        })
        .take(SLUG_CHARS)
        .collect();
    replaced.trim_matches('_').to_string()
}

/// `query_<YYYYMMDD>_<HHMMSS>_<token>_<slug>.md`.
pub fn file_name(question: &str, at: Instant, token: &str) -> String {
    format!(
        "query_{}_{}_{}.md",
        at.compact_stamp(),
        token,
        slug(question)
    )
}

/// Eight hex digits that differ between calls (Graphify's are random too).
pub fn token(at: Instant) -> String {
    let mut h = RandomState::new().build_hasher();
    h.write_i64(at.secs);
    h.write_u32(at.micros);
    h.write_u32(std::process::id());
    format!("{:08x}", h.finish() as u32)
}

/// Write `r` into `dir` (created with its parents) and return the file's path (`dir` joined with
/// the name).
pub fn save(dir: &Path, r: &NewResult, at: Instant) -> Result<PathBuf> {
    if dir.exists() && !dir.is_dir() {
        bail!("{} exists and is not a directory", dir.display());
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let date = at.iso_micros();
    let text = render(r, &date);
    loop {
        let path = dir.join(file_name(&r.question, at, &token(at)));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(text.as_bytes())
                    .with_context(|| format!("writing {}", path.display()))?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("writing {}", path.display())),
        }
    }
}

/// A result as `reflect` reads it (§4.2–§4.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    /// The file name, which breaks ties between results with equal dates.
    pub file: String,
    /// The raw `date` value (`""` when absent or not a quoted string).
    pub date: String,
    pub question: String,
    pub outcome: Option<Outcome>,
    pub correction: Option<String>,
    /// Distinct names in first-cited order.
    pub nodes: Vec<String>,
}

/// Read one result file's text, or `None` when it is not a result (it must start with `---`).
pub fn parse(file: &str, text: &str) -> Option<Saved> {
    let body = text.strip_prefix("---")?;
    let mut lines = body.split('\n');
    // The rest of the opening line belongs to it, whatever it is.
    lines.next();
    let mut saved = Saved {
        file: file.to_string(),
        date: String::new(),
        question: String::new(),
        outcome: None,
        correction: None,
        nodes: Vec::new(),
    };
    for line in lines {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line == "---" {
            break;
        }
        let Some((key, value)) = key_value(line) else {
            continue;
        };
        match key {
            "date" => {
                if let Some(v) = quoted(value) {
                    saved.date = v;
                }
            }
            "question" => {
                if let Some(v) = quoted(value) {
                    saved.question = v;
                }
            }
            "outcome" => saved.outcome = quoted(value).and_then(|v| Outcome::parse(&v)),
            "correction" => saved.correction = quoted(value),
            "source_nodes" => saved.nodes = name_list(value),
            _ => {}
        }
    }
    Some(saved)
}

/// `key: value` at column 0, the value trimmed.
fn key_value(line: &str) -> Option<(&str, &str)> {
    let end = line
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(line.len());
    if end == 0 {
        return None;
    }
    let (key, rest) = line.split_at(end);
    let rest = rest.trim_start_matches([' ', '\t']).strip_prefix(':')?;
    Some((key, rest.trim()))
}

/// A whole double-quoted value, unescaped (§4.3).
fn quoted(v: &str) -> Option<String> {
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    if v.len() < 2 {
        return None;
    }
    // A quote inside must be escaped.
    let mut escaped = false;
    for c in inner.chars() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return None,
            _ => escaped = false,
        }
    }
    if escaped {
        return None;
    }
    Some(unescape(inner))
}

/// The escapes `save-result` writes, plus `\0` and `\uNNNN`; any other backslash pair is kept.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(&n) = chars.peek() else {
            out.push('\\');
            break;
        };
        let simple = match n {
            '\\' => Some('\\'),
            '"' => Some('"'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            '0' => Some('\0'),
            'L' => Some('\u{2028}'),
            'P' => Some('\u{2029}'),
            _ => None,
        };
        if let Some(r) = simple {
            chars.next();
            out.push(r);
            continue;
        }
        let width = match n {
            'x' => 2,
            'u' => 4,
            _ => 0,
        };
        if width > 0 {
            let mut probe = chars.clone();
            probe.next();
            let hex: String = probe.by_ref().take(width).collect();
            if hex.len() == width && hex.chars().all(|h| h.is_ascii_hexdigit()) {
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                    chars = probe;
                    continue;
                }
            }
        }
        out.push('\\');
    }
    out
}

/// A one-line JSON array of names: strings kept (first occurrence), other items dropped; anything
/// that is not such an array gives no names. The `\xNN`, `\L` and `\P` escapes `save-result`
/// writes are accepted inside the strings.
fn name_list(v: &str) -> Vec<String> {
    let mut json = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            json.push(c);
            continue;
        }
        match chars.next() {
            Some('L') => json.push_str("\\u2028"),
            Some('P') => json.push_str("\\u2029"),
            Some('x') => json.push_str("\\u00"),
            Some(n) => {
                json.push('\\');
                json.push(n);
            }
            None => json.push('\\'),
        }
    }
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&json)
    else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for item in items {
        if let serde_json::Value::String(s) = item {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Every result directly in `dir` (`*.md`, extension case-sensitive), sorted by file name. A
/// missing or unreadable directory has none.
pub fn load_dir(dir: &Path) -> Vec<Saved> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            name.ends_with(".md").then(|| (name, e.path()))
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|(name, path)| {
            let bytes = fs::read(&path).ok()?;
            parse(&name, &String::from_utf8_lossy(&bytes))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> NewResult {
        NewResult {
            question: "How does auth work?".into(),
            answer: "Via login".into(),
            kind: "query".into(),
            nodes: vec!["UserService".into(), "login()".into()],
            outcome: Some(Outcome::Useful),
            correction: None,
        }
    }

    #[test]
    fn slugs_follow_the_word_rule() {
        assert_eq!(slug("How does auth work?"), "how_does_auth_work");
        assert_eq!(slug("Hello, World!"), "hello__world");
        assert_eq!(slug("!!!"), "");
        assert_eq!(slug("ẞ x"), "ß_x");
        let q = format!("{}!xyz", "9".repeat(49));
        assert_eq!(slug(&q), "9".repeat(49));
    }

    #[test]
    fn quoting_and_unquoting_round_trip() {
        let s = "q \" \\ \t\n\r\u{1}\u{7f}\u{2028}\u{2029}\u{85}é";
        let q = quote(s);
        assert_eq!(q, "\"q \\\" \\\\ \\t\\n\\r\\x01\\x7f\\L\\P\u{85}é\"");
        assert_eq!(quoted(&q).as_deref(), Some(s));
        assert_eq!(unescape("a\\qb\\N\\u00e9\\x41"), "a\\qb\\NéA");
        assert_eq!(quoted("'single'"), None);
        assert_eq!(quoted("\"a\"b\""), None);
    }

    #[test]
    fn renders_and_parses_a_result() {
        let text = render(&result(), "2026-01-01T00:00:00.000000+00:00");
        assert!(text.ends_with("- login()"));
        let saved = parse("x.md", &text).unwrap();
        assert_eq!(saved.question, "How does auth work?");
        assert_eq!(saved.outcome, Some(Outcome::Useful));
        assert_eq!(saved.nodes, ["UserService", "login()"]);
        assert_eq!(saved.date, "2026-01-01T00:00:00.000000+00:00");
    }

    #[test]
    fn reads_tolerantly() {
        let s = parse("a.md", "---\r\nquestion:\"q\"\r\noutcome: \"useful\" \r\nsource_nodes: [1, \"A\", \"A\", \"\"]\r\n---\r\noutcome: \"dead_end\"").unwrap();
        assert_eq!(s.outcome, Some(Outcome::Useful));
        assert_eq!(s.nodes, ["A", ""]);
        assert!(parse("b.md", "\u{feff}---\nquestion: \"q\"\n---").is_none());
        let s = parse("c.md", "---\n  outcome: \"useful\"\nsource_nodes: [A]\n").unwrap();
        assert_eq!((s.outcome, s.nodes.len()), (None, 0));
    }
}

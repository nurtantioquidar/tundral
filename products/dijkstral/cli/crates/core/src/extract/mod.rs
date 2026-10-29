//! Per-file extraction: one tree-sitter parse, one walk, then the tree is dropped.
//!
//! Each language module exposes `extract(&mut Ctx, Option<&Tree>)` and owns a serializable facts
//! type (`LangFacts` variant) for the cross-file pass in `crate::resolve`. Node and edge shapes are
//! specified in `docs/graphify-format.md`; use the `Ctx` constructors so shapes stay uniform.

pub mod c;
pub mod csharp;
pub mod dart;
pub mod data;
pub mod dotnet;
pub mod elixir;
pub mod fortran;
pub mod go;
pub mod groovy;
pub mod java;
pub mod julia;
pub mod kotlin;
pub mod legacy;
pub mod lua;
pub mod objc;
pub mod php;
pub mod powershell;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod scala;
pub mod script;
pub mod sfc;
pub mod sql;
pub mod swift;
pub mod verilog;
pub mod xml;
pub mod zig;

use tree_sitter::Node as TsNode;

use crate::facts::{Attr, Confidence, Edge, FileFacts, Loc, Node, NodeKind};
use crate::ids::{file_id, normalize};
use crate::lang::{parse, Lang};

/// Extract facts for one file. Never fails: unparsable input yields whatever the language emits
/// for an empty tree (normally just the file node).
pub fn extract_file(lang: Lang, rel: &str, source: &[u8]) -> FileFacts {
    let mut cx = Ctx::new(lang, rel, source);
    let tree = parse(lang, source);
    match lang {
        Lang::Python => python::extract(&mut cx, tree.as_ref()),
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => script::extract(&mut cx, tree.as_ref()),
        Lang::Rust => rust::extract(&mut cx, tree.as_ref()),
        Lang::Go => go::extract(&mut cx, tree.as_ref()),
        Lang::Java => java::extract(&mut cx, tree.as_ref()),
        Lang::CSharp => csharp::extract(&mut cx, tree.as_ref()),
        Lang::Php => php::extract(&mut cx, tree.as_ref()),
        Lang::Ruby => ruby::extract(&mut cx, tree.as_ref()),
        Lang::Kotlin => kotlin::extract(&mut cx, tree.as_ref()),
        Lang::Scala => scala::extract(&mut cx, tree.as_ref()),
        Lang::Swift => swift::extract(&mut cx, tree.as_ref()),
        Lang::C | Lang::Cpp => c::extract(&mut cx, tree.as_ref()),
        Lang::Elixir => elixir::extract(&mut cx, tree.as_ref()),
        Lang::Julia => julia::extract(&mut cx, tree.as_ref()),
        // Counted as code by Graphify, but they contribute nothing (§30, §31).
        Lang::Erlang | Lang::OCaml => {}
        Lang::Vue | Lang::Svelte | Lang::Astro => sfc::extract(&mut cx),
        Lang::PowerShell => powershell::extract(&mut cx, tree.as_ref()),
        // No file node and no symbols: Graphify's reference install has no VB.NET grammar (§42).
        Lang::VisualBasic => {}
        Lang::Zig => zig::extract(&mut cx, tree.as_ref()),
        Lang::ObjC => objc::extract(&mut cx, tree.as_ref()),
        Lang::Fortran => fortran::extract(&mut cx, tree.as_ref()),
        Lang::Verilog => verilog::extract(&mut cx, tree.as_ref()),
        // Graphify counts `.sol` files but has no grammar for them: no nodes at all (§37).
        Lang::Solidity => {}
        Lang::Sql => sql::extract(&mut cx, tree.as_ref()),
        Lang::Groovy => groovy::extract(&mut cx, tree.as_ref()),
        Lang::Dart => dart::extract(&mut cx),
        Lang::Lua => lua::extract(&mut cx, tree.as_ref()),
        Lang::GoMod => go::extract_mod(&mut cx),
        Lang::Json | Lang::Toml | Lang::Shell => data::extract(&mut cx),
        Lang::MsBuild | Lang::Sln | Lang::Slnx | Lang::Razor | Lang::Xaml => {
            dotnet::extract(&mut cx)
        }
        // Built by the markdown pass of a `pipeline::Documents` hook, never extracted here.
        Lang::Inert | Lang::Markdown => {}
        Lang::Pascal | Lang::DelphiForm | Lang::Lpk | Lang::Apex | Lang::Cobol | Lang::Byond => {
            legacy::extract(&mut cx)
        }
    }
    let mut facts = cx.facts;
    facts.words = count_words(source);
    facts
}

/// `len(text.split())` over the lossily decoded text: Python's `str.isspace` is Unicode
/// White_Space plus the ASCII separators U+001C–U+001F.
pub fn count_words(source: &[u8]) -> u64 {
    String::from_utf8_lossy(source)
        .split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|w| !w.is_empty())
        .count() as u64
}

/// `slug(s)` from the spec (§4.1).
pub fn slug(s: &str) -> String {
    normalize(s)
}

/// Shared extraction state for one file.
pub struct Ctx<'a> {
    pub lang: Lang,
    /// Repo-relative path with `/` separators.
    pub rel: &'a str,
    pub src: &'a [u8],
    /// `slug(path without last extension)`.
    pub file_id: String,
    pub facts: FileFacts,
}

impl<'a> Ctx<'a> {
    pub fn new(lang: Lang, rel: &'a str, src: &'a [u8]) -> Ctx<'a> {
        Ctx {
            lang,
            rel,
            src,
            file_id: file_id(rel),
            facts: FileFacts::default(),
        }
    }

    pub fn text(&self, node: TsNode) -> &'a str {
        std::str::from_utf8(&self.src[node.byte_range()]).unwrap_or("")
    }

    /// 1-based line of a tree-sitter node's start.
    pub fn line(node: TsNode) -> u32 {
        node.start_position().row as u32 + 1
    }

    /// `parent_slug(name)`: ids are always built by appending slugged parts to a parent id.
    pub fn child_id(parent: &str, name: &str) -> String {
        let s = slug(name);
        if s.is_empty() {
            parent.to_string()
        } else if parent.is_empty() {
            s
        } else {
            format!("{parent}_{s}")
        }
    }

    /// The file node (`label` is a placeholder; the build assigns the unique-suffix label).
    /// A 0-byte file gets `source_location: null` (§3).
    pub fn add_file_node(&mut self, extra: Vec<(String, Attr)>) {
        let loc = if self.src.is_empty() {
            Loc::Null
        } else {
            Loc::Line(1)
        };
        let mut attrs = extra;
        attrs.push(origin());
        let label = self.rel.rsplit('/').next().unwrap_or(self.rel).to_string();
        self.facts.nodes.push(Node {
            id: self.file_id.clone(),
            label,
            kind: NodeKind::File,
            file_type: "code".into(),
            source_file: self.rel.into(),
            loc,
            attrs,
        });
    }

    /// A definition node in this file (`_origin: "ast"` is added).
    pub fn add_def(&mut self, id: String, label: String, line: u32, extra: Vec<(String, Attr)>) {
        let mut attrs = extra;
        attrs.push(origin());
        self.facts.nodes.push(Node {
            id,
            label,
            kind: NodeKind::Def,
            file_type: "code".into(),
            source_file: self.rel.into(),
            loc: Loc::Line(line),
            attrs,
        });
    }

    /// An unresolved-reference stub: `{"id","label","file_type":"code","source_file":"","source_location":"","_origin":"ast"}`.
    pub fn add_stub(&mut self, id: String, label: String) {
        self.facts.nodes.push(Node {
            id,
            label,
            kind: NodeKind::Stub,
            file_type: "code".into(),
            source_file: String::new(),
            loc: Loc::Empty,
            attrs: vec![origin()],
        });
    }

    /// A shared external import node; label equals the id (§4.4).
    pub fn add_external(&mut self, id: String) {
        self.facts.nodes.push(external_node(id));
    }

    /// A rationale node (`file_type: "rationale"`).
    pub fn add_rationale(&mut self, id: String, label: String, line: u32) {
        self.facts.nodes.push(Node {
            id,
            label,
            kind: NodeKind::Rationale,
            file_type: "rationale".into(),
            source_file: self.rel.into(),
            loc: Loc::Line(line),
            attrs: vec![origin()],
        });
    }

    /// An in-file EXTRACTED edge with weight 1.0 and no score (the common case).
    pub fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        line: u32,
        context: Option<&str>,
    ) {
        let e = self.edge(source, target, relation, Loc::Line(line), context);
        self.facts.edges.push(e);
    }

    /// Build (but do not push) an in-file edge, for callers that need to set more fields.
    pub fn edge(
        &self,
        source: &str,
        target: &str,
        relation: &str,
        loc: Loc,
        context: Option<&str>,
    ) -> Edge {
        Edge {
            source: source.into(),
            target: target.into(),
            relation: relation.into(),
            confidence: Confidence::Extracted,
            score: None,
            context: context.map(str::to_string),
            source_file: self.rel.into(),
            loc,
            weight: Some(1.0),
            attrs: vec![origin()],
        }
    }
}

pub fn origin() -> (String, Attr) {
    ("_origin".into(), Attr::Str("ast".into()))
}

pub fn flag(key: &str) -> (String, Attr) {
    (key.into(), Attr::Bool(true))
}

/// `{"id","label":id,"file_type":"concept","type":"external","external":true,"source_file":""}`.
pub fn external_node(id: String) -> Node {
    Node {
        label: id.clone(),
        id,
        kind: NodeKind::External,
        file_type: "concept".into(),
        source_file: String::new(),
        loc: Loc::Absent,
        attrs: vec![
            ("external".into(), Attr::Bool(true)),
            ("type".into(), Attr::Str("external".into())),
        ],
    }
}

/// Rationale label: Python's `textwrap.shorten(text, 80, placeholder="…")`. Whitespace is
/// collapsed; a longer text keeps the most leading chunks that fit with the `…`, where chunks
/// break at spaces and after the hyphen of a hyphenated word (`cross-…`, `tree-…`).
pub fn rationale_label(text: &str) -> String {
    const WIDTH: usize = 80;
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = collapsed.chars().collect();
    if chars.len() <= WIDTH {
        return collapsed;
    }
    let mut chunks: Vec<Vec<char>> = wrap_chunks(&chars).into_iter().rev().collect();
    let mut line: Vec<Vec<char>> = Vec::new();
    let mut len = 0;
    while let Some(c) = chunks.last() {
        if len + c.len() > WIDTH {
            break;
        }
        len += c.len();
        line.push(chunks.pop().expect("checked"));
    }
    // `_handle_long_word`: a chunk longer than the whole width is split to fill the line.
    if let Some(c) = chunks.last().filter(|c| c.len() > WIDTH).cloned() {
        let space = WIDTH - len;
        let mut end = space;
        if let Some(h) = c[..space.min(c.len())].iter().rposition(|&x| x == '-') {
            if h > 0 && c[..h].iter().any(|&x| x != '-') {
                end = h + 1;
            }
        }
        len += end;
        line.push(c[..end].to_vec());
    }
    if line
        .last()
        .is_some_and(|c| c.iter().all(|x| x.is_whitespace()))
    {
        len -= line.pop().map_or(0, |c| c.len());
    }
    while let Some(last) = line.last() {
        if !last.iter().all(|x| x.is_whitespace()) && len < WIDTH {
            let mut out: String = line.iter().flatten().collect();
            out.push('…');
            return out;
        }
        len -= last.len();
        line.pop();
    }
    "…".to_string()
}

/// `textwrap.TextWrapper.wordsep_re` chunks (break on hyphens): runs of spaces, em-dashes
/// between words, and words split after a hyphen that has two letters (or `x-y-`) before it and
/// a letter (optionally hyphen-separated from another) after it.
fn wrap_chunks(t: &[char]) -> Vec<Vec<char>> {
    let letter = |c: char| (c.is_alphanumeric() && !c.is_numeric()) || c == '_';
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let punct = |c: char| word(c) || "!\"'&.,?".contains(c);
    let at = |i: usize| t.get(i).copied();
    let dashes_then_word = |k: usize| {
        let mut j = k;
        while at(j) == Some('-') {
            j += 1;
        }
        j - k >= 2 && at(j).is_some_and(word)
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if t[i].is_whitespace() {
            let mut j = i;
            while j < t.len() && t[j].is_whitespace() {
                j += 1;
            }
            out.push(t[i..j].to_vec());
            i = j;
            continue;
        }
        if i > 0 && punct(t[i - 1]) && dashes_then_word(i) {
            let mut j = i;
            while at(j) == Some('-') {
                j += 1;
            }
            out.push(t[i..j].to_vec());
            i = j;
            continue;
        }
        let mut k = i + 1;
        let end = loop {
            if k >= t.len() || t[k].is_whitespace() {
                break k;
            }
            if t[k] == '-' {
                let before = (k >= 2 && letter(t[k - 1]) && letter(t[k - 2]))
                    || (k >= 3 && letter(t[k - 3]) && t[k - 2] == '-' && letter(t[k - 1]));
                let after = at(k + 1).is_some_and(letter)
                    && (at(k + 2).is_some_and(letter)
                        || (at(k + 2) == Some('-') && at(k + 3).is_some_and(letter)));
                if before && after {
                    break k + 1;
                }
            }
            if punct(t[k - 1]) && dashes_then_word(k) {
                break k;
            }
            k += 1;
        };
        out.push(t[i..end].to_vec());
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rationale_labels_truncate_on_a_space() {
        assert_eq!(rationale_label("  a   b\n c "), "a b c");
        let long = "word ".repeat(30);
        let label = rationale_label(&long);
        assert!(label.ends_with('…'));
        assert_eq!(label.chars().count(), 80);
        // Hyphenated words break after the hyphen (graphify corpus, `tests/test_js_*.py`).
        let doc = "Yield identifier value-nodes of a JS/TS object/array literal that are \
                   function-valued entries.";
        assert_eq!(
            rationale_label(doc),
            "Yield identifier value-nodes of a JS/TS object/array literal that are function-…"
        );
        let doc = "A fuzzy-only run must still report the fuzzy count (#1857). Two long, \
                   high-similarity names.";
        assert_eq!(
            rationale_label(doc),
            "A fuzzy-only run must still report the fuzzy count (#1857). Two long, high-…"
        );
    }
}

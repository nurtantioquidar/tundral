//! Verilog extraction (§36). See `docs/graphify-format.md`.
//!
//! Graphify reads `.v` (also `.sv`, `.svh`, `.vh`, any case) as Verilog/SystemVerilog with
//! tree-sitter-verilog 1.0.3; a V-language file parses to garbage and yields only its file node.
//! One file yields:
//!
//! - its file node, at `L1` even for a 0-byte file (V8); every Verilog node, file nodes and stubs
//!   included, carries the node fields `confidence: "INFERRED"` and `confidence_score: 1.0`;
//!   every in-file edge carries `confidence_score: 1.0` (V1);
//! - modules (`file_slug(name)`, bare label) with `defines` from the file, nested modules too.
//!   The name is the first identifier anywhere in the header, so `(* keep *) module m` is named
//!   `keep` (V2);
//! - functions (`name()`) and tasks (`name`, no parentheses) with `contains` from the innermost
//!   module; outside any module (packages, interfaces, programs, file scope) their parent id is
//!   the slug of the path **with** its extension (`a_v_twice`, V3), contained by the file;
//! - `instantiates` from the innermost module for every `module_instantiation` and
//!   `checker_instantiation` (how tree-sitter reads `name inst(...)`; the grammar reads some
//!   instantiations as UDP, interface or gate instances, which give nothing, V4). A module of the
//!   same file is the target; any other name becomes a stub `slug(name)` (label as written), one
//!   per name and file, which `crate::resolve::verilog` binds by label;
//! - SystemVerilog classes, found by a text scan of the comment-stripped source rather than the
//!   tree (V5): `class Name ... ; body endclass` → class `file_slug(Name)` with `defines` from the
//!   file; `extends B` / `implements I, J` in the header → `inherits` / `implements` edges to a
//!   node `file_slug(B)` labelled `B` at the class's line (never bound to the real class elsewhere,
//!   V6); `function T name (` and `task name (` in the body → method `class_slug(name)`, bare
//!   label, `method` edge.
//!   Class lines come from offsets in the stripped text applied to the original text, so they
//!   drift upwards after comments; a method's line is its class's line plus the newlines of the
//!   stripped body before it (V7). Every function is scanned before the first task.
//!
//! Not emitted: ports, nets, variables, parameters, `always`/`initial` blocks, calls of functions
//! and tasks, `` `include ``/`` `define ``, packages, interfaces, programs and primitives themselves,
//! and comments (no rationale nodes).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{slug, Ctx};
use crate::facts::{Attr, LangFacts, Loc};

/// Facts for the cross-file pass (`crate::resolve::verilog`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VerilogFacts {
    /// Instantiation stubs as `(local id, label)`, in order of first use.
    pub stubs: Vec<(String, String)>,
}

/// The `confidence: "INFERRED"` and `confidence_score: 1.0` every Verilog node carries (V1).
pub fn score() -> Vec<(String, Attr)> {
    vec![
        ("confidence".into(), Attr::Str("INFERRED".into())),
        ("confidence_score".into(), Attr::Float(1.0)),
    ]
}

struct W<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    /// Module name → id, for every module declared in the file.
    modules: HashMap<String, String>,
    /// Parent id for definitions outside any module: slug of the path with its extension.
    free: String,
    stubs: Vec<(String, String)>,
    stub_ids: HashSet<String>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(score());
    // Even a 0-byte file is at `L1` (V8).
    if let Some(file) = cx.facts.nodes.first_mut() {
        file.loc = Loc::Line(1);
    }
    let free = slug(cx.rel);
    let mut w = W {
        modules: HashMap::new(),
        free,
        stubs: Vec::new(),
        stub_ids: HashSet::new(),
        cx,
    };
    if let Some(tree) = tree {
        let root = tree.root_node();
        w.collect_modules(root);
        w.walk(root, None);
    }
    let W { cx, stubs, .. } = w;
    classes(cx);
    cx.facts.lang = LangFacts::Verilog(VerilogFacts { stubs });
}

fn children(node: TsNode) -> Vec<TsNode> {
    let mut c = node.walk();
    node.children(&mut c).collect()
}

/// The first `simple_identifier` in a depth-first walk of `node`.
fn first_ident(node: TsNode) -> Option<TsNode> {
    if node.kind() == "simple_identifier" {
        return Some(node);
    }
    children(node).into_iter().find_map(first_ident)
}

fn child_of_kind<'t>(node: TsNode<'t>, kind: &str) -> Option<TsNode<'t>> {
    children(node).into_iter().find(|n| n.kind() == kind)
}

fn push(cx: &mut Ctx, source: &str, target: &str, relation: &str, line: u32) {
    let mut e = cx.edge(source, target, relation, Loc::Line(line), None);
    e.score = Some(1.0);
    cx.facts.edges.push(e);
}

impl W<'_, '_> {
    fn module_name(&self, node: TsNode) -> Option<String> {
        let header = child_of_kind(node, "module_header")?;
        first_ident(header).map(|n| self.cx.text(n).to_string())
    }

    fn collect_modules(&mut self, node: TsNode) {
        if node.kind() == "module_declaration" {
            if let Some(name) = self.module_name(node) {
                let id = Ctx::child_id(&self.cx.file_id, &name);
                self.modules.entry(name).or_insert(id);
            }
        }
        for child in children(node) {
            self.collect_modules(child);
        }
    }

    fn walk(&mut self, node: TsNode, module: Option<&str>) {
        match node.kind() {
            "class_declaration" => return,
            "module_declaration" => {
                if let Some(name) = self.module_name(node) {
                    let id = Ctx::child_id(&self.cx.file_id, &name);
                    let line = Ctx::line(node);
                    self.cx.add_def(id.clone(), name, line, score());
                    let file = self.cx.file_id.clone();
                    push(self.cx, &file, &id, "defines", line);
                    for child in children(node) {
                        self.walk(child, Some(&id));
                    }
                    return;
                }
            }
            "function_declaration" | "task_declaration" => {
                let task = node.kind() == "task_declaration";
                let (body, ident) = if task {
                    ("task_body_declaration", "task_identifier")
                } else {
                    ("function_body_declaration", "function_identifier")
                };
                let name = child_of_kind(node, body)
                    .and_then(|b| child_of_kind(b, ident))
                    .and_then(first_ident)
                    .map(|n| self.cx.text(n).to_string());
                if let Some(name) = name {
                    let (parent, owner) = match module {
                        Some(m) => (m.to_string(), m.to_string()),
                        None => (self.free.clone(), self.cx.file_id.clone()),
                    };
                    let id = Ctx::child_id(&parent, &name);
                    let label = if task { name } else { format!("{name}()") };
                    let line = Ctx::line(node);
                    self.cx.add_def(id.clone(), label, line, score());
                    push(self.cx, &owner, &id, "contains", line);
                }
            }
            "module_instantiation" | "checker_instantiation" => {
                if let (Some(m), Some(n)) = (module, first_ident(node)) {
                    let name = self.cx.text(n).to_string();
                    let target = match self.modules.get(&name) {
                        Some(id) => id.clone(),
                        None => {
                            let id = slug(&name);
                            if self.stub_ids.insert(id.clone()) {
                                self.cx.add_stub(id.clone(), name.clone());
                                if let Some(stub) = self.cx.facts.nodes.last_mut() {
                                    stub.attrs.extend(score());
                                }
                                self.stubs.push((id.clone(), name));
                            }
                            id
                        }
                    };
                    push(self.cx, m, &target, "instantiates", Ctx::line(node));
                }
                return;
            }
            _ => {}
        }
        for child in children(node) {
            self.walk(child, module);
        }
    }
}

/// Python's `\w` for str patterns.
fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Python's `\s` for str patterns.
fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// The source with `// ...` (up to the newline) and `/* ... */` removed, scanning left to right.
/// An unterminated `/*` is kept as text.
fn strip_comments(src: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if src[i] == '/' && i + 1 < src.len() {
            if src[i + 1] == '/' {
                while i < src.len() && src[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if src[i + 1] == '*' {
                if let Some(end) = find(src, &['*', '/'], i + 2) {
                    i = end + 2;
                    continue;
                }
            }
        }
        out.push(src[i]);
        i += 1;
    }
    out
}

/// First index `>= from` where `pat` occurs in `hay`.
fn find(hay: &[char], pat: &[char], from: usize) -> Option<usize> {
    if pat.is_empty() || hay.len() < pat.len() || from > hay.len() - pat.len() {
        return None;
    }
    (from..=hay.len() - pat.len()).find(|&i| hay[i..i + pat.len()] == *pat)
}

/// Index of the next occurrence of `pat` at or after `from` that starts a word (`\bpat`).
fn find_word(hay: &[char], pat: &[char], mut from: usize) -> Option<usize> {
    while let Some(at) = find(hay, pat, from) {
        if at == 0 || !is_word(hay[at - 1]) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

fn skip_space(s: &[char], mut i: usize) -> usize {
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    i
}

fn skip_word(s: &[char], mut i: usize) -> usize {
    while i < s.len() && is_word(s[i]) {
        i += 1;
    }
    i
}

/// The word of the first `keyword\s+(\w+)` in `s`.
fn word_after(s: &[char], keyword: &str) -> Option<String> {
    let kw: Vec<char> = keyword.chars().collect();
    let mut from = 0;
    while let Some(at) = find(s, &kw, from) {
        let ws = at + kw.len();
        let start = skip_space(s, ws);
        let end = skip_word(s, start);
        if start > ws && end > start {
            return Some(s[start..end].iter().collect());
        }
        from = at + 1;
    }
    None
}

/// One `class ... endclass` match of the text scan.
#[derive(Debug, PartialEq)]
struct ClassMatch {
    name: String,
    /// Index of `class` in the stripped text.
    start: usize,
    bases: Vec<String>,
    interfaces: Vec<String>,
    /// `(name, newlines in the body before it)`.
    methods: Vec<(String, usize)>,
}

/// `\bclass\s+(\w+)([^;]*);(.*?)endclass`, non-overlapping, over the stripped text.
fn scan_classes(s: &[char]) -> Vec<ClassMatch> {
    let kw: Vec<char> = "class".chars().collect();
    let endkw: Vec<char> = "endclass".chars().collect();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = find_word(s, &kw, from) {
        from = at + 1;
        let ws = at + kw.len();
        let name_start = skip_space(s, ws);
        if name_start == ws {
            continue;
        }
        let name_end = skip_word(s, name_start);
        if name_end == name_start {
            continue;
        }
        let Some(semi) = (name_end..s.len()).find(|&i| s[i] == ';') else {
            continue;
        };
        let Some(end) = find(s, &endkw, semi + 1) else {
            // No `endclass` after this point: no later `class` can match either.
            break;
        };
        let header = &s[name_end..semi];
        out.push(ClassMatch {
            name: s[name_start..name_end].iter().collect(),
            start: at,
            bases: word_after(header, "extends").into_iter().collect(),
            interfaces: implements_list(header),
            methods: scan_methods(&s[semi + 1..end]),
        });
        from = end + endkw.len();
    }
    out
}

/// `implements A, B` in a class header: the leading word of each comma-separated item.
fn implements_list(header: &[char]) -> Vec<String> {
    let kw: Vec<char> = "implements".chars().collect();
    let Some(at) = find(header, &kw, 0) else {
        return Vec::new();
    };
    let start = at + kw.len();
    if skip_space(header, start) == start {
        return Vec::new();
    }
    let rest: String = header[start..].iter().collect();
    rest.split(',')
        .map(|item| {
            item.trim_start_matches(is_space)
                .chars()
                .take_while(|&c| is_word(c))
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Methods of a class body: `\bfunction\s+\w+\s+(\w+)\s*\(` matches, then
/// `\btask\s+(\w+)\s*\(` matches, each scan non-overlapping (V7). A task and a function of the
/// same name are one node (the function's, first), with the task's `method` edge last.
fn scan_methods(body: &[char]) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for (kw, typed) in [("function", true), ("task", false)] {
        let kw: Vec<char> = kw.chars().collect();
        let mut from = 0;
        while let Some(at) = find_word(body, &kw, from) {
            from = at + 1;
            let i = at + kw.len();
            let j = skip_space(body, i);
            if j == i {
                continue;
            }
            // A function names its return type first; a task has none.
            let l = if typed {
                let k = skip_word(body, j);
                let l = skip_space(body, k);
                if k == j || l == k {
                    continue;
                }
                l
            } else {
                j
            };
            let name_end = skip_word(body, l);
            let p = skip_space(body, name_end);
            if name_end == l || body.get(p) != Some(&'(') {
                continue;
            }
            let newlines = body[..at].iter().filter(|&&c| c == '\n').count();
            out.push((body[l..name_end].iter().collect(), newlines));
            from = p + 1;
        }
    }
    out
}

/// SystemVerilog classes from the text scan (see the module docs).
fn classes(cx: &mut Ctx) {
    let src: Vec<char> = String::from_utf8_lossy(cx.src).chars().collect();
    let endkw: Vec<char> = "endclass".chars().collect();
    if find(&src, &endkw, 0).is_none() {
        return;
    }
    let stripped = strip_comments(&src);
    let file = cx.file_id.clone();
    for m in scan_classes(&stripped) {
        let upto = m.start.min(src.len());
        let line = src[..upto].iter().filter(|&&c| c == '\n').count() as u32 + 1;
        let id = Ctx::child_id(&file, &m.name);
        cx.add_def(id.clone(), m.name.clone(), line, score());
        push(cx, &file, &id, "defines", line);
        for (bases, relation) in [(&m.bases, "inherits"), (&m.interfaces, "implements")] {
            for b in bases {
                let bid = Ctx::child_id(&file, b);
                cx.add_def(bid.clone(), b.clone(), line, score());
                push(cx, &id, &bid, relation, line);
            }
        }
        for (name, newlines) in &m.methods {
            let mline = line + *newlines as u32;
            let mid = Ctx::child_id(&id, name);
            cx.add_def(mid.clone(), name.clone(), mline, score());
            push(cx, &id, &mid, "method", mline);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn comments_are_stripped_line_and_block() {
        let s: String = strip_comments(&chars("a // x\nb /* y\nz */c /* open"))
            .iter()
            .collect();
        assert_eq!(s, "a \nb c /* open");
    }

    #[test]
    fn class_scan_reads_headers_and_methods() {
        let src = "typedef class F;\nclass A;\nendclass\nclass Q2 #(int W=1) extends B2 #(1) implements I1, I2;\n  function automatic void a();\n  endfunction\n  virtual function void b();\n  endfunction\n  function new();\n  endfunction\n  function\n    int c\n    (int x);\n  endfunction\nendclass\nmyclass h;\n";
        let m = scan_classes(&chars(src));
        assert_eq!(m.len(), 2);
        // `typedef class F;` swallows `class A` up to the first `endclass`.
        assert_eq!(m[0].name, "F");
        assert_eq!(m[1].name, "Q2");
        assert_eq!(m[1].bases, ["B2"]);
        assert_eq!(m[1].interfaces, ["I1", "I2"]);
        let names: Vec<_> = m[1].methods.iter().map(|(n, l)| (n.as_str(), *l)).collect();
        assert_eq!(names, [("b", 3), ("c", 7)]);
    }

    #[test]
    fn unterminated_classes_match_nothing() {
        assert!(scan_classes(&chars("class A;\n function void f();\n")).is_empty());
    }
}

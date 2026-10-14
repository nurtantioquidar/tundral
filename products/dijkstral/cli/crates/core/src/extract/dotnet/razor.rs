//! Razor components and views: `.razor`, `.cshtml` (§43.3).
//!
//! A line scanner, like Graphify's: directives at the start of a line in line order (`@page`,
//! `@model`, `@using`, `@inject`, `@inherits`), then every capitalised tag in the file
//! (`<Name` followed by whitespace, `/` or `>`; HTML layout tags excepted) as a component call,
//! then the methods of every `@code` / `@functions` block (a declaration with at least one access
//! or member modifier). A directive or component names a node by `slug(name)`, reusing a node the
//! file already emitted under that id.
//!
//! Usings, injected types, base types and components are bound in the cross-file pass
//! (`crate::resolve::dotnet`), so their edges carry virtual targets listed in [`RazorFacts`].

use serde::{Deserialize, Serialize};

use super::{json, metadata, DotnetFacts};
use crate::extract::{origin, slug, Ctx};
use crate::facts::{Edge, LangFacts, Loc, Node, NodeKind};

/// Prefix of the virtual edge targets the cross-file pass redirects.
pub const VIRTUAL: &str = "@rz:";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RazorFacts {
    /// Namespaces of the file's plain `@using` lines, in order.
    pub usings: Vec<String>,
    /// `@using Alias = Target` pairs.
    pub aliases: Vec<(String, String)>,
    /// Edges to bind, in emission order.
    pub refs: Vec<RazorRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RazorRef {
    /// The virtual target the edge carries.
    pub target: String,
    /// The node the edge points at when nothing binds (local id).
    pub node: String,
    pub kind: RefKind,
    /// The name to bind: a namespace or type as written, a component tag.
    pub name: String,
    /// Generic arity of an injected type.
    pub arity: u32,
    /// The node is an unresolved-type stub (created by `@inject`): only such nodes bind by name.
    pub stub: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefKind {
    /// `@using N`: a local namespace.
    Namespace,
    /// `@using A = T`: a local type.
    Alias,
    /// `@using static T`: never bound.
    Static,
    /// `@inject T x`: C# name lookup from the file's usings, else the shared stub.
    Inject,
    /// `@inherits T` and `<Component>` on a node `@inject` created: the only local type of that
    /// name, when its namespace is global or imported by the file or by an `_Imports.razor`
    /// beside it.
    Bind,
}

/// Tags never read as components (compared case-insensitively).
const HTML_TAGS: &[&str] = &[
    "article", "aside", "body", "button", "div", "footer", "form", "head", "header", "html",
    "input", "label", "link", "main", "meta", "nav", "option", "script", "section", "select",
    "span", "style", "table", "textarea", "title",
];

/// Words that open a method declaration in a code block.
const MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "async",
    "override",
    "virtual",
    "abstract",
    "sealed",
];

struct Scan<'a> {
    text: &'a str,
    /// Byte offset of each line start.
    starts: Vec<usize>,
}

impl Scan<'_> {
    fn line(&self, offset: usize) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }
}

fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// `[\w.<>]+` at the start of `s`.
fn type_token(s: &str) -> &str {
    let end = s
        .find(|c: char| !(is_word(c) || matches!(c, '.' | '<' | '>')))
        .unwrap_or(s.len());
    &s[..end]
}

/// Split `rest` after a directive keyword: there must be whitespace first.
fn after_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some(rest.trim_start_matches([' ', '\t']))
}

/// A type as written: (name without generic arguments, arity).
fn generic(written: &str) -> (&str, u32) {
    match written.find('<') {
        Some(i) => {
            let args = &written[i + 1..];
            let mut depth = 0;
            let mut arity = 1;
            for c in args.chars() {
                match c {
                    '<' => depth += 1,
                    '>' if depth == 0 => break,
                    '>' => depth -= 1,
                    ',' if depth == 0 => arity += 1,
                    _ => {}
                }
            }
            (&written[..i], arity)
        }
        None => (written, 0),
    }
}

struct Emit<'a, 'c> {
    cx: &'a mut Ctx<'c>,
    file: String,
    facts: RazorFacts,
    serial: usize,
}

impl Emit<'_, '_> {
    fn has(&self, id: &str) -> bool {
        self.cx.facts.nodes.iter().any(|n| n.id == id)
    }

    /// A node of this file unless one with that id exists.
    fn def(&mut self, id: &str, label: &str, line: u32, file_type: &str) {
        if self.has(id) {
            return;
        }
        self.cx.facts.nodes.push(Node {
            id: id.into(),
            label: label.into(),
            kind: NodeKind::Def,
            file_type: file_type.into(),
            source_file: self.cx.rel.into(),
            loc: Loc::Line(line),
            attrs: vec![origin()],
        });
    }

    /// An unresolved-type stub of this file unless a node with that id exists.
    fn stub(&mut self, id: &str, label: &str) {
        if !self.has(id) {
            self.cx.add_stub(id.into(), label.into());
        }
    }

    fn edge(&mut self, target: &str, relation: &str, loc: Loc, context: Option<&str>) -> &mut Edge {
        let e = self.cx.edge(&self.file, target, relation, loc, context);
        self.cx.facts.edges.push(e);
        self.cx.facts.edges.last_mut().unwrap()
    }

    /// An edge to bind later: its target is virtual until the cross-file pass redirects it.
    #[allow(clippy::too_many_arguments)]
    fn bound(
        &mut self,
        node: &str,
        kind: RefKind,
        name: &str,
        arity: u32,
        relation: &str,
        line: u32,
        context: Option<&str>,
    ) -> &mut Edge {
        self.serial += 1;
        let target = format!("{VIRTUAL}{}|{}", self.cx.rel, self.serial);
        let stub = self
            .cx
            .facts
            .nodes
            .iter()
            .any(|n| n.id == node && n.kind == NodeKind::Stub);
        self.facts.refs.push(RazorRef {
            target: target.clone(),
            node: node.into(),
            kind,
            name: name.into(),
            arity,
            stub,
        });
        self.edge(&target, relation, Loc::Line(line), context)
    }
}

pub fn extract(cx: &mut Ctx) {
    super::add_null_file_node(cx);
    let text = String::from_utf8_lossy(cx.src).into_owned();
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let scan = Scan {
        text: &text,
        starts,
    };
    let file = cx.file_id.clone();
    let mut em = Emit {
        cx,
        file,
        facts: RazorFacts::default(),
        serial: 0,
    };
    directives(&scan, &mut em);
    components(&scan, &mut em);
    methods(&scan, &mut em);
    let facts = em.facts;
    cx_set(em.cx, facts);
}

fn cx_set(cx: &mut Ctx, razor: RazorFacts) {
    cx.facts.lang = LangFacts::Dotnet(DotnetFacts {
        razor: Some(razor),
        ..Default::default()
    });
}

fn directives(scan: &Scan, em: &mut Emit) {
    let mut routes: Vec<String> = Vec::new();
    for (i, raw) in scan.text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let n = i as u32 + 1;
        if let Some(rest) = after_keyword(line, "@page") {
            let Some(path) = rest
                .strip_prefix('"')
                .and_then(|r| r.split_once('"'))
                .map(|(p, _)| p)
            else {
                continue;
            };
            let id = Ctx::child_id("route", path);
            if routes.contains(&id) {
                continue;
            }
            routes.push(id.clone());
            em.def(&id, &format!("route:{path}"), n, "concept");
            em.edge(&id, "references", Loc::Absent, None);
        } else if let Some(rest) = after_keyword(line, "@model") {
            let name = rest.split_whitespace().next().unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let id = slug(name);
            em.def(&id, name, n, "code");
            em.edge(&id, "references", Loc::Line(n), None);
        } else if let Some(rest) = after_keyword(line, "@using") {
            using(em, rest, n);
        } else if let Some(rest) = after_keyword(line, "@inject") {
            let ty = type_token(rest);
            let after = &rest[ty.len()..];
            let name_ok = after.starts_with([' ', '\t']) && {
                let a = after.trim_start_matches([' ', '\t']);
                a.chars().next().is_some_and(is_word)
            };
            if ty.is_empty() || !name_ok {
                continue;
            }
            let (base, arity) = generic(ty);
            let (qualifier, token) = match base.rsplit_once('.') {
                Some((q, t)) => (Some(q), t),
                None => (None, base),
            };
            let id = slug(token);
            if id.is_empty() {
                continue;
            }
            em.stub(&id, token);
            let mut meta = vec![("ref_token", json(token))];
            if let Some(q) = qualifier {
                meta.push(("qualified", "true".into()));
                meta.push(("ref_qualifier", json(q)));
            }
            let e = em.bound(
                &id,
                RefKind::Inject,
                base,
                arity,
                "references",
                n,
                Some("field"),
            );
            e.attrs.push(metadata(&meta));
        } else if let Some(rest) = after_keyword(line, "@inherits") {
            let name = rest.split_whitespace().next().unwrap_or("");
            let id = slug(name);
            if id.is_empty() {
                continue;
            }
            em.def(&id, name, n, "code");
            em.bound(&id, RefKind::Bind, name, 0, "inherits", n, None);
        }
    }
}

/// `@using [static] [Alias =] Target[;]`.
fn using(em: &mut Emit, rest: &str, n: u32) {
    let (kind, rest) = match after_keyword(rest, "static") {
        Some(r) => (RefKind::Static, r),
        None => (RefKind::Namespace, rest),
    };
    let mut alias = None;
    let mut target = type_token(rest);
    if kind == RefKind::Namespace {
        let after = rest[target.len()..].trim_start_matches([' ', '\t']);
        if let Some(t) = after.strip_prefix('=') {
            if !target.is_empty() && target.chars().all(is_word) {
                alias = Some(target);
                target = type_token(t.trim_start_matches([' ', '\t']));
            }
        }
    }
    let id = slug(target);
    if id.is_empty() {
        return;
    }
    let kind = if alias.is_some() {
        RefKind::Alias
    } else {
        kind
    };
    em.def(&id, target, n, "code");
    match (kind, alias) {
        (RefKind::Namespace, _) => em.facts.usings.push(target.to_string()),
        (RefKind::Alias, Some(a)) => em.facts.aliases.push((a.to_string(), target.to_string())),
        _ => {}
    }
    let using_kind = match kind {
        RefKind::Static => "static",
        RefKind::Alias => "alias",
        _ => "namespace",
    };
    let mut meta = vec![
        ("using_kind", json(using_kind)),
        ("target_fqn", json(&escape_html(target))),
        ("scope_kind", json("file")),
    ];
    if let Some(a) = alias {
        meta.push(("alias", json(a)));
    }
    let e = em.bound(&id, kind, target, 0, "imports", n, Some("import"));
    e.attrs.push(metadata(&meta));
}

/// `html.escape(s, quote=False)`.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `<Name` (an uppercase letter and at least one more letter or digit) followed by whitespace,
/// `/` or `>`.
fn components(scan: &Scan, em: &mut Emit) {
    let bytes = scan.text.as_bytes();
    let mut i = 0;
    while let Some(off) = scan.text[i..].find('<') {
        let at = i + off;
        i = at + 1;
        let rest = &scan.text[at + 1..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if name.len() < 2 || !name.as_bytes()[0].is_ascii_uppercase() {
            continue;
        }
        let next = bytes.get(at + 1 + end).copied();
        if !matches!(
            next,
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c' | b'/' | b'>')
        ) {
            continue;
        }
        if HTML_TAGS.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        let line = scan.line(at);
        let id = slug(name);
        em.def(&id, name, line, "code");
        em.bound(&id, RefKind::Bind, name, 0, "calls", line, None);
    }
}

/// Methods declared in `@code { … }` and `@functions { … }` blocks.
fn methods(scan: &Scan, em: &mut Emit) {
    for (start, end) in code_blocks(scan.text) {
        let block = &scan.text[start..end];
        for (name, at) in declarations(block) {
            // `at` is the first modifier: Graphify locates the declaration there.
            let id = format!("{}_{}", em.file, slug(name));
            let line = scan.line(start + at);
            if !em.has(&id) {
                em.def(&id, name, line, "code");
                em.edge(&id, "contains", Loc::Absent, None);
            }
        }
    }
}

/// Byte ranges inside the braces of every `@code` / `@functions` block.
fn code_blocks(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find('@') {
        let at = i + off;
        i = at + 1;
        let rest = &text[at + 1..];
        let kw = if rest.starts_with("code") {
            4
        } else if rest.starts_with("functions") {
            9
        } else {
            continue;
        };
        let after = &rest[kw..];
        let ws = after.len() - after.trim_start().len();
        if !after[ws..].starts_with('{') {
            continue;
        }
        let open = at + 1 + kw + ws;
        let mut depth = 0usize;
        let mut close = text.len();
        for (k, c) in text[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + k;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push((open + 1, close));
        i = close.max(open + 1);
    }
    out
}

/// `(modifier\s+)+ type\s+ name\s*(` where the type part holds only word characters, `<>[],`
/// and whitespace: the method name and the byte offset of its first modifier.
fn declarations(block: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    'scan: while i < block.len() {
        let rest = &block[i..];
        let Some(m) = MODIFIERS.iter().find(|m| {
            rest.starts_with(**m) && block[..i].chars().next_back().is_none_or(|c| !is_word(c))
        }) else {
            i += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        };
        // One or more modifiers, each followed by whitespace.
        let mut j = i + m.len();
        let mut saw_ws = false;
        loop {
            let ws = block[j..].len() - block[j..].trim_start().len();
            if ws == 0 {
                break;
            }
            saw_ws = true;
            j += ws;
            match MODIFIERS.iter().find(|m| {
                block[j..].starts_with(**m)
                    && block[j + m.len()..]
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace)
            }) {
                Some(m) => j += m.len(),
                None => break,
            }
        }
        if !saw_ws {
            i += m.len();
            continue;
        }
        // The rest up to the next `(` must be `type\s+name\s*`.
        let Some(paren) = block[j..].find('(') else {
            break 'scan;
        };
        let head = &block[j..j + paren];
        let ok = head
            .chars()
            .all(|c| is_word(c) || c.is_whitespace() || matches!(c, '<' | '>' | '[' | ']' | ','));
        let trimmed = head.trim_end();
        let name_start = trimmed
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word(*c))
            .last()
            .map(|(k, _)| k);
        match name_start {
            Some(k)
                if ok
                    && k > 0
                    && trimmed[..k].ends_with(char::is_whitespace)
                    && !trimmed[..k].trim().is_empty() =>
            {
                out.push((&trimmed[k..], i));
                i = j + paren + 1;
            }
            _ => i += m.len(),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_need_a_modifier_and_a_plain_type() {
        let block = "public void Pub() {}\n void A() {}\n public static implicit operator int(X x) => 0;\n public T Gen<T>(T x) => x;\n public void\n   Split() {}\n public System.String Q() => null;\n public Dictionary<string, int> Map() => null;\n private class N { public void Inner() {} }\n";
        let names: Vec<&str> = declarations(block).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Pub", "int", "Split", "Map", "Inner"]);
    }

    #[test]
    fn generic_arity() {
        assert_eq!(generic("IFoo<A<B, C>, D>"), ("IFoo", 2));
        assert_eq!(generic("Foo"), ("Foo", 0));
    }
}

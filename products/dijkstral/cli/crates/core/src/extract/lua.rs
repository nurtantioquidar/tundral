//! Lua and Luau (§28) extraction. See `docs/graphify-format.md`.
//!
//! One walk over the tree-sitter-lua tree (Luau files use the same grammar and keep whatever its
//! error recovery leaves) emits:
//!
//! - a node per named `function` statement that is neither inside a named function's body nor
//!   inside a `local` declaration, at any depth otherwise (`if`, `do`, loops, and anonymous
//!   functions passed as arguments, returned or assigned, as in busted `describe`/`it` blocks):
//!   id `file_<slug(name as written)>`, label `name()` with the name's text (`M.sub:f()`,
//!   `M . spaced()`), `_callable`, and a `contains` edge from the file. Anonymous functions
//!   (`local f = function`, `M.f = function`) are not nodes;
//! - `imports` edges from the file for `require`, outside named function bodies, with Graphify's
//!   bare-number `source_location` (`"3"`, quirk LU1): a `local` declaration is searched as text
//!   for the first `require` (so `myrequire("x")`, `x.require("x")` and `"require('x')"` count,
//!   at the declaration's first line); any other `require …` call is read the same way at its own
//!   line. Their targets are virtual (`@lua:<importer>|<module>`) and bound by
//!   `crate::resolve::lua`;
//! - `calls` that bind in the file, after the walk: a call's name as written (`f`, `M.f`,
//!   `M.sub.f`) to the function with that label; `self:m()` inside a function named on a table
//!   `T` to `T:m`/`T.m`. Other method calls (`x:m()`, `Obj:m()`) never bind. Calls belong to the
//!   enclosing node (calls in anonymous functions included); calls at the top level or in a
//!   nested named function are dropped. Bare calls of builtin-like names are skipped. One edge
//!   per (caller, callee), first line kept.
//!
//! Calls that did not bind (not member calls) go to [`LuaFacts::calls`] for the cross-file pass.
//! No rationale: comments, doc comments and `NOTE:`/`WHY:` produce nothing.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, origin, Ctx};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc};

/// Prefix of the virtual `imports` targets: `@lua:<importer rel>|<module>`.
pub const VIRTUAL: &str = "@lua:";

/// A call that did not bind in its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LuaCall {
    /// Local id of the calling function.
    pub caller: String,
    /// The called name as written (`f`, `M.f`, `a.b.c`).
    pub callee: String,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::lua`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LuaFacts {
    pub calls: Vec<LuaCall>,
}

/// Sorted. Bare call names that never bind (probed one by one; `object`, `Uuid`, `pow`, Lua's own
/// globals such as `tostring` and `pairs` do bind).
const DENIED_CALLS: &[&str] = &[
    "Array",
    "BigInt",
    "Blob",
    "Bool",
    "Boolean",
    "Character",
    "Color",
    "Data",
    "Date",
    "Decimal",
    "Double",
    "Error",
    "EvalError",
    "File",
    "Float",
    "Font",
    "FormData",
    "Headers",
    "Int",
    "Int32",
    "Int64",
    "Intl",
    "JSON",
    "Locale",
    "Map",
    "Math",
    "Number",
    "Object",
    "Promise",
    "Proxy",
    "RangeError",
    "ReferenceError",
    "Reflect",
    "RegExp",
    "Request",
    "Response",
    "Set",
    "String",
    "Symbol",
    "SyntaxError",
    "TypeError",
    "URIError",
    "URL",
    "URLSearchParams",
    "UUID",
    "WeakMap",
    "WeakSet",
    "abs",
    "all",
    "any",
    "bool",
    "bytes",
    "callable",
    "console",
    "decodeURI",
    "decodeURIComponent",
    "delattr",
    "dict",
    "dir",
    "encodeURI",
    "encodeURIComponent",
    "enumerate",
    "filter",
    "float",
    "getattr",
    "hasattr",
    "hash",
    "id",
    "int",
    "isFinite",
    "isNaN",
    "isinstance",
    "iter",
    "len",
    "list",
    "map",
    "max",
    "min",
    "next",
    "open",
    "parseFloat",
    "parseInt",
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
];

/// Shared with Groovy, whose bare calls skip the same names (§26).
pub fn is_denied_call(name: &str) -> bool {
    DENIED_CALLS.binary_search(&name).is_ok()
}

/// What a call names.
enum Callee {
    /// A name matched against labels: an identifier or any other callee expression's text.
    Name(String),
    /// `self:m()` inside a function named on table `T`: matches `T:m` or `T.m`.
    SelfMethod(String, String),
}

struct Call {
    caller: String,
    callee: Callee,
    line: u32,
}

#[derive(Clone)]
struct Scope {
    /// The enclosing node function: (local id, table prefix of its name).
    caller: Option<(String, Option<String>)>,
    /// Inside a named function's body: no nodes, no imports.
    in_named: bool,
    /// Inside a `local` declaration: no nodes; it was searched for `require` as a whole.
    in_declaration: bool,
}

struct Walker<'a, 'b> {
    cx: &'a mut Ctx<'b>,
    /// Label without `()` → local id, first definition wins.
    labels: HashMap<String, String>,
    calls: Vec<Call>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(Vec::new());
    let mut w = Walker {
        cx,
        labels: HashMap::new(),
        calls: Vec::new(),
    };
    if let Some(tree) = tree {
        let scope = Scope {
            caller: None,
            in_named: false,
            in_declaration: false,
        };
        w.walk(tree.root_node(), &scope);
    }
    w.finish();
}

impl Walker<'_, '_> {
    fn walk(&mut self, node: TsNode, scope: &Scope) {
        match node.kind() {
            "function_declaration" => {
                let mut inner = Scope {
                    caller: None,
                    in_named: true,
                    in_declaration: scope.in_declaration,
                };
                if !scope.in_named && !scope.in_declaration {
                    if let Some(name) = node.child_by_field_name("name") {
                        inner.caller = Some(self.declare(node, name));
                    }
                }
                self.walk_children(node, &inner);
            }
            "variable_declaration" => {
                if !scope.in_named && !scope.in_declaration {
                    if let Some(module) = require_module(self.cx.text(node)) {
                        self.import(module, Ctx::line(node));
                    }
                }
                let inner = Scope {
                    in_declaration: true,
                    ..scope.clone()
                };
                self.walk_children(node, &inner);
            }
            "function_call" => {
                self.call(node, scope);
                self.walk_children(node, scope);
            }
            _ => self.walk_children(node, scope),
        }
    }

    fn walk_children(&mut self, node: TsNode, scope: &Scope) {
        let mut c = node.walk();
        for child in node.children(&mut c) {
            self.walk(child, scope);
        }
    }

    /// A function node; returns the scope's caller (id, table prefix).
    fn declare(&mut self, node: TsNode, name: TsNode) -> (String, Option<String>) {
        let text = self.cx.text(name);
        let id = Ctx::child_id(&self.cx.file_id, text);
        let line = Ctx::line(node);
        self.cx.add_def(
            id.clone(),
            format!("{text}()"),
            line,
            vec![flag("_callable")],
        );
        let file = self.cx.file_id.clone();
        self.cx.add_edge(&file, &id, "contains", line, None);
        self.labels
            .entry(text.to_string())
            .or_insert_with(|| id.clone());
        let table = match name.kind() {
            "dot_index_expression" | "method_index_expression" => name
                .child_by_field_name("table")
                .map(|t| self.cx.text(t).to_string()),
            _ => None,
        };
        (id, table)
    }

    fn call(&mut self, node: TsNode, scope: &Scope) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let line = Ctx::line(node);
        if name.kind() == "identifier"
            && self.cx.text(name) == "require"
            && !scope.in_named
            && !scope.in_declaration
        {
            if let Some(module) = require_module(self.cx.text(node)) {
                self.import(module, line);
            }
        }
        let Some((caller, table)) = &scope.caller else {
            return;
        };
        let callee = match name.kind() {
            "identifier" => {
                let text = self.cx.text(name);
                if is_denied_call(text) {
                    return;
                }
                Callee::Name(text.to_string())
            }
            "method_index_expression" => {
                let (Some(recv), Some(method), Some(table)) = (
                    name.child_by_field_name("table"),
                    name.child_by_field_name("method"),
                    table,
                ) else {
                    return;
                };
                if recv.kind() != "identifier" || self.cx.text(recv) != "self" {
                    return;
                }
                Callee::SelfMethod(table.clone(), self.cx.text(method).to_string())
            }
            _ => Callee::Name(self.cx.text(name).to_string()),
        };
        self.calls.push(Call {
            caller: caller.clone(),
            callee,
            line,
        });
    }

    fn import(&mut self, module: &str, line: u32) {
        let target = format!("{VIRTUAL}{}|{module}", self.cx.rel);
        let mut e = self.cx.edge(
            &self.cx.file_id,
            &target,
            "imports",
            Loc::Absent,
            Some("import"),
        );
        e.score = Some(1.0);
        // Graphify writes these locations as bare numbers (quirk LU1).
        e.attrs
            .push(("source_location".into(), Attr::Str(line.to_string())));
        self.cx.facts.edges.push(e);
    }

    fn finish(mut self) {
        let mut linked: HashSet<(String, String)> = HashSet::new();
        let mut facts = LuaFacts::default();
        for c in std::mem::take(&mut self.calls) {
            let target = match &c.callee {
                Callee::Name(n) => self.labels.get(n),
                Callee::SelfMethod(t, m) => self
                    .labels
                    .get(&format!("{t}:{m}"))
                    .or_else(|| self.labels.get(&format!("{t}.{m}"))),
            };
            match (target, c.callee) {
                (Some(t), _) => {
                    if linked.insert((c.caller.clone(), t.clone())) {
                        let e = call_edge(self.cx, &c.caller, t, c.line);
                        self.cx.facts.edges.push(e);
                    }
                }
                (None, Callee::Name(callee)) => facts.calls.push(LuaCall {
                    caller: c.caller,
                    callee,
                    line: c.line,
                }),
                (None, Callee::SelfMethod(..)) => {}
            }
        }
        self.cx.facts.lang = LangFacts::Lua(facts);
    }
}

fn call_edge(cx: &Ctx, source: &str, target: &str, line: u32) -> Edge {
    Edge {
        source: source.into(),
        target: target.into(),
        relation: "calls".into(),
        confidence: Confidence::Extracted,
        score: None,
        context: Some("call".into()),
        source_file: cx.rel.into(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

/// The module of the first `require` in `text`, read the way Graphify's pattern does:
/// `require`, optional space, then `(` with an optional `[[`, `"` or `'`, or a bare `"`/`'`,
/// optional space, then the longest run of word characters, `.`, `-` and `/`.
/// `require [[m]]`, `require(("m"))` and `require ""` name nothing.
pub fn require_module(text: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(at) = text[from..].find("require") {
        let start = from + at + "require".len();
        from = start;
        let mut rest = text[start..].trim_start();
        if let Some(r) = rest.strip_prefix('(') {
            rest = r.trim_start();
            rest = rest
                .strip_prefix("[[")
                .or_else(|| rest.strip_prefix(['"', '\'']))
                .unwrap_or(rest);
        } else if let Some(r) = rest.strip_prefix(['"', '\'']) {
            rest = r;
        } else {
            continue;
        }
        let rest = rest.trim_start();
        let end = rest
            .char_indices()
            .find(|&(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '/')))
            .map_or(rest.len(), |(i, _)| i);
        if end > 0 {
            return Some(&rest[..end]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::require_module;

    #[test]
    fn require_forms() {
        let cases = [
            (r#"local a = require "pl.utils""#, Some("pl.utils")),
            ("require('x')", Some("x")),
            ("require ( \"s11\" )", Some("s11")),
            ("require([[s3]])", Some("s3")),
            ("require(\n[[v13]])", Some("v13")),
            ("require(x1)", Some("x1")),
            ("require(\"s1\" .. x)", Some("s1")),
            ("require \"s13 x\"", Some("s13")),
            ("require \"s15\\\"x\"", Some("s15")),
            ("require \"  v25  \"", Some("v25")),
            ("local q = myrequire(\"r58\")", Some("r58")),
            ("require'S18-a.b/c'", Some("S18-a.b/c")),
            ("require [[s2]]", None),
            ("require [=[s16]=]", None),
            ("require((\"x7\"))", None),
            ("require \"\"", None),
            ("pcall(require, \"r27\")", None),
            ("x = require(-1)", Some("-1")),
        ];
        for (text, want) in cases {
            assert_eq!(require_module(text), want, "{text}");
        }
    }
}

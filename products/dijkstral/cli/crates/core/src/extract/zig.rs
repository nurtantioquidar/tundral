//! Zig extraction (§33), `.zig` files, tree-sitter-zig 1.1.2. See `docs/graphify-format.md`.
//!
//! One file yields its file node and, from declarations outside function bodies (`test` and
//! `comptime` blocks included, at any depth):
//!
//! - every `const`/`var` whose declaration holds a `struct`, `enum` or `union` declaration (any
//!   layout keyword, also in the type position: `const t: struct {...} = .{}`) or an error set:
//!   a node `file_slug(Name)`, label `Name`, contained by the **file** even when nested in
//!   another container (flat ids, as C++'s classes). A declaration that also holds a field chain
//!   (`var s: enum { a } = .a;`, `@import("x").T`) gives no container.
//! - functions (`file_slug(name)`, `name()`, `contains`) and container functions as methods
//!   (`container_slug(name)`, `.name()`, `method`), `extern`/`export`/`inline` included. The
//!   functions of an anonymous container in a `test` block's statement (`run(struct { fn cb()
//!   ... })`) are file-level functions; one in a variable declaration's value is not walked.
//! - `case_of` members: enum fields, fields of a tagged union (`union(enum)`, `union(Tag)`) and
//!   error-set names. A struct field whose type is an inline container hands that container's
//!   members to the struct (`kind: enum { a, b }` → cases `a`, `b` of the struct; an inline
//!   struct's functions become its methods); enum and union fields' types are not walked. A
//!   non-exhaustive `_` slugs to nothing, so it is a `case_of` self-loop on the enum (the build
//!   keeps the first of repeated self-loops, §8.2).
//!
//! Not emitted: struct and plain-union fields, `opaque` declarations (methods included), type
//! aliases, merged error sets, containers declared inside functions (generic `return struct`),
//! comments (no rationale), type references. No node carries `_callable`.
//!
//! `@import("p")` as a direct child of a variable declaration (alone or as the object of a field
//! chain, `@import("std").mem`) gives `file -imports_from-> slug(first dot-separated part of p's
//! last path segment)` (no context): `"sub/deep.zig"` → `deep`, `"x.y.zig"` → `x`. The per-file
//! edge targets [`virtual_target`], which `crate::resolve::zig` points at the node with that id,
//! the only file with that stem, or an external node. `usingnamespace`, arguments of calls and
//! conditionals, imports inside functions and `@cImport` give nothing.
//!
//! Calls are collected from function bodies, including the fields and constants of containers
//! declared there but not the bodies of their functions: the callee is the identifier or the
//! last member of a field chain (`a.b.f()` → `f`). A call binds in the file to the **first**
//! function or method of that name in declaration order, whatever the receiver (no
//! own-container preference); a function whose id an earlier node took answers to no call; a
//! call that binds to its caller is dropped; one edge per (caller, callee), first line kept,
//! EXTRACTED without context or score, after all definition edges. Bare calls that bind nothing
//! in the file are left for the cross-file pass in [`ZigFacts::calls`]; member calls that bind
//! nothing are dropped.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{slug, Ctx};
use crate::facts::{LangFacts, Loc};

/// Prefix of the virtual import targets; see the module docs.
pub const VIRTUAL: &str = "@zi:";

/// The virtual target of an import of `id` from the file at `rel`: per importing file, because
/// an import that lands on the importing file itself is dropped.
pub fn virtual_target(id: &str, rel: &str) -> String {
    format!("{VIRTUAL}{id}|{rel}")
}

/// A bare call the file could not bind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZCall {
    /// Local id of the calling function or method.
    pub caller: String,
    pub name: String,
    pub line: u32,
}

/// Facts for the cross-file pass (`crate::resolve::zig`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ZigFacts {
    pub file_id: String,
    /// Import target ids (`slug` of the imported name), in source order.
    pub imports: Vec<String>,
    pub calls: Vec<ZCall>,
}

const CONTAINERS: &[&str] = &[
    "struct_declaration",
    "enum_declaration",
    "union_declaration",
    "error_set_declaration",
];

struct RawCall {
    caller: String,
    name: String,
    bare: bool,
    line: u32,
}

struct Z<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    facts: ZigFacts,
    /// Function and method call names → first id.
    names: HashMap<String, String>,
    /// Ids of the definitions emitted so far.
    emitted: HashSet<String>,
    calls: Vec<RawCall>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let file_id = cx.file_id.clone();
    let mut z = Z {
        cx,
        facts: ZigFacts {
            file_id,
            ..Default::default()
        },
        names: HashMap::new(),
        emitted: HashSet::new(),
        calls: Vec::new(),
    };
    if let Some(tree) = tree {
        z.walk(tree.root_node());
    }
    z.bind_calls();
    let Z { cx, facts, .. } = z;
    cx.facts.lang = LangFacts::Zig(facts);
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut cur = node.walk();
    node.named_children(&mut cur).collect()
}

/// The identifier naming a declaration (`const Name = ...`, `fn name(...)`).
fn decl_name<'t>(node: TsNode<'t>) -> Option<TsNode<'t>> {
    node.child_by_field_name("name").or_else(|| {
        named_children(node)
            .into_iter()
            .find(|c| c.kind() == "identifier")
    })
}

impl Z<'_, '_> {
    /// Declarations outside named containers: functions are file-level functions (also those of
    /// an anonymous container in a `test` block's statement), variable declarations are handled
    /// on their own (never walked into), and everything else (`test` and `comptime` blocks,
    /// statements, anonymous containers) is walked through.
    fn walk(&mut self, node: TsNode) {
        for child in named_children(node) {
            match child.kind() {
                "function_declaration" => self.function(child, None),
                "variable_declaration" => self.variable(child),
                _ => self.walk(child),
            }
        }
    }

    fn function(&mut self, node: TsNode, owner: Option<&str>) {
        let Some(name) = decl_name(node) else {
            return;
        };
        let name = self.cx.text(name).to_string();
        let line = Ctx::line(node);
        let (id, label, src, rel) = match owner {
            Some(o) => (
                Ctx::child_id(o, &name),
                format!(".{name}()"),
                o.to_string(),
                "method",
            ),
            None => (
                Ctx::child_id(&self.cx.file_id, &name),
                format!("{name}()"),
                self.cx.file_id.clone(),
                "contains",
            ),
        };
        // A function whose id an earlier node already took (`fn sentinel` after `const
        // Sentinel`) is lost in that node: no call binds to it (its own calls still count).
        if self.emitted.insert(id.clone()) {
            self.names.entry(name).or_insert_with(|| id.clone());
        }
        self.cx.add_def(id.clone(), label, line, vec![]);
        self.cx.add_edge(&src, &id, rel, line, None);
        if let Some(body) = node.child_by_field_name("body") {
            self.collect_calls(body, &id);
        }
    }

    fn variable(&mut self, node: TsNode) {
        let children = named_children(node);
        if let Some(path) = children.iter().find_map(|c| self.import_path(*c)) {
            self.import(&path, Ctx::line(node));
        }
        // A field chain (an import's member, an enum literal `.a`) hides a container in the
        // type position: `var s: enum { a, b } = .a;` gives nothing.
        if children.iter().any(|c| c.kind() == "field_expression") {
            return;
        }
        let Some(value) = children
            .iter()
            .copied()
            .find(|c| CONTAINERS.contains(&c.kind()))
        else {
            return;
        };
        let Some(name) = decl_name(node) else {
            return;
        };
        let name = self.cx.text(name).to_string();
        let line = Ctx::line(node);
        let id = Ctx::child_id(&self.cx.file_id, &name);
        self.emitted.insert(id.clone());
        self.cx.add_def(id.clone(), name, line, vec![]);
        let file = self.cx.file_id.clone();
        self.cx.add_edge(&file, &id, "contains", line, None);
        if value.kind() == "error_set_declaration" {
            for member in named_children(value) {
                if member.kind() == "identifier" {
                    self.member(&id, member, member);
                }
            }
            return;
        }
        self.body(&id, value);
    }

    /// The members of container declaration `decl`, attributed to the named container `owner`.
    /// Fields of an enum or tagged union are cases (their types are not walked); a struct or plain
    /// union field whose type is an inline container contributes that container's members to
    /// `owner` too (`kind: enum { a, b }` → cases `a`, `b` of the struct).
    fn body(&mut self, owner: &str, decl: TsNode) {
        let cases = decl.kind() == "enum_declaration"
            || (decl.kind() == "union_declaration" && is_tagged_union(decl));
        for member in named_children(decl) {
            match member.kind() {
                "container_field" if cases => {
                    if let Some(n) = decl_name(member) {
                        self.member(owner, member, n);
                    }
                }
                "container_field" => {
                    for inline in inline_containers(member) {
                        self.body(owner, inline);
                    }
                }
                "function_declaration" => self.function(member, Some(owner)),
                "variable_declaration" => self.variable(member),
                "test_declaration" | "comptime_declaration" => self.walk(member),
                _ => {}
            }
        }
    }

    /// A `case_of` member of container `owner`.
    fn member(&mut self, owner: &str, at: TsNode, name: TsNode) {
        let label = self.cx.text(name).to_string();
        let id = Ctx::child_id(owner, &label);
        let line = Ctx::line(at);
        // The non-exhaustive `_` slugs to nothing: a self-loop.
        if id != owner {
            self.emitted.insert(id.clone());
            self.cx.add_def(id.clone(), label, line, vec![]);
        }
        self.cx.add_edge(owner, &id, "case_of", line, None);
    }

    /// `@import("p")`, directly or as the innermost object of a field chain.
    fn import_path(&self, node: TsNode) -> Option<String> {
        let mut cur = node;
        while cur.kind() == "field_expression" {
            cur = cur.child_by_field_name("object")?;
        }
        if cur.kind() != "builtin_function" {
            return None;
        }
        let children = named_children(cur);
        let builtin = children.iter().find(|c| c.kind() == "builtin_identifier")?;
        if self.cx.text(*builtin) != "@import" {
            return None;
        }
        let args = children.iter().find(|c| c.kind() == "arguments")?;
        let string = named_children(*args)
            .into_iter()
            .find(|c| c.kind() == "string")?;
        let content = named_children(string)
            .into_iter()
            .find(|c| c.kind() == "string_content")?;
        Some(self.cx.text(content).to_string())
    }

    fn import(&mut self, path: &str, line: u32) {
        let id = import_id(path);
        if id.is_empty() {
            return;
        }
        let file = self.cx.file_id.clone();
        let target = virtual_target(&id, self.cx.rel);
        self.cx.add_edge(&file, &target, "imports_from", line, None);
        self.facts.imports.push(id);
    }

    fn collect_calls(&mut self, node: TsNode, caller: &str) {
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            // Containers declared in the body are walked (field types, constants), the bodies
            // of their functions are not.
            if n.kind() == "function_declaration" && n != node {
                continue;
            }
            if n.kind() == "call_expression" {
                if let Some(f) = n.child_by_field_name("function") {
                    let (name, bare) = match f.kind() {
                        "identifier" => (Some(f), true),
                        "field_expression" => (f.child_by_field_name("member"), false),
                        _ => (None, false),
                    };
                    if let Some(name) = name {
                        self.calls.push(RawCall {
                            caller: caller.to_string(),
                            name: self.cx.text(name).to_string(),
                            bare,
                            line: Ctx::line(n),
                        });
                    }
                }
            }
            let mut cur = n.walk();
            let children: Vec<TsNode> = n.children(&mut cur).collect();
            stack.extend(children.into_iter().rev());
        }
    }

    fn bind_calls(&mut self) {
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for c in std::mem::take(&mut self.calls) {
            match self.names.get(&c.name) {
                Some(target) => {
                    if *target == c.caller || !seen.insert((c.caller.clone(), target.clone())) {
                        continue;
                    }
                    let e = self
                        .cx
                        .edge(&c.caller, target, "calls", Loc::Line(c.line), None);
                    self.cx.facts.edges.push(e);
                }
                None if c.bare => self.facts.calls.push(ZCall {
                    caller: c.caller,
                    name: c.name,
                    line: c.line,
                }),
                None => {}
            }
        }
    }
}

/// The container declarations in a field's type (`kind: enum { ... }`, `opt: ?enum { ... }`),
/// not looking inside them.
fn inline_containers<'t>(field: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut out = Vec::new();
    let mut stack: Vec<TsNode> = named_children(field).into_iter().rev().collect();
    while let Some(n) = stack.pop() {
        if CONTAINERS.contains(&n.kind()) && n.kind() != "error_set_declaration" {
            out.push(n);
        } else if n.kind() != "opaque_declaration" {
            stack.extend(named_children(n).into_iter().rev());
        }
    }
    out
}

/// A union with a tag (`union(enum)`, `union(Tag)`) lists its fields as cases.
fn is_tagged_union(node: TsNode) -> bool {
    let mut cur = node.walk();
    for c in node.children(&mut cur) {
        match c.kind() {
            "(" => return true,
            "{" => return false,
            _ => {}
        }
    }
    false
}

/// The import target id: `slug` of the first dot-separated part of the path's last segment.
pub fn import_id(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    slug(base.split('.').next().unwrap_or(base))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_file;
    use crate::lang::Lang;

    fn lines(src: &str) -> Vec<String> {
        let f = extract_file(Lang::Zig, "a.zig", src.as_bytes());
        f.edges
            .iter()
            .map(|e| {
                let Loc::Line(l) = e.loc else { unreachable!() };
                format!("{} -{}-> {} L{l}", e.source, e.relation, e.target)
            })
            .collect()
    }

    #[test]
    fn import_ids_take_the_first_part_of_the_last_segment() {
        assert_eq!(import_id("std"), "std");
        assert_eq!(import_id("sub/deep.zig"), "deep");
        assert_eq!(import_id("x.y.zig"), "x");
        assert_eq!(import_id("../Upper.zig"), "upper");
        assert_eq!(import_id(""), "");
    }

    #[test]
    fn containers_are_flat_and_members_are_cases() {
        let src = "const A = struct {\n    const In = enum { a, _ };\n    fn m() void {}\n};\nconst U = union { x: i32 };\nconst T = union(enum) { y: i32 };\nconst E = error{ Boom };\n";
        let got = lines(src);
        for want in [
            "a -contains-> a_a L1",
            "a -contains-> a_in L2",
            "a_in -case_of-> a_in_a L2",
            "a_in -case_of-> a_in L2",
            "a_a -method-> a_a_m L3",
            "a -contains-> a_u L5",
            "a_t -case_of-> a_t_y L6",
            "a_e -case_of-> a_e_boom L7",
        ] {
            assert!(got.iter().any(|g| g == want), "{want} not in {got:?}");
        }
        assert!(!got.iter().any(|g| g.contains("a_u_x")));
    }

    #[test]
    fn inline_field_types_and_test_blocks_add_members() {
        let src = "const S = struct {\n    kind: enum { p, q },\n    v: union(enum) { r: struct { s: enum { t } } },\n};\nvar e: enum { z } = .z;\ntest \"x\" {\n    run(struct {\n        fn cb() void {}\n    });\n}\n";
        let got = lines(src);
        for want in [
            "a_s -case_of-> a_s_p L2",
            "a_s -case_of-> a_s_q L2",
            "a_s -case_of-> a_s_r L3",
            "a -contains-> a_cb L8",
        ] {
            assert!(got.iter().any(|g| g == want), "{want} not in {got:?}");
        }
        assert!(!got.iter().any(|g| g.contains("a_s_t") || g.contains("a_e")));
    }

    #[test]
    fn calls_bind_to_the_first_definition_and_skip_self() {
        let src = "const A = struct {\n    fn go() void {}\n    fn run() void { go(); run(); }\n};\nfn go() void {}\nfn main() void { go(); go(); x.run(); far(); y.far(); }\n";
        let f = extract_file(Lang::Zig, "a.zig", src.as_bytes());
        let calls: Vec<String> = f
            .edges
            .iter()
            .filter(|e| e.relation == "calls")
            .map(|e| format!("{}>{}", e.source, e.target))
            .collect();
        assert_eq!(calls, ["a_a_run>a_a_go", "a_main>a_a_go", "a_main>a_a_run"]);
        let LangFacts::Zig(z) = f.lang else {
            unreachable!()
        };
        assert_eq!(z.calls.len(), 1);
        assert_eq!(z.calls[0].name, "far");
    }

    #[test]
    fn imports_outside_functions_target_virtual_ids() {
        let src = "const std = @import(\"std\");\nconst m = @import(\"std\").mem;\nfn f() void { const x = @import(\"in.zig\"); _ = x; }\ntest \"t\" { const t = @import(\"t.zig\"); _ = t; }\nusingnamespace @import(\"ns.zig\");\n";
        let got = lines(src);
        let imports: Vec<&String> = got.iter().filter(|g| g.contains("imports_from")).collect();
        assert_eq!(
            imports,
            [
                "a -imports_from-> @zi:std|a.zig L1",
                "a -imports_from-> @zi:std|a.zig L2",
                "a -imports_from-> @zi:t|a.zig L4"
            ]
        );
    }
}

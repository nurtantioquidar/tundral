//! Scala (§22) extraction, `.scala` only (`.sc` and `.sbt` are not classified). See
//! `docs/graphify-format.md`.
//!
//! One file yields its file node; every class, trait, object, enum and named `given` (`file_Name`,
//! flat for nested types; a companion object shares its class's id and adds a second
//! `contains`); `def`s as methods (`owner_name`) or, at the top level and in package objects and
//! extensions, functions (`file_name`); enum cases; per-file type stubs (`slug(Name)`, standard
//! types included: `String`, `Int`, `Unit`…); and the in-file edges:
//!
//! - `inherits` to the first type of an `extends` clause, `mixes_in` to every `with` type (each
//!   at its own line, no type-argument references), `requires` to every type named in a self
//!   type, `case_of` to enum cases.
//! - `references`: `field` from every primary-constructor parameter, typed `val`/`var` and type
//!   alias of a class (of the file, at the top level); `parameter_type` from the first parameter
//!   list, `return_type`, `generic_arg` for type arguments, `type_bound` for context bounds.
//!   Tuple, function, by-name, repeated, compound and projected types reference each part; a
//!   qualified type (`mutable.Map[K, V]`) references only its arguments.
//! - `calls` from `def` bodies, bound by name to the file's last definition of that name.
//!
//! No annotations, anonymous classes, local `def`s or rationale nodes. Declarations found inside
//! expressions of a class body (a `test("…") { val x: T = … }` block, an anonymous class in a
//! `val`) are taken as top-level declarations of the file, as Graphify does.
//!
//! Imports target `@si:<first segment>`, which `crate::resolve::scala` turns into an external
//! node (or the file with that stem); calls the file could not bind are recorded in
//! [`ScalaFacts::calls`].

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, slug, Ctx};
use crate::extract::kotlin::is_denied_call;
use crate::facts::{Edge, Loc};

/// A bare call the file could not bind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SCall {
    /// Local id of the calling `def`.
    pub caller: String,
    pub name: String,
    /// The file references a type named like the callee.
    pub via_stub: bool,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::scala`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScalaFacts {
    pub file_id: String,
    /// Per-file type stubs: (local id, label).
    pub stubs: Vec<(String, String)>,
    pub calls: Vec<SCall>,
}

/// Prefix of the virtual import targets; see the module docs.
pub const VIRTUAL: &str = "@si:";

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn is_class_def(kind: &str) -> bool {
    matches!(
        kind,
        "class_definition" | "object_definition" | "trait_definition" | "enum_definition"
    )
}

fn is_declaration(kind: &str) -> bool {
    is_class_def(kind)
        || matches!(
            kind,
            "function_definition"
                | "function_declaration"
                | "val_definition"
                | "var_definition"
                | "val_declaration"
                | "var_declaration"
                | "type_definition"
                | "given_definition"
                | "enum_case_definitions"
                | "self_type"
                | "import_declaration"
                | "package_clause"
        )
}

/// The declarations of a file or class body: direct ones (`false`), then, in place, those found
/// by walking into any other node (`true`: package objects, extensions, expression blocks).
fn members<'t>(node: TsNode<'t>) -> Vec<(TsNode<'t>, bool)> {
    let mut out = Vec::new();
    for ch in named_children(node) {
        if is_declaration(ch.kind()) {
            out.push((ch, false));
        } else {
            nested(ch, &mut out);
        }
    }
    out
}

fn nested<'t>(node: TsNode<'t>, out: &mut Vec<(TsNode<'t>, bool)>) {
    for ch in named_children(node) {
        if is_declaration(ch.kind()) {
            out.push((ch, true));
        } else {
            nested(ch, out);
        }
    }
}

struct Sc<'c, 'a, 't> {
    cx: &'c mut Ctx<'a>,
    /// `slug(path with extension)`: a definition with this id merges into the file node.
    merged: String,
    /// Ids of the definitions emitted so far: a type name binds in the file when
    /// `file_slug(Name)` is one of them (a class, object, `given` or top-level `def`).
    local_ids: HashSet<String>,
    /// Call name → local id of the file's last definition with that name.
    callables: HashMap<String, String>,
    seen_nodes: HashSet<String>,
    /// Stub label → id.
    stubs: HashMap<String, String>,
    refs_seen: HashSet<(String, String, &'static str, u32)>,
    bodies: Vec<(String, TsNode<'t>)>,
    facts: ScalaFacts,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let file_id = cx.file_id.clone();
    let merged = slug(cx.rel);
    let mut s = Sc {
        merged,
        local_ids: HashSet::new(),
        callables: HashMap::new(),
        seen_nodes: HashSet::from([file_id.clone()]),
        stubs: HashMap::new(),
        refs_seen: HashSet::new(),
        bodies: Vec::new(),
        facts: ScalaFacts {
            file_id,
            ..ScalaFacts::default()
        },
        cx,
    };
    if let Some(tree) = tree {
        s.file(tree.root_node());
    }
    let Sc { cx, facts, .. } = s;
    cx.facts.lang = crate::facts::LangFacts::Scala(facts);
}

impl<'c, 'a, 't> Sc<'c, 'a, 't> {
    fn text(&self, node: TsNode) -> &'a str {
        self.cx.text(node)
    }

    fn nid(&self, id: &str) -> String {
        if id == self.merged {
            self.cx.file_id.clone()
        } else {
            id.to_string()
        }
    }

    fn file(&mut self, root: TsNode<'t>) {
        let file_id = self.cx.file_id.clone();
        for (d, _) in members(root) {
            self.top_level(d, &file_id);
        }
        self.calls();
    }

    fn import(&mut self, imp: TsNode, file_id: &str) {
        let Some(first) = imp
            .child_by_field_name("path")
            .filter(|p| p.kind() == "identifier")
        else {
            return;
        };
        let target = format!("{VIRTUAL}{}|{}", self.cx.rel, slug(self.text(first)));
        self.cx
            .add_edge(file_id, &target, "imports", Ctx::line(imp), Some("import"));
    }

    /// A declaration in file context.
    fn top_level(&mut self, d: TsNode<'t>, file_id: &str) {
        match d.kind() {
            k if is_class_def(k) => self.class(d, file_id, file_id),
            "function_definition" | "function_declaration" => self.function(d, file_id, true),
            "val_definition" | "var_definition" | "val_declaration" | "var_declaration" => {
                self.value(d, file_id, file_id)
            }
            "type_definition" => self.type_alias(d, file_id),
            "given_definition" => self.given(d, file_id, file_id),
            "import_declaration" => self.import(d, file_id),
            _ => {}
        }
    }

    fn put_def(&mut self, id: &str, label: String, line: u32, class: bool) {
        if id == self.merged {
            let node = &mut self.cx.facts.nodes[0];
            let mut add = vec!["_callable"];
            if class {
                add.push("_callable_class");
            }
            for key in add {
                if node.attr(key).is_none() {
                    node.attrs.insert(node.attrs.len() - 1, flag(key));
                }
            }
            return;
        }
        self.local_ids.insert(id.to_string());
        if self.seen_nodes.insert(id.to_string()) {
            let mut extra = vec![flag("_callable")];
            if class {
                extra.push(flag("_callable_class"));
            }
            self.cx.add_def(id.to_string(), label, line, extra);
        }
    }

    fn edge(&mut self, source: &str, target: &str, relation: &str, line: u32) {
        let (s, t) = (self.nid(source), self.nid(target));
        self.cx.add_edge(&s, &t, relation, line, None);
    }

    fn class(&mut self, decl: TsNode<'t>, file_id: &str, parent: &str) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(file_id, &name);
        let line = Ctx::line(decl);
        self.put_def(&id, name.clone(), line, true);
        self.edge(parent, &id, "contains", line);
        if id != self.merged {
            self.callables.insert(name, id.clone());
        }
        if let Some(ext) = decl.child_by_field_name("extend") {
            self.supertypes(&id, ext);
        }
        for params in named_children(decl)
            .into_iter()
            .filter(|n| n.kind() == "class_parameters")
        {
            for p in named_children(params) {
                if p.kind() != "class_parameter" {
                    continue;
                }
                if let Some(t) = p.child_by_field_name("type") {
                    self.type_refs(&id, t, "field", Ctx::line(p));
                }
            }
        }
        if let Some(body) = decl.child_by_field_name("body") {
            self.body(body, file_id, &id);
        }
    }

    fn supertypes(&mut self, id: &str, ext: TsNode) {
        let mut c = ext.walk();
        let types: Vec<TsNode> = ext
            .children_by_field_name("type", &mut c)
            .filter(|t| t.is_named())
            .collect();
        for (i, t) in types.into_iter().enumerate() {
            let Some(name) = self.supertype_name(t) else {
                continue;
            };
            let target = self.target(&name);
            if self.nid(&target) == self.nid(id) {
                // `object Assertions extends Assertions`: no edge to itself.
                continue;
            }
            let relation = if i == 0 { "inherits" } else { "mixes_in" };
            self.edge(id, &target, relation, Ctx::line(t));
        }
    }

    /// `Base`, `Base[T]`, `a.b.Base` → `Base`.
    fn supertype_name(&self, t: TsNode) -> Option<String> {
        match t.kind() {
            "type_identifier" => Some(self.text(t).to_string()),
            "generic_type" => self.supertype_name(t.child_by_field_name("type")?),
            "stable_type_identifier" => named_children(t)
                .into_iter()
                .last()
                .map(|n| self.text(n).to_string()),
            _ => None,
        }
    }

    fn body(&mut self, body: TsNode<'t>, file_id: &str, owner: &str) {
        for (m, nested) in members(body) {
            if nested {
                self.top_level(m, file_id);
                continue;
            }
            match m.kind() {
                k if is_class_def(k) => self.class(m, file_id, owner),
                "function_definition" | "function_declaration" => self.function(m, owner, false),
                "val_definition" | "var_definition" | "val_declaration" | "var_declaration" => {
                    self.value(m, file_id, owner)
                }
                "type_definition" => self.type_alias(m, owner),
                "given_definition" => self.given(m, file_id, owner),
                "enum_case_definitions" => self.enum_cases(m, owner),
                "self_type" => self.self_type(m, owner),
                "import_declaration" => self.import(m, file_id),
                _ => {}
            }
        }
    }

    /// A typed `val`/`var` is a `field` reference of its owner; its value is walked for
    /// declarations in file context.
    fn value(&mut self, decl: TsNode<'t>, file_id: &str, owner: &str) {
        if let Some(t) = decl.child_by_field_name("type") {
            self.type_refs(owner, t, "field", Ctx::line(decl));
        }
        if let Some(v) = decl.child_by_field_name("value") {
            let mut found = Vec::new();
            if is_declaration(v.kind()) {
                found.push((v, true));
            } else {
                nested(v, &mut found);
            }
            for (d, _) in found {
                self.top_level(d, file_id);
            }
        }
    }

    fn type_alias(&mut self, decl: TsNode, owner: &str) {
        if let Some(t) = decl.child_by_field_name("type") {
            self.type_refs(owner, t, "field", Ctx::line(decl));
        }
    }

    /// A named `given` is a class-like node of its owner, with no references.
    fn given(&mut self, decl: TsNode, file_id: &str, owner: &str) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(file_id, &name);
        let line = Ctx::line(decl);
        self.put_def(&id, name.clone(), line, true);
        self.edge(owner, &id, "contains", line);
        if id != self.merged {
            self.callables.insert(name, id);
        }
    }

    fn enum_cases(&mut self, defs: TsNode, owner: &str) {
        for case in named_children(defs) {
            if !matches!(case.kind(), "simple_enum_case" | "full_enum_case") {
                continue;
            }
            let Some(name) = case.child_by_field_name("name") else {
                continue;
            };
            let name = self.text(name).to_string();
            let id = Ctx::child_id(owner, &name);
            let line = Ctx::line(case);
            if self.seen_nodes.insert(id.clone()) {
                self.cx.add_def(id.clone(), name.clone(), line, vec![]);
            }
            self.edge(owner, &id, "case_of", line);
            self.callables.insert(name, id);
        }
    }

    /// `self: A with B[C] =>`: `requires` to every type named in it.
    fn self_type(&mut self, st: TsNode, owner: &str) {
        let line = Ctx::line(st);
        let mut names = Vec::new();
        for ch in named_children(st).into_iter().skip(1) {
            self.type_names(ch, &mut names);
        }
        for name in names {
            let target = self.target(&name);
            self.edge(owner, &target, "requires", line);
        }
    }

    fn type_names(&self, t: TsNode, out: &mut Vec<String>) {
        if t.kind() == "type_identifier" {
            out.push(self.text(t).to_string());
            return;
        }
        for ch in named_children(t) {
            self.type_names(ch, out);
        }
    }

    fn function(&mut self, decl: TsNode<'t>, owner: &str, top_level: bool) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(owner, &name);
        let line = Ctx::line(decl);
        let label = if top_level {
            format!("{name}()")
        } else {
            format!(".{name}()")
        };
        self.put_def(&id, label, line, false);
        let relation = if top_level { "contains" } else { "method" };
        self.edge(owner, &id, relation, line);
        if id != self.merged {
            self.callables.insert(name, id.clone());
        }
        // Context bounds (`[T: Ordering]`) are `type_bound` references.
        for tps in named_children(decl)
            .into_iter()
            .filter(|n| n.kind() == "type_parameters")
        {
            for b in named_children(tps)
                .into_iter()
                .filter(|n| n.kind() == "context_bound")
            {
                if let Some(t) = b.child_by_field_name("type") {
                    self.type_refs(&id, t, "type_bound", line);
                }
            }
        }
        if let Some(params) = named_children(decl)
            .into_iter()
            .find(|n| n.kind() == "parameters")
        {
            for p in named_children(params) {
                if p.kind() != "parameter" {
                    continue;
                }
                if let Some(t) = p.child_by_field_name("type") {
                    self.type_refs(&id, t, "parameter_type", line);
                }
            }
        }
        if let Some(t) = decl.child_by_field_name("return_type") {
            self.type_refs(&id, t, "return_type", line);
        }
        if let Some(body) = decl.child_by_field_name("body") {
            self.bodies.push((id, body));
        }
    }

    // ---- type references ----------------------------------------------------------------------

    /// This file's definition `file_slug(Name)` emitted so far, or the file's stub for it.
    fn target(&mut self, name: &str) -> String {
        let local = Ctx::child_id(&self.facts.file_id, name);
        if self.local_ids.contains(&local) {
            return self.nid(&local);
        }
        if let Some(id) = self.stubs.get(name) {
            return id.clone();
        }
        let id = slug(name);
        self.stubs.insert(name.to_string(), id.clone());
        if !self.facts.stubs.iter().any(|(s, _)| *s == id) {
            self.cx.add_stub(id.clone(), name.to_string());
            self.facts.stubs.push((id.clone(), name.to_string()));
        }
        id
    }

    fn reference(&mut self, source: &str, name: &str, ctx: &'static str, line: u32) {
        let target = self.target(name);
        let (s, t) = (self.nid(source), self.nid(&target));
        // A definition never references itself (`case class User(friends: List[User])`).
        if s != t && self.refs_seen.insert((s.clone(), t.clone(), ctx, line)) {
            self.cx.add_edge(&s, &t, "references", line, Some(ctx));
        }
    }

    fn type_refs(&mut self, source: &str, t: TsNode, ctx: &'static str, line: u32) {
        match t.kind() {
            "type_identifier" => {
                let name = self.text(t).to_string();
                self.reference(source, &name, ctx, line);
            }
            "generic_type" => {
                if let Some(base) = t.child_by_field_name("type") {
                    if base.kind() != "stable_type_identifier" {
                        self.type_refs(source, base, ctx, line);
                    }
                }
                if let Some(args) = t.child_by_field_name("type_arguments") {
                    for a in named_children(args) {
                        self.type_refs(source, a, "generic_arg", line);
                    }
                }
            }
            // A function type references its result only (`() => Future[T]`).
            "function_type" => {
                if let Some(r) = t.child_by_field_name("return_type") {
                    self.type_refs(source, r, ctx, line);
                }
            }
            // Qualified types reference only their arguments; by-name and repeated parameter
            // types reference nothing.
            "stable_type_identifier"
            | "wildcard"
            | "literal_type"
            | "singleton_type"
            | "lazy_parameter_type"
            | "repeated_parameter_type" => {}
            "projected_type" => {
                let mut names = Vec::new();
                self.type_names(t, &mut names);
                for n in names {
                    self.reference(source, &n, ctx, line);
                }
            }
            _ => {
                // tuple, function, by-name, repeated, compound, infix, annotated…: every part.
                for ch in named_children(t) {
                    self.type_refs(source, ch, ctx, line);
                }
            }
        }
    }

    // ---- calls --------------------------------------------------------------------------------

    fn calls(&mut self) {
        let work = std::mem::take(&mut self.bodies);
        let mut bound: HashSet<(String, String)> = HashSet::new();
        let mut edges: Vec<Edge> = Vec::new();
        for (caller, body) in work {
            let caller = self.nid(&caller);
            let mut stack = vec![body];
            while let Some(node) = stack.pop() {
                if node.kind() == "call_expression" {
                    self.call(node, &caller, &mut bound, &mut edges);
                }
                let mut kids = named_children(node);
                kids.retain(|k| {
                    !matches!(k.kind(), "function_definition" | "template_body")
                        && !is_class_def(k.kind())
                });
                stack.extend(kids.into_iter().rev());
            }
        }
        self.cx.facts.edges.extend(edges);
    }

    fn call(
        &mut self,
        node: TsNode,
        caller: &str,
        bound: &mut HashSet<(String, String)>,
        edges: &mut Vec<Edge>,
    ) {
        let Some(f) = node.child_by_field_name("function") else {
            return;
        };
        let (name, bare) = match f.kind() {
            "identifier" => (self.text(f).to_string(), true),
            "field_expression" => match f.child_by_field_name("field") {
                Some(n) => (self.text(n).to_string(), false),
                None => return,
            },
            _ => return,
        };
        if is_denied_call(&name) {
            return;
        }
        if let Some(target) = self.callables.get(&name) {
            let target = self.nid(target);
            if bound.insert((caller.to_string(), target.clone())) {
                edges.push(self.cx.edge(
                    caller,
                    &target,
                    "calls",
                    Loc::Line(Ctx::line(node)),
                    Some("call"),
                ));
            }
            return;
        }
        if bare {
            self.facts.calls.push(SCall {
                caller: caller.to_string(),
                via_stub: self.stubs.contains_key(&name),
                name,
                line: Ctx::line(node),
            });
        }
    }
}

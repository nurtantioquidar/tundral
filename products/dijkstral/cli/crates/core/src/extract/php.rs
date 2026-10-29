//! PHP (§19) extraction. See `docs/graphify-format.md`.
//!
//! One walk emits the file node, classes, interfaces, traits and enums (`file_Name`, flat),
//! their methods (`type_name`), enum cases, functions at any depth (`file_name`, contained by the
//! file), methods of anonymous classes (as functions), closures and arrow functions
//! (`file_closure_<n>`, label `{closure#n}()`), and the in-file `contains`, `method`, `case_of`,
//! `inherits`, `implements`, `mixes_in` (trait `use`) and `references` (parameter, return and
//! property types) edges. Namespaces never appear in ids; `require`/`include` and comments
//! produce nothing.
//!
//! A type name resolves through the file's class imports (`use A\B [as C]`; `Psr7\X` under a
//! non-aliased `use G\Psr7`), then the file's own classes. An imported class is written as the
//! virtual id `@pi:<fqn>`, which `crate::resolve::php` redirects to the class with that FQN or to
//! the shared stub `slug(fqn)`. Any other parameter, return or property type becomes a per-file
//! stub (`slug(last segment)`, label as written) that the resolver binds to the only class of
//! that name; a supertype or trait becomes `@ps:<name>`, a stub shared by every file.
//! `use function`/`use const` give the external node `slug(last segment)`.
//!
//! Calls bind in the file by name (last definition wins over functions and methods, any
//! receiver: `f()`, `$x->m()`; not `?->`), and `new X` binds to a class of the file. A member
//! call whose name the file does not define is typed instead: `$v->m()` / `$this->v->m()`, where
//! `v` is the first typed parameter or property, or `$v = new T`, of that name anywhere in the
//! file (§19.4). `X::m()` binds to the own method `m` of `X` (`self`/`static`: the enclosing
//! class; `parent`: its parent), never to the class. What does not bind is recorded in
//! [`PhpFacts::calls`]. Builtin-like names (`Proxy`, `len`, …) never bind `f()` or `new X`. `X::CONST` / `X::class` and `X::$prop`
//! inside a function give `references_constant` and `uses_static_prop` (score 1.0) when the file
//! declares `X` or writes it in a signature (its per-file stub) or a supertype list (`@ps:X`).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::go::is_unbound_name;
use super::{flag, slug, Ctx};
use crate::facts::{Edge, LangFacts, Loc};

/// A class, interface, trait or enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PClass {
    pub name: String,
    /// Namespace + name, without a leading `\`.
    pub fqn: String,
    /// Local id.
    pub id: String,
    /// Its own methods (not inherited ones): (name, local id), last of a name wins.
    pub methods: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PCallKind {
    /// `f()` with no definition named `f` in the file.
    Bare(String),
    /// `new X` with `X` (as written, leading `\` kept) not a class of the file.
    Class(String),
    /// `X::m()` with `X` (as written) not a class of the file, or `parent::m()` naming the
    /// class's written parent; `report`: record the call in `metadata.unresolved_calls` when no
    /// class has that name (never for `parent::`).
    Static {
        scope: String,
        method: String,
        report: bool,
    },
    /// `$v->m()` / `$this->v->m()` with no callable `m` in the file, where the file types `v`
    /// (last segment of the type).
    Typed { ty: String, method: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PCall {
    /// Local id of the calling function, method or closure.
    pub caller: String,
    pub kind: PCallKind,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::php`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhpFacts {
    pub classes: Vec<PClass>,
    /// Functions and methods (anonymous-class methods included): (name, local id).
    pub callables: Vec<(String, String)>,
    /// Class imports: (FQN, alias).
    pub imports: Vec<(String, Option<String>)>,
    /// Per-file stubs: (local id, label, linked by an edge of the file).
    pub stubs: Vec<(String, String, bool)>,
    /// Short names the file declares as classes or writes in a signature or supertype list.
    pub known: Vec<String>,
    pub calls: Vec<PCall>,
}

/// Prefix of imported-class virtual ids (`@pi:<fqn>`).
pub const IMPORT: &str = "@pi:";
/// Prefix of supertype and trait virtual ids (`@ps:<name>`): the only class of that name, else
/// the shared stub `slug(name)` (never renamed per file).
pub const SUPER: &str = "@ps:";

#[derive(Debug, Clone)]
enum SiteKind {
    /// Bound in the file.
    Call(String),
    Unbound(PCallKind),
    Constant(String),
    StaticProp(String),
}

struct Site {
    kind: SiteKind,
    line: u32,
}

struct Walker<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    facts: PhpFacts,
    namespace: String,
    /// Alias (or last segment) → FQN, class imports only.
    aliases: HashMap<String, String>,
    /// Class name → local id (last wins).
    classes: HashMap<String, String>,
    /// Callable name → local id (last wins).
    callables: HashMap<String, String>,
    stubs: HashSet<String>,
    /// Short names the file declares as classes or writes as types, supertypes or traits: the
    /// only names whose `X::CONST` / `X::$p` Graphify links.
    known: HashSet<String>,
    /// Short names written in signatures (parameter, return, property types), in order.
    signature_names: Vec<String>,
    /// Short names written in `extends`/`implements`/trait `use`, in order.
    super_names: Vec<String>,
    closures: u32,
    /// Callers in order of creation, with their sites.
    callers: Vec<(String, Vec<Site>)>,
    /// Variable and property name (without `$`) → the last segment of its type: the first
    /// typed parameter or property, or `$v = new T(...)`, anywhere in the file.
    var_types: HashMap<String, String>,
    /// Class local id → its own methods, name → local id.
    class_methods: HashMap<String, HashMap<String, String>>,
    /// Class local id → its written parent (`extends`).
    class_parent: HashMap<String, String>,
    /// Caller index → the class of a method caller (closures and functions have none).
    caller_class: HashMap<usize, String>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let Some(tree) = tree else {
        cx.facts.lang = LangFacts::Php(PhpFacts::default());
        return;
    };
    let root = tree.root_node();
    let mut w = Walker {
        cx,
        facts: PhpFacts::default(),
        namespace: String::new(),
        aliases: HashMap::new(),
        classes: HashMap::new(),
        callables: HashMap::new(),
        stubs: HashSet::new(),
        known: HashSet::new(),
        signature_names: Vec::new(),
        super_names: Vec::new(),
        closures: 0,
        callers: Vec::new(),
        var_types: HashMap::new(),
        class_methods: HashMap::new(),
        class_parent: HashMap::new(),
        caller_class: HashMap::new(),
    };
    w.prescan(root);
    let mut known: Vec<String> = w.known.iter().cloned().collect();
    known.sort();
    w.facts.known = known;
    w.namespace.clear();
    let file_id = w.cx.file_id.clone();
    w.walk(root, None, &file_id);
    for c in &mut w.facts.classes {
        if let Some(methods) = w.class_methods.get(&c.id) {
            let mut list: Vec<(String, String)> = methods
                .iter()
                .map(|(n, i)| (n.clone(), i.clone()))
                .collect();
            list.sort();
            c.methods = list;
        }
    }
    w.emit_sites();
    w.register_stubs();
    let facts = std::mem::take(&mut w.facts);
    w.cx.facts.lang = LangFacts::Php(facts);
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn line(node: TsNode) -> u32 {
    Ctx::line(node)
}

fn is_class_like(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration" | "interface_declaration" | "trait_declaration" | "enum_declaration"
    )
}

/// A `named_type` in a parameter, property or return type (not `catch`, `instanceof`…).
fn is_signature_type(node: TsNode) -> bool {
    let mut child = node;
    while let Some(p) = child.parent() {
        match p.kind() {
            "optional_type"
            | "union_type"
            | "intersection_type"
            | "disjunctive_normal_form_type" => {
                child = p;
            }
            "simple_parameter"
            | "variadic_parameter"
            | "property_promotion_parameter"
            | "property_declaration" => return true,
            "method_declaration"
            | "function_definition"
            | "anonymous_function"
            | "arrow_function" => {
                return p
                    .child_by_field_name("return_type")
                    .is_some_and(|r| r.id() == child.id());
            }
            _ => return false,
        }
    }
    false
}

/// `\A\B\C` → `C`.
fn last_segment(name: &str) -> &str {
    name.rsplit('\\').next().unwrap_or(name)
}

/// A declared type that names one class: `T`, `?T`, `\A\T` (last segment). Unions,
/// intersections and primitive types name none.
fn single_type(node: TsNode, src: &[u8]) -> Option<String> {
    match node.kind() {
        "optional_type" => named_children(node)
            .into_iter()
            .find_map(|c| single_type(c, src)),
        "named_type" => named_children(node)
            .into_iter()
            .find(|c| matches!(c.kind(), "name" | "qualified_name" | "relative_name"))
            .and_then(|c| c.utf8_text(src).ok())
            .map(|t| last_segment(t).to_string()),
        _ => None,
    }
}

/// The name of a `variable_name` node, without `$`.
fn variable(node: TsNode, src: &[u8]) -> Option<String> {
    (node.kind() == "variable_name")
        .then(|| node.named_child(0))
        .flatten()
        .and_then(|n| n.utf8_text(src).ok())
        .map(str::to_string)
}

impl Walker<'_, '_> {
    fn text(&self, node: TsNode) -> String {
        self.cx.text(node).to_string()
    }

    /// Class names and callables of the whole file, so a call binds to a definition that
    /// follows it (last definition wins).
    fn prescan(&mut self, node: TsNode) {
        let file_id = self.cx.file_id.clone();
        let mut stack = vec![(node, None::<String>)];
        // Iterative pre-order with explicit owner tracking for methods.
        let mut order: Vec<(TsNode, Option<String>)> = Vec::new();
        while let Some((n, owner)) = stack.pop() {
            order.push((n, owner.clone()));
            let next_owner = if is_class_like(n.kind()) {
                n.child_by_field_name("name")
                    .map(|nm| Ctx::child_id(&file_id, self.cx.text(nm)))
            } else if n.kind() == "anonymous_class" {
                None
            } else {
                owner
            };
            let kids = named_children(n);
            for k in kids.into_iter().rev() {
                stack.push((k, next_owner.clone()));
            }
        }
        for (n, owner) in order {
            match n.kind() {
                k if is_class_like(k) => {
                    if let Some(nm) = n.child_by_field_name("name") {
                        let name = self.text(nm);
                        let id = Ctx::child_id(&file_id, &name);
                        let base = named_children(n)
                            .into_iter()
                            .find(|c| c.kind() == "base_clause")
                            .and_then(|b| named_children(b).into_iter().next())
                            .map(|b| self.text(b));
                        if let (Some(base), "class_declaration") = (base, k) {
                            self.class_parent.entry(id.clone()).or_insert(base);
                        }
                        self.known.insert(name.clone());
                        self.classes.insert(name, id);
                    }
                }
                "simple_parameter" | "property_promotion_parameter" => {
                    let src = self.cx.src;
                    let ty = n
                        .child_by_field_name("type")
                        .and_then(|t| single_type(t, src));
                    let var = n.child_by_field_name("name").and_then(|v| variable(v, src));
                    if let (Some(ty), Some(var)) = (ty, var) {
                        self.var_types.entry(var).or_insert(ty);
                    }
                }
                "property_declaration" => {
                    let src = self.cx.src;
                    if let Some(ty) = n
                        .child_by_field_name("type")
                        .and_then(|t| single_type(t, src))
                    {
                        for el in named_children(n) {
                            if el.kind() != "property_element" {
                                continue;
                            }
                            if let Some(var) = el
                                .child_by_field_name("name")
                                .and_then(|v| variable(v, src))
                            {
                                self.var_types.entry(var).or_insert_with(|| ty.clone());
                            }
                        }
                    }
                }
                "assignment_expression" => {
                    let src = self.cx.src;
                    let var = n.child_by_field_name("left").and_then(|v| variable(v, src));
                    let ty = n
                        .child_by_field_name("right")
                        .filter(|r| r.kind() == "object_creation_expression")
                        .and_then(|r| {
                            named_children(r)
                                .into_iter()
                                .find(|c| matches!(c.kind(), "name" | "qualified_name"))
                        })
                        .and_then(|c| c.utf8_text(src).ok())
                        .map(|t| last_segment(t).to_string());
                    if let (Some(var), Some(ty)) = (var, ty) {
                        self.var_types.entry(var).or_insert(ty);
                    }
                }
                "named_type" if !is_signature_type(n) => {}
                // An anonymous class's supertypes name nothing for the file.
                "base_clause" | "class_interface_clause"
                    if n.parent().is_some_and(|p| p.kind() == "anonymous_class") => {}
                "named_type" | "base_clause" | "class_interface_clause" | "use_declaration" => {
                    for ch in named_children(n) {
                        if matches!(ch.kind(), "name" | "qualified_name" | "relative_name") {
                            let text = self.text(ch);
                            let short = last_segment(&text).to_string();
                            self.known.insert(short.clone());
                            let list = if n.kind() == "named_type" {
                                &mut self.signature_names
                            } else {
                                &mut self.super_names
                            };
                            if !list.contains(&short) {
                                list.push(short);
                            }
                        }
                    }
                }
                "function_definition" => {
                    if let Some(nm) = n.child_by_field_name("name") {
                        let name = self.text(nm);
                        let id = Ctx::child_id(&file_id, &name);
                        if id != file_id {
                            self.callables.insert(name, id);
                        }
                    }
                }
                "method_declaration" => {
                    if let Some(nm) = n.child_by_field_name("name") {
                        let name = self.text(nm);
                        let parent = owner.clone().unwrap_or_else(|| file_id.clone());
                        let id = Ctx::child_id(&parent, &name);
                        if id != parent {
                            if let Some(class) = &owner {
                                self.class_methods
                                    .entry(class.clone())
                                    .or_default()
                                    .insert(name.clone(), id.clone());
                            }
                            self.callables.insert(name, id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn walk(&mut self, node: TsNode, caller: Option<usize>, owner: &str) {
        let mut c = node.walk();
        let kids: Vec<TsNode> = node.children(&mut c).collect();
        for k in kids {
            self.visit(k, caller, owner);
        }
    }

    fn visit(&mut self, node: TsNode, caller: Option<usize>, owner: &str) {
        match node.kind() {
            "namespace_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n))
                    .unwrap_or_default();
                if let Some(body) = node.child_by_field_name("body") {
                    let saved = std::mem::replace(&mut self.namespace, name);
                    self.walk(body, caller, owner);
                    self.namespace = saved;
                } else {
                    self.namespace = name;
                }
            }
            "namespace_use_declaration" => self.use_declaration(node),
            k if is_class_like(k) => self.class(node),
            "function_definition" => self.function(node, None),
            "anonymous_class" => {
                // Methods of an anonymous class become functions of the file.
                if let Some(body) = node.child_by_field_name("body") {
                    for m in named_children(body) {
                        if m.kind() == "method_declaration" {
                            self.function(m, None);
                        } else {
                            self.visit(m, caller, owner);
                        }
                    }
                }
                let mut c = node.walk();
                let rest: Vec<TsNode> = node
                    .children(&mut c)
                    .filter(|n| n.kind() == "arguments")
                    .collect();
                for r in rest {
                    self.walk(r, caller, owner);
                }
            }
            "anonymous_function" | "arrow_function" => self.closure(node),
            "function_call_expression" => {
                if let Some(ci) = caller {
                    if let Some(f) = node.child_by_field_name("function") {
                        if f.kind() == "name" && !is_unbound_name(self.cx.text(f), false) {
                            let name = self.text(f);
                            let kind = match self.callables.get(&name) {
                                Some(id) => SiteKind::Call(id.clone()),
                                None => SiteKind::Unbound(PCallKind::Bare(name)),
                            };
                            self.site(ci, kind, line(node));
                        }
                    }
                }
                self.walk(node, caller, owner);
            }
            "member_call_expression" => {
                if let Some(ci) = caller {
                    if let Some(n) = node.child_by_field_name("name") {
                        if n.kind() == "name" {
                            let method = self.text(n);
                            if let Some(id) = self.callables.get(&method).cloned() {
                                self.site(ci, SiteKind::Call(id), line(node));
                            } else if let Some(ty) = node
                                .child_by_field_name("object")
                                .and_then(|o| self.receiver(o))
                                .and_then(|v| self.var_types.get(&v).cloned())
                            {
                                let kind = PCallKind::Typed { ty, method };
                                self.site(ci, SiteKind::Unbound(kind), line(node));
                            }
                        }
                    }
                }
                self.walk(node, caller, owner);
            }
            "scoped_call_expression" => {
                if let Some(ci) = caller {
                    self.static_call(ci, node);
                }
                self.walk(node, caller, owner);
            }
            "object_creation_expression" => {
                if let Some(ci) = caller {
                    let target = named_children(node)
                        .into_iter()
                        .find(|n| matches!(n.kind(), "name" | "qualified_name"));
                    if let Some(t) = target {
                        self.class_use(ci, t, line(node));
                    }
                }
                self.walk(node, caller, owner);
            }
            "class_constant_access_expression" | "scoped_property_access_expression" => {
                if let Some(ci) = caller {
                    let scope = if node.kind() == "scoped_property_access_expression" {
                        node.child_by_field_name("scope")
                    } else {
                        node.named_child(0)
                    };
                    if let Some(scope) = scope.filter(|s| s.kind() == "name") {
                        let name = self.text(scope);
                        if !matches!(name.as_str(), "self" | "static" | "parent")
                            && self.known.contains(&name)
                        {
                            let target = self.scope_target(&name);
                            let kind = if node.kind() == "scoped_property_access_expression" {
                                SiteKind::StaticProp(target)
                            } else {
                                SiteKind::Constant(target)
                            };
                            self.site(ci, kind, line(node));
                        }
                    }
                }
                self.walk(node, caller, owner);
            }
            _ => self.walk(node, caller, owner),
        }
    }

    /// The typed name a member call's object stands for: `$v` (not `$this`) → `v`, `$this->p` → `p`.
    fn receiver(&self, object: TsNode) -> Option<String> {
        let src = self.cx.src;
        match object.kind() {
            "variable_name" => variable(object, src).filter(|v| v != "this"),
            "member_access_expression" => {
                let this = object
                    .child_by_field_name("object")
                    .and_then(|o| variable(o, src))
                    .is_some_and(|v| v == "this");
                object
                    .child_by_field_name("name")
                    .filter(|n| this && n.kind() == "name")
                    .map(|n| self.text(n))
            }
            _ => None,
        }
    }

    /// The method `name` a class of the file declares itself.
    fn own_method(&self, class_id: &str, name: &str) -> Option<String> {
        self.class_methods.get(class_id)?.get(name).cloned()
    }

    /// `X::m()`: `self::`/`static::` → the calling method's class's own `m`; `parent::` → the
    /// parent's own `m` (a class of the file, else the cross-file pass, which never reports it);
    /// closures have no class for these. A class of the file → its own `m`; any other `X` → the
    /// cross-file pass. Builtin-like names count; a method never statically calls itself.
    fn static_call(&mut self, caller: usize, node: TsNode) {
        let (Some(scope), Some(method)) = (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name")
                .filter(|n| n.kind() == "name"),
        ) else {
            return;
        };
        let method = self.text(method);
        let ln = line(node);
        let class = self.caller_class.get(&caller).cloned();
        let written = self.text(scope);
        let (target, unbound) = match (scope.kind(), written.as_str()) {
            ("relative_scope", "self" | "static") => {
                (class.and_then(|c| self.own_method(&c, &method)), None)
            }
            ("relative_scope", "parent") => {
                let Some(parent) = class.and_then(|c| self.class_parent.get(&c).cloned()) else {
                    return;
                };
                match self.classes.get(last_segment(&parent)).cloned() {
                    Some(id) => (self.own_method(&id, &method), None),
                    None => (
                        None,
                        Some(PCallKind::Static {
                            scope: parent,
                            method,
                            report: false,
                        }),
                    ),
                }
            }
            ("name" | "qualified_name", _) => {
                match self.classes.get(last_segment(&written)).cloned() {
                    Some(id) => (self.own_method(&id, &method), None),
                    None => (
                        None,
                        Some(PCallKind::Static {
                            scope: written,
                            method,
                            report: true,
                        }),
                    ),
                }
            }
            _ => return,
        };
        if let Some(id) = target {
            if id != self.callers[caller].0 {
                self.site(caller, SiteKind::Call(id), ln);
            }
        } else if let Some(kind) = unbound {
            self.site(caller, SiteKind::Unbound(kind), ln);
        }
    }

    fn site(&mut self, caller: usize, kind: SiteKind, line: u32) {
        self.callers[caller].1.push(Site { kind, line });
    }

    fn new_caller(&mut self, id: &str) -> usize {
        self.callers.push((id.to_string(), Vec::new()));
        self.callers.len() - 1
    }

    /// `new X`: a class of the file, else recorded for the cross-file pass.
    fn class_use(&mut self, caller: usize, scope: TsNode, line: u32) {
        let text = self.text(scope);
        match scope.kind() {
            "name" => {
                if matches!(text.as_str(), "self" | "static" | "parent")
                    || is_unbound_name(&text, false)
                {
                    return;
                }
                let kind = match self.classes.get(&text) {
                    Some(id) => SiteKind::Call(id.clone()),
                    None => SiteKind::Unbound(PCallKind::Class(text)),
                };
                self.site(caller, kind, line);
            }
            "qualified_name" if !is_unbound_name(last_segment(&text), false) => {
                self.site(caller, SiteKind::Unbound(PCallKind::Class(text)), line);
            }
            _ => {}
        }
    }

    /// The FQN of a non-aliased class import whose last segment is `name`.
    fn imports_plain(&self, name: &str) -> Option<String> {
        self.facts
            .imports
            .iter()
            .rev()
            .find(|(fqn, alias)| alias.is_none() && last_segment(fqn) == name)
            .map(|(fqn, _)| fqn.clone())
    }

    /// The target of `X::CONST` / `X::$p`: a class of the file, else a per-file stub.
    /// A name the file writes in a signature links its per-file stub; a name it only extends,
    /// implements or uses as a trait links the shared supertype stub.
    fn scope_target(&mut self, name: &str) -> String {
        if let Some(id) = self.classes.get(name) {
            return id.clone();
        }
        if self.signature_names.iter().any(|n| n == name) {
            return self.stub(name);
        }
        format!("{SUPER}{name}")
    }

    fn stub(&mut self, label: &str) -> String {
        let id = slug(label);
        if self.stubs.insert(id.clone()) {
            self.cx.add_stub(id.clone(), label.to_string());
            self.facts.stubs.push((id.clone(), label.to_string(), true));
        }
        id
    }

    /// Graphify reserves a per-file stub for every type name the file writes in a signature or
    /// supertype list, linked or not, so such ids collide (and are renamed per file, §4.5)
    /// whenever two files write the same name. Unlinked ones are dropped by the resolver.
    fn register_stubs(&mut self) {
        let names: Vec<String> = self
            .signature_names
            .iter()
            .chain(self.super_names.iter())
            .cloned()
            .collect();
        for name in names {
            let id = slug(&name);
            if id.is_empty() || self.classes.contains_key(&name) || !self.stubs.insert(id.clone()) {
                continue;
            }
            self.cx.add_stub(id.clone(), name.clone());
            self.facts.stubs.push((id, name, false));
        }
    }

    fn use_declaration(&mut self, node: TsNode) {
        let ln = line(node);
        let decl_type = node
            .child_by_field_name("type")
            .map(|t| self.text(t))
            .unwrap_or_default();
        let prefix = named_children(node)
            .into_iter()
            .find(|n| n.kind() == "namespace_name")
            .map(|n| self.text(n));
        let mut clauses = Vec::new();
        for ch in named_children(node) {
            match ch.kind() {
                "namespace_use_clause" => clauses.push(ch),
                "namespace_use_group" => {
                    clauses.extend(
                        named_children(ch)
                            .into_iter()
                            .filter(|n| n.kind() == "namespace_use_clause"),
                    );
                }
                _ => {}
            }
        }
        for clause in clauses {
            let kind = clause
                .child_by_field_name("type")
                .map(|t| self.text(t))
                .unwrap_or_else(|| decl_type.clone());
            let alias = clause.child_by_field_name("alias").map(|a| self.text(a));
            let Some(name) = named_children(clause)
                .into_iter()
                .find(|n| {
                    matches!(n.kind(), "name" | "qualified_name")
                        && Some(n.id()) != clause.child_by_field_name("alias").map(|a| a.id())
                })
                .map(|n| self.text(n))
            else {
                continue;
            };
            let mut fqn = match &prefix {
                Some(p) => format!("{p}\\{name}"),
                None => name,
            };
            if let Some(stripped) = fqn.strip_prefix('\\') {
                fqn = stripped.to_string();
            }
            let file_id = self.cx.file_id.clone();
            if kind == "function" || kind == "const" {
                let ext = slug(last_segment(&fqn));
                if ext.is_empty() {
                    continue;
                }
                self.cx.add_external(ext.clone());
                self.cx
                    .add_edge(&file_id, &ext, "imports", ln, Some("import"));
            } else {
                let key = alias
                    .clone()
                    .unwrap_or_else(|| last_segment(&fqn).to_string());
                self.aliases.insert(key, fqn.clone());
                self.facts.imports.push((fqn.clone(), alias));
                let target = format!("{IMPORT}{fqn}");
                self.cx
                    .add_edge(&file_id, &target, "imports", ln, Some("import"));
            }
        }
    }

    /// The node a written type or class name denotes (see the module docs). `supertype`: a
    /// name in `extends`/`implements`/trait `use`, whose stub is shared by every file.
    fn type_target(&mut self, written: &str, supertype: bool) -> Option<String> {
        let short = match written.strip_prefix('\\') {
            Some(rest) => last_segment(rest),
            None => {
                if let Some(fqn) = self.aliases.get(written) {
                    return Some(format!("{IMPORT}{fqn}"));
                }
                // `Psr7\Stream` under a non-aliased `use GuzzleHttp\Psr7` names its FQN; an
                // aliased namespace (`use A\B as P; P\C`) is not expanded.
                if let Some((first, rest)) = written.split_once('\\') {
                    if let Some(fqn) = self.imports_plain(first) {
                        return Some(format!("{IMPORT}{fqn}\\{rest}"));
                    }
                }
                last_segment(written)
            }
        };
        if short.is_empty() || slug(short).is_empty() {
            return None;
        }
        if let Some(id) = self.classes.get(short) {
            return Some(id.clone());
        }
        if supertype {
            return Some(format!("{SUPER}{short}"));
        }
        Some(self.stub(short))
    }

    /// Named types inside a type node (union, intersection, optional, DNF), in order.
    fn type_names(&self, node: TsNode, out: &mut Vec<String>) {
        match node.kind() {
            "named_type" => {
                if let Some(n) = node.named_child(0) {
                    out.push(self.text(n));
                }
            }
            "primitive_type" | "bottom_type" => {}
            _ => {
                for ch in named_children(node) {
                    self.type_names(ch, out);
                }
            }
        }
    }

    fn class(&mut self, node: TsNode) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let file_id = self.cx.file_id.clone();
        let id = Ctx::child_id(&file_id, &name);
        if id == file_id {
            return;
        }
        let ln = line(node);
        let fqn = if self.namespace.is_empty() {
            name.clone()
        } else {
            format!("{}\\{}", self.namespace, name)
        };
        self.facts.classes.push(PClass {
            name: name.clone(),
            fqn,
            id: id.clone(),
            methods: Vec::new(),
        });
        self.cx.add_def(
            id.clone(),
            name,
            ln,
            vec![flag("_callable"), flag("_callable_class")],
        );
        self.cx.add_edge(&file_id, &id, "contains", ln, None);
        let interface = node.kind() == "interface_declaration";
        for ch in named_children(node) {
            let relation = match ch.kind() {
                "base_clause" => "inherits",
                "class_interface_clause" if interface => "inherits",
                "class_interface_clause" => "implements",
                _ => continue,
            };
            for n in named_children(ch) {
                let written = self.text(n);
                if let Some(t) = self.type_target(&written, true) {
                    self.cx.add_edge(&id, &t, relation, ln, None);
                }
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.class_body(body, &id);
        }
    }

    fn class_body(&mut self, body: TsNode, class_id: &str) {
        for m in named_children(body) {
            match m.kind() {
                "method_declaration" => self.function(m, Some(class_id)),
                "use_declaration" => {
                    let ln = line(m);
                    for n in named_children(m) {
                        if matches!(n.kind(), "name" | "qualified_name" | "relative_name") {
                            let written = self.text(n);
                            if let Some(t) = self.type_target(&written, true) {
                                self.cx.add_edge(class_id, &t, "mixes_in", ln, None);
                            }
                        }
                    }
                }
                "property_declaration" => {
                    let ln = line(m);
                    if let Some(t) = m.child_by_field_name("type") {
                        let mut names = Vec::new();
                        self.type_names(t, &mut names);
                        for n in names {
                            if let Some(target) = self.type_target(&n, false) {
                                self.cx.add_edge(
                                    class_id,
                                    &target,
                                    "references",
                                    ln,
                                    Some("field"),
                                );
                            }
                        }
                    }
                }
                "enum_case" => {
                    if let Some(nm) = m.child_by_field_name("name") {
                        let name = self.text(nm);
                        let id = Ctx::child_id(class_id, &name);
                        if id != class_id {
                            let ln = line(m);
                            self.cx.add_def(id.clone(), name, ln, vec![]);
                            self.cx.add_edge(class_id, &id, "case_of", ln, None);
                        }
                    }
                }
                _ => {
                    // Closures in constant or property initialisers still become nodes.
                    self.walk(m, None, class_id);
                }
            }
        }
    }

    /// A function, a method (`owner` set) or an anonymous-class method (as a function).
    fn function(&mut self, node: TsNode, owner: Option<&str>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let file_id = self.cx.file_id.clone();
        let parent = owner.unwrap_or(&file_id).to_string();
        let id = Ctx::child_id(&parent, &name);
        if id == parent {
            return;
        }
        let ln = line(node);
        let label = if owner.is_some() {
            format!(".{name}()")
        } else {
            format!("{name}()")
        };
        self.facts.callables.push((name, id.clone()));
        self.cx
            .add_def(id.clone(), label, ln, vec![flag("_callable")]);
        let relation = if owner.is_some() {
            "method"
        } else {
            "contains"
        };
        self.cx.add_edge(&parent, &id, relation, ln, None);
        self.signature(node, &id, owner, ln);
        let ci = self.new_caller(&id);
        if let Some(class) = owner {
            self.caller_class.insert(ci, class.to_string());
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.walk(body, Some(ci), &id);
        }
    }

    fn closure(&mut self, node: TsNode) {
        self.closures += 1;
        let n = self.closures;
        let file_id = self.cx.file_id.clone();
        let id = format!("{file_id}_closure_{n}");
        let ln = line(node);
        self.cx.add_def(
            id.clone(),
            format!("{{closure#{n}}}()"),
            ln,
            vec![flag("_callable")],
        );
        self.cx.add_edge(&file_id, &id, "contains", ln, None);
        self.signature(node, &id, None, ln);
        let ci = self.new_caller(&id);
        if let Some(body) = node.child_by_field_name("body") {
            // An arrow function's body is an expression, possibly the call itself.
            if node.kind() == "arrow_function" {
                self.visit(body, Some(ci), &id);
            } else {
                self.walk(body, Some(ci), &id);
            }
        }
    }

    /// Parameter and return types (and promoted-property fields of a constructor).
    fn signature(&mut self, node: TsNode, id: &str, owner: Option<&str>, ln: u32) {
        if let Some(params) = node.child_by_field_name("parameters") {
            for p in named_children(params) {
                let promoted = p.kind() == "property_promotion_parameter";
                let Some(t) = p.child_by_field_name("type") else {
                    continue;
                };
                let mut names = Vec::new();
                self.type_names(t, &mut names);
                for n in names {
                    if let Some(target) = self.type_target(&n, false) {
                        self.cx
                            .add_edge(id, &target, "references", ln, Some("parameter_type"));
                        if promoted {
                            if let Some(class_id) = owner {
                                self.cx.add_edge(
                                    class_id,
                                    &target,
                                    "references",
                                    ln,
                                    Some("field"),
                                );
                            }
                        }
                    }
                }
            }
        }
        if let Some(t) = node.child_by_field_name("return_type") {
            let mut names = Vec::new();
            self.type_names(t, &mut names);
            for n in names {
                if let Some(target) = self.type_target(&n, false) {
                    self.cx
                        .add_edge(id, &target, "references", ln, Some("return_type"));
                }
            }
        }
    }

    /// Call sites, per caller in order of definition: bound calls (deduplicated per target,
    /// first line kept), constant and static-property uses (per target and relation).
    fn emit_sites(&mut self) {
        let callers = std::mem::take(&mut self.callers);
        for (caller, sites) in callers {
            let mut seen: HashSet<(&'static str, String)> = HashSet::new();
            for s in sites {
                let (relation, target) = match s.kind {
                    SiteKind::Call(target) => ("calls", target),
                    SiteKind::Unbound(kind) => {
                        self.facts.calls.push(PCall {
                            caller: caller.clone(),
                            kind,
                            line: s.line,
                        });
                        continue;
                    }
                    SiteKind::Constant(target) => ("references_constant", target),
                    SiteKind::StaticProp(target) => ("uses_static_prop", target),
                };
                if !seen.insert((relation, target.clone())) {
                    continue;
                }
                if relation == "calls" {
                    self.cx
                        .add_edge(&caller, &target, "calls", s.line, Some("call"));
                } else {
                    let mut e: Edge =
                        self.cx
                            .edge(&caller, &target, relation, Loc::Line(s.line), None);
                    e.score = Some(1.0);
                    self.cx.facts.edges.push(e);
                }
            }
        }
    }
}

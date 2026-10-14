//! Ruby (§19) extraction. See `docs/graphify-format.md`.
//!
//! One walk emits the file node (`metadata.ruby_resolution_schema`, plus `ruby_lookup_unsafe`
//! and `ruby_external_method_owners` when they apply), modules and classes (id
//! `file_<slug(qualified name)>`, label the qualified name `A::B::C`), constants bound to
//! `Class.new`/`Struct.new` (classes too, contained by the file), methods of a module or class
//! body (`owner_<method slug>`, label `.name()`, `metadata.ruby_method_kind`), and every other
//! `def` as a function of the file (`name()`): at top level, in a block, under `if`/`begin`, or
//! as an argument (`private def x`). A `def` inside a method body is nothing. Same-file ids merge
//! like Graphify's node table: a method defined twice becomes `ambiguous`, a module or class
//! defined twice is `ruby_reopened`, and the first definition's label wins.
//!
//! In-file edges: `contains` (from the enclosing body, or the file for a module or class
//! defined in a block), `method`, `inherits` (to the file's top-level class of that name, else
//! a per-file stub of the last segment, with the reference and lexical scopes in `metadata`),
//! and `calls` that bind in the file: `m`/`self.m` inside a method bind to the owner's method
//! of that name, else a function of the file; `x.m` and any call inside a function bind to the
//! file's last definition named `m`. Names are matched against the merged node labels.
//! `require`, comments and `attr_*` produce nothing.
//!
//! Everything else is recorded in [`RubyFacts`] for `crate::resolve::ruby`: bare calls that did
//! not bind (by name across files), calls on a constant (`Foo.bar`, `Foo::Bar.new`) and
//! `include`/`extend`/`prepend` directly in a body.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::go::is_unbound_name;
use super::{slug, Ctx};
use crate::facts::{Attr, LangFacts, Loc};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RCallKind {
    /// `m` / `m()` that did not bind in the file.
    Bare(String),
    /// `Path.m` (`m` = `new` for a constructor call).
    Const { path: String, method: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RCall {
    /// Local id of the calling method or function.
    pub caller: String,
    pub kind: RCallKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RMixin {
    /// Local id of the including module or class.
    pub source: String,
    pub path: String,
    pub scopes: Vec<String>,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::ruby`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RubyFacts {
    /// Modules and classes: (local id, qualified label).
    pub classes: Vec<(String, String)>,
    /// Methods: (owner local id, name, local id).
    pub methods: Vec<(String, String, String)>,
    /// Functions and methods by name, for bare calls across files: (name, local id).
    pub callables: Vec<(String, String)>,
    pub calls: Vec<RCall>,
    pub mixins: Vec<RMixin>,
}

/// Calls that define methods or change lookup and make the enclosing body `ruby_lookup_unsafe`.
const UNSAFE_CALLS: &[&str] = &[
    "alias_method",
    "attr",
    "attr_accessor",
    "attr_reader",
    "attr_writer",
    "define_method",
    "define_singleton_method",
    "extend",
    "include",
    "prepend",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Class,
    Method,
    Function,
    Stub,
}

struct Entry {
    id: String,
    label: String,
    kind: Kind,
    line: u32,
    callable: bool,
    class: bool,
    metadata: Vec<(String, String)>,
}

impl Entry {
    fn set(&mut self, key: &str, json: &str) {
        match self.metadata.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = json.to_string(),
            None => self.metadata.push((key.to_string(), json.to_string())),
        }
    }

    fn has(&self, key: &str) -> bool {
        self.metadata.iter().any(|(k, _)| k == key)
    }
}

/// Where a construct sits relative to the nearest module or class body.
#[derive(Clone)]
struct Scope {
    /// Local id of the owning module or class; `None` at file level.
    owner: Option<String>,
    /// Qualified labels of the enclosing modules and classes, outermost first.
    labels: Vec<String>,
    /// Directly in the owner's body (or the file's top level), not in a block, `if`, call…
    direct: bool,
    /// Inside `class << self`.
    singleton: bool,
}

#[derive(Clone)]
struct Caller {
    index: usize,
}

enum SiteKind {
    /// `m`, `m()` (`receiver: Bare`), `self.m` (`SelfCall`) or `x.m` (`Other`).
    Name {
        name: String,
        receiver: Recv,
    },
    Const {
        path: String,
        method: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Recv {
    Bare,
    SelfCall,
    Other,
}

struct Site {
    kind: SiteKind,
    line: u32,
}

struct CallerInfo {
    id: String,
    /// Owner of the method; `None` for a function.
    owner: Option<String>,
    sites: Vec<Site>,
}

struct Walker<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    facts: RubyFacts,
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    file_unsafe: bool,
    external_owners: Vec<String>,
    /// Labels of the file's modules and classes (for `inherits`).
    labels: HashSet<String>,
    callers: Vec<CallerInfo>,
    /// Local variables of the current def.
    locals: Vec<HashSet<String>>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    let mut w = Walker {
        cx,
        facts: RubyFacts::default(),
        entries: Vec::new(),
        by_id: HashMap::new(),
        file_unsafe: false,
        external_owners: Vec::new(),
        labels: HashSet::new(),
        callers: Vec::new(),
        locals: Vec::new(),
    };
    if let Some(tree) = tree {
        let root = tree.root_node();
        w.prescan(root, &[]);
        let scope = Scope {
            owner: None,
            labels: Vec::new(),
            direct: true,
            singleton: false,
        };
        w.walk(root, &scope, None);
    }
    w.finish();
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn line(node: TsNode) -> u32 {
    Ctx::line(node)
}

/// A method name's id part: a trailing `?`, `!` or `=` becomes `_pred`, `_bang`, `_eq`.
pub fn method_slug(name: &str) -> String {
    let base = if let Some(b) = name.strip_suffix('?') {
        format!("{b}_pred")
    } else if let Some(b) = name.strip_suffix('!') {
        format!("{b}_bang")
    } else if let Some(b) = name.strip_suffix('=') {
        format!("{b}_eq")
    } else {
        name.to_string()
    };
    slug(&base)
}

fn qualify(labels: &[String], written: &str) -> String {
    match labels.last() {
        Some(outer) => format!("{outer}::{written}"),
        None => written.to_string(),
    }
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn is_const(node: TsNode) -> bool {
    matches!(node.kind(), "constant" | "scope_resolution")
}

/// `Class.new(...)` / `Struct.new(...)`.
fn class_constructor(node: TsNode, src: &[u8]) -> bool {
    if node.kind() != "call" {
        return false;
    }
    let text = |n: TsNode| std::str::from_utf8(&src[n.byte_range()]).unwrap_or("");
    let recv = node.child_by_field_name("receiver").map(text);
    let method = node.child_by_field_name("method").map(text);
    matches!(recv, Some("Class" | "Struct")) && method == Some("new")
}

impl Walker<'_, '_> {
    fn text(&self, node: TsNode) -> String {
        self.cx.text(node).to_string()
    }

    /// Labels of every module and class in the file.
    fn prescan(&mut self, node: TsNode, labels: &[String]) {
        for ch in named_children(node) {
            match ch.kind() {
                "module" | "class" => {
                    let Some(name) = ch.child_by_field_name("name") else {
                        continue;
                    };
                    let label = qualify(labels, &self.text(name));
                    self.labels.insert(label.clone());
                    let mut inner = labels.to_vec();
                    inner.push(label);
                    self.prescan(ch, &inner);
                }
                _ => self.prescan(ch, labels),
            }
        }
    }

    fn entry(&mut self, id: &str) -> Option<&mut Entry> {
        let i = *self.by_id.get(id)?;
        self.entries.get_mut(i)
    }

    fn push_entry(&mut self, e: Entry) {
        self.by_id.insert(e.id.clone(), self.entries.len());
        self.entries.push(e);
    }

    fn mark_unsafe(&mut self, scope: &Scope) {
        match scope.owner.clone() {
            Some(owner) => {
                if let Some(e) = self.entry(&owner) {
                    e.set("ruby_lookup_unsafe", "true");
                }
            }
            None => self.file_unsafe = true,
        }
    }

    fn walk(&mut self, node: TsNode, scope: &Scope, caller: Option<&Caller>) {
        let mut c = node.walk();
        let kids: Vec<TsNode> = node.named_children(&mut c).collect();
        // Statements directly under a body keep `direct`; anything else is nested.
        let keeps_direct = matches!(node.kind(), "program" | "body_statement");
        let inner = if keeps_direct || !scope.direct {
            scope.clone()
        } else {
            Scope {
                direct: false,
                ..scope.clone()
            }
        };
        for k in kids {
            self.visit(k, &inner, caller);
        }
    }

    fn visit(&mut self, node: TsNode, scope: &Scope, caller: Option<&Caller>) {
        match node.kind() {
            "module" | "class" => self.class(node, scope, caller),
            "singleton_class" => {
                let is_self = node
                    .child_by_field_name("value")
                    .is_some_and(|v| v.kind() == "self");
                if let Some(body) = node.child_by_field_name("body") {
                    let inner = Scope {
                        direct: scope.direct && is_self,
                        singleton: is_self,
                        ..scope.clone()
                    };
                    self.walk(body, &inner, caller);
                }
            }
            "method" | "singleton_method" => self.def(node, scope, caller),
            "alias" => {
                if caller.is_none() {
                    self.mark_unsafe(scope);
                }
            }
            "assignment" => self.assignment(node, scope, caller),
            "operator_assignment" => {
                if let Some(l) = node.child_by_field_name("left") {
                    if l.kind() == "identifier" {
                        let name = self.text(l);
                        if let Some(set) = self.locals.last_mut() {
                            set.insert(name);
                        }
                    } else {
                        self.visit(l, scope, caller);
                    }
                }
                if let Some(r) = node.child_by_field_name("right") {
                    self.visit(r, scope, caller);
                }
            }
            "call" => self.call(node, scope, caller),
            "identifier" => {
                if let Some(c) = caller {
                    let name = self.text(node);
                    let local = self.locals.last().is_some_and(|s| s.contains(&name));
                    if !local && !is_unbound_name(&name, false) {
                        self.site(
                            c,
                            SiteKind::Name {
                                name,
                                receiver: Recv::Bare,
                            },
                            line(node),
                        );
                    }
                }
            }
            "block_parameters" | "lambda_parameters" | "method_parameters" => {
                self.declare_params(node);
            }
            "do_block" | "block" => {
                let inner = Scope {
                    direct: false,
                    ..scope.clone()
                };
                for ch in named_children(node) {
                    self.visit(ch, &inner, caller);
                }
            }
            "exception_variable" => {
                for ch in named_children(node) {
                    if ch.kind() == "identifier" {
                        let name = self.text(ch);
                        if let Some(set) = self.locals.last_mut() {
                            set.insert(name);
                        }
                    }
                }
            }
            // Symbols, strings without interpolation and other leaves carry no calls.
            "simple_symbol" | "hash_key_symbol" | "constant" | "self" | "comment" => {}
            _ => self.walk(node, scope, caller),
        }
    }

    fn declare_params(&mut self, node: TsNode) {
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if n.kind() == "identifier" {
                let name = self.text(n);
                if let Some(set) = self.locals.last_mut() {
                    set.insert(name);
                }
                continue;
            }
            for ch in named_children(n) {
                stack.push(ch);
            }
        }
    }

    fn declare_lhs(&mut self, node: TsNode) {
        match node.kind() {
            "identifier" => {
                let name = self.text(node);
                if let Some(set) = self.locals.last_mut() {
                    set.insert(name);
                }
            }
            "left_assignment_list" | "destructured_left_assignment" | "rest_assignment" => {
                for ch in named_children(node) {
                    self.declare_lhs(ch);
                }
            }
            _ => {}
        }
    }

    fn site(&mut self, caller: &Caller, kind: SiteKind, line: u32) {
        self.callers[caller.index].sites.push(Site { kind, line });
    }

    fn assignment(&mut self, node: TsNode, scope: &Scope, caller: Option<&Caller>) {
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");
        if let Some(l) = left {
            if l.kind() == "constant" {
                let name = self.text(l);
                self.external_owners.push(format!("{name}::*"));
                if let Some(r) = right.filter(|r| is_const(*r)) {
                    let rt = self.text(r);
                    self.external_owners.push(format!("{rt}::*"));
                }
                if let Some(r) = right.filter(|r| class_constructor(*r, self.cx.src)) {
                    self.constant_class(&name, r, scope, caller);
                    return;
                }
            } else {
                self.declare_lhs(l);
                if !matches!(l.kind(), "identifier" | "left_assignment_list") {
                    self.visit(l, scope, caller);
                }
            }
        }
        if let Some(r) = right {
            self.visit(r, scope, caller);
        }
    }

    /// `X = Class.new(Base) do … end` / `X = Struct.new(:a) do … end`.
    fn constant_class(&mut self, name: &str, call: TsNode, scope: &Scope, caller: Option<&Caller>) {
        let label = qualify(&scope.labels, name);
        let file_id = self.cx.file_id.clone();
        let id = Ctx::child_id(&file_id, &label);
        let ln = line(call);
        self.add_class(&id, &label, ln);
        if let Some(e) = self.entry(&id) {
            e.set("ruby_lookup_unsafe", "true");
        }
        self.facts.classes.push((id.clone(), label.clone()));
        self.cx.add_edge(&file_id, &id, "contains", ln, None);
        let recv = call
            .child_by_field_name("receiver")
            .map(|r| self.text(r))
            .unwrap_or_default();
        if recv == "Class" {
            let base = call
                .child_by_field_name("arguments")
                .and_then(|a| named_children(a).into_iter().next())
                .filter(|a| is_const(*a));
            if let Some(b) = base {
                self.inherits(&id, b, &scope.labels, ln);
            }
        }
        if let Some(args) = call.child_by_field_name("arguments") {
            self.walk(args, scope, caller);
        }
        if let Some(block) = call.child_by_field_name("block") {
            let mut labels = scope.labels.clone();
            labels.push(label);
            let inner = Scope {
                owner: Some(id),
                labels,
                direct: true,
                singleton: false,
            };
            let mut has_def = false;
            for ch in named_children(block) {
                if matches!(ch.kind(), "body_statement" | "block_body") {
                    has_def |= named_children(ch)
                        .iter()
                        .any(|s| matches!(s.kind(), "method" | "singleton_method"));
                    self.walk(ch, &inner, None);
                }
            }
            if has_def {
                self.mark_unsafe(scope);
            }
        }
    }

    fn add_class(&mut self, id: &str, label: &str, ln: u32) {
        if let Some(e) = self.entry(id) {
            e.set("ruby_reopened", "true");
            e.callable = true;
            e.class = true;
            return;
        }
        self.push_entry(Entry {
            id: id.to_string(),
            label: label.to_string(),
            kind: Kind::Class,
            line: ln,
            callable: true,
            class: true,
            metadata: Vec::new(),
        });
    }

    fn stub(&mut self, label: &str) -> String {
        let id = slug(label);
        if !self.by_id.contains_key(&id) {
            self.push_entry(Entry {
                id: id.clone(),
                label: label.to_string(),
                kind: Kind::Stub,
                line: 0,
                callable: false,
                class: false,
                metadata: Vec::new(),
            });
        }
        id
    }

    /// `inherits` to the file's top-level class named like the reference's last segment, else
    /// a per-file stub.
    fn inherits(&mut self, id: &str, superclass: TsNode, enclosing: &[String], ln: u32) {
        let written = self.text(superclass);
        let last = written.rsplit("::").next().unwrap_or(&written).to_string();
        if slug(&last).is_empty() {
            return;
        }
        let file_id = self.cx.file_id.clone();
        let target = if self.labels.contains(&last) {
            Ctx::child_id(&file_id, &last)
        } else {
            self.stub(&last)
        };
        let scopes: Vec<String> = enclosing.iter().rev().map(|s| json_str(s)).collect();
        let metadata = format!(
            "{{\"ruby_superclass_ref\":{},\"ruby_lexical_scopes\":[{}]}}",
            json_str(&written),
            scopes.join(",")
        );
        let mut e = self.cx.edge(id, &target, "inherits", Loc::Line(ln), None);
        e.attrs.push(("metadata".into(), Attr::Json(metadata)));
        self.cx.facts.edges.push(e);
    }

    fn class(&mut self, node: TsNode, scope: &Scope, caller: Option<&Caller>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let written = self.text(name_node);
        if written.contains("::") {
            self.external_owners.push(written.clone());
        }
        let label = qualify(&scope.labels, &written);
        let file_id = self.cx.file_id.clone();
        let id = Ctx::child_id(&file_id, &label);
        if id == file_id {
            return;
        }
        let ln = line(node);
        // A module or class reopening a method's id keeps the `method` link alone.
        let shadows_method = self
            .entry(&id)
            .is_some_and(|e| matches!(e.kind, Kind::Method | Kind::Function));
        self.add_class(&id, &label, ln);
        if written.starts_with("::") {
            if let Some(e) = self.entry(&id) {
                e.set("ruby_lookup_unsafe", "true");
            }
        }
        self.facts.classes.push((id.clone(), label.clone()));
        if !shadows_method {
            // Only a definition directly in a module or class body is contained by it.
            let parent = match (&scope.owner, scope.direct) {
                (Some(owner), true) => owner.clone(),
                _ => file_id,
            };
            self.cx.add_edge(&parent, &id, "contains", ln, None);
        }
        if let Some(sup) = node.child_by_field_name("superclass") {
            if let Some(expr) = named_children(sup).into_iter().find(|n| is_const(*n)) {
                self.inherits(&id, expr, &scope.labels, ln);
            } else {
                for ch in named_children(sup) {
                    self.visit(ch, scope, caller);
                }
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            let mut labels = scope.labels.clone();
            labels.push(label);
            let inner = Scope {
                owner: Some(id),
                labels,
                direct: true,
                singleton: false,
            };
            self.walk(body, &inner, None);
        }
    }

    fn def(&mut self, node: TsNode, scope: &Scope, caller: Option<&Caller>) {
        if caller.is_some() {
            // A `def` inside a method body (`def obj.m`, `class_eval do def m`) is not a node,
            // and its calls are not recorded.
            return;
        }
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let mslug = method_slug(&name);
        if mslug.is_empty() {
            return;
        }
        let file_id = self.cx.file_id.clone();
        let mut kind = if scope.singleton {
            "singleton"
        } else {
            "instance"
        };
        if node.kind() == "singleton_method" {
            let object = node.child_by_field_name("object");
            match object {
                Some(o) if o.kind() == "self" => kind = "singleton",
                Some(o) => {
                    if is_const(o) {
                        let t = self.text(o);
                        self.external_owners.push(t);
                    }
                    kind = "ambiguous";
                }
                None => kind = "ambiguous",
            }
        }
        let ln = line(node);
        let (id, owner) = match (&scope.owner, scope.direct) {
            (Some(owner), true) => (format!("{owner}_{mslug}"), Some(owner.clone())),
            _ => {
                if !scope.direct {
                    self.mark_unsafe(scope);
                }
                (format!("{file_id}_{mslug}"), None)
            }
        };
        match &owner {
            Some(o) => {
                let existed = self.by_id.contains_key(&id);
                if existed {
                    if let Some(e) = self.entry(&id) {
                        let k = if e.has("ruby_method_kind") {
                            "ambiguous"
                        } else {
                            kind
                        };
                        e.set("ruby_method_kind", &json_str(k));
                        e.callable = true;
                    }
                } else {
                    self.push_entry(Entry {
                        id: id.clone(),
                        label: format!(".{name}()"),
                        kind: Kind::Method,
                        line: ln,
                        callable: true,
                        class: false,
                        metadata: vec![("ruby_method_kind".into(), json_str(kind))],
                    });
                }
                self.facts
                    .methods
                    .push((o.clone(), name.clone(), id.clone()));
                self.cx.add_edge(o, &id, "method", ln, None);
            }
            None => {
                if !self.by_id.contains_key(&id) {
                    self.push_entry(Entry {
                        id: id.clone(),
                        label: format!("{name}()"),
                        kind: Kind::Function,
                        line: ln,
                        callable: true,
                        class: false,
                        metadata: Vec::new(),
                    });
                }
                self.cx.add_edge(&file_id, &id, "contains", ln, None);
            }
        }
        self.facts.callables.push((name, id.clone()));

        // The body: calls are this def's.
        self.callers.push(CallerInfo {
            id: id.clone(),
            owner: owner.clone(),
            sites: Vec::new(),
        });
        let c = Caller {
            index: self.callers.len() - 1,
        };
        self.locals.push(HashSet::new());
        if let Some(p) = node.child_by_field_name("parameters") {
            self.declare_params(p);
        }
        let inner = Scope {
            direct: false,
            singleton: false,
            ..scope.clone()
        };
        for f in ["parameters", "body"] {
            if let Some(b) = node.child_by_field_name(f) {
                if f == "body" {
                    self.visit(b, &inner, Some(&c));
                } else {
                    // Default values may call.
                    for ch in named_children(b) {
                        for v in named_children(ch).into_iter().skip(1) {
                            self.visit(v, &inner, Some(&c));
                        }
                    }
                }
            }
        }
        self.locals.pop();
    }

    fn call(&mut self, node: TsNode, scope: &Scope, caller: Option<&Caller>) {
        let receiver = node.child_by_field_name("receiver");
        let method = node
            .child_by_field_name("method")
            .map(|m| self.text(m))
            .unwrap_or_default();
        let ln = line(node);
        // Lookup-changing calls and mixins count in a body, not inside a method.
        if receiver.is_none() && caller.is_none() {
            let args = node.child_by_field_name("arguments");
            let has_args = args.is_some_and(|a| a.named_child_count() > 0);
            if UNSAFE_CALLS.contains(&method.as_str()) || (method == "module_function" && has_args)
            {
                self.mark_unsafe(scope);
            }
            if matches!(method.as_str(), "include" | "extend" | "prepend") && scope.direct {
                if let (Some(owner), Some(args)) = (&scope.owner, args) {
                    let scopes: Vec<String> = scope.labels.iter().rev().cloned().collect();
                    for a in named_children(args) {
                        if is_const(a) {
                            let path = self.text(a);
                            self.facts.mixins.push(RMixin {
                                source: owner.clone(),
                                path,
                                scopes: scopes.clone(),
                                line: ln,
                            });
                        }
                    }
                }
            }
        }
        if let Some(c) = caller {
            match receiver {
                Some(r) if is_const(r) && !self.text(r).starts_with("::") => {
                    let path = self.text(r);
                    self.site(c, SiteKind::Const { path, method }, ln);
                }
                Some(r) => {
                    if !is_unbound_name(&method, false) {
                        let receiver = if r.kind() == "self" {
                            Recv::SelfCall
                        } else {
                            Recv::Other
                        };
                        self.site(
                            c,
                            SiteKind::Name {
                                name: method,
                                receiver,
                            },
                            ln,
                        );
                    }
                }
                None => {
                    if !is_unbound_name(&method, false) {
                        self.site(
                            c,
                            SiteKind::Name {
                                name: method,
                                receiver: Recv::Bare,
                            },
                            ln,
                        );
                    }
                }
            }
        }
        for f in ["receiver", "arguments", "block"] {
            if let Some(ch) = node.child_by_field_name(f) {
                // A receiver that is a constant or a plain name is not itself a call.
                if f == "receiver" && (is_const(ch) || ch.kind() == "identifier") {
                    continue;
                }
                self.visit(ch, scope, caller);
            }
        }
    }

    /// Emit nodes, then bind call sites in the file and record the rest.
    fn finish(&mut self) {
        let mut md: Vec<String> = vec!["\"ruby_resolution_schema\":1".into()];
        if self.file_unsafe {
            md.push("\"ruby_lookup_unsafe\":true".into());
        }
        if !self.external_owners.is_empty() {
            let mut owners = self.external_owners.clone();
            owners.sort();
            owners.dedup();
            let list: Vec<String> = owners.iter().map(|o| json_str(o)).collect();
            md.push(format!(
                "\"ruby_external_method_owners\":[{}]",
                list.join(",")
            ));
        }
        let file_meta = format!("{{{}}}", md.join(","));
        // Graphify resolves names through the merged node table: a definition whose id an
        // earlier one already took answers to that earlier node's label only.
        let labels: HashMap<String, String> = self
            .entries
            .iter()
            .map(|e| (e.id.clone(), e.label.clone()))
            .collect();
        let named = |id: &str, name: &str| -> bool {
            labels
                .get(id)
                .is_some_and(|l| *l == format!(".{name}()") || *l == format!("{name}()"))
        };
        self.facts.methods.retain(|(_, name, id)| named(id, name));
        self.facts.callables.retain(|(name, id)| named(id, name));

        // The file node first, then entries in order.
        self.cx
            .add_file_node(vec![("metadata".to_string(), Attr::Json(file_meta))]);
        for e in std::mem::take(&mut self.entries) {
            if e.kind == Kind::Stub {
                self.cx.add_stub(e.id, e.label);
                continue;
            }
            let mut attrs = Vec::new();
            if !e.metadata.is_empty() {
                let body: Vec<String> = e
                    .metadata
                    .iter()
                    .map(|(k, v)| format!("{}:{}", json_str(k), v))
                    .collect();
                attrs.push((
                    "metadata".to_string(),
                    Attr::Json(format!("{{{}}}", body.join(","))),
                ));
            }
            if e.callable {
                attrs.push(("_callable".to_string(), Attr::Bool(true)));
            }
            if e.class {
                attrs.push(("_callable_class".to_string(), Attr::Bool(true)));
            }
            self.cx.add_def(e.id, e.label, e.line, attrs);
        }
        self.emit_sites();
        // Classes are recorded once per id.
        let mut seen = HashSet::new();
        self.facts.classes.retain(|(id, _)| seen.insert(id.clone()));
        let facts = std::mem::take(&mut self.facts);
        self.cx.facts.lang = LangFacts::Ruby(facts);
    }

    fn emit_sites(&mut self) {
        // In-file name tables: (owner, method slug) → id, and functions by slug; for function
        // callers, every definition by name (last wins).
        let mut owned: HashSet<String> = HashSet::new();
        let mut by_owner: HashMap<(String, String), String> = HashMap::new();
        for (owner, name, id) in &self.facts.methods {
            owned.insert(id.clone());
            by_owner.insert((owner.clone(), name.clone()), id.clone());
        }
        let mut functions: HashMap<String, String> = HashMap::new();
        let mut any: HashMap<String, String> = HashMap::new();
        for (name, id) in &self.facts.callables {
            any.insert(name.clone(), id.clone());
            if !owned.contains(id) {
                functions.insert(name.clone(), id.clone());
            }
        }
        let callers = std::mem::take(&mut self.callers);
        for c in callers {
            let mut seen: HashSet<String> = HashSet::new();
            for s in c.sites {
                match s.kind {
                    SiteKind::Name { name, receiver } => {
                        let bound = match (&c.owner, receiver) {
                            // `x.m` binds to the file's last definition named `m`.
                            (_, Recv::Other) | (None, _) => any.get(&name).cloned(),
                            (Some(owner), _) => by_owner
                                .get(&(owner.clone(), name.clone()))
                                .or_else(|| functions.get(&name))
                                .cloned(),
                        };
                        let bare = receiver == Recv::Bare;
                        match bound {
                            Some(target) => {
                                if seen.insert(target.clone()) {
                                    self.cx
                                        .add_edge(&c.id, &target, "calls", s.line, Some("call"));
                                }
                            }
                            None if bare => self.facts.calls.push(RCall {
                                caller: c.id.clone(),
                                kind: RCallKind::Bare(name),
                                line: s.line,
                            }),
                            None => {}
                        }
                    }
                    SiteKind::Const { path, method } => self.facts.calls.push(RCall {
                        caller: c.id.clone(),
                        kind: RCallKind::Const { path, method },
                        line: s.line,
                    }),
                }
            }
        }
    }
}

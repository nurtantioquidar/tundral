//! C and C++ extraction (§18, §19). See `docs/graphify-format.md`.
//!
//! One walker serves both grammars; C (`.c`, C-looking `.h`) only gets functions, type references
//! and calls, C++ (`.cpp .cc .cxx .hpp .cu`, C++-looking `.h`) adds classes, structs, unions and
//! enums (flat `file_Name` ids, also when nested), fields (`defines`), methods (`method`),
//! enumerators (`case_of`), bases (`inherits`) and out-of-line definitions (`file_slug(A::f)`,
//! label `A::f()`, contained by the file).
//!
//! Everything that needs other files is recorded in [`CFacts`] for `crate::resolve::c`:
//! `#include` targets (they resolve against the set of indexed files), type stubs (they bind to a
//! unique definition by label), calls that do not bind in the file, and the classes with their
//! members.
//!
//! Graphify behaviours reproduced because its ids and edges depend on them:
//!
//! - A `.h` file is C++ when its bytes contain `class `, `namespace `, `template`, `::`,
//!   `public:`, `private:` or `protected:` anywhere, comments included ([`is_cpp_header`]).
//! - Every node outside function bodies is walked (preprocessor branches, error-recovery nodes);
//!   a named class specifier anywhere there is a class, a class body only keeps its fields and
//!   methods (`#if` inside it, template members: file level).
//! - Declarations (prototypes, constructor declarations) emit nothing; method declarations in a
//!   class body emit a method without type references; a declarator behind a reference
//!   (`T &f()`, `T &field`) emits nothing.
//! - Type names bind to the node with the id a file-level definition would have, else a stub.
//! - In-file calls bind by name to the last node created with that name (functions, methods,
//!   classes, fields, enumerators), ignoring the receiver; a method prefers its own class.
//! - `x.m()` binds across files through `x`'s declared type, taken from the file's first
//!   single-declarator local declaration of `x` with a capitalised type, wherever it is.
//! - `class MACRO Name { ... }` (tree-sitter reads a function definition) still yields class
//!   `Name` with the methods and fields its body declares.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, Ctx};
use crate::facts::{Edge, LangFacts};
use crate::lang::Lang;

/// One `#include` directive with a literal path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CInclude {
    /// The path as written, without quotes or angle brackets.
    pub path: String,
    /// `"..."` (resolved relative to the including file) rather than `<...>`.
    pub quoted: bool,
    pub line: u32,
}

/// A class, struct, union or enum and the members a qualified or typed call can reach.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CClass {
    /// Local id.
    pub id: String,
    pub label: String,
    /// (member name, local id): methods, fields and enumerators.
    pub members: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CCallKind {
    /// `f()` with no `f` in the file.
    Bare,
    /// `x.m()` / `x->m()` with no `m` in the file, `x` declared with type `receiver`.
    Typed,
    /// `Q::m()`: `receiver` is the outermost scope, `name` the rest.
    Qualified,
}

/// A call left for the cross-file pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CCall {
    /// Local id of the calling function or method.
    pub caller: String,
    pub kind: CCallKind,
    pub name: String,
    pub receiver: String,
    /// Qualified calls: the in-file definition named like the rest of the name, if any.
    pub fallback: Option<String>,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::c`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CFacts {
    /// Local id of the file node.
    pub file_id: String,
    pub includes: Vec<CInclude>,
    /// Type stubs: (local id, label).
    pub stubs: Vec<(String, String)>,
    pub classes: Vec<CClass>,
    pub calls: Vec<CCall>,
    /// Out-of-line definitions `A::f()`: (local id, line).
    pub out_of_line: Vec<(String, u32)>,
}

/// `.h` files are C++ when any of these byte strings occurs in them.
const CPP_MARKERS: [&[u8]; 7] = [
    b"class ",
    b"namespace ",
    b"template",
    b"::",
    b"public:",
    b"private:",
    b"protected:",
];

/// Whether a `.h` file is C++ (see the module docs).
pub fn is_cpp_header(src: &[u8]) -> bool {
    CPP_MARKERS
        .iter()
        .any(|m| src.windows(m.len()).any(|w| w == *m))
}

/// The language of a `.h` file, from its content (unreadable files count as C).
pub fn header_lang(path: &Path) -> Option<Lang> {
    let src = std::fs::read(path).unwrap_or_default();
    Some(if is_cpp_header(&src) {
        Lang::Cpp
    } else {
        Lang::C
    })
}

#[derive(Clone)]
enum Scope {
    File,
    Class(String),
}

/// A call site, bound after the walk (the in-file table needs every definition).
enum Callee {
    Bare(String),
    /// Receiver variable (when it is a plain identifier) and member name.
    Member(Option<String>, String),
    /// Outermost scope text and the rest of the name.
    Qualified(String, String),
}

struct RawCall {
    caller: String,
    callee: Callee,
    line: u32,
}

struct C<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    cpp: bool,
    /// Node ids emitted so far (the build keeps the first node of an id).
    emitted: HashSet<String>,
    /// In-file classes by name (first wins), from a pre-pass.
    classes: HashMap<String, String>,
    /// In-file definitions by the name a call uses (last wins).
    names: HashMap<String, String>,
    /// Local variable → declared type (first capitalised single declaration wins).
    vars: HashMap<String, String>,
    calls: Vec<RawCall>,
    /// Index into `facts.classes` by class id.
    class_index: HashMap<String, usize>,
    facts: CFacts,
    /// Pre-pass: walk the declarations only to learn the file's class names.
    dry: bool,
    /// Methods defined in a class body: method id → class id.
    method_class: HashMap<String, String>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let cpp = cx.lang == Lang::Cpp;
    let file_id = cx.file_id.clone();
    let mut w = C {
        cpp,
        emitted: HashSet::from([file_id.clone()]),
        classes: HashMap::new(),
        names: HashMap::new(),
        vars: HashMap::new(),
        calls: Vec::new(),
        class_index: HashMap::new(),
        facts: CFacts {
            file_id,
            ..CFacts::default()
        },
        dry: cpp,
        method_class: HashMap::new(),
        cx,
    };
    if let Some(tree) = tree {
        let root = tree.root_node();
        if cpp {
            w.items(root, &Scope::File);
            w.dry = false;
        }
        w.items(root, &Scope::File);
    }
    w.bind_calls();
    let C { cx, facts, .. } = w;
    cx.facts.lang = LangFacts::C(facts);
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn is_class_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class_specifier" | "struct_specifier" | "union_specifier"
    )
}

/// The name a call uses for a definition label: `.m()` → `m`, `f()` → `f`, `A::f()` → `A::f`.
pub fn call_name(label: &str) -> &str {
    let l = label.strip_prefix('.').unwrap_or(label);
    l.strip_suffix("()").unwrap_or(l)
}

impl<'a> C<'_, 'a> {
    fn text(&self, node: TsNode) -> &'a str {
        self.cx.text(node)
    }

    // ---- emission helpers ---------------------------------------------------------------------

    /// Emit a definition node once (first wins). Calls bind by node: a node registers its call
    /// name when it is created (the last node of a name wins), a later definition merged into an
    /// existing id does not.
    fn def(
        &mut self,
        id: &str,
        label: String,
        line: u32,
        attrs: Vec<(String, crate::facts::Attr)>,
    ) {
        if self.emitted.insert(id.to_string()) {
            self.names
                .insert(call_name(&label).to_string(), id.to_string());
            self.cx.add_def(id.to_string(), label, line, attrs);
        }
    }

    fn member(&mut self, class_id: &str, name: &str, id: &str) {
        if let Some(&i) = self.class_index.get(class_id) {
            self.facts.classes[i]
                .members
                .push((name.to_string(), id.to_string()));
        }
    }

    /// The target of a type name: an in-file class, else a per-file stub.
    fn type_target(&mut self, name: &str) -> String {
        self.type_target_as(name, name, None)
    }

    /// The node a type name binds to in this file: whatever node has the id a file-level
    /// definition of `key` would have (a class, but also a function or a field: Graphify looks
    /// the id up), except `exclude`; else the per-file stub labelled `label`.
    fn type_target_as(&mut self, key: &str, label: &str, exclude: Option<&str>) -> String {
        let local = Ctx::child_id(&self.cx.file_id, key);
        let known = self.emitted.contains(&local) || self.classes.get(key) == Some(&local);
        if known && exclude != Some(local.as_str()) {
            return local;
        }
        let id = super::slug(label);
        if self.emitted.insert(id.clone()) {
            self.cx.add_stub(id.clone(), label.to_string());
            self.facts.stubs.push((id.clone(), label.to_string()));
        }
        id
    }

    fn reference(&mut self, source: &str, name: &str, context: &'static str, line: u32) {
        let target = self.type_target(name);
        self.cx
            .add_edge(source, &target, "references", line, Some(context));
    }

    /// `references` edges for a type: the named type with `context`, template arguments with
    /// `generic_arg` (C: plain type names only).
    fn type_refs(&mut self, ty: TsNode, context: &'static str, source: &str, line: u32) {
        match ty.kind() {
            "type_identifier" => {
                let name = self.text(ty);
                self.reference(source, name, context, line);
            }
            "qualified_identifier" if self.cpp => {
                if let Some(name) = ty.child_by_field_name("name") {
                    self.type_refs(name, context, source, line);
                }
            }
            "template_type" if self.cpp => {
                if let Some(name) = ty.child_by_field_name("name") {
                    self.type_refs(name, context, source, line);
                }
                if let Some(args) = ty.child_by_field_name("arguments") {
                    for arg in named_children(args) {
                        if arg.kind() == "type_descriptor" {
                            if let Some(t) = arg.child_by_field_name("type") {
                                self.type_refs(t, "generic_arg", source, line);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ---- scopes -------------------------------------------------------------------------------

    fn parent_id(&self, scope: &Scope) -> String {
        match scope {
            Scope::File => self.cx.file_id.clone(),
            Scope::Class(c) => c.clone(),
        }
    }

    fn items(&mut self, node: TsNode, scope: &Scope) {
        for child in named_children(node) {
            self.item(child, scope);
        }
    }

    /// Graphify walks every node outside function bodies: definitions are found wherever the
    /// parse put them (namespaces, `extern "C"`, preprocessor branches, error-recovery nodes),
    /// and a named class specifier anywhere outside a function, a field or a friend declaration
    /// is a class (`struct S *gs;`, a parameter of a prototype) contained by the file.
    fn item(&mut self, n: TsNode, scope: &Scope) {
        let kind = n.kind();
        match kind {
            "preproc_include" => self.include(n),
            "function_definition" => self.function(n, scope),
            "template_declaration" if self.cpp => {
                for c in named_children(n) {
                    if c.kind() != "template_parameter_list" {
                        self.item(c, &Scope::File);
                    }
                }
            }
            k if self.cpp && is_class_kind(k) => self.class(n, scope),
            "enum_specifier" if self.cpp => self.enumeration(n, scope),
            "friend_declaration" | "comment" => {}
            _ => self.items(n, scope),
        }
    }

    fn include(&mut self, n: TsNode) {
        if self.dry {
            return;
        }
        let Some(path) = n.child_by_field_name("path") else {
            return;
        };
        let (text, quoted) = match path.kind() {
            "string_literal" => (self.text(path).trim_matches('"'), true),
            "system_lib_string" => (
                self.text(path)
                    .trim_start_matches('<')
                    .trim_end_matches('>'),
                false,
            ),
            _ => return,
        };
        self.facts.includes.push(CInclude {
            path: text.to_string(),
            quoted,
            line: Ctx::line(n),
        });
    }

    // ---- classes ------------------------------------------------------------------------------

    fn new_class(&mut self, name: &str, line: u32, scope: &Scope) -> String {
        let id = Ctx::child_id(&self.cx.file_id, name);
        if self.dry {
            self.classes.entry(name.to_string()).or_insert(id.clone());
            return id;
        }
        self.def(
            &id,
            name.to_string(),
            line,
            vec![flag("_callable"), flag("_callable_class")],
        );
        let parent = self.parent_id(scope);
        self.cx.add_edge(&parent, &id, "contains", line, None);
        if let Scope::Class(outer) = scope {
            let outer = outer.clone();
            self.member(&outer, name, &id);
        }
        if !self.class_index.contains_key(&id) {
            self.class_index
                .insert(id.clone(), self.facts.classes.len());
            self.facts.classes.push(CClass {
                id: id.clone(),
                label: name.to_string(),
                members: Vec::new(),
            });
        }
        id
    }

    fn class(&mut self, n: TsNode, scope: &Scope) {
        let Some(name) = n.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name);
        let line = Ctx::line(n);
        let id = self.new_class(name, line, scope);
        let bases = if self.dry {
            Vec::new()
        } else {
            named_children(n)
        };
        for c in bases {
            if c.kind() == "base_class_clause" {
                for base in named_children(c) {
                    if let Some((key, label)) = base_name(base, self.cx.src) {
                        let target = self.type_target_as(key, label, Some(&id));
                        self.cx.add_edge(&id, &target, "inherits", line, None);
                    }
                }
            }
        }
        if let Some(body) = n.child_by_field_name("body") {
            self.class_body(body, &id);
        }
    }

    fn class_body(&mut self, body: TsNode, class_id: &str) {
        let scope = Scope::Class(class_id.to_string());
        for c in named_children(body) {
            match c.kind() {
                "field_declaration" => self.field_declaration(c, class_id),
                "function_definition" => self.function(c, &scope),
                "template_declaration" => self.item(c, &Scope::File),
                // Anything else, preprocessor branches included, is walked like file scope:
                // `#if` in a class body turns its methods into file-level functions.
                "friend_declaration" | "access_specifier" | "comment" => {}
                _ => self.item(c, &Scope::File),
            }
        }
    }

    fn enumeration(&mut self, n: TsNode, scope: &Scope) {
        let Some(name) = n.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name);
        let id = self.new_class(name, Ctx::line(n), scope);
        let Some(body) = n.child_by_field_name("body").filter(|_| !self.dry) else {
            return;
        };
        for e in named_children(body) {
            if e.kind() != "enumerator" {
                continue;
            }
            let Some(en) = e.child_by_field_name("name") else {
                continue;
            };
            let en = self.text(en);
            let eid = Ctx::child_id(&id, en);
            let line = Ctx::line(e);
            self.def(&eid, en.to_string(), line, vec![]);
            self.cx.add_edge(&id, &eid, "case_of", line, None);
            self.member(&id, en, &eid);
        }
    }

    fn field_declaration(&mut self, n: TsNode, class_id: &str) {
        let scope = Scope::Class(class_id.to_string());
        let ty = n.child_by_field_name("type");
        let declarators = children_by_field(n, "declarator");
        if let Some(ty) = ty {
            let kind = ty.kind();
            let nested = ty.child_by_field_name("body").is_some() || declarators.is_empty();
            if nested && is_class_kind(kind) {
                self.class(ty, &scope);
            } else if nested && kind == "enum_specifier" {
                self.enumeration(ty, &scope);
            }
        }
        if declarators.is_empty() || self.dry {
            return;
        }
        let line = Ctx::line(n);
        if let Some(fd) = function_declarator(declarators[0]) {
            self.method_declaration(fd, class_id, line);
            return;
        }
        if let Some(ty) = ty {
            self.type_refs(ty, "field", class_id, line);
        }
        for d in declarators {
            if let Some(name) = field_name(d) {
                let name = self.text(name);
                self.field(class_id, name, line);
            }
        }
    }

    fn field(&mut self, class_id: &str, name: &str, line: u32) {
        let id = Ctx::child_id(class_id, name);
        if self.emitted.contains(&id) {
            // A field whose id is already a method of the class: Graphify keeps the method link
            // and gives it the field context.
            if let Some(e) = self
                .cx
                .facts
                .edges
                .iter_mut()
                .rev()
                .find(|e| e.source == class_id && e.target == id && is_structural(e))
            {
                if e.context.is_none() {
                    e.context = Some("field".into());
                }
                return;
            }
        } else {
            self.def(&id, name.to_string(), line, vec![]);
        }
        self.cx
            .add_edge(class_id, &id, "defines", line, Some("field"));
        self.member(class_id, name, &id);
    }

    /// A method declared (not defined) in a class body: node and `method` edge only.
    fn method_declaration(&mut self, fd: TsNode, class_id: &str, line: u32) {
        let Some(name) = fd.child_by_field_name("declarator") else {
            return;
        };
        if !matches!(
            name.kind(),
            "field_identifier" | "identifier" | "destructor_name" | "operator_name"
        ) {
            return;
        }
        let name = self.text(name);
        let id = Ctx::child_id(class_id, name);
        self.def(&id, format!(".{name}()"), line, vec![flag("_callable")]);
        self.method_edge(class_id, &id, line);
        self.member(class_id, name, &id);
    }

    /// `class -method-> m`. After a same-id field (`kind_` then `kind()`), Graphify keeps one
    /// `method` link that also carries the field's context.
    fn method_edge(&mut self, class_id: &str, id: &str, line: u32) {
        if let Some(e) = self
            .cx
            .facts
            .edges
            .iter_mut()
            .rev()
            .find(|e| e.source == class_id && e.target == id && e.relation == "defines")
        {
            e.relation = "method".into();
            e.loc = crate::facts::Loc::Line(line);
            return;
        }
        self.cx.add_edge(class_id, id, "method", line, None);
    }

    // ---- functions ----------------------------------------------------------------------------

    fn function(&mut self, n: TsNode, scope: &Scope) {
        if self.cpp {
            if let Some((name, base)) = misparsed_class_name(n) {
                self.misparsed_class(n, name, base, scope);
                return;
            }
        }
        if self.dry {
            return;
        }
        let Some(fd) = n
            .child_by_field_name("declarator")
            .and_then(function_declarator)
        else {
            return;
        };
        let Some(name_node) = fd.child_by_field_name("declarator") else {
            return;
        };
        let name = self.text(name_node);
        let line = Ctx::line(n);
        let plain = match name_node.kind() {
            "identifier" | "field_identifier" => true,
            "destructor_name" | "operator_name" | "template_function" if self.cpp => true,
            "qualified_identifier" if self.cpp => false,
            _ => return,
        };
        let id = match (scope, plain) {
            (Scope::Class(class_id), true) => {
                let id = Ctx::child_id(class_id, name);
                self.def(&id, format!(".{name}()"), line, vec![flag("_callable")]);
                let class_id = class_id.clone();
                self.method_edge(&class_id, &id, line);
                self.member(&class_id, name, &id);
                self.method_class.insert(id.clone(), class_id);
                id
            }
            _ => {
                let id = Ctx::child_id(&self.cx.file_id, name);
                self.def(&id, format!("{name}()"), line, vec![flag("_callable")]);
                let file = self.cx.file_id.clone();
                self.cx.add_edge(&file, &id, "contains", line, None);
                if !plain {
                    self.facts.out_of_line.push((id.clone(), line));
                }
                id
            }
        };
        if let Some(ret) = n.child_by_field_name("type") {
            self.type_refs(ret, "return_type", &id, line);
        }
        if let Some(params) = fd.child_by_field_name("parameters") {
            for p in named_children(params) {
                if let Some(t) = p.child_by_field_name("type") {
                    self.type_refs(t, "parameter_type", &id, line);
                }
            }
        }
        if let Some(body) = n.child_by_field_name("body") {
            self.scan_body(body, &id);
        }
    }

    /// `class MACRO Name { ... }` read as a function definition: a class `Name` whose body
    /// declarations are its methods and fields.
    fn misparsed_class(&mut self, n: TsNode, name: TsNode, base: Option<TsNode>, scope: &Scope) {
        let name = self.text(name);
        let line = Ctx::line(n);
        let id = self.new_class(name, line, scope);
        if let Some((key, label)) = base
            .filter(|_| !self.dry)
            .and_then(|b| base_name(b, self.cx.src))
        {
            let target = self.type_target_as(key, label, Some(&id));
            self.cx.add_edge(&id, &target, "inherits", line, None);
        }
        if let Some(body) = n.child_by_field_name("body") {
            self.misparsed_body(body, &id);
        }
    }

    fn misparsed_body(&mut self, body: TsNode, class_id: &str) {
        let scope = Scope::Class(class_id.to_string());
        for c in named_children(body) {
            match c.kind() {
                "labeled_statement" => self.misparsed_body(c, class_id),
                "function_definition" => self.function(c, &scope),
                "declaration" => {
                    if let Some(ty) = c
                        .child_by_field_name("type")
                        .filter(|t| t.child_by_field_name("body").is_some())
                    {
                        if is_class_kind(ty.kind()) {
                            self.class(ty, &scope);
                        } else if ty.kind() == "enum_specifier" {
                            self.enumeration(ty, &scope);
                        }
                    }
                    if self.dry {
                        continue;
                    }
                    let declarators = children_by_field(c, "declarator");
                    let Some(&first) = declarators.first() else {
                        continue;
                    };
                    let line = Ctx::line(c);
                    if let Some(fd) = function_declarator(first).or_else(|| error_declarator(c)) {
                        self.method_declaration(fd, class_id, line);
                        continue;
                    }
                    if has_error_child(c) {
                        continue;
                    }
                    // `void f()` in statement position reads as a variable initialised with
                    // `()`: still a method.
                    if let Some(name) = (first.kind() == "init_declarator")
                        .then(|| first.child_by_field_name("value"))
                        .flatten()
                        .filter(|v| v.kind() == "argument_list")
                        .and_then(|_| first.child_by_field_name("declarator"))
                        .filter(|d| d.kind() == "identifier")
                    {
                        let name = self.text(name);
                        let id = Ctx::child_id(class_id, name);
                        self.def(&id, format!(".{name}()"), line, vec![flag("_callable")]);
                        self.method_edge(class_id, &id, line);
                        self.member(class_id, name, &id);
                        continue;
                    }
                    if let Some(ty) = c.child_by_field_name("type") {
                        self.type_refs(ty, "field", class_id, line);
                    }
                    for d in declarators {
                        if let Some(name) = field_name(d) {
                            let name = self.text(name);
                            if !name.is_empty() {
                                self.field(class_id, name, line);
                            }
                        }
                    }
                }
                // The body often swallows what follows the class (error recovery): classes and
                // functions in it are file level.
                _ => self.item(c, &Scope::File),
            }
        }
    }

    // ---- bodies: calls and local variable types ---------------------------------------------

    fn scan_body(&mut self, body: TsNode, caller: &str) {
        let mut stack = vec![body];
        while let Some(n) = stack.pop() {
            match n.kind() {
                // A definition the parse nested in this body (error recovery) is neither a node
                // nor part of this function.
                "function_definition" => continue,
                "call_expression" => self.call(n, caller),
                "declaration" if self.cpp => self.local(n),
                _ => {}
            }
            let mut c = n.walk();
            let children: Vec<_> = n.named_children(&mut c).collect();
            stack.extend(children.into_iter().rev());
        }
    }

    fn call(&mut self, n: TsNode, caller: &str) {
        let Some(f) = n.child_by_field_name("function") else {
            return;
        };
        let callee = match f.kind() {
            "identifier" => Callee::Bare(self.text(f).to_string()),
            "field_expression" => {
                let Some(field) = f.child_by_field_name("field") else {
                    return;
                };
                let recv = f
                    .child_by_field_name("argument")
                    .filter(|a| matches!(a.kind(), "identifier" | "this"))
                    .map(|a| self.text(a).to_string());
                Callee::Member(recv, self.text(field).to_string())
            }
            "qualified_identifier" if self.cpp => {
                let Some(name) = f.child_by_field_name("name") else {
                    return;
                };
                let name = self.text(name).to_string();
                match f.child_by_field_name("scope") {
                    Some(scope) => Callee::Qualified(self.text(scope).to_string(), name),
                    None => Callee::Bare(name),
                }
            }
            _ => return,
        };
        self.calls.push(RawCall {
            caller: caller.to_string(),
            callee,
            line: Ctx::line(n),
        });
    }

    /// `T x;`, `T *x = ...;`, `const T &x = ...;` with one declarator and a capitalised type
    /// name: the first such declaration of a name in the file gives its type.
    fn local(&mut self, decl: TsNode) {
        let declarators = children_by_field(decl, "declarator");
        if declarators.len() != 1 {
            return;
        }
        let Some(ty) = decl.child_by_field_name("type") else {
            return;
        };
        let ty = match ty.kind() {
            "type_identifier" => ty,
            "qualified_identifier" => match last_name(ty) {
                Some(t) if t.kind() == "type_identifier" => t,
                _ => return,
            },
            _ => return,
        };
        let ty = self.text(ty);
        if !ty.chars().next().is_some_and(char::is_uppercase) {
            return;
        }
        let mut d = declarators[0];
        loop {
            match d.kind() {
                "identifier" => break,
                "init_declarator" | "pointer_declarator" => {
                    match d.child_by_field_name("declarator") {
                        Some(inner) => d = inner,
                        None => return,
                    }
                }
                "reference_declarator" => match d.named_child(0) {
                    Some(inner) => d = inner,
                    None => return,
                },
                _ => return,
            }
        }
        let name = self.text(d).to_string();
        self.vars.entry(name).or_insert_with(|| ty.to_string());
    }

    /// Bind calls against the in-file table; record the rest for the cross-file pass.
    fn bind_calls(&mut self) {
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let calls = std::mem::take(&mut self.calls);
        for c in calls {
            let denied = match &c.callee {
                Callee::Bare(n) | Callee::Member(_, n) | Callee::Qualified(_, n) => {
                    DENIED_CALLS.binary_search(&n.as_str()).is_ok()
                }
            };
            // Inside a class body, `f()` and `this->f()` prefer the class's own member.
            let own = match &c.callee {
                Callee::Bare(name) => Some(name),
                Callee::Member(Some(r), name) if r == "this" => Some(name),
                _ => None,
            }
            .and_then(|name| {
                let class = self.method_class.get(&c.caller)?;
                let &i = self.class_index.get(class)?;
                self.facts.classes[i]
                    .members
                    .iter()
                    .rev()
                    .find(|(m, _)| m == name)
                    .map(|(_, id)| id.clone())
            });
            let local = own
                .or_else(|| match &c.callee {
                    Callee::Bare(name) | Callee::Member(_, name) => self.names.get(name).cloned(),
                    Callee::Qualified(..) => None,
                })
                .filter(|_| !denied);
            if let Some(target) = local {
                if seen.insert((c.caller.clone(), target.clone())) {
                    self.cx
                        .add_edge(&c.caller, &target, "calls", c.line, Some("call"));
                }
                continue;
            }
            // Builtin-like names never bind by name; a typed receiver or a class scope still
            // binds them (`listener.str()` on a `StringMatchResultListener`).
            let deferred = match c.callee {
                Callee::Bare(_) if denied => None,
                Callee::Bare(name) => Some((CCallKind::Bare, name, String::new(), None)),
                Callee::Member(Some(recv), name) if self.cpp => self
                    .vars
                    .get(&recv)
                    .map(|ty| (CCallKind::Typed, name, ty.clone(), None)),
                Callee::Member(..) => None,
                Callee::Qualified(scope, name) => {
                    let fallback = self.names.get(&name).cloned().filter(|_| !denied);
                    Some((CCallKind::Qualified, name, scope, fallback))
                }
            };
            if let Some((kind, name, receiver, fallback)) = deferred {
                self.facts.calls.push(CCall {
                    caller: c.caller,
                    kind,
                    name,
                    receiver,
                    fallback,
                    line: c.line,
                });
            }
        }
    }
}

/// Sorted. Names a call never binds to by name (in the file, across files, or as the fallback of a
/// qualified call): Graphify's builtin-name denylist (Python builtins and JavaScript globals) as
/// observed on C and C++ calls.
const DENIED_CALLS: &[&str] = &[
    "Array",
    "BigInt",
    "Bool",
    "Boolean",
    "Date",
    "Error",
    "EvalError",
    "Float",
    "Int",
    "Int32",
    "Int64",
    "JSON",
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
    "Set",
    "String",
    "Symbol",
    "SyntaxError",
    "TypeError",
    "URIError",
    "URL",
    "URLSearchParams",
    "WeakMap",
    "WeakSet",
    "abs",
    "all",
    "any",
    "bool",
    "bytes",
    "callable",
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
    "object",
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

/// An `ERROR` child (a `MISSING ;` alone does not count).
fn has_error_child(n: TsNode) -> bool {
    let mut c = n.walk();
    let found = n.children(&mut c).any(|ch| ch.is_error());
    found
}

fn is_structural(e: &Edge) -> bool {
    e.relation == "method" || e.relation == "contains"
}

fn children_by_field<'t>(node: TsNode<'t>, field: &str) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.children_by_field_name(field, &mut c).collect()
}

/// The function declarator under pointer declarators, if the declarator declares a function.
fn function_declarator(mut d: TsNode) -> Option<TsNode> {
    loop {
        match d.kind() {
            "function_declarator" => return Some(d),
            "pointer_declarator" | "init_declarator" => d = d.child_by_field_name("declarator")?,
            _ => return None,
        }
    }
}

/// The field identifier a field declarator names (references declare nothing).
fn field_name(mut d: TsNode) -> Option<TsNode> {
    loop {
        match d.kind() {
            "field_identifier" | "identifier" => return Some(d),
            "pointer_declarator" | "array_declarator" | "init_declarator" => {
                d = d.child_by_field_name("declarator")?
            }
            _ => return None,
        }
    }
}

/// The innermost `name` of a qualified identifier (`a::b::C` → `C`).
fn last_name(mut q: TsNode) -> Option<TsNode> {
    while q.kind() == "qualified_identifier" {
        q = q.child_by_field_name("name")?;
    }
    Some(q)
}

/// A base-clause entry as (lookup name, stub label): `Base`; `Tmpl<int>` → `Tmpl`; a qualified
/// base drops its first qualifier only (`::testing::Test` → `Test` / `testing::Test`,
/// `detail::buffer<char>` → `buffer` / `buffer<char>`).
fn base_name<'s>(base: TsNode, src: &'s [u8]) -> Option<(&'s str, &'s str)> {
    let text = |n: TsNode| std::str::from_utf8(&src[n.byte_range()]).ok();
    let labelled = match base.kind() {
        "type_identifier" => base,
        "template_type" => base.child_by_field_name("name")?,
        "qualified_identifier" => base.child_by_field_name("name")?,
        _ => return None,
    };
    let mut key = last_name(labelled)?;
    if key.kind() == "template_type" {
        key = key.child_by_field_name("name")?;
    }
    if !matches!(key.kind(), "type_identifier" | "identifier") {
        return None;
    }
    Some((text(key)?, text(labelled)?))
}

/// `class MACRO Name { ... }` parses as a function definition whose type is a bodiless class
/// specifier and whose declarator is the identifier `Name`.
/// With a base list (`class MACRO Name : public ns::Base {`), the name lands in an `ERROR`
/// node and the base becomes the declarator: (name, base).
fn misparsed_class_name(n: TsNode) -> Option<(TsNode, Option<TsNode>)> {
    let ty = n.child_by_field_name("type")?;
    if !matches!(ty.kind(), "class_specifier" | "struct_specifier")
        || ty.child_by_field_name("body").is_some()
    {
        return None;
    }
    let d = n.child_by_field_name("declarator")?;
    let mut c = n.walk();
    let error = n.children(&mut c).find(|ch| ch.is_error());
    match error {
        None => (d.kind() == "identifier").then_some((d, None)),
        Some(e) => {
            let name = e.named_child(0).filter(|x| x.kind() == "identifier")?;
            matches!(d.kind(), "identifier" | "qualified_identifier").then_some((name, Some(d)))
        }
    }
}

/// A method declarator the parse moved into an `ERROR` child of a declaration
/// (`explicit Name(...)` in a misread class body).
fn error_declarator(decl: TsNode) -> Option<TsNode> {
    let mut c = decl.walk();
    let errors: Vec<_> = decl.children(&mut c).filter(|ch| ch.is_error()).collect();
    errors.into_iter().find_map(|e| {
        let mut c = e.walk();
        let found = e
            .named_children(&mut c)
            .find(|x| x.kind() == "function_declarator");
        found
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_language_follows_markers() {
        assert!(is_cpp_header(b"// class Foo\nstruct S { int w; };"));
        assert!(is_cpp_header(b"struct S { int w; };\n// a::b"));
        assert!(is_cpp_header(b"// templates"));
        assert!(!is_cpp_header(b"// class\nstruct S { int w; };"));
        assert!(!is_cpp_header(b"// namespace\n// public :"));
        assert!(!is_cpp_header(b"extern \"C\" { struct F { int r; }; }"));
    }

    #[test]
    fn denied_calls_are_sorted() {
        assert!(DENIED_CALLS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn call_names_strip_method_dots_and_parens() {
        assert_eq!(call_name(".area()"), "area");
        assert_eq!(call_name("main()"), "main");
        assert_eq!(call_name("Shape::name()"), "Shape::name");
        assert_eq!(call_name("Shape"), "Shape");
    }
}

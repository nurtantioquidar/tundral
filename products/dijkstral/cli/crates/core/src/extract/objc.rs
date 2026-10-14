//! Objective-C extraction (§34). See `docs/graphify-format.md`.
//!
//! `.m` and `.mm` files, and `.h` files with Objective-C markers ([`header_lang`]), parse with
//! tree-sitter-objc. Graphify models the Objective-C layer only: C functions, structs, macros and
//! comments give nothing (no rationale nodes).
//!
//! Graphify behaviours reproduced because its ids and edges depend on them:
//!
//! - `NS_ASSUME_NONNULL_BEGIN`/`END` and `NS_HEADER_AUDIT_BEGIN`/`END(...)` are blanked before
//!   parsing (tree-sitter-objc misreads the rest of the file otherwise); other macros are not.
//! - `@interface`, `@implementation` and class extensions or categories of a class share one node
//!   `file_slug(Name)` (first wins; `contains` from the file once). In a file whose stem is
//!   exactly `Before+After`, categories and extensions use `slug(dir/Before)_slug(Name)` instead
//!   (Graphify prefixes these with the absolute scan root: quirk OC1, not reproduced).
//! - Protocols are `file_slug(Name)` labelled `<Name>`; typedef'd and named C enums with a body
//!   are nodes with `case_of` enumerators (`NS_ENUM` is not: the macro defeats the parse).
//! - Methods are `owner_slug(selector)`, labelled `-sel`/`+sel` where the selector is its keyword
//!   parts concatenated without colons (trailing macro identifiers included:
//!   `-fooNS_REQUIRES_SUPER`). Every declaration and definition emits a `method` edge.
//! - `inherits` (superclass) and `implements` (adopted or inherited protocols) target the in-file
//!   class, protocol or enum of that name, else a per-file stub; so do property types
//!   (`references`, context `field`, every type identifier of the declared type) and `[X alloc]`
//!   (`references`, context `type`, also `[self alloc]` to a stub `self`).
//! - A message `[r sel]` in a method binds in the file to every node (not the file) whose id ends
//!   with the lower-cased selector, except the caller (quirk OC2); `x.name` binds to the file's
//!   methods named exactly `name` (`accesses`). The first of `calls`/`accesses` per pair wins.
//!   These body edges follow all structural edges of the file.
//! - Everything that needs other files goes to [`ObjcFacts`] for `crate::resolve::objc`: imports,
//!   stubs, classes, definitions (header/implementation merge), typed message sends, and, in
//!   implementation files, every edge leaving a definition, whose target is an alias the
//!   resolver either redirects or leaves dangling (Graphify drops the duplicates a merged
//!   definition brings from its implementation file).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{slug, Ctx};
use crate::facts::LangFacts;
use crate::lang::{parse, Lang};

/// One `#import`/`#include` with a literal path, or `@import Module`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjcImport {
    /// The path as written, without quotes or angle brackets (`@import A.B` → `A.B`).
    pub path: String,
    /// `"..."`: resolved relative to the importing file first.
    pub quoted: bool,
    pub line: u32,
}

/// A class (interface, implementation, category or extension) of this file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjcClass {
    /// Local id.
    pub id: String,
    pub name: String,
    /// The superclass named by the first `@interface` that has one.
    pub superclass: Option<String>,
    /// (property, declared class) of its `@property` declarations in this file, first wins.
    pub props: Vec<(String, String)>,
    /// (instance variable, declared class) of its `{ T *ivar; }` block in this file.
    pub ivars: Vec<(String, String)>,
}

/// How a message receiver was typed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ObjcRecv {
    /// `self`, `super` or a capitalised name: the class itself (EXTRACTED 1.0).
    Class(String),
    /// A variable or instance variable with a declared class type (INFERRED 0.85).
    Typed(String),
    /// `self.prop`: typed by the property of that name on the class or a superclass (INFERRED
    /// 0.85).
    Prop { class: String, prop: String },
    /// An undeclared lower-case name: an instance variable of the class or a superclass, maybe
    /// declared in another file (INFERRED 0.85).
    Ivar { class: String, name: String },
}

/// A message send with a typed receiver, for the cross-file pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjcSend {
    /// Local id of the calling method.
    pub caller: String,
    pub recv: ObjcRecv,
    pub selector: String,
    pub line: u32,
}

/// An edge of an implementation file leaving a definition, emitted with target `alias`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjcAlias {
    pub alias: String,
    /// Local ids.
    pub source: String,
    pub relation: String,
    pub target: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ObjcFacts {
    /// Local id of the file node.
    pub file_id: String,
    pub imports: Vec<ObjcImport>,
    /// Per-file stubs: (local id, label).
    pub stubs: Vec<(String, String)>,
    pub classes: Vec<ObjcClass>,
    /// Definitions (every node but the file and stubs): local id, line of the first one.
    pub defs: Vec<(String, u32)>,
    pub sends: Vec<ObjcSend>,
    /// (caller, target) local ids of the in-file body edges, which the cross-file pass skips.
    pub linked: Vec<(String, String)>,
    pub aliases: Vec<ObjcAlias>,
}

/// The language of a `.h` file: Objective-C when its bytes contain `@interface`,
/// `@implementation`, `@protocol`, `#import` or `@import` anywhere (comments included), else C or
/// C++ by `super::c::is_cpp_header`. Unreadable files count as C.
pub fn header_lang(path: &Path) -> Option<Lang> {
    let src = std::fs::read(path).unwrap_or_default();
    Some(if is_objc_header(&src) {
        Lang::ObjC
    } else if super::c::is_cpp_header(&src) {
        Lang::Cpp
    } else {
        Lang::C
    })
}

const OBJC_MARKERS: &[&[u8]] = &[
    b"@interface",
    b"@implementation",
    b"@protocol",
    b"#import",
    b"@import",
];

/// Whether a `.h` file is Objective-C (see [`header_lang`]).
pub fn is_objc_header(src: &[u8]) -> bool {
    OBJC_MARKERS
        .iter()
        .any(|m| src.windows(m.len()).any(|w| w == *m))
}

/// Macros Graphify removes before parsing; `true` when an argument list follows.
const BLANKED: &[(&[u8], bool)] = &[
    (b"NS_ASSUME_NONNULL_BEGIN", false),
    (b"NS_ASSUME_NONNULL_END", false),
    (b"NS_HEADER_AUDIT_BEGIN", true),
    (b"NS_HEADER_AUDIT_END", true),
];

/// The source with [`BLANKED`] macros replaced by spaces (offsets and lines unchanged), or `None`
/// when there is none.
fn blank_macros(src: &[u8]) -> Option<Vec<u8>> {
    let mut out: Option<Vec<u8>> = None;
    for &(name, args) in BLANKED {
        let mut from = 0;
        while let Some(pos) = find(&src[from..], name).map(|p| p + from) {
            let buf = out.get_or_insert_with(|| src.to_vec());
            let mut end = pos + name.len();
            if args {
                let mut i = end;
                while i < src.len() && (src[i] == b' ' || src[i] == b'\t') {
                    i += 1;
                }
                if src.get(i) == Some(&b'(') {
                    if let Some(close) = src[i..].iter().position(|&b| b == b')') {
                        end = i + close + 1;
                    }
                }
            }
            for b in &mut buf[pos..end] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
            from = end;
        }
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.children(&mut c).collect()
}

/// A body event, bound after the walk (in-file binding needs every node of the file).
enum Event {
    Send {
        caller: String,
        selector: String,
        line: u32,
    },
    Access {
        caller: String,
        name: String,
        line: u32,
    },
    /// `@selector(name)`: `calls` to the file's methods named exactly `name`.
    Selector {
        caller: String,
        name: String,
        line: u32,
    },
    Alloc {
        caller: String,
        target: String,
        line: u32,
    },
}

/// The class, category or protocol whose body is being walked.
struct Scope {
    /// Node id of the class or protocol.
    owner: String,
    /// Class name (`self` in method bodies); `None` for protocols.
    class: Option<String>,
}

/// A deferred body edge: (source, target, relation, line, context).
type BodyEdge = (String, String, &'static str, u32, Option<&'static str>);

struct Objc<'c, 'a, 's> {
    cx: &'c mut Ctx<'a>,
    /// The parsed text (macros blanked, offsets unchanged).
    src: &'s [u8],
    /// `slug(dir/Before)` when the file stem is `Before+After`.
    category_prefix: Option<String>,
    /// Local ids of every node but the file, in creation order, and every emitted id.
    order: Vec<String>,
    /// Stubs first created by a method body (`[X alloc]`): not bound by in-file messages.
    body_stubs: HashSet<String>,
    /// Inside a method body.
    in_body: bool,
    emitted: HashSet<String>,
    /// In-file classes, protocols and enums by name (pre-pass, first wins).
    names: HashMap<String, String>,
    /// Variable → declared class (declarations anywhere and instance variables, first wins).
    vars: HashMap<String, String>,
    /// Index into `facts.classes` by class id.
    class_index: HashMap<String, usize>,
    /// (method id, selector) of every method node.
    selectors: Vec<(String, String, String)>,
    events: Vec<Event>,
    aliases: usize,
    facts: ObjcFacts,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let original: &[u8] = cx.src;
    let blanked = blank_macros(original);
    let reparsed = blanked.as_deref().and_then(|b| parse(Lang::ObjC, b));
    let tree = if blanked.is_some() {
        reparsed.as_ref()
    } else {
        tree
    };
    let src: &[u8] = blanked.as_deref().unwrap_or(original);
    let file_id = cx.file_id.clone();
    let rel = cx.rel;
    let mut w = Objc {
        src,
        category_prefix: category_prefix(rel),
        order: Vec::new(),
        body_stubs: HashSet::new(),
        in_body: false,
        emitted: HashSet::from([file_id.clone()]),
        names: HashMap::new(),
        vars: HashMap::new(),
        class_index: HashMap::new(),
        selectors: Vec::new(),
        events: Vec::new(),
        aliases: 0,
        facts: ObjcFacts {
            file_id,
            ..ObjcFacts::default()
        },
        cx,
    };
    if let Some(tree) = tree {
        let root = tree.root_node();
        w.prepass(root);
        w.visit(root, None);
    }
    w.bind_events();
    w.tidy();
    let Objc { cx, facts, .. } = w;
    cx.facts.lang = LangFacts::ObjC(facts);
}

/// `slug(dir/Before)` for a file whose stem is exactly two non-empty parts joined by `+`.
fn category_prefix(rel: &str) -> Option<String> {
    let (dir, name) = match rel.rfind('/') {
        Some(i) => (&rel[..i], &rel[i + 1..]),
        None => ("", rel),
    };
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    };
    let parts: Vec<&str> = stem.split('+').collect();
    if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    Some(if dir.is_empty() {
        slug(parts[0])
    } else {
        slug(&format!("{dir}/{}", parts[0]))
    })
}

/// The first identifier child that has no field name (a class or protocol name).
fn unfielded_identifier(node: TsNode) -> Option<TsNode> {
    let mut c = node.walk();
    if !c.goto_first_child() {
        return None;
    }
    loop {
        let n = c.node();
        if n.kind() == "identifier" && c.field_name().is_none() {
            return Some(n);
        }
        if !c.goto_next_sibling() {
            return None;
        }
    }
}

/// `@interface X (Cat)`, `@interface X ()`, `@implementation X (Cat)`.
fn is_category(node: TsNode) -> bool {
    node.child_by_field_name("category").is_some() || children(node).iter().any(|c| c.kind() == "(")
}

impl<'s> Objc<'_, '_, 's> {
    fn text(&self, node: TsNode) -> &'s str {
        std::str::from_utf8(&self.src[node.byte_range()]).unwrap_or("")
    }

    fn file_child(&self, name: &str) -> String {
        Ctx::child_id(&self.cx.file_id, name)
    }

    fn class_id(&self, name: &str, category: bool) -> String {
        match (&self.category_prefix, category) {
            (Some(prefix), true) => Ctx::child_id(prefix, name),
            _ => self.file_child(name),
        }
    }

    // ---- pre-pass ------------------------------------------------------------------------------

    /// Names of the file's classes, protocols and enums, and the declared class of every
    /// variable, instance variable and property.
    fn prepass(&mut self, node: TsNode) {
        match node.kind() {
            "class_interface" | "class_implementation" => {
                if let Some(name) = unfielded_identifier(node) {
                    let name = self.text(name);
                    let id = self.class_id(name, is_category(node));
                    self.names.entry(name.to_string()).or_insert(id);
                }
            }
            "protocol_declaration" => {
                if let Some(name) = unfielded_identifier(node) {
                    let name = self.text(name);
                    let id = self.file_child(name);
                    self.names.entry(name.to_string()).or_insert(id);
                }
            }
            "type_definition" | "enum_specifier" => {
                if let Some((name, _)) = self.enum_of(node) {
                    let id = self.file_child(name);
                    self.names.entry(name.to_string()).or_insert(id);
                }
            }
            "declaration" => {
                if let Some(ty) = node.child_by_field_name("type") {
                    if ty.kind() == "type_identifier" {
                        let ty = self.text(ty);
                        let mut c = node.walk();
                        let decls: Vec<TsNode> =
                            node.children_by_field_name("declarator", &mut c).collect();
                        for d in decls {
                            if let Some(var) = self.pointer_name(d) {
                                self.vars
                                    .entry(var.to_string())
                                    .or_insert_with(|| ty.to_string());
                            }
                        }
                    }
                }
            }
            "instance_variable" => {
                for decl in named_children(node) {
                    if decl.kind() == "struct_declaration" {
                        if let Some((name, ty)) = self.declared(decl, false) {
                            self.vars
                                .entry(name.to_string())
                                .or_insert_with(|| ty.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
        for child in named_children(node) {
            self.prepass(child);
        }
    }

    /// `(name, class)` of an instance variable (`T *name`) or a property (`T *name`, `T name`):
    /// the first type identifier, also of a generic (`NSArray<X *>` → `NSArray`).
    fn declared(&self, decl: TsNode, property: bool) -> Option<(&'s str, &'s str)> {
        let mut ty = None;
        for c in named_children(decl) {
            match c.kind() {
                "type_identifier" if ty.is_none() => ty = Some(self.text(c)),
                // `T<P> *x`, `NSArray<X *> *x`: not a receiver type.
                "generic_specifier" => return None,
                "struct_declarator" => {
                    let ty = ty?;
                    let name = if property {
                        self.declarator_name(c)
                    } else {
                        named_children(c)
                            .into_iter()
                            .find_map(|d| self.pointer_name(d))
                    };
                    return name.map(|n| (n, ty));
                }
                _ => {}
            }
        }
        None
    }

    /// The variable of a pointer declarator (`*v`, `*v = x`).
    fn pointer_name(&self, d: TsNode) -> Option<&'s str> {
        match d.kind() {
            "init_declarator" => self.pointer_name(d.child_by_field_name("declarator")?),
            "pointer_declarator" => {
                let inner = d.child_by_field_name("declarator")?;
                (inner.kind() == "identifier").then(|| self.text(inner))
            }
            _ => None,
        }
    }

    /// The name a declarator declares, through pointers.
    fn declarator_name(&self, d: TsNode) -> Option<&'s str> {
        match d.kind() {
            "identifier" | "field_identifier" => Some(self.text(d)),
            _ => {
                let inner = d
                    .child_by_field_name("declarator")
                    .or_else(|| named_children(d).into_iter().next())?;
                self.declarator_name(inner)
            }
        }
    }

    /// `typedef enum { ... } Name` or `enum Name { ... }`: the name and the enumerator list.
    fn enum_of<'t>(&self, node: TsNode<'t>) -> Option<(&'s str, TsNode<'t>)> {
        match node.kind() {
            "type_definition" => {
                let ty = node.child_by_field_name("type")?;
                if ty.kind() != "enum_specifier" {
                    return None;
                }
                let body = ty.child_by_field_name("body")?;
                let decl = node.child_by_field_name("declarator")?;
                (decl.kind() == "type_identifier").then(|| (self.text(decl), body))
            }
            "enum_specifier" => {
                let body = node.child_by_field_name("body")?;
                let name = node.child_by_field_name("name")?;
                Some((self.text(name), body))
            }
            _ => None,
        }
    }

    // ---- emission ------------------------------------------------------------------------------

    /// A definition node, once per id (first wins). Returns whether it was new.
    fn def(&mut self, id: &str, label: String, line: u32) -> bool {
        if !self.emitted.insert(id.to_string()) {
            return false;
        }
        self.cx.add_def(id.to_string(), label, line, vec![]);
        self.order.push(id.to_string());
        self.facts.defs.push((id.to_string(), line));
        true
    }

    /// The in-file class, protocol or enum called `name`, else the per-file stub.
    fn type_target(&mut self, name: &str) -> Option<String> {
        if let Some(id) = self.names.get(name) {
            return Some(id.clone());
        }
        let id = slug(name);
        if id.is_empty() {
            return None;
        }
        if self.emitted.insert(id.clone()) {
            self.cx.add_stub(id.clone(), name.to_string());
            self.order.push(id.clone());
            self.facts.stubs.push((id.clone(), name.to_string()));
            if self.in_body {
                self.body_stubs.insert(id.clone());
            }
        } else if !self.in_body {
            self.body_stubs.remove(&id);
        }
        Some(id)
    }

    fn edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &'static str,
        line: u32,
        context: Option<&str>,
    ) {
        // Every edge targets an alias the resolver redirects to the real target, or leaves
        // dangling when Graphify drops the edge (header/implementation merges).
        let alias = format!("#objc:{}:{}", self.cx.rel, self.aliases);
        self.aliases += 1;
        self.facts.aliases.push(ObjcAlias {
            alias: alias.clone(),
            source: source.to_string(),
            relation: relation.to_string(),
            target: target.to_string(),
        });
        self.cx.add_edge(source, &alias, relation, line, context);
    }

    fn type_edge(&mut self, source: &str, name: &str, relation: &'static str, line: u32) {
        if let Some(target) = self.type_target(name) {
            self.edge(source, &target, relation, line, None);
        }
    }

    // ---- structure -----------------------------------------------------------------------------

    /// Every node outside C function bodies; methods and properties belong to the innermost
    /// enclosing class, category or protocol (`scope`), methods else to the file.
    fn visit(&mut self, node: TsNode, scope: Option<&Scope>) {
        match node.kind() {
            "preproc_include" => self.include(node),
            "module_import" => {
                let mut c = node.walk();
                let parts: Vec<&str> = node
                    .children_by_field_name("path", &mut c)
                    .map(|p| self.text(p))
                    .collect();
                if !parts.is_empty() {
                    self.facts.imports.push(ObjcImport {
                        path: parts.join("."),
                        quoted: false,
                        line: Ctx::line(node),
                    });
                }
            }
            "class_interface" | "class_implementation" => self.class(node),
            "protocol_declaration" => self.protocol(node),
            "type_definition" | "enum_specifier" if self.enum_of(node).is_some() => {
                if let Some((name, body)) = self.enum_of(node) {
                    self.enumeration(name, body, Ctx::line(node));
                }
            }
            "method_declaration" | "method_definition" => self.method(node, scope),
            // C functions are not modelled, nor anything in their bodies.
            "function_definition" => {}
            "identifier"
            | "parameterized_arguments"
            | "protocol_reference_list"
            | "instance_variables" => {}
            _ => {
                for child in named_children(node) {
                    self.visit(child, scope);
                }
            }
        }
    }

    fn include(&mut self, node: TsNode) {
        let Some(path) = node.child_by_field_name("path") else {
            return;
        };
        let (text, quoted) = match path.kind() {
            "string_literal" => {
                let inner = named_children(path)
                    .into_iter()
                    .find(|c| c.kind() == "string_content");
                (inner.map_or("", |c| self.text(c)), true)
            }
            "system_lib_string" => {
                let t = self.text(path);
                (t.trim_start_matches('<').trim_end_matches('>'), false)
            }
            _ => return,
        };
        if !text.is_empty() {
            self.facts.imports.push(ObjcImport {
                path: text.to_string(),
                quoted,
                line: Ctx::line(node),
            });
        }
    }

    fn class(&mut self, node: TsNode) {
        let Some(name) = unfielded_identifier(node) else {
            for child in named_children(node) {
                self.visit(child, None);
            }
            return;
        };
        let name = self.text(name);
        let id = self.class_id(name, is_category(node));
        let line = Ctx::line(node);
        let new = self.def(&id, name.to_string(), line);
        if !self.class_index.contains_key(&id) {
            self.class_index
                .insert(id.clone(), self.facts.classes.len());
            self.facts.classes.push(ObjcClass {
                id: id.clone(),
                name: name.to_string(),
                superclass: None,
                props: Vec::new(),
                ivars: Vec::new(),
            });
        }
        // Every `@interface` (categories and extensions too) is contained by the file again; an
        // `@implementation` only when it creates the class.
        if new || node.kind() == "class_interface" {
            let file = self.cx.file_id.clone();
            self.edge(&file, &id, "contains", line, None);
        }
        if let Some(sup) = node.child_by_field_name("superclass") {
            let sup = self.text(sup);
            let class = &mut self.facts.classes[self.class_index[&id]];
            class.superclass.get_or_insert_with(|| sup.to_string());
            self.type_edge(&id, sup, "inherits", line);
        }
        for child in named_children(node) {
            self.protocol_list(child, &id, line);
        }
        let scope = Scope {
            owner: id,
            class: Some(name.to_string()),
        };
        if node.kind() == "class_interface" {
            // Only direct members: anything under `#if` in an interface is not modelled.
            for child in named_children(node) {
                match child.kind() {
                    "method_declaration" => self.method(child, Some(&scope)),
                    "property_declaration" => self.property(child, &scope),
                    "instance_variables" => {
                        let i = self.class_index[&scope.owner];
                        for var in named_children(child) {
                            for decl in named_children(var) {
                                if decl.kind() != "struct_declaration" {
                                    continue;
                                }
                                if let Some((name, ty)) = self.declared(decl, false) {
                                    let ivars = &mut self.facts.classes[i].ivars;
                                    if !ivars.iter().any(|(n, _)| n == name) {
                                        ivars.push((name.to_string(), ty.to_string()));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else {
            self.members(node, "method_definition", &scope);
        }
    }

    /// Methods of `kind` anywhere below an implementation or protocol body (`#if` included).
    fn members(&mut self, node: TsNode, kind: &str, scope: &Scope) {
        for child in named_children(node) {
            if child.kind() == kind {
                self.method(child, Some(scope));
            } else if !matches!(child.kind(), "method_definition" | "method_declaration") {
                self.members(child, kind, scope);
            }
        }
    }

    /// `implements` edges for `<P, Q>` after a class, category or protocol name.
    fn protocol_list(&mut self, node: TsNode, owner: &str, line: u32) {
        match node.kind() {
            "parameterized_arguments" => {
                for tn in named_children(node) {
                    for t in named_children(tn) {
                        if t.kind() == "type_identifier" {
                            let p = self.text(t);
                            self.type_edge(owner, p, "implements", line);
                        }
                    }
                }
            }
            "protocol_reference_list" => {
                for t in named_children(node) {
                    if t.kind() == "identifier" {
                        let p = self.text(t);
                        self.type_edge(owner, p, "implements", line);
                    }
                }
            }
            _ => {}
        }
    }

    fn protocol(&mut self, node: TsNode) {
        let Some(name) = unfielded_identifier(node) else {
            return;
        };
        let name = self.text(name);
        let id = self.file_child(name);
        let line = Ctx::line(node);
        if self.def(&id, format!("<{name}>"), line) {
            let file = self.cx.file_id.clone();
            self.edge(&file, &id, "contains", line, None);
        }
        for child in named_children(node) {
            self.protocol_list(child, &id, line);
        }
        let scope = Scope {
            owner: id,
            class: None,
        };
        // Protocol properties are not modelled.
        self.members(node, "method_declaration", &scope);
    }

    /// `references` (context `field`) to every type identifier of a property's type; a class
    /// also records the property's declared class.
    fn property(&mut self, n: TsNode, scope: &Scope) {
        let line = Ctx::line(n);
        let owner = scope.owner.clone();
        for decl in named_children(n) {
            if decl.kind() != "struct_declaration" {
                continue;
            }
            if let (Some(&i), Some((prop, ty))) =
                (self.class_index.get(&owner), self.declared(decl, true))
            {
                let props = &mut self.facts.classes[i].props;
                if !props.iter().any(|(p, _)| p == prop) {
                    props.push((prop.to_string(), ty.to_string()));
                }
            }
            let mut types = Vec::new();
            self.type_identifiers(decl, &mut types);
            for t in types {
                if let Some(target) = self.type_target(t) {
                    self.edge(&owner, &target, "references", line, Some("field"));
                }
            }
        }
    }

    fn type_identifiers(&self, node: TsNode, out: &mut Vec<&'s str>) {
        for c in named_children(node) {
            match c.kind() {
                "type_identifier" => out.push(self.text(c)),
                "struct_declarator" => {}
                _ => self.type_identifiers(c, out),
            }
        }
    }

    /// A method declaration or definition: owned by the enclosing class or protocol, else by the
    /// file (id prefix `slug(path with extension)`, `method` edge from the file node).
    fn method(&mut self, n: TsNode, scope: Option<&Scope>) {
        let mut selector = String::new();
        let mut sign = '-';
        let mut body = None;
        for (i, c) in children(n).into_iter().enumerate() {
            match c.kind() {
                "+" if i == 0 => sign = '+',
                "identifier" => selector.push_str(self.text(c)),
                "compound_statement" => body = Some(c),
                _ => {}
            }
        }
        if selector.is_empty() {
            return;
        }
        let (prefix, source) = match scope {
            Some(s) => (s.owner.clone(), s.owner.clone()),
            None => (slug(self.cx.rel), self.cx.file_id.clone()),
        };
        let id = Ctx::child_id(&prefix, &selector);
        let line = Ctx::line(n);
        if self.def(&id, format!("{sign}{selector}"), line) {
            self.selectors.push((id.clone(), selector, prefix.clone()));
        }
        self.edge(&source, &id, "method", line, None);
        if let Some(body) = body {
            let class = scope.and_then(|s| s.class.clone());
            self.in_body = true;
            self.body(body, &id, class.as_deref());
            self.in_body = false;
        }
    }

    fn enumeration(&mut self, name: &str, body: TsNode, line: u32) {
        let id = self.file_child(name);
        if self.def(&id, name.to_string(), line) {
            let file = self.cx.file_id.clone();
            self.edge(&file, &id, "contains", line, None);
        }
        for e in named_children(body) {
            if e.kind() != "enumerator" {
                continue;
            }
            let Some(en) = e.child_by_field_name("name") else {
                continue;
            };
            let en = self.text(en);
            let eid = Ctx::child_id(&id, en);
            let eline = Ctx::line(e);
            self.def(&eid, en.to_string(), eline);
            self.edge(&id, &eid, "case_of", eline, None);
        }
    }

    // ---- method bodies -------------------------------------------------------------------------

    fn body(&mut self, node: TsNode, caller: &str, class: Option<&str>) {
        match node.kind() {
            "message_expression" => {
                let line = Ctx::line(node);
                let mut selector = String::new();
                let mut c = node.walk();
                let parts: Vec<TsNode> = node.children_by_field_name("method", &mut c).collect();
                for m in parts {
                    selector.push_str(self.text(m));
                }
                if !selector.is_empty() {
                    self.events.push(Event::Send {
                        caller: caller.to_string(),
                        selector: selector.clone(),
                        line,
                    });
                    if let Some(recv) = node.child_by_field_name("receiver") {
                        if selector == "alloc" && recv.kind() == "identifier" {
                            let name = self.text(recv);
                            if let Some(target) = self.type_target(name) {
                                self.events.push(Event::Alloc {
                                    caller: caller.to_string(),
                                    target,
                                    line,
                                });
                            }
                        }
                        if let Some(recv) = self.receiver(recv, class) {
                            self.facts.sends.push(ObjcSend {
                                caller: caller.to_string(),
                                recv,
                                selector,
                                line,
                            });
                        }
                    }
                }
            }
            "selector_expression" => {
                let mut selector = String::new();
                self.identifiers(node, &mut selector);
                if !selector.is_empty() {
                    self.events.push(Event::Selector {
                        caller: caller.to_string(),
                        name: selector,
                        line: Ctx::line(node),
                    });
                }
            }
            "field_expression" => {
                if let Some(f) = node.child_by_field_name("field") {
                    self.events.push(Event::Access {
                        caller: caller.to_string(),
                        name: self.text(f).to_string(),
                        line: Ctx::line(node),
                    });
                }
            }
            _ => {}
        }
        for child in named_children(node) {
            self.body(child, caller, class);
        }
    }

    /// Every identifier under `node`, concatenated (an `@selector(a:b:)`'s keywords).
    fn identifiers(&self, node: TsNode, out: &mut String) {
        for c in named_children(node) {
            if c.kind() == "identifier" {
                out.push_str(self.text(c));
            } else {
                self.identifiers(c, out);
            }
        }
    }

    fn receiver(&self, r: TsNode, class: Option<&str>) -> Option<ObjcRecv> {
        match r.kind() {
            "self" | "super" => class.map(|c| ObjcRecv::Class(c.to_string())),
            "identifier" => {
                let t = self.text(r);
                if t == "self" || t == "super" {
                    class.map(|c| ObjcRecv::Class(c.to_string()))
                } else if let Some(ty) = self.vars.get(t) {
                    Some(ObjcRecv::Typed(ty.clone()))
                } else if t.starts_with(|c: char| c.is_uppercase()) {
                    Some(ObjcRecv::Class(t.to_string()))
                } else {
                    class.map(|c| ObjcRecv::Ivar {
                        class: c.to_string(),
                        name: t.to_string(),
                    })
                }
            }
            "field_expression" => {
                let arg = r.child_by_field_name("argument")?;
                if self.text(arg) != "self" {
                    return None;
                }
                let field = self.text(r.child_by_field_name("field")?);
                Some(ObjcRecv::Prop {
                    class: class?.to_string(),
                    prop: field.to_string(),
                })
            }
            _ => None,
        }
    }

    /// Two of Graphify's folds differ from the shared one for Objective-C edges; reproduce them by
    /// dropping the edge that would leave a trace:
    ///
    /// - a property `references` (context `field`) on a pair that also has `inherits` or
    ///   `implements` is absorbed without its context, whatever the order;
    /// - a repeated self-loop of one relation keeps the first.
    fn tidy(&mut self) {
        let real = |e: &crate::facts::Edge, aliases: &HashMap<&str, &str>| -> String {
            aliases
                .get(e.target.as_str())
                .map_or_else(|| e.target.clone(), |t| t.to_string())
        };
        let aliases: HashMap<&str, &str> = self
            .facts
            .aliases
            .iter()
            .map(|a| (a.alias.as_str(), a.target.as_str()))
            .collect();
        let typed: HashSet<(String, String)> = self
            .cx
            .facts
            .edges
            .iter()
            .filter(|e| e.relation == "inherits" || e.relation == "implements")
            .map(|e| (e.source.clone(), real(e, &aliases)))
            .collect();
        let mut loops: HashSet<(String, String)> = HashSet::new();
        let keep: Vec<bool> = self
            .cx
            .facts
            .edges
            .iter()
            .map(|e| {
                let target = real(e, &aliases);
                if e.relation == "references"
                    && e.context.as_deref() == Some("field")
                    && typed.contains(&(e.source.clone(), target.clone()))
                {
                    return false;
                }
                target != e.source || loops.insert((e.source.clone(), e.relation.clone()))
            })
            .collect();
        let mut i = 0;
        self.cx.facts.edges.retain(|_| {
            i += 1;
            keep[i - 1]
        });
    }

    /// In-file binding of the body events, emitted after every structural edge.
    fn bind_events(&mut self) {
        let mut linked: HashSet<(String, String)> = HashSet::new();
        let mut edges: Vec<BodyEdge> = Vec::new();
        for ev in std::mem::take(&mut self.events) {
            match ev {
                Event::Send {
                    caller,
                    selector,
                    line,
                } => {
                    let s = selector.to_lowercase();
                    for id in &self.order {
                        if *id != caller
                            && !self.body_stubs.contains(id)
                            && id.ends_with(&s)
                            && linked.insert((caller.clone(), id.clone()))
                        {
                            edges.push((caller.clone(), id.clone(), "calls", line, Some("call")));
                        }
                    }
                }
                Event::Access { caller, name, line } => {
                    // Only the caller's own class's methods.
                    let owner = self
                        .selectors
                        .iter()
                        .find(|(id, _, _)| *id == caller)
                        .map(|(_, _, o)| o.clone());
                    for (id, sel, o) in &self.selectors {
                        if *sel == name
                            && Some(o) == owner.as_ref()
                            && *id != caller
                            && linked.insert((caller.clone(), id.clone()))
                        {
                            edges.push((caller.clone(), id.clone(), "accesses", line, None));
                        }
                    }
                }
                Event::Selector { caller, name, line } => {
                    for (id, sel, _) in &self.selectors {
                        if *sel == name
                            && *id != caller
                            && linked.insert((caller.clone(), id.clone()))
                        {
                            edges.push((caller.clone(), id.clone(), "calls", line, Some("call")));
                        }
                    }
                }
                Event::Alloc {
                    caller,
                    target,
                    line,
                } => edges.push((caller, target, "references", line, Some("type"))),
            }
        }
        for (source, target, relation, line, context) in edges {
            self.facts.linked.push((source.clone(), target.clone()));
            self.edge(&source, &target, relation, line, context);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_language_follows_objc_markers() {
        assert!(is_objc_header(b"@interface Foo\n@end"));
        assert!(is_objc_header(b"// #import in a comment"));
        assert!(is_objc_header(b"@import Foundation;"));
        assert!(!is_objc_header(b"@class Foo;\n@end"));
        assert!(!is_objc_header(b"# import \"x.h\""));
    }

    #[test]
    fn category_prefix_needs_two_parts() {
        assert_eq!(
            category_prefix("Sub/View+Extra.h").as_deref(),
            Some("sub_view")
        );
        assert_eq!(category_prefix("X+Y.m").as_deref(), Some("x"));
        assert_eq!(category_prefix("A+B+C.h"), None);
        assert_eq!(category_prefix("d/+L.h"), None);
        assert_eq!(category_prefix("d/T+.h"), None);
        assert_eq!(category_prefix("Plain.m"), None);
    }

    #[test]
    fn macros_are_blanked_in_place() {
        let src = b"NS_ASSUME_NONNULL_BEGIN\n@interface A\n@end\nNS_HEADER_AUDIT_END(x)\n";
        let out = blank_macros(src).unwrap();
        assert_eq!(out.len(), src.len());
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("                       \n@interface A"));
        assert!(!text.contains("AUDIT"));
        assert!(blank_macros(b"@interface A\n@end\n").is_none());
    }
}

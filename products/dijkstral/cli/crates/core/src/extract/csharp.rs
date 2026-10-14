//! C# (§18) extraction. See `docs/graphify-format.md`.
//!
//! One file yields its file node and, in one walk in source order:
//!
//! - `imports` edges for every `using` directive (with Graphify's `metadata`), and a `contains`
//!   edge from the file to the namespace node of every namespace declaration; the node
//!   (`csharp_namespace:<sha1(name)[:16]>`) is shared, so it is emitted with the external kind
//!   (no collision renaming) and the build keeps the first declaring file's;
//! - classes, structs, interfaces, enums and records (`file_slug(ns.Name)[_arity]`, flat for
//!   nested types), with `contains`, base-list `inherits`/`implements` and `generic_arg`
//!   references;
//! - methods (`type_slug(name)`, overloads merged), properties (`defines`, context `field`), enum
//!   members (`case_of`); field and property types are `references` from the type, parameter and
//!   return types from the method, and method attributes `references` with context `attribute`;
//! - methods outside any type (a tree-sitter error recovery, e.g. an interface broken by `#if`)
//!   as functions `file_slug(name)` with label `name()`;
//! - in-file `calls` for bare calls, and calls on a receiver that is neither a name nor
//!   `this`/`base` (`f().m()`, `a.b.m()`), to a method of the file, and `new X()` of a type of
//!   the file; `generic_arg` references for the type arguments of generic method calls.
//!
//! Constructors, fields, events, indexers, operators, delegates and top-level statements produce
//! no node; constructor bodies count as calls from their type. Property accessors and field
//! initializers are not walked. Calls named like a predefined type (`Int()`) are ignored.
//!
//! Targets that need other files are private virtual ids that `crate::resolve::csharp` redirects
//! (they never reach the output):
//!
//! - `@cs:t:<path>|<scope>|<arity>|<name>`: a type reference → the type it names from that
//!   namespace scope, else the shared stub `slug(Name)`;
//! - `@cs:b:<path>|<scope>|<arity>|<name>|<i|m>`: a base-list entry, written twice (as
//!   `inherits` and as `implements`); the resolver keeps the one matching the base's kind;
//! - `@cs:n:<namespace>`: a `using` of a namespace → its namespace node, else an external;
//! - `@cs:a:<path>|<scope>|<target>|<written>`: a `using` alias → the type, else an external.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, slug, Ctx};
use crate::facts::{Attr, Loc, Node, NodeKind};

/// A namespace scope: the file itself (index 0) or one namespace declaration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CsScope {
    /// Full namespace name (`""` for the file scope).
    pub ns: String,
    pub parent: Option<u32>,
    /// `using N;` directives of this scope.
    pub usings: Vec<String>,
    /// `using A = T;` directives: (alias, target without type arguments).
    pub aliases: Vec<(String, String)>,
}

/// A class, struct, interface, enum or record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CsType {
    /// Local id.
    pub id: String,
    pub name: String,
    pub arity: u32,
    /// Namespace, enclosing types and name, without arity: the lookup key.
    pub fqn: String,
    pub partial: bool,
    pub interface: bool,
    /// Innermost namespace scope.
    pub scope: u32,
    pub line: u32,
    /// Base-list entries: (name as written without type arguments, arity).
    pub bases: Vec<(String, u32)>,
    /// The node's `metadata` members (JSON without braces), possibly empty.
    pub meta: String,
}

/// A method or property of a type, or a function outside any type (`owner` is then empty).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CsMember {
    pub owner: String,
    pub name: String,
    pub id: String,
    pub method: bool,
    pub meta: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CsCallKind {
    /// `m()` not bound in the file.
    Bare,
    /// `new X()` not bound in the file, `X` not used as a type in it.
    New,
    /// `new X()` not bound in the file, `X` also used as a type in it.
    NewTyped,
    /// `this.m()`.
    This,
    /// `base.m()`.
    Base,
    /// `X.m()` with a capitalized identifier receiver.
    Static,
    /// `x.m()` on a lower-case receiver of known declared type.
    Member,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CsCall {
    /// Local id of the calling method, function or (for constructors) type.
    pub caller: String,
    /// Local id of the enclosing type (empty for functions).
    pub owner: String,
    pub scope: u32,
    pub kind: CsCallKind,
    pub name: String,
    /// `Static`: the receiver identifier. `Member`: unused.
    pub receiver: String,
    /// The receiver's declared type (simple name), when known.
    pub receiver_type: String,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::csharp`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CsFacts {
    pub file_id: String,
    pub scopes: Vec<CsScope>,
    pub types: Vec<CsType>,
    pub members: Vec<CsMember>,
    /// Namespaces declared in the file: (name, line of the first declaration).
    pub namespaces: Vec<(String, u32)>,
    pub calls: Vec<CsCall>,
    /// Members CommunityToolkit.Mvvm generates; only XAML view models use them (§43.4).
    #[serde(default)]
    pub generated: Vec<CsGenerated>,
}

/// A `[ObservableProperty]` field's property or a `[RelayCommand]` method's command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CsGenerated {
    /// Local id of the declaring type.
    pub owner: String,
    /// The generated member's name (`userName` → `UserName`, `SaveAsync` → `SaveCommand`).
    pub name: String,
    /// Line of the attribute list naming the generator.
    pub line: u32,
    pub command: bool,
}

/// Prefix of the virtual ids above; see the module docs.
pub const VIRTUAL: &str = "@cs:";

/// `csharp_namespace:<sha1(name)[:16]>`.
pub fn namespace_id(ns: &str) -> String {
    let digest = sha1_smol::Sha1::from(ns.as_bytes()).digest().to_string();
    format!("csharp_namespace:{}", &digest[..16])
}

/// A JSON string literal.
pub fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn is_type_decl(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration"
            | "struct_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "record_struct_declaration"
    )
}

/// Containers that only group declarations (preprocessor branches, error recovery).
fn is_transparent(kind: &str) -> bool {
    matches!(
        kind,
        "ERROR"
            | "preproc_if"
            | "preproc_elif"
            | "preproc_else"
            | "preproc_ifdef"
            | "preproc_region"
            | "declaration_list"
    )
}

/// A pending call site, bound after the walk.
struct Site {
    caller: String,
    owner: String,
    scope: u32,
    kind: SiteKind,
    line: u32,
}

enum SiteKind {
    Bare(String),
    This(String),
    Base(String),
    /// (receiver, receiver type, method).
    Static(String, String, String),
    /// (receiver type, method).
    Member(String, String),
    /// `new T()`: the type name as written, qualifier included (`A.B`).
    New(String),
    /// A call on any other receiver expression (`f().m()`, `a.b.m()`): bound by name to a
    /// method of the file, else dropped.
    Chained(String),
}

/// C# predefined type keywords (`int`, `string`…), compared lower-cased.
fn is_predefined(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "byte"
            | "char"
            | "decimal"
            | "double"
            | "float"
            | "int"
            | "long"
            | "object"
            | "sbyte"
            | "short"
            | "string"
            | "uint"
            | "ulong"
            | "ushort"
            | "void"
            | "nint"
            | "nuint"
    )
}

/// Where a declaration sits: namespace scope index, scope-id chain, namespace name.
#[derive(Clone)]
struct Place {
    scope: u32,
    chain: Vec<String>,
    ns: String,
}

struct Cs<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    facts: CsFacts,
    seen_nodes: HashSet<String>,
    /// Scope index → `s<row>:<col>` of its namespace declaration.
    scope_ids: Vec<String>,
    /// Simple name → local id of the first type declared with it in the file.
    file_types: HashMap<String, String>,
    /// (owner id, name) → method id.
    own_methods: HashMap<(String, String), String>,
    /// Name → first method (or function) of the file with it.
    file_methods: HashMap<String, String>,
    /// Local type id → name.
    type_names: HashMap<String, String>,
    /// Simple names referenced as types in the file.
    typed_names: HashSet<String>,
    /// Emitting references from a body (generic call arguments): not declaration types.
    in_body: bool,
    sites: Vec<Site>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let file_id = cx.file_id.clone();
    let mut cs = Cs {
        facts: CsFacts {
            file_id: file_id.clone(),
            scopes: vec![CsScope::default()],
            ..CsFacts::default()
        },
        seen_nodes: HashSet::from([file_id]),
        scope_ids: vec![String::new()],
        file_types: HashMap::new(),
        own_methods: HashMap::new(),
        file_methods: HashMap::new(),
        type_names: HashMap::new(),
        typed_names: HashSet::new(),
        in_body: false,
        sites: Vec::new(),
        cx,
    };
    if let Some(tree) = tree {
        let root = tree.root_node();
        cs.declare(root);
        let place = Place {
            scope: 0,
            chain: Vec::new(),
            ns: String::new(),
        };
        cs.items(root, place);
        cs.bind_calls();
    }
    let Cs { cx, facts, .. } = cs;
    cx.facts.lang = crate::facts::LangFacts::CSharp(facts);
}

/// The simple name, arity and qualifier of a type node (`a.B<T>` → (`B`, 1, `a`)).
fn type_name(cx: &Ctx, node: TsNode) -> Option<(String, u32, Option<String>)> {
    match node.kind() {
        "identifier" => Some((cx.text(node).to_string(), 0, None)),
        "generic_name" => {
            let kids = named_children(node);
            let base = kids.iter().find(|k| k.kind() == "identifier")?;
            let arity = kids
                .iter()
                .find(|k| k.kind() == "type_argument_list")
                .map_or(0, |l| named_children(*l).len() as u32);
            Some((cx.text(*base).to_string(), arity, None))
        }
        "qualified_name" => {
            let name = node.child_by_field_name("name")?;
            let (n, arity, _) = type_name(cx, name)?;
            let q = node
                .child_by_field_name("qualifier")
                .map(|q| cx.text(q).to_string());
            Some((n, arity, q))
        }
        "nullable_type" => type_name(cx, node.child_by_field_name("type")?),
        _ => None,
    }
}

/// The declared type of a variable as a receiver type: a simple name, never a predefined type.
fn receiver_type(cx: &Ctx, node: TsNode) -> Option<String> {
    match node.kind() {
        "identifier" | "generic_name" | "qualified_name" | "nullable_type" => {
            type_name(cx, node).map(|(n, _, _)| n)
        }
        _ => None,
    }
}

/// Strip type arguments: `A.B<C.D>` → `A.B`.
fn strip_generics(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            c if depth == 0 && !c.is_whitespace() => out.push(c),
            _ => {}
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl<'a> Cs<'_, 'a> {
    fn text(&self, node: TsNode) -> &'a str {
        self.cx.text(node)
    }

    fn meta(&self, place: &Place, nested: bool, partial: bool) -> String {
        let mut parts = Vec::new();
        if nested {
            parts.push("\"is_nested_type\":true".to_string());
        }
        if partial {
            parts.push("\"is_partial\":true".to_string());
        }
        if !place.ns.is_empty() {
            parts.push(format!("\"namespace\":{}", json_str(&place.ns)));
            let chain: Vec<String> = place.chain.iter().map(|s| json_str(s)).collect();
            parts.push(format!("\"scope_chain\":[{}]", chain.join(",")));
        }
        parts.join(",")
    }

    fn meta_attr(meta: &str) -> Option<(String, Attr)> {
        (!meta.is_empty()).then(|| ("metadata".to_string(), Attr::Json(format!("{{{meta}}}"))))
    }

    fn put_def(&mut self, id: &str, label: String, line: u32, extra: Vec<(String, Attr)>) {
        if self.seen_nodes.insert(id.to_string()) {
            self.cx.add_def(id.to_string(), label, line, extra);
        }
    }

    fn edge_meta(
        &mut self,
        source: &str,
        target: &str,
        rel: &str,
        line: u32,
        ctx: Option<&str>,
        meta: String,
    ) {
        let mut e = self.cx.edge(source, target, rel, Loc::Line(line), ctx);
        e.attrs.insert(0, ("metadata".into(), Attr::Json(meta)));
        self.cx.facts.edges.push(e);
    }

    // ---- pre-pass ---------------------------------------------------------------------------

    /// Names of every type and method of the file (in-file binding looks forward too).
    fn declare(&mut self, root: TsNode) {
        let mut stack = vec![(root, String::new(), None::<String>)];
        // (node, namespace, enclosing type id)
        while let Some((node, ns, owner)) = stack.pop() {
            let mut ns = ns;
            for child in named_children(node) {
                let kind = child.kind();
                match kind {
                    "file_scoped_namespace_declaration" | "namespace_declaration" => {
                        let Some(name) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let full = join(&ns, self.text(name));
                        if kind == "namespace_declaration" {
                            if let Some(body) = child.child_by_field_name("body") {
                                stack.push((body, full, None));
                            }
                        } else {
                            ns = full;
                        }
                    }
                    k if is_type_decl(k) => {
                        let Some(name) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let name = self.text(name).to_string();
                        let id = self.type_id(&ns, &name, type_arity(child));
                        self.file_types.entry(name).or_insert_with(|| id.clone());
                        if let Some(body) = child.child_by_field_name("body") {
                            stack.push((body, ns.clone(), Some(id)));
                        }
                    }
                    "method_declaration" => {
                        let Some(name) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let name = self.text(name).to_string();
                        let parent = owner.clone().unwrap_or_else(|| self.cx.file_id.clone());
                        let id = Ctx::child_id(&parent, &name);
                        if let Some(o) = &owner {
                            self.own_methods
                                .entry((o.clone(), name.clone()))
                                .or_insert_with(|| id.clone());
                        }
                        self.file_methods.entry(name).or_insert(id);
                    }
                    k if is_transparent(k) => stack.push((child, ns.clone(), owner.clone())),
                    _ => {}
                }
            }
        }
    }

    fn type_id(&self, ns: &str, name: &str, arity: u32) -> String {
        let id = Ctx::child_id(&self.cx.file_id, &join(ns, name));
        if arity > 0 {
            format!("{id}_{arity}")
        } else {
            id
        }
    }

    // ---- top level --------------------------------------------------------------------------

    fn items(&mut self, node: TsNode, place: Place) {
        let mut place = place;
        for child in named_children(node) {
            match child.kind() {
                "using_directive" => self.using(child, &place),
                "file_scoped_namespace_declaration" => {
                    if let Some(p) = self.enter_namespace(child, &place) {
                        place = p;
                    }
                }
                "namespace_declaration" => {
                    if let Some(p) = self.enter_namespace(child, &place) {
                        if let Some(body) = child.child_by_field_name("body") {
                            self.items(body, p);
                        }
                    }
                }
                k if is_type_decl(k) => {
                    let file_id = self.cx.file_id.clone();
                    self.type_decl(child, &place, &file_id, None, &mut Vec::new());
                }
                "method_declaration" => self.function(child, &place),
                k if is_transparent(k) => self.items(child, place.clone()),
                _ => {}
            }
        }
    }

    fn enter_namespace(&mut self, decl: TsNode, place: &Place) -> Option<Place> {
        let name = decl.child_by_field_name("name")?;
        let full = join(&place.ns, &strip_generics(self.text(name)));
        let pos = decl.start_position();
        let sid = format!("s{}:{}", pos.row, pos.column);
        let scope = self.facts.scopes.len() as u32;
        self.facts.scopes.push(CsScope {
            ns: full.clone(),
            parent: Some(place.scope),
            ..CsScope::default()
        });
        self.scope_ids.push(sid.clone());
        let line = Ctx::line(decl);
        if !self.facts.namespaces.iter().any(|(n, _)| *n == full) {
            self.facts.namespaces.push((full.clone(), line));
            // Shared by every declaring file: like an external it is exempt from collision
            // renaming, and the build keeps the first file's node (Graphify's node).
            self.cx.facts.nodes.push(Node {
                id: namespace_id(&full),
                label: full.clone(),
                kind: NodeKind::External,
                file_type: "code".into(),
                source_file: self.cx.rel.into(),
                loc: Loc::Line(line),
                attrs: vec![
                    (
                        "metadata".into(),
                        Attr::Json(format!(
                            "{{\"kind\":\"csharp_namespace\",\"namespace\":{}}}",
                            json_str(&full)
                        )),
                    ),
                    ("type".into(), Attr::Str("namespace".into())),
                    super::origin(),
                ],
            });
        }
        let file_id = self.cx.file_id.clone();
        self.cx
            .add_edge(&file_id, &namespace_id(&full), "contains", line, None);
        let mut chain = place.chain.clone();
        chain.push(sid);
        Some(Place {
            scope,
            chain,
            ns: full,
        })
    }

    fn using(&mut self, decl: TsNode, place: &Place) {
        let alias = decl.child_by_field_name("name");
        let Some(target) = named_children(decl)
            .into_iter()
            .find(|n| Some(n.id()) != alias.map(|a| a.id()))
        else {
            return;
        };
        let mut c = decl.walk();
        let is_static = decl.children(&mut c).any(|ch| ch.kind() == "static");
        let written = self.text(target).to_string();
        let stripped = strip_generics(&written);
        let line = Ctx::line(decl);
        let scope = place.scope as usize;
        let (kind, target_id) = if let Some(a) = alias {
            let a = self.text(a).to_string();
            self.facts.scopes[scope].aliases.push((a, stripped.clone()));
            (
                "alias",
                format!(
                    "{VIRTUAL}a:{}|{}|{}|{}",
                    self.cx.rel, place.scope, stripped, written
                ),
            )
        } else if is_static {
            let ext = slug(&written);
            if ext.is_empty() {
                return;
            }
            self.cx.add_external(ext.clone());
            ("static", ext)
        } else {
            self.facts.scopes[scope].usings.push(stripped.clone());
            ("namespace", format!("{VIRTUAL}n:{stripped}"))
        };
        let mut meta = format!("{{\"using_kind\":{}", json_str(kind));
        if let Some(a) = alias {
            meta.push_str(&format!(",\"alias\":{}", json_str(self.text(a))));
        }
        meta.push_str(&format!(
            ",\"target_fqn\":{}",
            json_str(&html_escape(&written))
        ));
        if place.scope == 0 {
            meta.push_str(",\"scope_kind\":\"file\"}");
        } else {
            meta.push_str(&format!(
                ",\"scope_kind\":\"namespace\",\"scope_id\":{}}}",
                json_str(&self.scope_ids[scope])
            ));
        }
        let file_id = self.cx.file_id.clone();
        self.edge_meta(&file_id, &target_id, "imports", line, Some("import"), meta);
    }

    // ---- types ------------------------------------------------------------------------------

    /// A type declaration and everything in it. `parent` gets the `contains` edge; `outer` is
    /// the enclosing type's lookup name for nested types.
    fn type_decl(
        &mut self,
        decl: TsNode,
        place: &Place,
        parent: &str,
        outer: Option<&str>,
        tparams: &mut Vec<String>,
    ) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let arity = type_arity(decl);
        let id = self.type_id(&place.ns, &name, arity);
        let fqn = match outer {
            Some(o) => format!("{o}.{name}"),
            None => join(&place.ns, &name),
        };
        let partial = named_children(decl)
            .into_iter()
            .any(|m| m.kind() == "modifier" && self.text(m) == "partial");
        let interface = decl.kind() == "interface_declaration";
        let line = Ctx::line(decl);
        self.type_names.insert(id.clone(), name.clone());
        let meta = self.meta(place, outer.is_some(), partial);
        let mut extra = vec![flag("_callable"), flag("_callable_class")];
        extra.extend(Self::meta_attr(&meta));
        self.put_def(&id, name.clone(), line, extra);
        self.cx.add_edge(parent, &id, "contains", line, None);

        let depth = tparams.len();
        if let Some(list) = named_children(decl)
            .into_iter()
            .find(|n| n.kind() == "type_parameter_list")
        {
            for p in named_children(list) {
                if let Some(n) = p.child_by_field_name("name") {
                    tparams.push(self.text(n).to_string());
                }
            }
        }

        // Base list: one `inherits`/`implements` per entry, then its type arguments.
        let mut bases = Vec::new();
        if let Some(list) = named_children(decl)
            .into_iter()
            .find(|n| n.kind() == "base_list")
        {
            for entry in named_children(list) {
                let ty = if entry.kind() == "primary_constructor_base_type" {
                    match entry.child_by_field_name("type") {
                        Some(t) => t,
                        None => continue,
                    }
                } else {
                    entry
                };
                let Some((bname, barity, qual)) = type_name(self.cx, ty) else {
                    continue;
                };
                let lookup = match &qual {
                    Some(q) => format!("{}.{bname}", strip_generics(q)),
                    None => bname.clone(),
                };
                bases.push((lookup.clone(), barity));
                self.typed_names.insert(bname.clone());
                let meta = ref_meta(&bname, qual.as_deref());
                let key = format!(
                    "{VIRTUAL}b:{}|{}|{}|{}",
                    self.cx.rel, place.scope, barity, lookup
                );
                if interface {
                    let target = format!(
                        "{VIRTUAL}t:{}|{}|{}|{}",
                        self.cx.rel, place.scope, barity, lookup
                    );
                    self.edge_meta(&id, &target, "inherits", line, None, meta);
                } else {
                    self.edge_meta(
                        &id,
                        &format!("{key}|i"),
                        "inherits",
                        line,
                        None,
                        meta.clone(),
                    );
                    self.edge_meta(&id, &format!("{key}|m"), "implements", line, None, meta);
                }
                self.type_args(&id, ty, line, place, tparams);
            }
        }
        // Record and primary-constructor parameters are fields.
        {
            if let Some(params) = named_children(decl)
                .into_iter()
                .find(|n| n.kind() == "parameter_list")
            {
                for p in named_children(params) {
                    if let Some(t) = p.child_by_field_name("type") {
                        self.type_refs(&id, t, "field", Ctx::line(p), place, tparams);
                    }
                }
            }
        }

        self.facts.types.push(CsType {
            id: id.clone(),
            name,
            arity,
            fqn: fqn.clone(),
            partial,
            interface,
            scope: place.scope,
            line,
            bases,
            meta,
        });

        if let Some(body) = decl.child_by_field_name("body") {
            let fields = self.field_table(body);
            self.members(body, place, &id, &fqn, &fields, tparams);
        }
        tparams.truncate(depth);
    }

    /// Field and property name → declared type (simple name; predefined types left out).
    fn field_table(&self, body: TsNode) -> HashMap<String, String> {
        let mut table = HashMap::new();
        let mut stack = vec![body];
        while let Some(node) = stack.pop() {
            for m in named_children(node) {
                match m.kind() {
                    "field_declaration" => {
                        let Some(decl) = named_children(m)
                            .into_iter()
                            .find(|n| n.kind() == "variable_declaration")
                        else {
                            continue;
                        };
                        let Some(ty) = decl
                            .child_by_field_name("type")
                            .and_then(|t| receiver_type(self.cx, t))
                        else {
                            continue;
                        };
                        for d in named_children(decl) {
                            if d.kind() == "variable_declarator" {
                                if let Some(n) = d.child_by_field_name("name") {
                                    table
                                        .entry(self.text(n).to_string())
                                        .or_insert_with(|| ty.clone());
                                }
                            }
                        }
                    }
                    "property_declaration" => {
                        let ty = m
                            .child_by_field_name("type")
                            .and_then(|t| receiver_type(self.cx, t));
                        if let (Some(ty), Some(n)) = (ty, m.child_by_field_name("name")) {
                            table.entry(self.text(n).to_string()).or_insert(ty);
                        }
                    }
                    k if is_transparent(k) => stack.push(m),
                    _ => {}
                }
            }
        }
        table
    }

    fn members(
        &mut self,
        body: TsNode,
        place: &Place,
        owner: &str,
        fqn: &str,
        fields: &HashMap<String, String>,
        tparams: &mut Vec<String>,
    ) {
        for member in named_children(body) {
            let line = Ctx::line(member);
            match member.kind() {
                "field_declaration" => {
                    self.generated_properties(member, owner);
                    if let Some(t) = named_children(member)
                        .into_iter()
                        .find(|n| n.kind() == "variable_declaration")
                        .and_then(|d| d.child_by_field_name("type"))
                    {
                        self.type_refs(owner, t, "field", line, place, tparams);
                    }
                }
                "property_declaration" => {
                    let Some(name) = member.child_by_field_name("name") else {
                        continue;
                    };
                    let name = self.text(name).to_string();
                    let id = Ctx::child_id(owner, &name);
                    let meta = self.meta(place, false, false);
                    let extra = Self::meta_attr(&meta).into_iter().collect();
                    self.put_def(&id, name.clone(), line, extra);
                    self.cx.add_edge(owner, &id, "defines", line, Some("field"));
                    self.facts.members.push(CsMember {
                        owner: owner.to_string(),
                        name,
                        id,
                        method: false,
                        meta,
                    });
                    if let Some(t) = member.child_by_field_name("type") {
                        self.type_refs(owner, t, "field", line, place, tparams);
                    }
                }
                "method_declaration" => {
                    self.generated_command(member, owner);
                    self.method(member, place, owner, fields, tparams)
                }
                "constructor_declaration" => {
                    if let Some(b) = member.child_by_field_name("body") {
                        let mut locals = HashMap::new();
                        self.params(member, None, place, tparams, &mut locals);
                        self.walk_body(b, owner, owner, place, fields, tparams, &mut locals);
                    }
                }
                "enum_member_declaration" => {
                    let Some(name) = member.child_by_field_name("name") else {
                        continue;
                    };
                    let name = self.text(name).to_string();
                    let id = Ctx::child_id(owner, &name);
                    let meta = self.meta(place, false, false);
                    let extra = Self::meta_attr(&meta).into_iter().collect();
                    self.put_def(&id, name, line, extra);
                    self.cx.add_edge(owner, &id, "case_of", line, None);
                }
                k if is_type_decl(k) => {
                    self.type_decl(member, place, owner, Some(fqn), tparams);
                }
                k if is_transparent(k) => {
                    self.members(member, place, owner, fqn, fields, tparams);
                }
                _ => {}
            }
        }
    }

    /// Line of the attribute list holding `[<name>]` (or `[<name>Attribute]`, qualified or not).
    fn generator_attribute(&self, decl: TsNode, name: &str) -> Option<u32> {
        named_children(decl)
            .into_iter()
            .filter(|n| n.kind() == "attribute_list")
            .find(|list| {
                named_children(*list).into_iter().any(|a| {
                    a.child_by_field_name("name").is_some_and(|n| {
                        let text = self.text(n);
                        let last = text.rsplit('.').next().unwrap_or(text);
                        last == name || last.strip_suffix("Attribute") == Some(name)
                    })
                })
            })
            .map(Ctx::line)
    }

    /// `[ObservableProperty] T field;` → the generated property (`_x`, `m_x`, `x` → `X`).
    fn generated_properties(&mut self, decl: TsNode, owner: &str) {
        let Some(line) = self.generator_attribute(decl, "ObservableProperty") else {
            return;
        };
        let Some(vars) = named_children(decl)
            .into_iter()
            .find(|n| n.kind() == "variable_declaration")
        else {
            return;
        };
        for d in named_children(vars) {
            let Some(n) = d
                .child_by_field_name("name")
                .filter(|_| d.kind() == "variable_declarator")
            else {
                continue;
            };
            let field = self.text(n);
            let bare = field
                .strip_prefix("m_")
                .or_else(|| field.strip_prefix('_'))
                .unwrap_or(field);
            let mut chars = bare.chars();
            let Some(first) = chars.next() else {
                continue;
            };
            self.facts.generated.push(CsGenerated {
                owner: owner.to_string(),
                name: first.to_uppercase().chain(chars).collect(),
                line,
                command: false,
            });
        }
    }

    /// `[RelayCommand] void Save()` / `SaveAsync()` → the generated `SaveCommand`.
    fn generated_command(&mut self, decl: TsNode, owner: &str) {
        let Some(line) = self.generator_attribute(decl, "RelayCommand") else {
            return;
        };
        let Some(n) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(n);
        let base = name.strip_suffix("Async").unwrap_or(name);
        self.facts.generated.push(CsGenerated {
            owner: owner.to_string(),
            name: format!("{base}Command"),
            line,
            command: true,
        });
    }

    /// Parameter types (`parameter_type` from `source` when given) and their names as typed
    /// locals.
    fn params(
        &mut self,
        decl: TsNode,
        source: Option<&str>,
        place: &Place,
        tparams: &[String],
        locals: &mut HashMap<String, String>,
    ) {
        let Some(list) = decl.child_by_field_name("parameters") else {
            return;
        };
        let line = Ctx::line(decl);
        for p in named_children(list) {
            if p.kind() != "parameter" {
                continue;
            }
            let Some(t) = p.child_by_field_name("type") else {
                continue;
            };
            if let Some(src) = source {
                self.type_refs(src, t, "parameter_type", line, place, tparams);
            }
            if let (Some(rt), Some(n)) = (receiver_type(self.cx, t), p.child_by_field_name("name"))
            {
                locals.insert(self.text(n).to_string(), rt);
            }
        }
    }

    fn method(
        &mut self,
        decl: TsNode,
        place: &Place,
        owner: &str,
        fields: &HashMap<String, String>,
        tparams: &mut Vec<String>,
    ) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(owner, &name);
        let line = Ctx::line(decl);
        let meta = self.meta(place, false, false);
        let mut extra = vec![flag("_callable")];
        extra.extend(Self::meta_attr(&meta));
        self.put_def(&id, format!(".{name}()"), line, extra);
        self.cx.add_edge(owner, &id, "method", line, None);
        self.facts.members.push(CsMember {
            owner: owner.to_string(),
            name,
            id: id.clone(),
            method: true,
            meta,
        });
        self.callable_body(decl, &id, owner, place, fields, tparams);
    }

    /// A method outside any type: `file_slug(name)`, label `name()`, contained by the file.
    fn function(&mut self, decl: TsNode, place: &Place) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let file_id = self.cx.file_id.clone();
        let id = Ctx::child_id(&file_id, &name);
        let line = Ctx::line(decl);
        let meta = self.meta(place, false, false);
        let mut extra = vec![flag("_callable")];
        extra.extend(Self::meta_attr(&meta));
        self.put_def(&id, format!("{name}()"), line, extra);
        self.cx.add_edge(&file_id, &id, "contains", line, None);
        self.facts.members.push(CsMember {
            owner: String::new(),
            name,
            id: id.clone(),
            method: true,
            meta,
        });
        self.callable_body(decl, &id, "", place, &HashMap::new(), &mut Vec::new());
    }

    /// Parameter, return and attribute references of a method, then the calls in its body.
    fn callable_body(
        &mut self,
        decl: TsNode,
        id: &str,
        owner: &str,
        place: &Place,
        fields: &HashMap<String, String>,
        tparams: &mut Vec<String>,
    ) {
        let line = Ctx::line(decl);
        let depth = tparams.len();
        if let Some(list) = named_children(decl)
            .into_iter()
            .find(|n| n.kind() == "type_parameter_list")
        {
            for p in named_children(list) {
                if let Some(n) = p.child_by_field_name("name") {
                    tparams.push(self.text(n).to_string());
                }
            }
        }
        let mut locals = HashMap::new();
        self.params(decl, Some(id), place, tparams, &mut locals);
        if let Some(t) = decl.child_by_field_name("returns") {
            self.type_refs(id, t, "return_type", line, place, tparams);
        }
        self.attributes(decl, id, line, place);
        if let Some(body) = decl.child_by_field_name("body") {
            self.walk_body(body, id, owner, place, fields, tparams, &mut locals);
        }
        tparams.truncate(depth);
    }

    fn attributes(&mut self, decl: TsNode, source: &str, line: u32, place: &Place) {
        // Attribute lists inside `#if` branches do not count.
        let lists: Vec<TsNode> = named_children(decl)
            .into_iter()
            .filter(|n| n.kind() == "attribute_list")
            .collect();
        for list in lists {
            for a in named_children(list) {
                if a.kind() != "attribute" {
                    continue;
                }
                let Some(name) = a.child_by_field_name("name") else {
                    continue;
                };
                let Some((n, arity, qual)) = type_name(self.cx, name) else {
                    continue;
                };
                self.reference(source, &n, arity, qual.as_deref(), "attribute", line, place);
            }
        }
    }

    /// `references` from `source` to every type a type node names.
    fn type_refs(
        &mut self,
        source: &str,
        node: TsNode,
        context: &str,
        line: u32,
        place: &Place,
        tparams: &[String],
    ) {
        match node.kind() {
            "identifier" => {
                let name = self.text(node).to_string();
                if !tparams.contains(&name) {
                    self.reference(source, &name, 0, None, context, line, place);
                }
            }
            "generic_name" | "qualified_name" => {
                if let Some((n, arity, qual)) = type_name(self.cx, node) {
                    if qual.is_some() || !tparams.contains(&n) {
                        self.reference(source, &n, arity, qual.as_deref(), context, line, place);
                    }
                }
                self.type_args(source, node, line, place, tparams);
            }
            "nullable_type" | "array_type" | "pointer_type" | "ref_type" | "scoped_type" => {
                if let Some(t) = node.child_by_field_name("type") {
                    self.type_refs(source, t, context, line, place, tparams);
                }
            }
            "tuple_type" => {
                for el in named_children(node) {
                    if let Some(t) = el.child_by_field_name("type") {
                        self.type_refs(source, t, context, line, place, tparams);
                    }
                }
            }
            _ => {}
        }
    }

    /// `generic_arg` references for the type arguments of a (possibly qualified) generic name.
    fn type_args(
        &mut self,
        source: &str,
        node: TsNode,
        line: u32,
        place: &Place,
        tparams: &[String],
    ) {
        let generic = match node.kind() {
            "generic_name" => Some(node),
            "qualified_name" => node
                .child_by_field_name("name")
                .filter(|n| n.kind() == "generic_name"),
            "nullable_type" => {
                if let Some(t) = node.child_by_field_name("type") {
                    self.type_args(source, t, line, place, tparams);
                }
                None
            }
            _ => None,
        };
        let Some(g) = generic else {
            return;
        };
        if let Some(args) = named_children(g)
            .into_iter()
            .find(|k| k.kind() == "type_argument_list")
        {
            for a in named_children(args) {
                self.type_refs(source, a, "generic_arg", line, place, tparams);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reference(
        &mut self,
        source: &str,
        name: &str,
        arity: u32,
        qualifier: Option<&str>,
        context: &str,
        line: u32,
        place: &Place,
    ) {
        // A type's field of its own type (`static readonly T Instance`) is not a reference.
        if context == "field"
            && qualifier.is_none()
            && self.type_names.get(source).is_some_and(|n| n == name)
        {
            return;
        }
        if !self.in_body {
            self.typed_names.insert(name.to_string());
        }
        let lookup = match qualifier {
            Some(q) => format!("{}.{name}", strip_generics(q)),
            None => name.to_string(),
        };
        let target = format!(
            "{VIRTUAL}t:{}|{}|{}|{}",
            self.cx.rel, place.scope, arity, lookup
        );
        let meta = ref_meta(name, qualifier);
        self.edge_meta(source, &target, "references", line, Some(context), meta);
    }

    // ---- bodies and calls ---------------------------------------------------------------------

    /// Calls in a body, lambdas and local functions included.
    #[allow(clippy::too_many_arguments)]
    fn walk_body(
        &mut self,
        body: TsNode,
        caller: &str,
        owner: &str,
        place: &Place,
        fields: &HashMap<String, String>,
        tparams: &[String],
        locals: &mut HashMap<String, String>,
    ) {
        let mut stack = vec![body];
        while let Some(node) = stack.pop() {
            let line = Ctx::line(node);
            let site = |kind: SiteKind, sites: &mut Vec<Site>| {
                sites.push(Site {
                    caller: caller.to_string(),
                    owner: owner.to_string(),
                    scope: place.scope,
                    kind,
                    line,
                });
            };
            match node.kind() {
                "invocation_expression" => {
                    if let Some(f) = node.child_by_field_name("function") {
                        if let Some(kind) = self.call_kind(f, fields, locals) {
                            site(kind, &mut self.sites);
                        }
                        // Type arguments of a generic method call are references.
                        let generic = match f.kind() {
                            "generic_name" => Some(f),
                            "member_access_expression" => f
                                .child_by_field_name("name")
                                .filter(|n| n.kind() == "generic_name"),
                            _ => None,
                        };
                        if let Some(g) = generic {
                            self.in_body = true;
                            self.type_args(caller, g, line, place, tparams);
                            self.in_body = false;
                        }
                    }
                }
                "object_creation_expression" => {
                    if let Some((name, _, qual)) = node
                        .child_by_field_name("type")
                        .and_then(|t| type_name(self.cx, t))
                    {
                        let name = match qual {
                            Some(q) => format!("{}.{name}", strip_generics(&q)),
                            None => name,
                        };
                        site(SiteKind::New(name), &mut self.sites);
                    }
                }
                "local_declaration_statement" | "using_statement" => {
                    if let Some(decl) = named_children(node)
                        .into_iter()
                        .find(|n| n.kind() == "variable_declaration")
                    {
                        self.type_locals(decl, locals);
                    }
                }
                "declaration_pattern" => {
                    let ty = node
                        .child_by_field_name("type")
                        .and_then(|t| receiver_type(self.cx, t));
                    if let (Some(ty), Some(n)) = (ty, node.child_by_field_name("name")) {
                        locals.insert(self.text(n).to_string(), ty);
                    }
                }
                k if is_type_decl(k) => continue,
                _ => {}
            }
            let start = stack.len();
            let mut c = node.walk();
            stack.extend(node.named_children(&mut c));
            stack[start..].reverse();
        }
    }

    /// `T x = …;` types every declarator; `var x = new T(…);` types `x`.
    fn type_locals(&self, decl: TsNode, locals: &mut HashMap<String, String>) {
        let Some(t) = decl.child_by_field_name("type") else {
            return;
        };
        let explicit = receiver_type(self.cx, t);
        for d in named_children(decl) {
            if d.kind() != "variable_declarator" {
                continue;
            }
            let Some(n) = d.child_by_field_name("name") else {
                continue;
            };
            let ty = match &explicit {
                Some(ty) => Some(ty.clone()),
                None if t.kind() == "implicit_type" => named_children(d)
                    .into_iter()
                    .find(|k| k.kind() == "object_creation_expression")
                    .and_then(|o| o.child_by_field_name("type"))
                    .and_then(|o| receiver_type(self.cx, o)),
                None => None,
            };
            if let Some(ty) = ty {
                locals.insert(self.text(n).to_string(), ty);
            }
        }
    }

    fn call_kind(
        &self,
        f: TsNode,
        fields: &HashMap<String, String>,
        locals: &HashMap<String, String>,
    ) -> Option<SiteKind> {
        let ident = |n: TsNode| -> Option<String> {
            match n.kind() {
                "identifier" => Some(self.text(n).to_string()),
                "generic_name" => named_children(n)
                    .into_iter()
                    .find(|k| k.kind() == "identifier")
                    .map(|k| self.text(k).to_string()),
                _ => None,
            }
        };
        let (receiver, name) = match f.kind() {
            "identifier" | "generic_name" => {
                // `Int()`, `String()`: names of predefined types are never calls.
                return ident(f)
                    .filter(|n| !is_predefined(&n.to_ascii_lowercase()))
                    .map(SiteKind::Bare);
            }
            "member_access_expression" => {
                let name = ident(f.child_by_field_name("name")?)?;
                (f.child_by_field_name("expression")?, name)
            }
            "conditional_access_expression" => {
                let binding = named_children(f)
                    .into_iter()
                    .find(|k| k.kind() == "member_binding_expression")?;
                let name = ident(binding.child_by_field_name("name")?)?;
                (f.child_by_field_name("condition")?, name)
            }
            _ => return None,
        };
        let var = match receiver.kind() {
            "this" => return Some(SiteKind::This(name)),
            "base" => return Some(SiteKind::Base(name)),
            "identifier" => self.text(receiver).to_string(),
            // `this.x.m()` is `x.m()`.
            "member_access_expression" => {
                let inner = receiver.child_by_field_name("expression");
                let n = receiver.child_by_field_name("name");
                match (inner, n) {
                    (Some(i), Some(n)) if i.kind() == "this" && n.kind() == "identifier" => {
                        self.text(n).to_string()
                    }
                    _ => return Some(SiteKind::Chained(name)),
                }
            }
            "parenthesized_expression" => {
                // `(x as T).m()` and `((T)x).m()` call `T.m()`.
                let cast = named_children(receiver)
                    .into_iter()
                    .find_map(|k| match k.kind() {
                        "as_expression" => k.child_by_field_name("right"),
                        "cast_expression" => k.child_by_field_name("type"),
                        _ => None,
                    })
                    .and_then(|t| receiver_type(self.cx, t));
                return Some(match cast {
                    Some(t) => SiteKind::Static(t, String::new(), name),
                    None => SiteKind::Chained(name),
                });
            }
            // Any other receiver expression binds by name within the file only.
            _ => return Some(SiteKind::Chained(name)),
        };
        let ty = locals.get(&var).or_else(|| fields.get(&var)).cloned();
        if var.starts_with(char::is_uppercase) {
            Some(SiteKind::Static(var, ty.unwrap_or_default(), name))
        } else {
            ty.map(|t| SiteKind::Member(t, name))
        }
    }

    /// Bind bare calls and `new` to the file's own methods and types; record the rest for the
    /// cross-file pass. In-file edges: one per (caller, callee), first site kept.
    fn bind_calls(&mut self) {
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut local: Vec<(String, String, u32)> = Vec::new();
        let sites = std::mem::take(&mut self.sites);
        for s in sites {
            let (kind, name, receiver, receiver_type) = match s.kind {
                SiteKind::Bare(name) => {
                    let hit = self
                        .own_methods
                        .get(&(s.owner.clone(), name.clone()))
                        .or_else(|| self.file_methods.get(&name))
                        .cloned();
                    if let Some(id) = hit {
                        if seen.insert((s.caller.clone(), id.clone())) {
                            local.push((s.caller, id, s.line));
                        }
                        continue;
                    }
                    (CsCallKind::Bare, name, String::new(), String::new())
                }
                SiteKind::Chained(name) => {
                    let hit = self
                        .own_methods
                        .get(&(s.owner.clone(), name.clone()))
                        .or_else(|| self.file_methods.get(&name))
                        .cloned();
                    if let Some(id) = hit {
                        if seen.insert((s.caller.clone(), id.clone())) {
                            local.push((s.caller, id, s.line));
                        }
                    }
                    continue;
                }
                SiteKind::New(name) => {
                    let simple = name.rsplit('.').next().unwrap_or(&name);
                    if let Some(id) = self.file_types.get(simple).cloned() {
                        if seen.insert((s.caller.clone(), id.clone())) {
                            local.push((s.caller, id, s.line));
                        }
                        continue;
                    }
                    let kind = if self.typed_names.contains(simple) {
                        CsCallKind::NewTyped
                    } else {
                        CsCallKind::New
                    };
                    (kind, name, String::new(), String::new())
                }
                SiteKind::This(name) => (CsCallKind::This, name, String::new(), String::new()),
                SiteKind::Base(name) => (CsCallKind::Base, name, String::new(), String::new()),
                SiteKind::Static(recv, ty, name) => (CsCallKind::Static, name, recv, ty),
                SiteKind::Member(ty, name) => (CsCallKind::Member, name, String::new(), ty),
            };
            self.facts.calls.push(CsCall {
                caller: s.caller,
                owner: s.owner,
                scope: s.scope,
                kind,
                name,
                receiver,
                receiver_type,
                line: s.line,
            });
        }
        for (caller, callee, line) in local {
            self.cx
                .add_edge(&caller, &callee, "calls", line, Some("call"));
        }
    }
}

fn type_arity(decl: TsNode) -> u32 {
    named_children(decl)
        .into_iter()
        .find(|n| n.kind() == "type_parameter_list")
        .map_or(0, |l| {
            named_children(l)
                .into_iter()
                .filter(|p| p.kind() == "type_parameter")
                .count() as u32
        })
}

fn join(ns: &str, name: &str) -> String {
    if ns.is_empty() {
        name.to_string()
    } else {
        format!("{ns}.{name}")
    }
}

/// `{"ref_token": Name}` plus `qualified`/`ref_qualifier` for a qualified name.
fn ref_meta(name: &str, qualifier: Option<&str>) -> String {
    match qualifier {
        Some(q) => format!(
            "{{\"ref_token\":{},\"qualified\":true,\"ref_qualifier\":{}}}",
            json_str(name),
            json_str(q)
        ),
        None => format!("{{\"ref_token\":{}}}", json_str(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_file;
    use crate::facts::LangFacts;
    use crate::lang::Lang;

    fn edges(f: &crate::facts::FileFacts) -> Vec<String> {
        f.edges
            .iter()
            .map(|e| {
                format!(
                    "{} {} {} {}",
                    e.source,
                    e.relation,
                    e.target,
                    e.context.as_deref().unwrap_or("-")
                )
            })
            .collect()
    }

    #[test]
    fn ids_namespaces_and_metadata() {
        let f = extract_file(
            Lang::CSharp,
            "a/A.cs",
            b"using System;\nnamespace Outer\n{\n    namespace Inner { class C<T> { class N { } } }\n    class D { int P { get; } void M() { } }\n}\nclass G { }\n",
        );
        let ids: Vec<_> = f.nodes.iter().map(|n| n.id.clone()).collect();
        assert_eq!(
            ids,
            [
                "a_a".to_string(),
                namespace_id("Outer"),
                namespace_id("Outer.Inner"),
                "a_a_outer_inner_c_1".into(),
                "a_a_outer_inner_n".into(),
                "a_a_outer_d".into(),
                "a_a_outer_d_p".into(),
                "a_a_outer_d_m".into(),
                "a_a_g".into(),
            ]
        );
        let meta = |id: &str| {
            f.nodes
                .iter()
                .find(|n| n.id == id)
                .and_then(|n| n.attr("metadata").cloned())
        };
        assert_eq!(
            meta("a_a_outer_inner_n"),
            Some(Attr::Json(
                "{\"is_nested_type\":true,\"namespace\":\"Outer.Inner\",\"scope_chain\":[\"s1:0\",\"s3:4\"]}".into()
            ))
        );
        assert_eq!(meta("a_a_g"), None);
        let e = edges(&f);
        assert!(e.contains(&"a_a imports @cs:n:System import".to_string()));
        assert!(e.contains(&"a_a_outer_inner_c_1 contains a_a_outer_inner_n -".to_string()));
        assert!(e.contains(&"a_a_outer_d defines a_a_outer_d_p field".to_string()));
    }

    #[test]
    fn calls_bind_in_file_or_are_recorded() {
        let f = extract_file(
            Lang::CSharp,
            "b/B.cs",
            b"namespace N;\nclass B\n{\n    Repo _repo;\n    Prop Thing { get; }\n    B() { Init(); }\n    void Init() { }\n    void Run(Svc s)\n    {\n        var r = new Repo();\n        r.Save();\n        _repo.Load();\n        s.Go();\n        Thing.Do();\n        this.Init();\n        base.Base();\n        Make().Init();\n        Console.WriteLine();\n        (s as IDo)?.Act();\n        Int();\n        new Other();\n    }\n    B Make() => null;\n    int Int() => 1;\n}\n",
        );
        let e = edges(&f);
        assert!(e.contains(&"b_b_n_b calls b_b_n_b_init call".to_string()));
        assert!(e.contains(&"b_b_n_b_run calls b_b_n_b_init call".to_string()));
        assert!(!e.iter().any(|x| x.contains("calls b_b_n_b_int ")));
        let LangFacts::CSharp(cs) = &f.lang else {
            panic!()
        };
        let calls: Vec<_> = cs
            .calls
            .iter()
            .map(|c| {
                (
                    c.kind,
                    c.receiver.as_str(),
                    c.receiver_type.as_str(),
                    c.name.as_str(),
                )
            })
            .collect();
        assert_eq!(
            calls,
            [
                (CsCallKind::NewTyped, "", "", "Repo"),
                (CsCallKind::Member, "", "Repo", "Save"),
                (CsCallKind::Member, "", "Repo", "Load"),
                (CsCallKind::Member, "", "Svc", "Go"),
                (CsCallKind::Static, "Thing", "Prop", "Do"),
                (CsCallKind::This, "", "", "Init"),
                (CsCallKind::Base, "", "", "Base"),
                (CsCallKind::Static, "Console", "", "WriteLine"),
                (CsCallKind::Static, "IDo", "", "Act"),
                (CsCallKind::New, "", "", "Other"),
            ]
        );
    }
}

//! Kotlin (§21) extraction, `.kt` and `.kts`. See `docs/graphify-format.md`.
//!
//! One file yields its file node, every named class, interface, object and enum (`file_Name`,
//! flat for nested types), functions (`file_name`) and methods (`owner_name`; companion object
//! members belong to the enclosing class), enum entries and their methods, anonymous objects
//! inside functions (`method_object_Type_l<line>`), per-file type stubs (`slug(Name)`), and the
//! in-file `contains`, `method`, `case_of`, `inherits`, `implements`, `references` and `calls`
//! edges. Properties, type aliases, constructors, `init` blocks, accessors, local functions and
//! script statements yield no nodes; KDoc and comments yield no rationale.
//!
//! - Supertypes: a constructor invocation (`Base()`) `inherits`, any other specifier (`Iface`,
//!   `Iface by impl`) `implements` (last name segment, never denied); their type arguments are
//!   `generic_arg` references, except for anonymous objects.
//! - References: `field` from `val`/`var` constructor parameters (at the parameter's line) and
//!   member properties, `parameter_type`/`return_type` from functions (at the function's line),
//!   `generic_arg` for every type argument, `attribute` for every annotation (class annotations
//!   also for their `X::class` arguments). A qualified type is referenced by its first segment;
//!   common Kotlin/JDK types never produce a reference ([`is_denied_type`]); generic type
//!   parameters do (`T` becomes a stub). A type binds locally only to a class declared earlier.
//! - Calls bind in the file by name to the file's last function, method, class or anonymous
//!   object of that name, whatever the receiver; names in [`is_denied_call`] never bind.
//!   Function bodies come first, then property initializers (caller: the class, or the file at
//!   top level). Calls in local functions, accessors, `init` blocks, constructors and default
//!   arguments are not seen.
//! - Declarations nested in other nodes (parse errors, `init` blocks, secondary constructors)
//!   are taken as top-level declarations of the file, as Graphify's walk does.
//!
//! Import edges and unbound calls need other files: imports target the private virtual id
//! `@ki:<path>|<fqn>` that `crate::resolve::kotlin` redirects, and calls are recorded in
//! [`KotlinFacts::calls`].

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, slug, Ctx};
use crate::facts::{Attr, Edge};

/// A named class, interface, object or enum (anonymous objects excluded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KClass {
    pub name: String,
    /// Local id.
    pub id: String,
    /// Package + enclosing types + name.
    pub fqn: String,
}

/// A method of a named class (companion members included).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KMethod {
    /// Local id of the owning class.
    pub owner: String,
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KCallKind {
    /// `name(...)`; `via_stub` when the file references a type named `name`.
    Bare { via_stub: bool },
    /// `Receiver.name(...)` with a capitalised identifier receiver.
    Object(String),
}

/// A call the file could not bind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KCall {
    /// Local id of the caller (function, method, class or file).
    pub caller: String,
    pub name: String,
    pub kind: KCallKind,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::kotlin`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KotlinFacts {
    pub package: String,
    pub file_id: String,
    pub classes: Vec<KClass>,
    /// Top-level functions: (name, local id).
    pub functions: Vec<(String, String)>,
    pub methods: Vec<KMethod>,
    /// Non-wildcard imports: (last segment, FQN).
    pub imports: Vec<(String, String)>,
    /// Per-file type stubs: (local id, label).
    pub stubs: Vec<(String, String)>,
    pub calls: Vec<KCall>,
}

/// Prefix of the virtual import targets; see the module docs.
pub const VIRTUAL: &str = "@ki:";

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn has_token(node: TsNode, token: &str) -> bool {
    let mut c = node.walk();
    let found = node.children(&mut c).any(|ch| ch.kind() == token);
    found
}

fn is_class_decl(kind: &str) -> bool {
    matches!(kind, "class_declaration" | "object_declaration")
}

fn is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "user_type"
            | "nullable_type"
            | "function_type"
            | "parenthesized_type"
            | "non_nullable_type"
            | "definitely_non_nullable_type"
    )
}

/// Type names that never produce a reference or stub (Kotlin builtins and common JDK types).
pub fn is_denied_type(name: &str) -> bool {
    DENIED_TYPES.binary_search(&name).is_ok()
}

/// Sorted.
const DENIED_TYPES: &[&str] = &[
    "Any",
    "Array",
    "ArrayDeque",
    "ArrayList",
    "AtomicBoolean",
    "AtomicInteger",
    "AtomicLong",
    "AtomicReference",
    "BiFunction",
    "BigDecimal",
    "BigInteger",
    "BlockingQueue",
    "Boolean",
    "Byte",
    "Char",
    "CharSequence",
    "Character",
    "Class",
    "Clock",
    "Closeable",
    "Collection",
    "Collector",
    "Comparator",
    "CompletableFuture",
    "ConcurrentHashMap",
    "CountDownLatch",
    "Date",
    "Deque",
    "Double",
    "Duration",
    "Exception",
    "Executor",
    "ExecutorService",
    "File",
    "Float",
    "Function",
    "Future",
    "HashMap",
    "HashSet",
    "IOException",
    "IllegalArgumentException",
    "IllegalStateException",
    "InputStream",
    "Instant",
    "Int",
    "Integer",
    "Iterable",
    "Iterator",
    "LinkedHashMap",
    "LinkedHashSet",
    "LinkedList",
    "List",
    "LocalTime",
    "Locale",
    "Long",
    "Map",
    "MutableCollection",
    "MutableIterable",
    "MutableIterator",
    "MutableList",
    "MutableMap",
    "MutableSet",
    "NavigableMap",
    "Nothing",
    "Number",
    "Object",
    "Optional",
    "OutputStream",
    "Pair",
    "Path",
    "PriorityQueue",
    "Process",
    "Queue",
    "Random",
    "Reader",
    "Regex",
    "Runnable",
    "RuntimeException",
    "ScheduledExecutorService",
    "Sequence",
    "Serializable",
    "Set",
    "Short",
    "SortedSet",
    "Stream",
    "String",
    "StringBuilder",
    "Supplier",
    "Thread",
    "ThreadFactory",
    "Throwable",
    "TimeUnit",
    "Triple",
    "Unit",
    "Void",
    "ZoneId",
];

/// Call names that never bind, in the file or across files (case-sensitive): Python builtins,
/// a few JavaScript globals, and type-like constructor names.
pub fn is_denied_call(name: &str) -> bool {
    DENIED_CALLS.binary_search(&name).is_ok()
}

/// Sorted. Probed one by one (`docs/graphify-format.md` §21).
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
    "Reflect",
    "RegExp",
    "Request",
    "Response",
    "Set",
    "String",
    "Symbol",
    "SyntaxError",
    "TypeError",
    "URL",
    "URLSearchParams",
    "UUID",
    "Uuid",
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

struct Kt<'c, 'a, 't> {
    cx: &'c mut Ctx<'a>,
    /// `slug(path with extension)`: a definition with this id merges into the file node.
    merged: String,
    /// Simple name → local id of the first class declared with it.
    local_types: HashMap<String, String>,
    /// Call name → local id of the file's last definition with that name.
    callables: HashMap<String, String>,
    seen_nodes: HashSet<String>,
    /// Stub label → id.
    stubs: HashMap<String, String>,
    refs_seen: HashSet<(String, String, &'static str, u32)>,
    /// (caller, body) of functions, in node order.
    bodies: Vec<(String, TsNode<'t>)>,
    /// (caller, expression) of property initializers, in source order.
    deferred: Vec<(String, TsNode<'t>)>,
    facts: KotlinFacts,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let file_id = cx.file_id.clone();
    let merged = slug(cx.rel);
    let mut k = Kt {
        merged,
        local_types: HashMap::new(),
        callables: HashMap::new(),
        seen_nodes: HashSet::from([file_id.clone()]),
        stubs: HashMap::new(),
        refs_seen: HashSet::new(),
        bodies: Vec::new(),
        deferred: Vec::new(),
        facts: KotlinFacts {
            file_id,
            ..KotlinFacts::default()
        },
        cx,
    };
    if let Some(tree) = tree {
        k.file(tree.root_node());
    }
    let Kt { cx, facts, .. } = k;
    cx.facts.lang = crate::facts::LangFacts::Kotlin(facts);
}

impl<'c, 'a, 't> Kt<'c, 'a, 't> {
    fn text(&self, node: TsNode) -> &'a str {
        self.cx.text(node)
    }

    /// The id a definition is emitted under: its own, or the file's when it collides with the
    /// file's internal id (`fun kt()` in `a.kt`, §21.1).
    fn nid(&self, id: &str) -> String {
        if id == self.merged {
            self.cx.file_id.clone()
        } else {
            id.to_string()
        }
    }

    fn file(&mut self, root: TsNode<'t>) {
        let file_id = self.cx.file_id.clone();
        let mut decls = Vec::new();
        for (child, _) in members(root) {
            match child.kind() {
                "package_header" => {
                    if let Some(q) = named_children(child)
                        .into_iter()
                        .find(|n| matches!(n.kind(), "qualified_identifier" | "identifier"))
                    {
                        self.facts.package = self.text(q).to_string();
                    }
                }
                "import" => self.import(child, &file_id),
                _ => decls.push(child),
            }
        }
        let package = self.facts.package.clone();
        for d in &decls {
            if is_class_decl(d.kind()) {
                self.declare(*d, &file_id, &package);
            }
        }
        for d in decls {
            match d.kind() {
                k if is_class_decl(k) => self.class(d, &file_id, &file_id),
                "function_declaration" => self.function(d, &file_id, true),
                "property_declaration" => self.property(d, &file_id, &file_id, false),
                _ => {}
            }
        }
        self.calls();
    }

    // ---- imports ------------------------------------------------------------------------------

    fn import(&mut self, imp: TsNode, file_id: &str) {
        let Some(path) = named_children(imp)
            .into_iter()
            .find(|n| matches!(n.kind(), "qualified_identifier" | "identifier"))
        else {
            return;
        };
        let wildcard = has_token(imp, "*") || has_token(imp, ".*");
        if wildcard {
            return;
        }
        let fqn = self.text(path).to_string();
        let alias = named_children(imp)
            .into_iter()
            .rev()
            .find(|n| n.kind() == "identifier" && n.id() != path.id())
            .map(|n| self.text(n).to_string());
        let last = fqn.rsplit('.').next().unwrap_or(&fqn).to_string();
        self.facts.imports.push((last, fqn.clone()));
        let mut meta = format!("{{\"target_fqn\":{}", json_str(&fqn));
        if let Some(a) = &alias {
            meta.push_str(&format!(",\"alias\":{}", json_str(a)));
        }
        meta.push('}');
        let target = format!("{VIRTUAL}{}|{fqn}", self.cx.rel);
        let line = Ctx::line(imp);
        let mut e = self.cx.edge(
            file_id,
            &target,
            "imports",
            crate::facts::Loc::Line(line),
            Some("import"),
        );
        e.attrs.push(("metadata".into(), Attr::Json(meta)));
        self.cx.facts.edges.push(e);
    }

    // ---- declarations -------------------------------------------------------------------------

    /// Pre-pass: local ids and FQNs of every named class, object and enum of the file.
    fn declare(&mut self, decl: TsNode, file_id: &str, outer_fqn: &str) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(file_id, &name);
        let fqn = if outer_fqn.is_empty() {
            name.clone()
        } else {
            format!("{outer_fqn}.{name}")
        };
        self.facts.classes.push(KClass {
            name,
            id: self.nid(&id),
            fqn: fqn.clone(),
        });
        for body in named_children(decl)
            .into_iter()
            .filter(|n| matches!(n.kind(), "class_body" | "enum_class_body"))
        {
            for (m, _) in members(body) {
                if is_class_decl(m.kind()) {
                    self.declare(m, file_id, &fqn);
                } else if m.kind() == "companion_object" {
                    for cb in named_children(m)
                        .into_iter()
                        .filter(|n| n.kind() == "class_body")
                    {
                        for cm in named_children(cb) {
                            if is_class_decl(cm.kind()) {
                                self.declare(cm, file_id, &fqn);
                            }
                        }
                    }
                }
            }
        }
    }

    fn put_def(&mut self, id: &str, label: String, line: u32, class: bool) {
        if id == self.merged {
            // Merged into the file node: it gains the definition's flags.
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
        if self.seen_nodes.insert(id.to_string()) {
            let mut extra = vec![flag("_callable")];
            if class {
                extra.push(flag("_callable_class"));
            }
            self.cx.add_def(id.to_string(), label, line, extra);
        }
    }

    fn edge(&mut self, source: &str, target: &str, relation: &str, line: u32, ctx: Option<&str>) {
        let (s, t) = (self.nid(source), self.nid(target));
        self.cx.add_edge(&s, &t, relation, line, ctx);
    }

    /// A class, interface, object or enum and everything in it. `parent` gets `contains`.
    fn class(&mut self, decl: TsNode<'t>, file_id: &str, parent: &str) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(file_id, &name);
        let line = Ctx::line(decl);
        self.put_def(&id, name.clone(), line, true);
        self.edge(parent, &id, "contains", line, None);
        let nid = self.nid(&id);
        self.local_types.entry(name.clone()).or_insert(nid);
        if !self.merged_id(&id) {
            self.callables.insert(name, id.clone());
        }
        self.header(decl, &id, line);
        for body in named_children(decl)
            .into_iter()
            .filter(|n| matches!(n.kind(), "class_body" | "enum_class_body"))
        {
            self.body(body, file_id, &id);
        }
    }

    fn merged_id(&self, id: &str) -> bool {
        id == self.merged
    }

    /// Supertypes, annotations and `val`/`var` constructor parameters of a class or object.
    fn header(&mut self, decl: TsNode, id: &str, line: u32) {
        for spec in named_children(decl)
            .into_iter()
            .filter(|n| n.kind() == "delegation_specifiers")
            .flat_map(named_children)
        {
            self.supertype(id, spec, line, true);
        }
        self.annotations(decl, id, line, true);
        for params in named_children(decl)
            .into_iter()
            .filter(|n| n.kind() == "primary_constructor")
            .flat_map(named_children)
            .filter(|n| n.kind() == "class_parameters")
        {
            for p in named_children(params) {
                if p.kind() != "class_parameter" || !(has_token(p, "val") || has_token(p, "var")) {
                    continue;
                }
                let pl = Ctx::line(p);
                if let Some(t) = named_children(p)
                    .into_iter()
                    .find(|n| is_type_node(n.kind()))
                {
                    self.type_refs(id, t, "field", pl);
                }
                self.annotations(p, id, pl, false);
            }
        }
    }

    fn supertype(&mut self, id: &str, spec: TsNode, line: u32, args: bool) {
        let inner = named_children(spec).into_iter().next();
        let (ty, relation) = match inner.map(|n| (n, n.kind())) {
            Some((n, "constructor_invocation")) => (
                named_children(n)
                    .into_iter()
                    .find(|c| is_type_node(c.kind())),
                "inherits",
            ),
            Some((n, "explicit_delegation")) => (
                named_children(n)
                    .into_iter()
                    .find(|c| is_type_node(c.kind())),
                "implements",
            ),
            Some((n, k)) if is_type_node(k) => (Some(n), "implements"),
            _ => (None, "implements"),
        };
        let Some(ty) = ty.map(strip_nullable) else {
            return;
        };
        if ty.kind() != "user_type" {
            return;
        }
        let Some(name) = self.last_identifier(ty) else {
            return;
        };
        let target = self.target(&name);
        self.edge(id, &target, relation, line, None);
        if args {
            self.type_args(id, ty, line);
        }
    }

    fn annotations(&mut self, decl: TsNode, id: &str, line: u32, literals: bool) {
        let Some(mods) = named_children(decl)
            .into_iter()
            .find(|n| n.kind() == "modifiers")
        else {
            return;
        };
        for a in named_children(mods) {
            if a.kind() != "annotation" {
                continue;
            }
            let ty = named_children(a).into_iter().find_map(|n| match n.kind() {
                "user_type" => Some(n),
                "constructor_invocation" => named_children(n)
                    .into_iter()
                    .find(|c| c.kind() == "user_type"),
                _ => None,
            });
            if let Some(name) = ty.and_then(|t| self.last_identifier(t)) {
                let target = self.target(&name);
                self.reference(id, &target, "attribute", line);
            }
            // Class annotations also reference their `X::class` arguments (`@RunWith(X::class)`);
            // function and property annotations do not (`@Throws(E::class)`).
            let mut found = Vec::new();
            if literals {
                class_literals(a, self.cx.src, &mut found);
            }
            for name in found {
                if !is_denied_type(&name) {
                    let target = self.target(&name);
                    self.reference(id, &target, "attribute", line);
                }
            }
        }
    }

    fn body(&mut self, body: TsNode<'t>, file_id: &str, owner: &str) {
        for (m, nested) in members(body) {
            if nested {
                // Found inside `init`, a secondary constructor or a parse error: Graphify takes
                // it as a top-level declaration of the file.
                match m.kind() {
                    k if is_class_decl(k) => self.class(m, file_id, file_id),
                    "function_declaration" => self.function(m, file_id, true),
                    "property_declaration" => self.property(m, file_id, file_id, false),
                    _ => {}
                }
                continue;
            }
            match m.kind() {
                "property_declaration" => self.property(m, file_id, owner, true),
                "function_declaration" => self.function(m, owner, false),
                k if is_class_decl(k) => self.class(m, file_id, owner),
                "companion_object" => {
                    for cb in named_children(m)
                        .into_iter()
                        .filter(|n| n.kind() == "class_body")
                    {
                        self.body(cb, file_id, owner);
                    }
                }
                "enum_entry" => self.enum_entry(m, owner),
                _ => {}
            }
        }
    }

    fn enum_entry(&mut self, entry: TsNode<'t>, owner: &str) {
        let Some(name) = named_children(entry)
            .into_iter()
            .find(|n| n.kind() == "identifier")
        else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(owner, &name);
        let line = Ctx::line(entry);
        if self.seen_nodes.insert(id.clone()) {
            self.cx.add_def(id.clone(), name, line, vec![]);
        }
        self.edge(owner, &id, "case_of", line, None);
        for cb in named_children(entry)
            .into_iter()
            .filter(|n| n.kind() == "class_body")
        {
            for m in named_children(cb) {
                if m.kind() == "function_declaration" {
                    self.function(m, &id, false);
                }
            }
        }
    }

    /// A property: its type and annotations are references of `owner` (class members only),
    /// and its initializer is walked for calls after every function body.
    fn property(&mut self, decl: TsNode<'t>, _file_id: &str, owner: &str, member: bool) {
        let line = Ctx::line(decl);
        if member {
            if let Some(t) = named_children(decl)
                .into_iter()
                .find(|n| n.kind() == "variable_declaration")
                .and_then(|v| {
                    named_children(v)
                        .into_iter()
                        .find(|n| is_type_node(n.kind()))
                })
            {
                self.type_refs(owner, t, "field", line);
            }
            self.annotations(decl, owner, line, false);
        }
        let mut seen_var = false;
        for ch in named_children(decl) {
            match ch.kind() {
                "variable_declaration" | "multi_variable_declaration" => seen_var = true,
                "getter" | "setter" | "modifiers" | "type_constraints" | "type_parameters" => {}
                _ if seen_var => self.deferred.push((owner.to_string(), ch)),
                _ => {}
            }
        }
    }

    /// A function (top-level) or method, its references, and its body's anonymous objects.
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
        if top_level {
            self.edge(owner, &id, "contains", line, None);
            if !self.merged_id(&id) {
                self.facts.functions.push((name.clone(), self.nid(&id)));
            }
        } else {
            self.edge(owner, &id, "method", line, None);
            self.facts.methods.push(KMethod {
                owner: self.nid(owner),
                name: name.clone(),
                id: self.nid(&id),
            });
        }
        if !self.merged_id(&id) {
            self.callables.insert(name, id.clone());
        }
        let children = named_children(decl);
        let params_at = children
            .iter()
            .position(|n| n.kind() == "function_value_parameters");
        if let Some(pi) = params_at {
            for p in named_children(children[pi]) {
                if p.kind() != "parameter" {
                    continue;
                }
                if let Some(t) = named_children(p)
                    .into_iter()
                    .find(|n| is_type_node(n.kind()))
                {
                    self.type_refs(&id, t, "parameter_type", line);
                }
            }
            if let Some(ret) = children[pi + 1..].iter().find(|n| is_type_node(n.kind())) {
                self.type_refs(&id, *ret, "return_type", line);
            }
        }
        self.annotations(decl, &id, line, false);
        if let Some(body) = children.iter().find(|n| n.kind() == "function_body") {
            self.bodies.push((id.clone(), *body));
            self.object_literals(*body, &id);
        }
    }

    /// Anonymous objects in a function body (not inside local functions or classes).
    fn object_literals(&mut self, node: TsNode<'t>, method: &str) {
        for ch in named_children(node) {
            match ch.kind() {
                "object_literal" => self.anonymous(ch, method),
                "function_declaration" | "class_declaration" | "object_declaration" => {}
                _ => self.object_literals(ch, method),
            }
        }
    }

    fn anonymous(&mut self, obj: TsNode<'t>, method: &str) {
        let specs: Vec<TsNode> = named_children(obj)
            .into_iter()
            .filter(|n| n.kind() == "delegation_specifiers")
            .flat_map(named_children)
            .collect();
        let first = specs.first().and_then(|s| {
            let n = named_children(*s).into_iter().next()?;
            let t =
                match n.kind() {
                    "constructor_invocation" | "explicit_delegation" => named_children(n)
                        .into_iter()
                        .find(|c| is_type_node(c.kind()))?,
                    _ => n,
                };
            self.last_identifier(strip_nullable(t))
        });
        let line = Ctx::line(obj);
        let type_name = first.unwrap_or_else(|| "object".to_string());
        let id = format!("{method}_object_{}_l{line}", slug(&type_name));
        self.put_def(&id, type_name.clone(), line, true);
        self.callables.insert(type_name, id.clone());
        self.edge(method, &id, "contains", line, None);
        for s in specs {
            self.supertype(&id, s, line, false);
        }
        for cb in named_children(obj)
            .into_iter()
            .filter(|n| n.kind() == "class_body")
        {
            for m in named_children(cb) {
                match m.kind() {
                    "function_declaration" => self.function(m, &id, false),
                    "property_declaration" => {
                        let pl = Ctx::line(m);
                        if let Some(t) = named_children(m)
                            .into_iter()
                            .find(|n| n.kind() == "variable_declaration")
                            .and_then(|v| {
                                named_children(v)
                                    .into_iter()
                                    .find(|n| is_type_node(n.kind()))
                            })
                        {
                            self.type_refs(&id, t, "field", pl);
                        }
                        self.annotations(m, &id, pl, false);
                    }
                    _ => {}
                }
            }
        }
    }

    // ---- type references ----------------------------------------------------------------------

    /// The name a `user_type` is referenced by: its first identifier (`Interceptor.Chain` →
    /// `Interceptor`, `java.io.File` → `java`).
    fn user_type_name(&self, t: TsNode) -> Option<String> {
        if t.kind() != "user_type" {
            return None;
        }
        named_children(t)
            .into_iter()
            .find(|n| n.kind() == "identifier")
            .map(|n| self.text(n).to_string())
    }

    /// The last identifier of a `user_type` (`b.Base<T>` → `Base`): supertypes and annotations.
    fn last_identifier(&self, t: TsNode) -> Option<String> {
        if t.kind() != "user_type" {
            return None;
        }
        named_children(t)
            .into_iter()
            .rev()
            .find(|n| n.kind() == "identifier")
            .map(|n| self.text(n).to_string())
    }

    /// The local class named `name`, or this file's stub for it.
    fn target(&mut self, name: &str) -> String {
        if let Some(id) = self.local_types.get(name) {
            return id.clone();
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

    fn reference(&mut self, source: &str, target: &str, ctx: &'static str, line: u32) {
        let (s, t) = (self.nid(source), self.nid(target));
        if self.refs_seen.insert((s.clone(), t.clone(), ctx, line)) {
            self.cx.add_edge(&s, &t, "references", line, Some(ctx));
        }
    }

    /// References from `source` to the type `t` (`ctx`) and to its type arguments
    /// (`generic_arg`). Function types and denied names produce nothing.
    fn type_refs(&mut self, source: &str, t: TsNode, ctx: &'static str, line: u32) {
        match t.kind() {
            "user_type" => {
                if let Some(name) = self.user_type_name(t) {
                    if !is_denied_type(&name) {
                        let target = self.target(&name);
                        self.reference(source, &target, ctx, line);
                    }
                }
                self.type_args(source, t, line);
            }
            "nullable_type"
            | "parenthesized_type"
            | "non_nullable_type"
            | "definitely_non_nullable_type"
            | "type_projection" => {
                if let Some(inner) = named_children(t)
                    .into_iter()
                    .find(|n| is_type_node(n.kind()))
                {
                    self.type_refs(source, inner, ctx, line);
                }
            }
            _ => {}
        }
    }

    fn type_args(&mut self, source: &str, t: TsNode, line: u32) {
        for args in named_children(t)
            .into_iter()
            .filter(|n| n.kind() == "type_arguments")
        {
            for proj in named_children(args) {
                self.type_refs(source, proj, "generic_arg", line);
            }
        }
    }

    // ---- calls --------------------------------------------------------------------------------

    fn calls(&mut self) {
        let mut bound: HashSet<(String, String)> = HashSet::new();
        let mut work = std::mem::take(&mut self.bodies);
        work.extend(std::mem::take(&mut self.deferred));
        let mut edges: Vec<Edge> = Vec::new();
        for (caller, body) in work {
            let caller = self.nid(&caller);
            let mut stack = vec![body];
            // Pre-order: a call before the calls in its receiver and arguments.
            while let Some(node) = stack.pop() {
                if node.kind() == "call_expression" {
                    self.call(node, &caller, &mut bound, &mut edges);
                }
                let mut kids = named_children(node);
                kids.retain(|k| {
                    !matches!(
                        k.kind(),
                        "function_declaration" | "class_declaration" | "object_declaration"
                    )
                });
                if node.kind() == "object_literal" {
                    kids.retain(|k| k.kind() != "class_body");
                }
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
        let Some(callee) = named_children(node).into_iter().next() else {
            return;
        };
        let line = Ctx::line(node);
        let (name, kind) = match callee.kind() {
            "identifier" => (
                self.text(callee).to_string(),
                Some(KCallKind::Bare { via_stub: false }),
            ),
            "navigation_expression" => {
                let parts = named_children(callee);
                let Some(last) = parts.last().filter(|n| n.kind() == "identifier") else {
                    return;
                };
                let recv = parts.first().copied();
                let kind = match recv {
                    Some(r)
                        if parts.len() == 2
                            && r.kind() == "identifier"
                            && self.text(r).starts_with(|c: char| c.is_uppercase()) =>
                    {
                        Some(KCallKind::Object(self.text(r).to_string()))
                    }
                    _ => None,
                };
                (self.text(*last).to_string(), kind)
            }
            _ => return,
        };
        let denied = is_denied_call(&name);
        if denied && !matches!(kind, Some(KCallKind::Object(_))) {
            return;
        }
        if let Some(target) = self.callables.get(&name).filter(|_| !denied) {
            let target = self.nid(target);
            if bound.insert((caller.to_string(), target.clone())) {
                edges.push(self.cx.edge(
                    caller,
                    &target,
                    "calls",
                    crate::facts::Loc::Line(line),
                    Some("call"),
                ));
            }
            return;
        }
        let kind = match kind {
            Some(KCallKind::Bare { .. }) => KCallKind::Bare {
                via_stub: self.stubs.contains_key(&name),
            },
            Some(k) => k,
            None => return,
        };
        self.facts.calls.push(KCall {
            caller: caller.to_string(),
            name,
            kind,
            line,
        });
    }
}

/// The declarations of a file or class body: its direct declarations (`false`), then, in
/// place, those found by walking into any other node (`true`): parse errors, misparsed
/// expressions, `init` blocks, secondary constructors and the anonymous objects in them.
/// Graphify's walk reaches these too, outside any class: a class whose header failed to parse
/// leaves its functions at the top level.
fn members<'t>(node: TsNode<'t>) -> Vec<(TsNode<'t>, bool)> {
    let mut out = Vec::new();
    for ch in named_children(node) {
        if is_declaration(ch.kind()) {
            out.push((ch, false));
        } else {
            nested_members(ch, &mut out);
        }
    }
    out
}

fn nested_members<'t>(node: TsNode<'t>, out: &mut Vec<(TsNode<'t>, bool)>) {
    for ch in named_children(node) {
        if is_declaration(ch.kind()) {
            out.push((ch, true));
        } else {
            nested_members(ch, out);
        }
    }
}

fn is_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration"
            | "object_declaration"
            | "function_declaration"
            | "property_declaration"
            | "companion_object"
            | "enum_entry"
            | "type_alias"
            | "import"
            | "package_header"
    )
}

/// `X::class` arguments anywhere inside an annotation, in source order.
fn class_literals(node: TsNode, src: &[u8], out: &mut Vec<String>) {
    for ch in named_children(node) {
        if ch.kind() == "navigation_expression" {
            let parts = named_children(ch);
            if let [recv, last] = parts.as_slice() {
                if recv.kind() == "identifier" && last.utf8_text(src).ok() == Some("class") {
                    if let Ok(name) = recv.utf8_text(src) {
                        out.push(name.to_string());
                    }
                    continue;
                }
            }
        }
        class_literals(ch, src, out);
    }
}

fn strip_nullable(t: TsNode) -> TsNode {
    if t.kind() == "nullable_type" {
        if let Some(inner) = named_children(t)
            .into_iter()
            .find(|n| is_type_node(n.kind()))
        {
            return inner;
        }
    }
    t
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_lists_are_sorted() {
        assert!(DENIED_TYPES.windows(2).all(|w| w[0] < w[1]));
        assert!(DENIED_CALLS.windows(2).all(|w| w[0] < w[1]));
    }
}

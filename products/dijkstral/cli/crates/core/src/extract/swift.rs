//! Swift (§18) extraction. See `docs/graphify-format.md`.
//!
//! One file yields its file node, every type declaration (`class`, `struct`, `enum`, `actor`,
//! `protocol` and `extension`, flat `file_Name` ids also for nested types), methods, initializers,
//! subscripts and `deinit` (`type_name`, overloads merged), computed and observed properties
//! (`.name`), top-level functions, enum cases, per-file type stubs, and the in-file `contains`,
//! `method`, `case_of`, `inherits`, `implements`, `references` and `calls` edges. `import X`
//! links the file to the shared module node `x`, which the cross-file pass creates.
//!
//! - An extension is a node of its own (`file_slug(type text)`); `crate::resolve::swift` merges
//!   it into the declaration it extends when exactly one declaration has that name.
//! - Names resolve in one pass: a type name refers to a node of the file defined so far
//!   (`file_slug(name)`, so `Swift.Error` names the file node itself, Graphify's `file_swift`),
//!   else to a stub created at its first use, shared by every file when that use is a supertype
//!   and per file otherwise. The cross-file pass binds stubs to the one type of that name.
//! - Supertypes: every one of a protocol is `inherits`, every one of an enum or extension is
//!   `implements`. For a class or actor the first is `inherits` unless it is a protocol the file
//!   defined so far; later ones are `implements` unless a class. A struct's supertypes are
//!   `implements` unless a class. Type arguments of a supertype are `references generic_arg`.
//! - References name the first identifier of a type (`Outer.Inner` → `Outer`) and type
//!   attributes (`@Sendable`); type arguments are `generic_arg`. A parameter written with
//!   attributes names only those. A return type written as a plain named type also carries
//!   `metadata.swift_plain_return`.
//! - Members are found by their kept node's label (same-id definitions merge, first label wins).
//!   In-file calls: a bare call binds to a member of the caller's type or its superclasses in
//!   the file, a type of the file (its initializer), or the file's last top-level function of
//!   that name; `super.m()` to the superclass's member; `self.m()` to the own type's member; any
//!   other `x.m()` to the file's last function, member or type named `m`. Builtin-like names
//!   never bind in the file; bare ones are dropped. Nested functions and subscript bodies are not
//!   scanned; a stored property's value only when it is a call (the type is the caller).
//!   Everything else is recorded in [`SwiftFacts::calls`] for the cross-file pass, with the
//!   receiver `x` of `x.m()`, `x.y.m()` or `self.x.m()`.
//! - [`SwiftFacts::table`] is Graphify's per-file variable-type table: parameters and typed
//!   properties (last write wins) over locals initialised by `Type(...)` or `Type.member` (first
//!   wins). [`SwiftFacts::factory`] holds variables first initialised by `Type.m(...)`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, slug, Ctx};
use crate::facts::{Attr, LangFacts};

/// A call the file could not bind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SCall {
    /// Local id of the calling function, property or type.
    pub caller: String,
    pub callee: String,
    pub member: bool,
    /// Leftmost identifier of the receiver (`self.` skipped), when it is a plain chain.
    pub receiver: Option<String>,
    /// A bare call whose name the file references as an undeclared type.
    pub via_stub: bool,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::swift`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SwiftFacts {
    /// `import` declarations: (module path as written, line).
    pub imports: Vec<(String, u32)>,
    /// Local ids of extension nodes (also when the file declares the extended type itself).
    pub extensions: Vec<String>,
    /// Local ids of `class`, `struct`, `enum`, `actor` and `protocol` declarations.
    pub decls: Vec<String>,
    /// Local ids of per-file type stubs.
    pub stubs: Vec<String>,
    /// Supertype stubs shared by every file: (id, label). The cross-file pass creates them.
    pub global_stubs: Vec<(String, String)>,
    pub calls: Vec<SCall>,
    /// Variable name → type name.
    pub table: Vec<(String, String)>,
    /// Variable name → (type name, static member) for `let x = Type.m(...)`.
    pub factory: Vec<(String, String, String)>,
}

/// Names never bound in the file; bare calls with them are dropped. Python builtins and
/// JavaScript globals as for Go, except `pow`, which Graphify binds for Swift. Sorted.
const DENIED_CALLS: &[&str] = &[
    "Array",
    "ArrayBuffer",
    "BigInt",
    "Bool",
    "Boolean",
    "DataView",
    "Date",
    "Error",
    "EvalError",
    "Float",
    "Float32Array",
    "Float64Array",
    "Function",
    "Int",
    "Int16Array",
    "Int32",
    "Int32Array",
    "Int64",
    "Int8Array",
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
    "Uint16Array",
    "Uint32Array",
    "Uint8Array",
    "WeakMap",
    "WeakSet",
    "abs",
    "all",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "complex",
    "decodeURI",
    "decodeURIComponent",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "encodeURI",
    "encodeURIComponent",
    "enumerate",
    "filter",
    "float",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isFinite",
    "isNaN",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "oct",
    "open",
    "ord",
    "parseFloat",
    "parseInt",
    "print",
    "property",
    "range",
    "repr",
    "require",
    "reversed",
    "round",
    "set",
    "setattr",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
];

/// Foundation and standard-library initialisers Graphify also never records (observed on
/// Alamofire; `Progress` and `URLError` are recorded). Sorted.
const DENIED_SWIFT_TYPES: &[&str] = &[
    "Bundle",
    "Calendar",
    "Data",
    "DateFormatter",
    "DispatchGroup",
    "DispatchQueue",
    "Double",
    "JSONDecoder",
    "JSONEncoder",
    "NSError",
    "NSLock",
    "NumberFormatter",
    "OperationQueue",
    "TimeZone",
    "UInt32",
    "UInt64",
    "UInt8",
    "URLComponents",
    "URLRequest",
    "URLSession",
    "UUID",
];

pub(crate) fn is_denied(name: &str) -> bool {
    DENIED_CALLS.binary_search(&name).is_ok() || DENIED_SWIFT_TYPES.binary_search(&name).is_ok()
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.children(&mut c).collect()
}

/// `class`, `struct`, `enum`, `actor`, `extension` or `protocol`.
fn decl_kind(node: TsNode) -> &'static str {
    if node.kind() == "protocol_declaration" {
        return "protocol";
    }
    match node
        .child_by_field_name("declaration_kind")
        .map(|k| k.kind())
    {
        Some("struct") => "struct",
        Some("enum") => "enum",
        Some("actor") => "actor",
        Some("extension") => "extension",
        _ => "class",
    }
}

fn is_type_decl(kind: &str) -> bool {
    matches!(kind, "class_declaration" | "protocol_declaration")
}

fn is_function(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "protocol_function_declaration"
            | "init_declaration"
            | "deinit_declaration"
            | "subscript_declaration"
    )
}

fn is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "user_type"
            | "tuple_type"
            | "function_type"
            | "array_type"
            | "dictionary_type"
            | "optional_type"
            | "metatype"
            | "opaque_type"
            | "existential_type"
            | "protocol_composition_type"
            | "suppressed_constraint"
            | "type_pack_expansion"
            | "type_parameter_pack"
    )
}

/// The declared name of a type declaration, or the extended type's text for an extension.
fn type_name<'a>(cx: &Ctx<'a>, node: TsNode) -> Option<&'a str> {
    let name = node.child_by_field_name("name")?;
    Some(cx.text(name))
}

/// A function's name: `init`, `deinit`, `subscript`, or its identifier (operators: none).
fn function_name<'a>(cx: &Ctx<'a>, node: TsNode) -> Option<&'a str> {
    match node.kind() {
        "init_declaration" => Some("init"),
        "deinit_declaration" => Some("deinit"),
        "subscript_declaration" => Some("subscript"),
        _ => named_children(node)
            .into_iter()
            .take_while(|c| c.kind() != "function_body")
            .find(|c| c.kind() == "simple_identifier")
            .map(|c| cx.text(c)),
    }
}

/// The return type: the type modifiers and the type after `->`.
fn return_type(node: TsNode) -> Vec<TsNode> {
    let mut seen_arrow = false;
    let mut out = Vec::new();
    for c in children(node) {
        if c.kind() == "->" {
            seen_arrow = true;
            continue;
        }
        if !seen_arrow || !c.is_named() {
            continue;
        }
        if c.kind() == "type_modifiers" {
            out.push(c);
        } else {
            if is_type_node(c.kind()) {
                out.push(c);
            }
            break;
        }
    }
    out
}

/// A parameter's internal name: the last identifier before its type.
fn parameter_name<'a>(cx: &Ctx<'a>, node: TsNode) -> Option<&'a str> {
    let mut name = None;
    for c in named_children(node) {
        match c.kind() {
            "simple_identifier" => name = Some(cx.text(c)),
            "parameter_modifiers" => {}
            _ => break,
        }
    }
    name
}

/// Every type identifier a type mentions, in pre-order: `(name, generic_arg)`. A `user_type`
/// names its first identifier (`Outer.Inner` → `Outer`); type arguments are generic arguments;
/// type attributes (`@Sendable`) count as types; names, parameter modifiers (`@escaping`) and
/// default values do not.
fn type_refs<'a>(cx: &Ctx<'a>, node: TsNode, generic: bool, out: &mut Vec<(&'a str, bool)>) {
    match node.kind() {
        "simple_identifier"
        | "value_argument_label"
        | "parameter_modifiers"
        | "wildcard_pattern" => {}
        "attribute" => {
            if let Some(t) = named_children(node)
                .into_iter()
                .find(|c| c.kind() == "user_type")
            {
                type_refs(cx, t, generic, out);
            }
        }
        "user_type" => {
            let mut first = true;
            for c in named_children(node) {
                match c.kind() {
                    "type_identifier" if first => {
                        out.push((cx.text(c), generic));
                        first = false;
                    }
                    "type_arguments" => {
                        for a in named_children(c) {
                            type_refs(cx, a, true, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        "type_identifier" => out.push((cx.text(node), generic)),
        _ => {
            let mut c = node.walk();
            for child in node.children(&mut c) {
                // A parameter's default value starts at `=`.
                if child.kind() == "=" {
                    break;
                }
                if child.is_named() {
                    type_refs(cx, child, generic, out);
                }
            }
        }
    }
}

/// The first type identifier of a type (the variable-type table's value).
fn first_type_ident<'a>(cx: &Ctx<'a>, node: TsNode) -> Option<&'a str> {
    let mut out = Vec::new();
    type_refs(cx, node, false, &mut out);
    out.first().map(|(n, _)| *n)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Recv<'a> {
    /// `self.m()`
    SelfRef,
    /// `super.m()`
    Super,
    /// `x.m()`, `x.y.m()`, `self.x.m()`.
    Ident(&'a str),
    /// Anything else (a longer chain, a call, a literal).
    Other,
}

#[derive(Debug, Clone)]
struct Site<'a> {
    caller: String,
    /// The type whose body contains the caller, if any.
    owner: Option<String>,
    name: &'a str,
    /// `None` for a bare call.
    recv: Option<Recv<'a>>,
    line: u32,
}

struct Walker<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    facts: SwiftFacts,
    /// Type id → (label, kind) of the first type declared or extended with it, in walk order.
    types: HashMap<String, (&'a str, &'static str)>,
    /// Type name → its stub id (shared or per file) once the file used it undeclared.
    stubs: HashMap<&'a str, String>,
    /// Node id → index of its first node in `cx.facts.nodes`.
    node_index: HashMap<String, usize>,
    /// Type id → member name → member id (first wins).
    members: HashMap<String, HashMap<String, String>>,
    /// Name → (id, top-level function) of the file's last definition with that name.
    last_def: HashMap<String, (String, bool)>,
    /// Type id → its superclass's id, when it inherits from a class of the file.
    superclass: HashMap<String, String>,
    sites: Vec<Site<'a>>,
    property_sites: Vec<Site<'a>>,
    /// Parameters and typed properties (last write wins), over initialised locals.
    table: Vec<(&'a str, &'a str)>,
    local_table: Vec<(&'a str, &'a str)>,
    factory: Vec<(&'a str, &'a str, &'a str)>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let Some(tree) = tree else {
        cx.facts.lang = LangFacts::Swift(SwiftFacts::default());
        return;
    };
    let root = tree.root_node();
    let mut w = Walker {
        cx,
        facts: SwiftFacts::default(),
        types: HashMap::new(),
        stubs: HashMap::new(),
        node_index: HashMap::new(),
        members: HashMap::new(),
        last_def: HashMap::new(),
        superclass: HashMap::new(),
        sites: Vec::new(),
        property_sites: Vec::new(),
        table: Vec::new(),
        local_table: Vec::new(),
        factory: Vec::new(),
    };
    let file_id = w.cx.file_id.clone();
    w.walk_body(root, &file_id, None);
    w.bind_calls();
    let mut facts = w.facts;
    let mut table = w.local_table;
    for (k, t) in w.table {
        put(&mut table, k, t);
    }
    facts.factory = w
        .factory
        .iter()
        .map(|(k, t, m)| (k.to_string(), t.to_string(), m.to_string()))
        .collect();
    facts.table = table
        .iter()
        .map(|(k, t)| (k.to_string(), t.to_string()))
        .collect();
    cx.facts.lang = LangFacts::Swift(facts);
}

fn capitalized(s: &str) -> bool {
    s.chars().next().is_some_and(char::is_uppercase)
}

/// Insert or overwrite `key` in an insertion-ordered map.
fn put<'a>(map: &mut Vec<(&'a str, &'a str)>, key: &'a str, value: &'a str) {
    match map.iter_mut().find(|(k, _)| *k == key) {
        Some(e) => e.1 = value,
        None => map.push((key, value)),
    }
}

impl<'a> Walker<'_, 'a> {
    fn type_id(&self, name: &str) -> String {
        Ctx::child_id(&self.cx.file_id, name)
    }

    /// A name the file has defined so far: `file_slug(name)` is a node of the file. Graphify's
    /// own id for the file node is `file_swift`, so `Swift.Error` names the file itself.
    fn local(&self, name: &str) -> Option<String> {
        let id = self.type_id(name);
        if id.len() == self.cx.file_id.len() + 6 && id.ends_with("_swift") {
            return Some(self.cx.file_id.clone());
        }
        self.node_index.contains_key(&id).then_some(id)
    }

    /// A definition node; a later node with the same id only adds the fields the first lacks.
    fn add_def(&mut self, id: String, label: String, line: u32, extra: Vec<(String, Attr)>) {
        if let Some(&i) = self.node_index.get(&id) {
            let node = &mut self.cx.facts.nodes[i];
            for (k, v) in extra {
                if node.attr(&k).is_none() {
                    node.attrs.push((k, v));
                }
            }
            return;
        }
        self.node_index
            .insert(id.clone(), self.cx.facts.nodes.len());
        self.cx.add_def(id, label, line, extra);
    }

    /// The id a type name refers to from this file: a node the file defined so far, else a
    /// stub (an undeclared supertype makes a stub shared by every file, a reference a per-file
    /// one; the first use decides).
    fn type_target(&mut self, name: &'a str, shared: bool) -> String {
        if let Some(id) = self.local(name) {
            return id;
        }
        if let Some(id) = self.stubs.get(name) {
            return id.clone();
        }
        if shared {
            let id = slug(name);
            if !self.facts.global_stubs.iter().any(|(g, _)| *g == id) {
                self.facts.global_stubs.push((id.clone(), name.to_string()));
            }
            self.stubs.insert(name, id.clone());
            return id;
        }
        let id = slug(name);
        self.cx.add_stub(id.clone(), name.to_string());
        self.facts.stubs.push(id.clone());
        self.stubs.insert(name, id.clone());
        id
    }

    fn set_table(&mut self, var: &'a str, ty: &'a str) {
        put(&mut self.table, var, ty);
    }

    fn set_local(&mut self, var: &'a str, ty: &'a str) {
        if !self.local_table.iter().any(|(k, _)| *k == var) {
            self.local_table.push((var, ty));
        }
    }

    fn set_factory(&mut self, var: &'a str, ty: &'a str, member: &'a str) {
        let known = |m: &Vec<(&str, &str)>| m.iter().any(|(k, _)| *k == var);
        if !known(&self.table)
            && !known(&self.local_table)
            && !self.factory.iter().any(|(k, _, _)| *k == var)
        {
            self.factory.push((var, ty, member));
        }
    }

    /// Walk the declarations of a file or a type body. `owner` is the enclosing type's id.
    fn walk_body(&mut self, node: TsNode, parent: &str, owner: Option<&str>) {
        for c in named_children(node) {
            let k = c.kind();
            if is_type_decl(k) {
                self.type_decl(c, parent);
            } else if is_function(k) {
                self.function(c, parent, owner);
            } else if k == "property_declaration" {
                if let Some(o) = owner {
                    self.property(c, o);
                }
            } else if k == "enum_entry" {
                if let Some(o) = owner {
                    self.enum_entry(c, o);
                }
            } else if k == "import_declaration" && owner.is_none() {
                self.import(c);
            }
        }
    }

    fn import(&mut self, node: TsNode) {
        let Some(ident) = named_children(node)
            .into_iter()
            .find(|c| c.kind() == "identifier")
        else {
            return;
        };
        let path = self.cx.text(ident).to_string();
        let line = Ctx::line(node);
        let file_id = self.cx.file_id.clone();
        self.cx
            .add_edge(&file_id, &slug(&path), "imports", line, Some("import"));
        self.facts.imports.push((path, line));
    }

    fn type_decl(&mut self, node: TsNode, parent: &str) {
        let Some(name) = type_name(self.cx, node) else {
            return;
        };
        let kind = decl_kind(node);
        let id = self.type_id(name);
        let line = Ctx::line(node);
        self.add_def(
            id.clone(),
            name.to_string(),
            line,
            vec![flag("_callable"), flag("_callable_class")],
        );
        self.types.entry(id.clone()).or_insert((name, kind));
        self.last_def.insert(name.to_string(), (id.clone(), false));
        self.cx.add_edge(parent, &id, "contains", line, None);
        if kind == "extension" {
            self.facts.extensions.push(id.clone());
        } else if !self.facts.decls.contains(&id) {
            self.facts.decls.push(id.clone());
        }
        // Supertypes.
        let supers: Vec<TsNode> = named_children(node)
            .into_iter()
            .filter(|c| c.kind() == "inheritance_specifier")
            .collect();
        for (i, s) in supers.iter().enumerate() {
            let Some(target_name) = first_type_ident(self.cx, *s) else {
                continue;
            };
            let known = self
                .local(target_name)
                .and_then(|t| self.types.get(&t).map(|(_, k)| *k));
            let relation = match kind {
                "protocol" => "inherits",
                "enum" | "extension" => "implements",
                "struct" => {
                    if known == Some("class") {
                        "inherits"
                    } else {
                        "implements"
                    }
                }
                _ => {
                    if i == 0 {
                        if known == Some("protocol") {
                            "implements"
                        } else {
                            "inherits"
                        }
                    } else if known == Some("class") {
                        "inherits"
                    } else {
                        "implements"
                    }
                }
            };
            let target = self.type_target(target_name, true);
            if relation == "inherits" && matches!(kind, "class" | "actor") && known == Some("class")
            {
                self.superclass.entry(id.clone()).or_insert(target.clone());
            }
            if target != id {
                self.cx.add_edge(&id, &target, relation, line, None);
            }
            // `WKHostingController<ContentView>`: the type arguments are referenced.
            let mut refs = Vec::new();
            type_refs(self.cx, *s, false, &mut refs);
            let mut seen = HashSet::new();
            for (arg, generic) in refs {
                if !generic {
                    continue;
                }
                let t = self.type_target(arg, false);
                if t != id && seen.insert(t.clone()) {
                    self.cx
                        .add_edge(&id, &t, "references", line, Some("generic_arg"));
                }
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.members.entry(id.clone()).or_default();
            self.walk_body(body, &id, Some(&id));
        }
    }

    /// The name a definition is found by: its kept node's label without `.` and `()`, so
    /// `_response()` merged into `.response()` is found as `response`.
    fn kept_name(&self, id: &str) -> String {
        let label = self
            .node_index
            .get(id)
            .map_or("", |&i| self.cx.facts.nodes[i].label.as_str());
        let l = label.strip_prefix('.').unwrap_or(label);
        l.strip_suffix("()").unwrap_or(l).to_string()
    }

    /// Register a `method` target of `owner`.
    fn add_member(&mut self, owner: &str, id: &str) {
        let name = self.kept_name(id);
        self.members
            .entry(owner.to_string())
            .or_default()
            .entry(name.clone())
            .or_insert_with(|| id.to_string());
        self.last_def.insert(name, (id.to_string(), false));
    }

    fn function(&mut self, node: TsNode, parent: &str, owner: Option<&str>) {
        let Some(name) = function_name(self.cx, node) else {
            return;
        };
        let line = Ctx::line(node);
        let id = Ctx::child_id(parent, name);
        match owner {
            Some(o) => {
                self.add_def(
                    id.clone(),
                    format!(".{name}()"),
                    line,
                    vec![flag("_callable")],
                );
                self.cx.add_edge(o, &id, "method", line, None);
                self.add_member(o, &id);
            }
            None => {
                if matches!(name, "init" | "deinit" | "subscript") {
                    return;
                }
                self.add_def(
                    id.clone(),
                    format!("{name}()"),
                    line,
                    vec![flag("_callable")],
                );
                self.cx.add_edge(parent, &id, "contains", line, None);
                let kept = self.kept_name(&id);
                self.last_def.insert(kept, (id.clone(), true));
            }
        }
        // Parameter and return types: an attributed type (`@Sendable (…) -> Void`) names only
        // its attributes.
        let mut seen: HashSet<(String, &'static str)> = HashSet::new();
        for p in named_children(node) {
            if p.kind() != "parameter" {
                continue;
            }
            let ty = p.child_by_field_name("type").or_else(|| {
                named_children(p)
                    .into_iter()
                    .find(|c| is_type_node(c.kind()))
            });
            let Some(ty) = ty else { continue };
            if let (Some(pn), Some(t)) = (parameter_name(self.cx, p), first_type_ident(self.cx, ty))
            {
                self.set_table(pn, t);
            }
            self.refs(&id, &[ty], "parameter_type", line, &mut seen, false);
        }
        let ret = return_type(node);
        if let Some(first) = ret.first() {
            let plain = first.kind() == "user_type";
            self.refs(&id, &ret[..1], "return_type", line, &mut seen, plain);
        }
        // Subscript bodies are not scanned for calls.
        if node.kind() == "subscript_declaration" {
            return;
        }
        let body = named_children(node)
            .into_iter()
            .find(|c| matches!(c.kind(), "function_body" | "computed_property"));
        if let Some(b) = body {
            self.scan(b, &id, owner, false);
        }
    }

    /// Emit `references` edges from `source` for every type the nodes mention. `seen` dedupes
    /// by (target, context); `plain` marks the first reference of a plain return type.
    fn refs(
        &mut self,
        source: &str,
        nodes: &[TsNode],
        context: &'static str,
        line: u32,
        seen: &mut HashSet<(String, &'static str)>,
        plain: bool,
    ) {
        let mut out = Vec::new();
        for n in nodes {
            type_refs(self.cx, *n, false, &mut out);
        }
        for (i, (name, generic)) in out.into_iter().enumerate() {
            let ctx = if generic { "generic_arg" } else { context };
            let target = self.type_target(name, false);
            if target == source || !seen.insert((target.clone(), ctx)) {
                continue;
            }
            let mut e = self.cx.edge(
                source,
                &target,
                "references",
                crate::facts::Loc::Line(line),
                Some(ctx),
            );
            if plain && i == 0 {
                e.attrs.push((
                    "metadata".into(),
                    Attr::Json("{\"swift_plain_return\":true}".into()),
                ));
            }
            self.cx.facts.edges.push(e);
        }
    }

    fn property(&mut self, node: TsNode, owner: &str) {
        let line = Ctx::line(node);
        let mut name: Option<&'a str> = None;
        let mut seen: HashSet<(String, &'static str)> = HashSet::new();
        for c in named_children(node) {
            match c.kind() {
                "pattern" => {
                    name = c
                        .child_by_field_name("bound_identifier")
                        .map(|n| self.cx.text(n));
                }
                "type_annotation" => {
                    if let (Some(n), Some(t)) = (name, first_type_ident(self.cx, c)) {
                        self.set_table(n, t);
                    }
                    self.refs(owner, &[c], "field", line, &mut seen, false);
                }
                "computed_property" | "willset_didset_block" => {
                    if let Some(n) = name {
                        let id = Ctx::child_id(owner, n);
                        self.add_def(id.clone(), format!(".{n}"), line, vec![]);
                        self.add_member(owner, &id);
                        self.cx.add_edge(owner, &id, "method", line, None);
                        self.scan(c, &id, Some(owner), false);
                    }
                }
                "modifiers" | "value_binding_pattern" => {}
                _ => {
                    // The initial value.
                    if let Some(n) = name {
                        self.value_type(n, c, true);
                    }
                    // Only a call is scanned (`= Type()`, `= { … }()`), not a closure or literal.
                    if c.kind() == "call_expression" {
                        self.scan(c, owner, Some(owner), true);
                    }
                }
            }
        }
    }

    /// Record `x = Type(...)`, `x = Type.member` and `x = Type.m(...)` in the variable tables;
    /// `decl` for a property of a type, else a local.
    fn value_type(&mut self, var: &'a str, value: TsNode, decl: bool) {
        let ident = |n: TsNode| -> Option<&'a str> {
            (n.kind() == "simple_identifier")
                .then(|| self.cx.text(n))
                .filter(|t| capitalized(t))
        };
        let typed = match value.kind() {
            "call_expression" => {
                let Some(f) = value.named_child(0) else {
                    return;
                };
                match f.kind() {
                    "simple_identifier" => ident(f),
                    "navigation_expression" => {
                        let (Some(target), Some(m)) = (
                            f.child_by_field_name("target"),
                            f.child_by_field_name("suffix")
                                .and_then(|s| s.child_by_field_name("suffix")),
                        ) else {
                            return;
                        };
                        if let Some(t) = ident(target) {
                            let m = self.cx.text(m);
                            self.set_factory(var, t, m);
                        }
                        None
                    }
                    _ => None,
                }
            }
            "navigation_expression" => value.child_by_field_name("target").and_then(ident),
            _ => None,
        };
        if let Some(t) = typed {
            if decl {
                self.set_table(var, t);
            } else {
                self.set_local(var, t);
            }
        }
    }

    fn enum_entry(&mut self, node: TsNode, owner: &str) {
        let mut seen: HashSet<(String, &'static str)> = HashSet::new();
        for c in named_children(node) {
            match c.kind() {
                "simple_identifier" => {
                    let name = self.cx.text(c);
                    let line = Ctx::line(c);
                    let id = Ctx::child_id(owner, name);
                    // Cases are no members (no `method` edge) unless a method shares the id.
                    self.add_def(id.clone(), name.to_string(), line, vec![]);
                    self.cx.add_edge(owner, &id, "case_of", line, None);
                }
                "enum_type_parameters" => {
                    let line = Ctx::line(c);
                    self.refs(owner, &[c], "type", line, &mut seen, false);
                }
                _ => {}
            }
        }
    }

    /// Collect the call sites (and local variable types) under a body, nested types included;
    /// nested functions are skipped.
    fn scan(&mut self, node: TsNode<'_>, caller: &str, owner: Option<&str>, property: bool) {
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            let k = n.kind();
            if is_function(k) && n != node {
                continue;
            }
            if k == "property_declaration" && !property {
                // A local: only its value types it.
                let kids = named_children(n);
                if let Some(var) = kids
                    .iter()
                    .find(|c| c.kind() == "pattern")
                    .and_then(|p| p.child_by_field_name("bound_identifier"))
                    .map(|b| self.cx.text(b))
                {
                    if let Some(v) = kids
                        .iter()
                        .rev()
                        .find(|c| matches!(c.kind(), "call_expression" | "navigation_expression"))
                    {
                        self.value_type(var, *v, false);
                    }
                }
            }
            if k == "call_expression" {
                if let Some(site) = self.site(n, caller, owner) {
                    if property {
                        self.property_sites.push(site);
                    } else {
                        self.sites.push(site);
                    }
                }
            }
            let mut kids = named_children(n);
            kids.reverse();
            stack.extend(kids);
        }
    }

    fn site(&self, call: TsNode, caller: &str, owner: Option<&str>) -> Option<Site<'a>> {
        let f = call.named_child(0)?;
        let line = Ctx::line(call);
        let (name, recv) = match f.kind() {
            "simple_identifier" => (self.cx.text(f), None),
            "navigation_expression" => {
                let suffix = f.child_by_field_name("suffix")?;
                let name = suffix.child_by_field_name("suffix")?;
                if name.kind() != "simple_identifier" {
                    return None;
                }
                let target = f.child_by_field_name("target")?;
                (self.cx.text(name), Some(self.receiver(target)))
            }
            _ => return None,
        };
        Some(Site {
            caller: caller.to_string(),
            owner: owner.map(str::to_string),
            name,
            recv,
            line,
        })
    }

    /// `x` for `x.m()`, `x.y.m()` and `self.x.m()`; nothing longer.
    fn receiver(&self, target: TsNode) -> Recv<'a> {
        match target.kind() {
            "self_expression" => Recv::SelfRef,
            "super_expression" => Recv::Super,
            "simple_identifier" => Recv::Ident(self.cx.text(target)),
            "navigation_expression" => {
                let Some(inner) = target.child_by_field_name("target") else {
                    return Recv::Other;
                };
                match inner.kind() {
                    "simple_identifier" => Recv::Ident(self.cx.text(inner)),
                    "self_expression" => target
                        .child_by_field_name("suffix")
                        .and_then(|s| s.child_by_field_name("suffix"))
                        .filter(|s| s.kind() == "simple_identifier")
                        .map_or(Recv::Other, |s| Recv::Ident(self.cx.text(s))),
                    _ => Recv::Other,
                }
            }
            _ => Recv::Other,
        }
    }

    fn member_of(&self, ty: &str, name: &str) -> Option<String> {
        self.members.get(ty)?.get(name).cloned()
    }

    /// A member of `ty` or of its superclasses in the file.
    fn inherited_member(&self, ty: &str, name: &str) -> Option<String> {
        let mut cur = ty.to_string();
        for _ in 0..16 {
            if let Some(m) = self.member_of(&cur, name) {
                return Some(m);
            }
            cur = self.superclass.get(&cur)?.clone();
        }
        None
    }

    /// The type declared (not merely extended) in the file under exactly this name.
    fn declared_type(&self, name: &str) -> Option<String> {
        let id = self.type_id(name);
        match self.types.get(&id) {
            Some((label, _)) if *label == name => Some(id),
            _ => None,
        }
    }

    /// Bind call sites in the file; record the rest for the cross-file pass.
    fn bind_calls(&mut self) {
        let sites: Vec<Site> = std::mem::take(&mut self.sites)
            .into_iter()
            .chain(std::mem::take(&mut self.property_sites))
            .collect();
        let mut bound: HashSet<(String, String)> = HashSet::new();
        for s in sites {
            match self.bind(&s) {
                Some(Some(t)) => {
                    if bound.insert((s.caller.clone(), t.clone())) {
                        self.cx
                            .add_edge(&s.caller, &t, "calls", s.line, Some("call"));
                    }
                }
                Some(None) => {}
                None => {
                    let (member, receiver) = match s.recv {
                        None => (false, None),
                        Some(Recv::Ident(r)) => (true, Some(r.to_string())),
                        Some(_) => (true, None),
                    };
                    let via_stub = !member && self.stubs.contains_key(s.name);
                    self.facts.calls.push(SCall {
                        caller: s.caller,
                        callee: s.name.to_string(),
                        member,
                        receiver,
                        via_stub,
                        line: s.line,
                    });
                }
            }
        }
    }

    /// `Some(Some(id))`: bound; `Some(None)`: dropped; `None`: left for the cross-file pass.
    fn bind(&self, s: &Site) -> Option<Option<String>> {
        let name = s.name;
        match s.recv {
            None => {
                if is_denied(name) {
                    return Some(None);
                }
                if let Some(t) = s
                    .owner
                    .as_deref()
                    .and_then(|o| self.inherited_member(o, name))
                {
                    return Some(Some(t));
                }
                if let Some(t) = self.declared_type(name) {
                    return Some(Some(t));
                }
                match self.last_def.get(name) {
                    Some((id, true)) => Some(Some(id.clone())),
                    _ => None,
                }
            }
            Some(Recv::Super) => {
                let sup = s.owner.as_deref().and_then(|o| self.superclass.get(o))?;
                self.inherited_member(sup, name).map(Some)
            }
            Some(Recv::SelfRef) => s
                .owner
                .as_deref()
                .and_then(|o| self.member_of(o, name))
                .map(Some),
            Some(_) => {
                if is_denied(name) {
                    return None;
                }
                self.last_def.get(name).map(|(id, _)| Some(id.clone()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_file;
    use crate::facts::{FileFacts, Loc};
    use crate::lang::Lang;

    fn facts(rel: &str, src: &str) -> (FileFacts, SwiftFacts) {
        let f = extract_file(Lang::Swift, rel, src.as_bytes());
        let LangFacts::Swift(s) = f.lang.clone() else {
            panic!("no swift facts");
        };
        (f, s)
    }

    fn edges(f: &FileFacts) -> Vec<String> {
        f.edges
            .iter()
            .map(|e| {
                let line = match e.loc {
                    Loc::Line(n) => n,
                    _ => 0,
                };
                format!(
                    "{} {} {} {} L{line}",
                    e.source,
                    e.relation,
                    e.target,
                    e.context.as_deref().unwrap_or("-")
                )
            })
            .collect()
    }

    #[test]
    fn names_resolve_in_one_pass() {
        let src = "struct A: Error {\n    var b: B\n}\nextension Error {}\nstruct C {\n    var e: Error\n    var s: Swift.Error\n}\nstruct B {}\n";
        let (f, s) = facts("m/F.swift", src);
        let e = edges(&f);
        // A supertype names a shared stub; a forward reference a per-file one.
        assert!(
            e.contains(&"m_f_a implements error - L1".to_string()),
            "{e:?}"
        );
        assert!(
            e.contains(&"m_f_a references b field L2".to_string()),
            "{e:?}"
        );
        assert_eq!(
            s.global_stubs,
            vec![("error".to_string(), "Error".to_string())]
        );
        assert_eq!(s.stubs, vec!["b".to_string()]);
        // After `extension Error`, `Error` is the extension; `Swift.X` names the file itself.
        assert!(
            e.contains(&"m_f_c references m_f_error field L6".to_string()),
            "{e:?}"
        );
        assert!(
            e.contains(&"m_f_c references m_f field L7".to_string()),
            "{e:?}"
        );
        assert_eq!(s.extensions, vec!["m_f_error".to_string()]);
        assert_eq!(s.decls, vec!["m_f_a", "m_f_c", "m_f_b"]);
    }

    #[test]
    fn calls_bind_in_file_or_are_recorded() {
        let src = "class P {\n    func up() {}\n}\nclass K: P {\n    let q = Q()\n    func run(t: T) {\n        up()\n        t.go()\n        self.missing()\n        print(1)\n        let v = Make.it()\n    }\n}\nclass Q {}\n";
        let (f, s) = facts("K.swift", src);
        let e = edges(&f);
        assert!(
            e.contains(&"k_k_run calls k_p_up call L7".to_string()),
            "{e:?}"
        );
        assert!(e.contains(&"k_k calls k_q call L5".to_string()), "{e:?}");
        let calls: Vec<_> = s
            .calls
            .iter()
            .map(|c| (c.callee.as_str(), c.member, c.receiver.as_deref()))
            .collect();
        assert_eq!(
            calls,
            vec![
                ("go", true, Some("t")),
                ("missing", true, None),
                ("it", true, Some("Make"))
            ]
        );
        assert_eq!(
            s.table,
            vec![
                ("q".to_string(), "Q".to_string()),
                ("t".to_string(), "T".to_string())
            ]
        );
        assert_eq!(
            s.factory,
            vec![("v".to_string(), "Make".to_string(), "it".to_string())]
        );
    }

    #[test]
    fn plain_returns_and_attributed_parameters() {
        let src =
            "func f(h: @escaping @Sendable (Int) -> Void) -> Result<A, B> {}\nfunc g() -> [A] {}\n";
        let (f, _) = facts("R.swift", src);
        let plain: Vec<_> = f
            .edges
            .iter()
            .filter(|e| e.attrs.iter().any(|(k, _)| k == "metadata"))
            .map(|e| e.target.as_str())
            .collect();
        assert_eq!(plain, vec!["result"]);
        let e = edges(&f);
        assert!(
            e.contains(&"r_f references sendable parameter_type L1".to_string()),
            "{e:?}"
        );
        assert!(!e.iter().any(|x| x.contains("references int")), "{e:?}");
        assert!(
            e.contains(&"r_f references a generic_arg L1".to_string()),
            "{e:?}"
        );
        assert!(
            e.contains(&"r_g references a return_type L2".to_string()),
            "{e:?}"
        );
    }
}

//! Go (§12) extraction. See `docs/graphify-format.md`.
//!
//! One file yields its file node, top-level types (`dir_type`), functions (`file_fn`), methods
//! (`dir_receiver_m`), interface methods, type-parameter and unresolved-type stubs, `contains`,
//! `method`, `embeds` and `references` edges, and the calls that resolve inside the file. Everything
//! that needs other files (imports, qualified types, cross-file calls) is recorded in [`GoFacts`]
//! for `crate::resolve::go`, together with the declarations and symbolic receiver types of local
//! type inference ([`infer`], Q24).
//!
//! Graphify behaviours reproduced for id parity (spec §12.4, quirk Q10), because Graphify's ids
//! depend on them:
//!
//! - Types are keyed by their directory, not their file (`store_model`), so a file `x/x.go` and a
//!   `type X` in it share the id `x_x`: one node, in the file node's position, with the type's
//!   label and line, and a `contains` self-loop (G2).
//! - A method whose receiver type is not declared earlier in the same file creates a receiver node
//!   (label = receiver type text, e.g. `Store[T]`) at the method's line (G3).
//! - Every qualified type reference `q.T` emits a per-file stub `q_t`; when that id collides with a
//!   real type `dir_t` elsewhere, the §4.5 collision rename re-keys the real type
//!   (`store_store_go_store_model`, G1). The resolver then replaces the stub.
//! - Calls through a selector on anything but an import are resolved by bare method name among the
//!   file's callables, the last definition winning (G5).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{slug, Ctx};
use crate::facts::NodeKind;

mod infer;

/// Placeholder for the directory slug of files at the scan root. Graphify uses the scan root's own
/// directory name, which is not part of a file's cache key, so the resolver substitutes it.
pub const ROOT_DIR: &str = "@root";

/// One import spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoImport {
    pub path: String,
    /// `None` (no name), `Some(".")`, `Some("_")` or an alias.
    pub alias: Option<String>,
    pub line: u32,
}

/// A qualified type reference `q.Name`, recorded once per distinct stub.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualStub {
    /// Local stub id `slug(q)_slug(Name)`.
    pub id: String,
    pub qualifier: String,
    pub name: String,
}

/// A call left for the cross-file pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoCall {
    /// Local id of the calling function or method.
    pub caller: String,
    /// Import path for `pkg.F()`; `None` for a bare `F()` not defined in the file.
    pub path: Option<String>,
    pub name: String,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::go`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoFacts {
    /// `go.mod` only: the module path.
    pub module: Option<String>,
    /// Local id of the file node (possibly merged with a same-id type, G2).
    pub file_id: String,
    pub imports: Vec<GoImport>,
    /// Real type declarations: (name, local id).
    pub types: Vec<(String, String)>,
    pub qual_stubs: Vec<QualStub>,
    /// Unqualified type names not declared in the file: (local stub id, name).
    pub bare_stubs: Vec<(String, String)>,
    pub calls: Vec<GoCall>,
    /// Methods: (receiver type name without type arguments, method name, local id).
    pub methods: Vec<(String, String, String)>,
    /// Interface types the file declares: a call through one dispatches at run time.
    pub interfaces: Vec<String>,
    /// Every field of the file's struct types (embedded fields under their type's name).
    pub fields: Vec<GoField>,
    /// Struct types the file declares.
    pub structs: Vec<String>,
    /// Package-level variables: (name, index of their type in [`GoFacts::tys`]).
    pub vars: Vec<(String, u32)>,
    /// Declared results of the file's functions and methods.
    pub returns: Vec<GoReturn>,
    /// Symbolic types of method-call receivers (local type inference, [`infer`]).
    pub tys: Vec<GoTy>,
    /// `recv.M(..)` calls whose receiver has a symbolic type, in call order.
    pub method_calls: Vec<GoMethodCall>,
}

/// A named type as written: `T` or `pkg.T` (pointers and type arguments peeled).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoWritten {
    /// The import qualifier of `pkg.T`.
    pub qual: Option<String>,
    pub name: String,
}

/// A field `owner.name`, with its type when that is a named type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoField {
    pub owner: String,
    pub name: String,
    pub ty: Option<GoWritten>,
    /// Embedded: its methods and fields are promoted.
    pub embedded: bool,
}

/// The declared results of a function or method (`None` where a result is not a named type).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoReturn {
    /// Local id of the function or method.
    pub id: String,
    pub results: Vec<Option<GoWritten>>,
}

/// What a call names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GoCallee {
    /// `F(..)`: a function of the package, or a conversion `T(x)`.
    Bare(String),
    /// `pkg.F(..)` through an import qualifier (or a conversion `pkg.T(x)`).
    Qual(String, String),
    /// `recv.M(..)` on a typed receiver.
    Method(u32, String),
}

/// The symbolic type of an expression in a function body, evaluated by the cross-file pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GoTy {
    /// A type written in the body or the signature, resolved in the file.
    Written(GoWritten),
    /// Result `index` of a call.
    Call { callee: GoCallee, index: u32 },
    /// The declared type of the field `recv.name`.
    Field { recv: u32, name: String },
    /// A package-level variable of the file's package, by name.
    PkgVar(String),
    /// `pkg.Var` through an import qualifier.
    QualVar(String, String),
}

/// `recv.M(..)` with a typed receiver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoMethodCall {
    /// Local id of the calling function or method.
    pub caller: String,
    /// Index of the receiver's type in [`GoFacts::tys`].
    pub recv: u32,
    pub name: String,
    pub line: u32,
    /// Local id of the node the in-file name rule chose (G5), if any.
    pub in_file: Option<String>,
}

/// Predeclared types: unqualified references to them produce nothing.
const BUILTIN_TYPES: &[&str] = &[
    "any",
    "bool",
    "byte",
    "comparable",
    "complex128",
    "complex64",
    "error",
    "float32",
    "float64",
    "int",
    "int16",
    "int32",
    "int64",
    "int8",
    "rune",
    "string",
    "uint",
    "uint16",
    "uint32",
    "uint64",
    "uint8",
    "uintptr",
];

/// Predeclared functions: bare calls to them never resolve.
const BUILTIN_FUNCS: &[&str] = &[
    "append", "cap", "clear", "close", "complex", "copy", "delete", "imag", "len", "make", "max",
    "min", "new", "panic", "print", "println", "real", "recover",
];

fn is_builtin_type(name: &str) -> bool {
    BUILTIN_TYPES.contains(&name)
}

/// What an in-file node is, for the same-file collision rules (G2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    File,
    Type,
    Func,
    Method,
    Receiver,
    Stub,
}

struct Go<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    /// `slug(dir name)` or [`ROOT_DIR`].
    dir: String,
    /// In-file node index and kind by id.
    slots: HashMap<String, (usize, Slot)>,
    /// Types (and receiver nodes) declared so far in the file, by name.
    types: HashMap<String, String>,
    /// Final id of each (name, base id) function or method.
    ids: HashMap<(String, String), String>,
    /// Import qualifier → path (alias, or the path's last segment).
    qualifiers: HashMap<String, String>,
    facts: GoFacts,
    qual_seen: HashSet<String>,
    bare_seen: HashSet<String>,
    /// Calls in walk order, bound after the walk (the binding table needs every node).
    calls: Vec<RawCall>,
    tys: Vec<GoTy>,
    method_calls_raw: Vec<infer::RawGoCall>,
}

/// A call site: `F()` / `x.F()` (`path: None`) or `pkg.F()` through an import.
struct RawCall {
    caller: String,
    path: Option<String>,
    name: String,
    bare: bool,
    line: u32,
}

/// One pending edge of a declaration.
struct Ref {
    target: String,
    relation: &'static str,
    context: Option<&'static str>,
    line: u32,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let dir = dir_slug(cx.rel);
    let file_id = cx.file_id.clone();
    let mut go = Go {
        dir,
        slots: HashMap::from([(file_id.clone(), (0, Slot::File))]),
        types: HashMap::new(),
        ids: HashMap::new(),
        qualifiers: HashMap::new(),
        facts: GoFacts {
            file_id,
            ..GoFacts::default()
        },
        qual_seen: HashSet::new(),
        bare_seen: HashSet::new(),
        calls: Vec::new(),
        tys: Vec::new(),
        method_calls_raw: Vec::new(),
        cx,
    };
    if let Some(tree) = tree {
        let root = tree.root_node();
        go.declare(root);
        go.emit(root);
    }
    go.bind_calls();
    let Go { cx, facts, .. } = go;
    cx.facts.lang = crate::facts::LangFacts::Go(facts);
}

/// `go.mod`: the module path for import resolution and a `package` node (§12).
pub fn extract_mod(cx: &mut Ctx) {
    let text = String::from_utf8_lossy(cx.src);
    let Some(module) = module_path(&text) else {
        return;
    };
    let id = format!("pkg_{}", slug(&module));
    cx.add_def(
        id,
        module.clone(),
        1,
        vec![
            ("type".into(), crate::facts::Attr::Str("package".into())),
            ("ecosystem".into(), crate::facts::Attr::Str("go".into())),
        ],
    );
    cx.facts.lang = crate::facts::LangFacts::Go(GoFacts {
        module: Some(module),
        ..GoFacts::default()
    });
}

/// The `module` directive of a `go.mod` (quoted or bare, `//` comments ignored).
fn module_path(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if let Some(rest) = line.strip_prefix("module") {
            if !rest.starts_with([' ', '\t', '"']) {
                continue;
            }
            let path = rest.trim().trim_matches('"').trim();
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    None
}

/// `slug` of the directory holding `rel`, or [`ROOT_DIR`] at the scan root.
fn dir_slug(rel: &str) -> String {
    let mut parts = rel.rsplit('/');
    parts.next();
    match parts.next() {
        Some(dir) => slug(dir),
        None => ROOT_DIR.to_string(),
    }
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

impl<'a> Go<'_, 'a> {
    fn text(&self, node: TsNode) -> &'a str {
        self.cx.text(node)
    }

    fn type_id(&self, name: &str) -> String {
        Ctx::child_id(&self.dir, name)
    }

    /// Receiver type text without `*` and the receiver's type parameter names.
    fn receiver(&self, method: TsNode<'_>) -> Option<(String, Vec<String>)> {
        let list = method.child_by_field_name("receiver")?;
        let param = named_children(list)
            .into_iter()
            .find(|n| n.kind() == "parameter_declaration")?;
        let mut ty = param.child_by_field_name("type")?;
        while ty.kind() == "pointer_type" || ty.kind() == "parenthesized_type" {
            ty = ty.named_child(0)?;
        }
        let mut params = Vec::new();
        if ty.kind() == "generic_type" {
            if let Some(args) = ty.child_by_field_name("type_arguments") {
                collect_identifiers(args, self.cx.src, &mut params);
            }
        }
        Some((self.text(ty).to_string(), params))
    }

    /// First pass over top-level declarations: the final ids of functions and methods, with
    /// case variants disambiguated.
    fn declare(&mut self, root: TsNode) {
        // (name, base id) of every callable, in source order.
        let mut order: Vec<(String, String)> = Vec::new();
        for decl in named_children(root) {
            match decl.kind() {
                "type_declaration" => {
                    for spec in named_children(decl) {
                        if spec.kind() != "type_spec" {
                            continue;
                        }
                        let Some(name) = spec.child_by_field_name("name") else {
                            continue;
                        };
                        let name = self.text(name);
                        let id = self.type_id(name);
                        order.push((name.to_string(), id.clone()));
                        if let Some(iface) = spec
                            .child_by_field_name("type")
                            .filter(|t| t.kind() == "interface_type")
                        {
                            for elem in named_children(iface) {
                                if let Some(m) = (elem.kind() == "method_elem")
                                    .then(|| elem.child_by_field_name("name"))
                                    .flatten()
                                {
                                    let m = self.text(m);
                                    order.push((m.to_string(), Ctx::child_id(&id, m)));
                                }
                            }
                        }
                    }
                }
                "function_declaration" => {
                    if let Some(name) = decl.child_by_field_name("name") {
                        let name = self.text(name);
                        order.push((name.to_string(), Ctx::child_id(&self.cx.file_id, name)));
                    }
                }
                "method_declaration" => {
                    let (Some(name), Some((recv, _))) =
                        (decl.child_by_field_name("name"), self.receiver(decl))
                    else {
                        continue;
                    };
                    let name = self.text(name);
                    let owner = Ctx::child_id(&self.dir, &recv);
                    order.push((name.to_string(), Ctx::child_id(&owner, name)));
                }
                _ => {}
            }
        }
        let mut names_by_id: HashMap<&str, Vec<&str>> = HashMap::new();
        for (name, base) in &order {
            let names = names_by_id.entry(base.as_str()).or_default();
            if !names.contains(&name.as_str()) {
                names.push(name);
            }
        }
        for (name, base) in &order {
            let names = &names_by_id[base.as_str()];
            let id = if names.len() > 1 {
                variant_id(base, name, names)
            } else {
                base.clone()
            };
            self.ids.insert((name.clone(), base.clone()), id);
        }
    }

    /// The final id of a function or method declared under `base` (see [`variant_id`]).
    fn final_id(&self, name: &str, base: &str) -> String {
        self.ids
            .get(&(name.to_string(), base.to_string()))
            .cloned()
            .unwrap_or_else(|| base.to_string())
    }

    /// Insert a node subject to the same-file collision rules. Returns false when an existing
    /// node kept the id.
    fn put(&mut self, id: String, label: String, line: u32, slot: Slot) -> bool {
        if let Some(&(idx, existing)) = self.slots.get(&id) {
            // A type (or a function named `_`) replaces a same-id file node in place (G2); a
            // function replaces a same-id method. Every other duplicate keeps the first node.
            let replace = matches!(
                (existing, slot),
                (Slot::File, Slot::Type | Slot::Func) | (Slot::Method, Slot::Func)
            );
            if replace {
                let node = &mut self.cx.facts.nodes[idx];
                node.label = label;
                node.loc = crate::facts::Loc::Line(line);
                node.kind = NodeKind::Def;
                self.slots.insert(id, (idx, slot));
            }
            return replace;
        }
        let idx = self.cx.facts.nodes.len();
        self.cx.add_def(id.clone(), label, line, vec![]);
        self.slots.insert(id, (idx, slot));
        true
    }

    fn stub(&mut self, id: &str, label: &str) {
        if !self.slots.contains_key(id) {
            let idx = self.cx.facts.nodes.len();
            self.cx.add_stub(id.to_string(), label.to_string());
            self.slots.insert(id.to_string(), (idx, Slot::Stub));
        }
    }

    fn emit(&mut self, root: TsNode) {
        for decl in named_children(root) {
            match decl.kind() {
                "import_declaration" => self.imports(decl),
                "type_declaration" => {
                    for spec in named_children(decl) {
                        if spec.kind() == "type_spec" {
                            self.type_spec(spec);
                        }
                    }
                }
                "function_declaration" => self.function(decl),
                "method_declaration" => self.method(decl),
                "var_declaration" => self.package_vars(decl),
                _ => {}
            }
        }
    }

    fn imports(&mut self, decl: TsNode) {
        let mut specs = Vec::new();
        for child in named_children(decl) {
            match child.kind() {
                "import_spec" => specs.push(child),
                "import_spec_list" => specs.extend(
                    named_children(child)
                        .into_iter()
                        .filter(|n| n.kind() == "import_spec"),
                ),
                _ => {}
            }
        }
        for spec in specs {
            let Some(path) = spec.child_by_field_name("path") else {
                continue;
            };
            let path = self
                .text(path)
                .trim_matches(|c| c == '"' || c == '`')
                .to_string();
            if path.is_empty() {
                continue;
            }
            let alias = spec
                .child_by_field_name("name")
                .map(|n| self.text(n).to_string());
            let qualifier = match alias.as_deref() {
                Some("." | "_") => None,
                Some(a) => Some(a.to_string()),
                None => path.rsplit('/').next().map(str::to_string),
            };
            if let Some(q) = qualifier {
                self.qualifiers.entry(q).or_insert_with(|| path.clone());
            }
            self.facts.imports.push(GoImport {
                path,
                alias,
                line: Ctx::line(spec),
            });
        }
    }

    fn type_spec(&mut self, spec: TsNode) {
        let Some(name) = spec.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name);
        let id = self.type_id(name);
        let line = Ctx::line(spec);
        let previous = self.slots.get(&id).map(|&(_, s)| s);
        let placed = self.put(id.clone(), name.to_string(), line, Slot::Type);
        // Only types declared so far bind in the file; later ones resolve as stubs.
        self.types.insert(name.to_string(), id.clone());
        match previous {
            // An earlier `q.T` stub already holds this id (`ktesting.TContext` in dir
            // `ktesting`): the stub keeps the node and nothing resolves it.
            Some(Slot::Stub) => {
                self.facts.qual_stubs.retain(|q| q.id != id);
                self.facts.bare_stubs.retain(|(s, _)| *s != id);
            }
            // The type owns its node (or its own receiver node already did). A case variant
            // (`type manager` then `type Manager`) folds into the first and is not findable
            // under its own name.
            _ if placed || previous == Some(Slot::Receiver) => {
                self.facts.types.push((name.to_string(), id.clone()));
            }
            _ => {}
        }
        let file_id = self.cx.file_id.clone();
        self.cx.add_edge(&file_id, &id, "contains", line, None);

        let mut scope = Vec::new();
        if let Some(tp) = spec.child_by_field_name("type_parameters") {
            type_param_names(tp, self.cx.src, &mut scope);
        }
        let Some(body) = spec.child_by_field_name("type") else {
            return;
        };
        let mut refs = Vec::new();
        match body.kind() {
            "struct_type" => {
                self.facts.structs.push(name.to_string());
                let Some(list) = named_children(body)
                    .into_iter()
                    .find(|n| n.kind() == "field_declaration_list")
                else {
                    return;
                };
                for field in named_children(list) {
                    if field.kind() != "field_declaration" {
                        continue;
                    }
                    let Some(ty) = field.child_by_field_name("type") else {
                        continue;
                    };
                    let fline = Ctx::line(field);
                    let w = self.written_ty(ty, &scope);
                    let mut c = field.walk();
                    let names: Vec<String> = field
                        .children_by_field_name("name", &mut c)
                        .map(|n| self.text(n).to_string())
                        .collect();
                    // An embedded field is named by its type.
                    let embedded = names.is_empty();
                    let names = if embedded {
                        vec![w.as_ref().map_or_else(String::new, |w| w.name.clone())]
                    } else {
                        names
                    };
                    for fname in names {
                        self.facts.fields.push(GoField {
                            owner: name.to_string(),
                            name: fname,
                            ty: w.clone(),
                            embedded,
                        });
                    }
                    if field.child_by_field_name("name").is_none() {
                        self.embed(ty, fline, &scope, &mut refs);
                    } else {
                        self.type_refs(ty, Some("field"), fline, &scope, &mut refs);
                    }
                }
            }
            "interface_type" => {
                self.facts.interfaces.push(name.to_string());
                let mut methods = Vec::new();
                for elem in named_children(body) {
                    match elem.kind() {
                        "method_elem" => methods.push(elem),
                        "type_elem" => {
                            let parts = named_children(elem);
                            let eline = Ctx::line(elem);
                            if parts.len() == 1 && parts[0].kind() != "negated_type" {
                                self.embed(parts[0], eline, &scope, &mut refs);
                            } else {
                                for p in parts {
                                    if p.kind() != "negated_type" {
                                        self.type_refs(
                                            p,
                                            Some("type_constraint"),
                                            eline,
                                            &scope,
                                            &mut refs,
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                self.flush(&id, refs);
                for m in methods {
                    self.interface_method(&id, m, &scope);
                }
                return;
            }
            _ => {}
        }
        self.flush(&id, refs);
    }

    /// An embedded field or interface element: `embeds` to the base type, `generic_arg` for its
    /// type arguments.
    fn embed(&mut self, mut ty: TsNode, line: u32, scope: &[String], refs: &mut Vec<Ref>) {
        while ty.kind() == "pointer_type" || ty.kind() == "parenthesized_type" {
            match ty.named_child(0) {
                Some(inner) => ty = inner,
                None => return,
            }
        }
        let (base, args) = if ty.kind() == "generic_type" {
            (
                ty.child_by_field_name("type"),
                ty.child_by_field_name("type_arguments"),
            )
        } else {
            (Some(ty), None)
        };
        if let Some(target) = base.and_then(|b| self.type_target(b, scope)) {
            refs.push(Ref {
                target,
                relation: "embeds",
                context: None,
                line,
            });
        }
        if let Some(args) = args {
            self.type_refs(args, Some("generic_arg"), line, scope, refs);
        }
    }

    /// Emit a declaration's pending edges (an `embeds` link absorbs a `references` to the same
    /// type in the build, §8.2).
    fn flush(&mut self, source: &str, refs: Vec<Ref>) {
        for r in refs {
            if r.target == source {
                continue;
            }
            self.cx
                .add_edge(source, &r.target, r.relation, r.line, r.context);
        }
    }

    /// The node a named type resolves to in this file, creating stubs as needed.
    fn type_target(&mut self, node: TsNode, scope: &[String]) -> Option<String> {
        match node.kind() {
            "type_identifier" => {
                let name = self.text(node);
                // Type parameters are plain stubs too (and bind by label like any other).
                let param = scope.iter().any(|s| s == name);
                if !param {
                    if let Some(id) = self.types.get(name) {
                        return Some(id.clone());
                    }
                    if is_builtin_type(name) {
                        return None;
                    }
                }
                let id = slug(name);
                if id.is_empty() {
                    return None;
                }
                self.stub(&id, name);
                if self.bare_seen.insert(id.clone()) {
                    self.facts.bare_stubs.push((id.clone(), name.to_string()));
                }
                Some(id)
            }
            "qualified_type" => {
                let q = self.text(node.child_by_field_name("package")?);
                let name = self.text(node.child_by_field_name("name")?);
                let id = Ctx::child_id(&slug(q), name);
                self.stub(&id, &format!("{q}.{name}"));
                if self.qual_seen.insert(id.clone()) {
                    self.facts.qual_stubs.push(QualStub {
                        id: id.clone(),
                        qualifier: q.to_string(),
                        name: name.to_string(),
                    });
                }
                Some(id)
            }
            _ => None,
        }
    }

    /// Every named type inside a type expression, with `context`; type arguments of a generic
    /// type get `generic_arg`. Variadic parameters and constraint bounds produce nothing.
    fn type_refs(
        &mut self,
        node: TsNode,
        context: Option<&'static str>,
        line: u32,
        scope: &[String],
        refs: &mut Vec<Ref>,
    ) {
        match node.kind() {
            "type_identifier" | "qualified_type" => {
                if let Some(target) = self.type_target(node, scope) {
                    refs.push(Ref {
                        target,
                        relation: "references",
                        context,
                        line,
                    });
                }
            }
            "generic_type" => {
                if let Some(base) = node.child_by_field_name("type") {
                    self.type_refs(base, context, line, scope, refs);
                }
                if let Some(args) = node.child_by_field_name("type_arguments") {
                    self.type_refs(args, Some("generic_arg"), line, scope, refs);
                }
            }
            "array_type" | "slice_type" => {
                if let Some(e) = node.child_by_field_name("element") {
                    self.type_refs(e, context, line, scope, refs);
                }
            }
            "map_type" => {
                for f in ["key", "value"] {
                    if let Some(e) = node.child_by_field_name(f) {
                        self.type_refs(e, context, line, scope, refs);
                    }
                }
            }
            "channel_type" => {
                if let Some(e) = node.child_by_field_name("value") {
                    self.type_refs(e, context, line, scope, refs);
                }
            }
            "function_type" => {
                if let Some(p) = node.child_by_field_name("parameters") {
                    self.params(p, context, line, scope, refs);
                }
                if let Some(r) = node.child_by_field_name("result") {
                    self.result(r, context, line, scope, refs);
                }
            }
            "struct_type" => {
                for list in named_children(node) {
                    for field in named_children(list) {
                        if let Some(ty) = field.child_by_field_name("type") {
                            self.type_refs(ty, context, line, scope, refs);
                        }
                    }
                }
            }
            "pointer_type" | "parenthesized_type" | "type_arguments" | "type_elem" => {
                for c in named_children(node) {
                    self.type_refs(c, context, line, scope, refs);
                }
            }
            _ => {}
        }
    }

    fn params(
        &mut self,
        list: TsNode,
        context: Option<&'static str>,
        line: u32,
        scope: &[String],
        refs: &mut Vec<Ref>,
    ) {
        for p in named_children(list) {
            if p.kind() == "parameter_declaration" {
                if let Some(ty) = p.child_by_field_name("type") {
                    self.type_refs(ty, context, line, scope, refs);
                }
            }
        }
    }

    fn result(
        &mut self,
        result: TsNode,
        context: Option<&'static str>,
        line: u32,
        scope: &[String],
        refs: &mut Vec<Ref>,
    ) {
        if result.kind() == "parameter_list" {
            self.params(result, context, line, scope, refs);
        } else {
            self.type_refs(result, context, line, scope, refs);
        }
    }

    fn signature(&mut self, decl: TsNode, scope: &[String], line: u32) -> Vec<Ref> {
        let mut refs = Vec::new();
        if let Some(p) = decl.child_by_field_name("parameters") {
            self.params(p, Some("parameter_type"), line, scope, &mut refs);
        }
        if let Some(r) = decl.child_by_field_name("result") {
            self.result(r, Some("return_type"), line, scope, &mut refs);
        }
        refs
    }

    fn interface_method(&mut self, iface: &str, elem: TsNode, scope: &[String]) {
        let Some(name) = elem.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name);
        let id = self.final_id(name, &Ctx::child_id(iface, name));
        let line = Ctx::line(elem);
        self.put(id.clone(), format!(".{name}()"), line, Slot::Method);
        self.cx.add_edge(iface, &id, "method", line, None);
        let refs = self.signature(elem, scope, line);
        self.flush(&id, refs);
    }

    fn function(&mut self, decl: TsNode) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name);
        let id = self.final_id(name, &Ctx::child_id(&self.cx.file_id, name));
        let line = Ctx::line(decl);
        let kept = self.put(id.clone(), format!("{name}()"), line, Slot::Func);
        let file_id = self.cx.file_id.clone();
        self.cx.add_edge(&file_id, &id, "contains", line, None);
        let mut scope = Vec::new();
        if let Some(tp) = decl.child_by_field_name("type_parameters") {
            type_param_names(tp, self.cx.src, &mut scope);
        }
        let refs = self.signature(decl, &scope, line);
        self.flush(&id, refs);
        self.record_results(&id, decl, &scope);
        if let Some(body) = decl.child_by_field_name("body") {
            self.walk_body(&id, decl, body, &scope, None, kept);
        }
    }

    fn method(&mut self, decl: TsNode) {
        let (Some(name), Some((recv, scope))) =
            (decl.child_by_field_name("name"), self.receiver(decl))
        else {
            return;
        };
        let name = self.text(name);
        let owner = Ctx::child_id(&self.dir, &recv);
        let id = self.final_id(name, &Ctx::child_id(&owner, name));
        let line = Ctx::line(decl);
        if !self.slots.contains_key(&owner) {
            self.put(owner.clone(), recv.clone(), line, Slot::Receiver);
            // The receiver node binds later references to its name in this file.
            self.types
                .entry(recv.clone())
                .or_insert_with(|| owner.clone());
        }
        let kept = self.put(id.clone(), format!(".{name}()"), line, Slot::Method);
        self.cx.add_edge(&owner, &id, "method", line, None);
        let refs = self.signature(decl, &scope, line);
        self.flush(&id, refs);
        let base = recv.split('[').next().unwrap_or(&recv).to_string();
        self.facts
            .methods
            .push((base, name.to_string(), id.clone()));
        self.record_results(&id, decl, &scope);
        if let Some(body) = decl.child_by_field_name("body") {
            self.walk_body(&id, decl, body, &scope, Some(&recv), kept);
        }
    }

    /// The declared results of function or method `id`, when any is a named type.
    fn record_results(&mut self, id: &str, decl: TsNode, scope: &[String]) {
        let results = self.results(decl, scope);
        if results.iter().any(Option::is_some) {
            self.facts.returns.push(GoReturn {
                id: id.to_string(),
                results,
            });
        }
    }

    fn call(&mut self, caller: &str, function: TsNode, line: u32) {
        let (name, path, bare) = match function.kind() {
            "identifier" => (self.text(function), None, true),
            "selector_expression" => {
                let (Some(operand), Some(field)) = (
                    function.child_by_field_name("operand"),
                    function.child_by_field_name("field"),
                ) else {
                    return;
                };
                let path = (operand.kind() == "identifier")
                    .then(|| self.qualifiers.get(self.text(operand)).cloned())
                    .flatten();
                (self.text(field), path, false)
            }
            // `pkg.F[T](..)`: an explicitly instantiated generic function of an imported
            // package (dijkstral-native; Graphify drops the call).
            "index_expression" | "type_instantiation_expression" => {
                let Some((q, name)) = self.instantiated(function) else {
                    return;
                };
                let Some(path) = q.and_then(|q| self.qualifiers.get(q).cloned()) else {
                    return;
                };
                (name, Some(path), false)
            }
            _ => return,
        };
        self.calls.push(RawCall {
            caller: caller.to_string(),
            path,
            name: name.to_string(),
            bare,
            line,
        });
    }

    /// The generic function an explicit instantiation names: `F[T]` / `pkg.F[T]` as
    /// (qualifier, name). An index into anything else (`fns[i]`) is not one when the operand is
    /// a local value, which callers check.
    fn instantiated(&self, n: TsNode) -> Option<(Option<&'a str>, &'a str)> {
        let target = match n.kind() {
            "index_expression" => n.child_by_field_name("operand")?,
            "type_instantiation_expression" => n.child_by_field_name("type")?,
            _ => return None,
        };
        match target.kind() {
            "identifier" | "type_identifier" => Some((None, self.text(target))),
            "selector_expression" => {
                let q = target.child_by_field_name("operand")?;
                (q.kind() == "identifier").then_some(())?;
                Some((
                    Some(self.text(q)),
                    self.text(target.child_by_field_name("field")?),
                ))
            }
            "qualified_type" => Some((
                Some(self.text(target.child_by_field_name("package")?)),
                self.text(target.child_by_field_name("name")?),
            )),
            _ => None,
        }
    }

    /// Bind the file's calls once every node exists. `pkg.F()` goes to the cross-file pass; any
    /// other name binds to the last node of the file carrying it (functions, methods, types,
    /// receiver nodes and type stubs alike; G5), and a bare name that is not in the file goes to
    /// the cross-file pass. One edge per (caller, callee), first site kept.
    fn bind_calls(&mut self) {
        let mut table: HashMap<&str, &str> = HashMap::new();
        for n in &self.cx.facts.nodes {
            if n.kind == NodeKind::File || n.kind == NodeKind::External {
                continue;
            }
            table.insert(call_name(&n.label), n.id.as_str());
        }
        let mut local: Vec<(String, String, u32)> = Vec::new();
        let mut seen: HashSet<(&str, &str)> = HashSet::new();
        let mut cross_seen: HashSet<(&str, Option<&str>, &str)> = HashSet::new();
        for c in &self.calls {
            if is_unbound_name(&c.name, c.bare) {
                continue;
            }
            if c.path.is_none() {
                if let Some(&target) = table.get(c.name.as_str()) {
                    if target != c.caller && seen.insert((c.caller.as_str(), target)) {
                        local.push((c.caller.clone(), target.to_string(), c.line));
                    }
                    continue;
                }
                if !c.bare {
                    continue;
                }
            }
            if cross_seen.insert((c.caller.as_str(), c.path.as_deref(), c.name.as_str())) {
                self.facts.calls.push(GoCall {
                    caller: c.caller.clone(),
                    path: c.path.clone(),
                    name: c.name.clone(),
                    line: c.line,
                });
            }
        }
        let raw = std::mem::take(&mut self.method_calls_raw);
        let tys = std::mem::take(&mut self.tys);
        let (tys, method_calls) = infer::finish_inference(raw, tys, &mut self.facts.vars, &table);
        self.facts.tys = tys;
        self.facts.method_calls = method_calls;
        for (caller, callee, line) in local {
            self.cx
                .add_edge(&caller, &callee, "calls", line, Some("call"));
        }
    }
}

/// The name a node answers to in a call: `.M()` → `M`, `F()` → `F`, `T` → `T`.
fn call_name(label: &str) -> &str {
    let l = label.strip_prefix('.').unwrap_or(label);
    l.strip_suffix("()").unwrap_or(l)
}

/// Names a call never binds to: Go's predeclared functions when called bare (`close(ch)`; a
/// method `x.close()` still binds), and the builtin-like names Graphify skips (compare Q15).
pub(crate) fn is_unbound_name(name: &str, bare: bool) -> bool {
    (bare && BUILTIN_FUNCS.contains(&name)) || DENIED_CALLS.binary_search(&name).is_ok()
}

/// Sorted. Python builtins and JavaScript globals, minus the names kubernetes shows Graphify
/// binding (`format`, `compile`, `slice`, `eval`, `exec`, `object`), plus the type-like names
/// it never binds (`Bool`, `Int`, `Int32`, `Int64`, `Float`).
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
    "pow",
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

/// Distinct names that slug to the same id in one file (`Set`/`set`, `run`/`run_`) are told
/// apart by `_<sha1(name)[:6]>`, except that an exported name whose only difference from another
/// is the capital first letter keeps the plain id (`Set` stays `…_set`, `set` becomes
/// `…_set_65c10d`). Observed on 875 kubernetes groups.
fn variant_id(base: &str, name: &str, names: &[&str]) -> String {
    let exported_twin = name.chars().next().is_some_and(char::is_uppercase)
        && names.iter().any(|other| {
            let mut c = other.chars();
            *other != name
                && c.next().is_some_and(char::is_lowercase)
                && other
                    .chars()
                    .next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    == Some(name.to_string())
        });
    if exported_twin {
        return base.to_string();
    }
    let digest = sha1_smol::Sha1::from(name.as_bytes()).digest().to_string();
    format!("{base}_{}", &digest[..6])
}

fn type_param_names(list: TsNode, src: &[u8], out: &mut Vec<String>) {
    for decl in named_children(list) {
        if decl.kind() != "type_parameter_declaration" {
            continue;
        }
        let mut c = decl.walk();
        for name in decl.children_by_field_name("name", &mut c) {
            if let Ok(t) = name.utf8_text(src) {
                out.push(t.to_string());
            }
        }
    }
}

fn collect_identifiers(node: TsNode, src: &[u8], out: &mut Vec<String>) {
    if matches!(node.kind(), "type_identifier" | "identifier") {
        if let Ok(t) = node.utf8_text(src) {
            out.push(t.to_string());
        }
        return;
    }
    for c in named_children(node) {
        collect_identifiers(c, src, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_file;
    use crate::facts::LangFacts;
    use crate::lang::Lang;

    fn facts(rel: &str, src: &str) -> crate::facts::FileFacts {
        extract_file(Lang::Go, rel, src.as_bytes())
    }

    fn edges(f: &crate::facts::FileFacts) -> Vec<String> {
        f.edges
            .iter()
            .map(|e| {
                format!(
                    "{} {} {} {} L{}",
                    e.source,
                    e.relation,
                    e.target,
                    e.context.as_deref().unwrap_or("-"),
                    match e.loc {
                        crate::facts::Loc::Line(n) => n,
                        _ => 0,
                    }
                )
            })
            .collect()
    }

    #[test]
    fn module_directive_is_parsed() {
        assert_eq!(
            module_path("// c\nmodule example.com/shop // x\n"),
            Some("example.com/shop".into())
        );
        assert_eq!(module_path("module \"a/b\"\n"), Some("a/b".into()));
        assert_eq!(module_path("modules x\n"), None);
    }

    #[test]
    fn type_named_like_its_file_replaces_the_file_node() {
        let f = facts("x/x.go", "package x\n\nfunc F() {}\n\ntype X struct{}\n");
        assert_eq!(f.nodes[0].id, "x_x");
        assert_eq!(f.nodes[0].label, "X");
        assert_eq!(f.nodes[0].kind, NodeKind::Def);
        assert!(edges(&f).contains(&"x_x contains x_x - L5".to_string()));
    }

    #[test]
    fn receiver_declared_later_or_elsewhere_gets_a_receiver_node() {
        let f = facts(
            "q/q.go",
            "package q\n\nfunc (m Mixed) Run() {}\n\ntype Mixed struct{}\n\nfunc (s *S[T]) Put(v T) {}\n",
        );
        let ids: Vec<_> = f
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect();
        assert!(ids.contains(&("q_mixed", "Mixed")));
        assert!(ids.contains(&("q_s_t", "S[T]")));
        assert!(ids.contains(&("q_s_t_put", ".Put()")));
        assert!(ids.contains(&("t", "T")));
        // The receiver node keeps the method's line even though the type is declared later.
        let mixed = f.nodes.iter().find(|n| n.id == "q_mixed").unwrap();
        assert_eq!(mixed.loc, crate::facts::Loc::Line(3));
    }

    #[test]
    fn references_cover_composite_types_but_not_variadics_or_self() {
        let f = facts(
            "q/a.go",
            "package q\n\nimport \"io\"\n\ntype A struct{}\ntype S struct {\n\tio.Reader\n\tNext *S\n\tM map[A]func(B) []C\n}\n\nfunc V(xs ...A) {}\nfunc W(a A) (D, error) { return D{}, nil }\n",
        );
        let e = edges(&f);
        assert!(e.contains(&"q_s embeds io_reader - L7".to_string()));
        assert!(e.contains(&"q_s references q_a field L9".to_string()));
        assert!(e.contains(&"q_s references b field L9".to_string()));
        assert!(e.contains(&"q_s references c field L9".to_string()));
        assert!(!e.iter().any(|x| x.starts_with("q_s references q_s")));
        assert!(!e.iter().any(|x| x.starts_with("q_a_v references")));
        assert!(e.contains(&"q_a_w references q_a parameter_type L13".to_string()));
        assert!(e.contains(&"q_a_w references d return_type L13".to_string()));
        let LangFacts::Go(g) = &f.lang else { panic!() };
        assert_eq!(g.qual_stubs[0].id, "io_reader");
        assert_eq!(
            g.bare_stubs,
            vec![
                ("b".to_string(), "B".to_string()),
                ("c".into(), "C".into()),
                ("d".into(), "D".into())
            ]
        );
    }

    #[test]
    fn same_file_calls_bind_by_name_last_definition_wins() {
        let f = facts(
            "q/a.go",
            "package q\n\nimport \"ex.com/z/lib\"\n\ntype A struct{}\ntype B struct{}\n\nfunc (a A) Do() {}\nfunc (b B) Do() {}\nfunc (a A) Self() {\n\ta.Self()\n\ta.Do()\n\tlen(nil)\n\tlib.Do()\n\tOther()\n\ta.Do()\n}\n",
        );
        let e = edges(&f);
        assert!(e.contains(&"q_a_self calls q_b_do call L12".to_string()));
        assert_eq!(e.iter().filter(|x| x.contains(" calls ")).count(), 1);
        let LangFacts::Go(g) = &f.lang else { panic!() };
        let calls: Vec<_> = g
            .calls
            .iter()
            .map(|c| (c.path.as_deref(), c.name.as_str()))
            .collect();
        assert_eq!(calls, vec![(Some("ex.com/z/lib"), "Do"), (None, "Other")]);
    }

    #[test]
    fn root_level_types_use_the_root_placeholder() {
        let f = facts(
            "root.go",
            "package m\n\ntype R struct{}\n\nfunc (r R) Go() {}\n",
        );
        let ids: Vec<_> = f.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["root", "@root_r", "@root_r_go"]);
    }
}

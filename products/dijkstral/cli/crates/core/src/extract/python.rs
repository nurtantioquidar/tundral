//! Python (§9) extraction. See `docs/graphify-format.md`.
//!
//! One walk over the tree emits the file's own structure (definitions, `contains`/`method`
//! edges, rationale) and records everything whose meaning depends on other files in `PyFacts`:
//! imports, type mentions (annotations, bases, decorators), call sites, functions passed as
//! values, and mentions of imported names. `crate::resolve::python` turns those into edges once
//! every file is known, so in-file and cross-file names resolve through one set of rules.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, rationale_label, Ctx};
use crate::facts::{Attr, LangFacts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DefKind {
    /// Top-level or nested function (label `name()`).
    Function,
    /// Function directly in a class body (label `.name()`).
    Method,
    Class,
}

/// One emitted definition, in pre-order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyDef {
    /// Id as emitted by this file (before cross-file collision renaming).
    pub id: String,
    pub name: String,
    pub kind: DefKind,
    /// Enclosing definition (index into `PyFacts::defs`); `None` at module level.
    pub parent: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ImportKind {
    /// `import a.b.c` (`alias: None`) or `import a.b.c as x`.
    Module { path: String, alias: Option<String> },
    /// `from <level dots><module> import names` / `import *`.
    From {
        level: u32,
        module: String,
        /// `(name, alias)`.
        names: Vec<(String, Option<String>)>,
        star: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyImport {
    pub line: u32,
    /// Innermost enclosing function for a function-level import; `None` at module level
    /// (including `if`/`try` blocks such as `if TYPE_CHECKING:`).
    pub scope: Option<u32>,
    pub kind: ImportKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefKind {
    /// Parameter annotation (`references`, context `parameter_type`).
    Param,
    /// Return annotation (`return_type`).
    Return,
    /// Inside a subscript or call of an annotation (`generic_arg`).
    Generic,
    /// `@decorator` (`references`, context `decorator`).
    Decorator,
    /// Bare-identifier base class (`inherits`).
    Base,
    /// Class-level annotation `name: T [= value]` (`references`, context `field`). Graphify
    /// emits nothing for class attributes; dijkstral records the field's type like Rust and Go.
    Field,
}

/// A name used as a type, base or decorator by definition `source`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeRef {
    pub source: u32,
    pub kind: RefKind,
    /// Last component as written (`Foo` in `m.Foo`).
    pub name: String,
    /// Dotted qualifier before the name (`m` in `m.Foo`), when written qualified.
    pub qual: Option<String>,
    pub line: u32,
    /// From a source Graphify ignores (a class field, a string annotation): emitted only when
    /// it resolves to a definition, never as a stub.
    pub resolved_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CallKind {
    /// `f()`.
    Name,
    /// `self.m()` inside class `defs[class]`.
    SelfMethod(u32),
    /// `super().m()` inside class `defs[class]`.
    Super(u32),
    /// `R.m()` where `R` is a dotted name that is not a local variable (module, class…).
    Attr(String),
    /// `x.m()` where `x`'s class is known from an annotation or `x = Cls(...)`; the class name
    /// as written.
    Typed(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyCall {
    /// Nearest enclosing function.
    pub caller: u32,
    pub kind: CallKind,
    /// Called name (`f`, or `m` in `x.m()`).
    pub name: String,
    pub line: u32,
}

/// A function name used as a value inside function `caller` (`indirect_call`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyIndirect {
    /// Enclosing function; `None` at module level (the file is the source).
    pub caller: Option<u32>,
    pub name: String,
    /// `argument`, `return`, `collection`, `assignment` or `getattr`.
    pub context: String,
    pub line: u32,
}

/// First mention of a `from … import` name inside a top-level function or class (`uses`), per
/// innermost function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyMention {
    /// Top-level function or class the `uses` edge starts from.
    pub scope: u32,
    /// Innermost enclosing function (where function-level imports are looked up).
    pub at: Option<u32>,
    pub name: String,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::python`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PyFacts {
    pub defs: Vec<PyDef>,
    pub imports: Vec<PyImport>,
    pub refs: Vec<TypeRef>,
    pub calls: Vec<PyCall>,
    pub indirect: Vec<PyIndirect>,
    pub mentions: Vec<PyMention>,
}

/// Annotation names that never produce a `references` edge (builtins and typing constructs).
static EXCLUDED_TYPES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "int",
        "float",
        "str",
        "bool",
        "bytes",
        "list",
        "dict",
        "set",
        "tuple",
        "type",
        "object",
        "None",
        "complex",
        "frozenset",
        "bytearray",
        "Ellipsis",
        "Optional",
        "Union",
        "List",
        "Dict",
        "Set",
        "Tuple",
        "FrozenSet",
        "Type",
        "Callable",
        "Iterable",
        "Iterator",
        "Sequence",
        "Mapping",
        "MutableMapping",
        "Generator",
        "AsyncGenerator",
        "AsyncIterator",
        "AsyncIterable",
        "Awaitable",
        "Coroutine",
        "ContextManager",
        "AsyncContextManager",
        "Literal",
        "TypeVar",
        "ClassVar",
        "Final",
        "Annotated",
        "ParamSpec",
        "Concatenate",
    ]
    .into_iter()
    .collect()
});

/// Decorators that never produce a `references` edge (matched on the last name component).
static EXCLUDED_DECORATORS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "property",
        "staticmethod",
        "classmethod",
        "abstractmethod",
        "abstractproperty",
        "cached_property",
        "dataclass",
        "wraps",
        "lru_cache",
        "cache",
        "total_ordering",
        "singledispatch",
        "singledispatchmethod",
        "contextmanager",
        "asynccontextmanager",
        "overload",
        "override",
        "final",
        "runtime_checkable",
    ]
    .into_iter()
    .collect()
});

/// Comment markers that make a `# MARKER:` comment a rationale node (§9.2).
const COMMENT_MARKERS: [&str; 7] = [
    "NOTE",
    "WHY",
    "TODO",
    "FIXME",
    "HACK",
    "IMPORTANT",
    "RATIONALE",
];

/// Expression nesting deeper than this is not descended (generated code, giant literals).
const MAX_DEPTH: u32 = 1_000;

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let Some(tree) = tree else {
        cx.facts.lang = LangFacts::Python(PyFacts::default());
        return;
    };
    let root = tree.root_node();
    let mut w = Walker {
        from_names: module_from_names(cx, root),
        plain_names: plain_def_names(cx, root),
        cx,
        facts: PyFacts::default(),
        docstrings: Vec::new(),
        comments: Vec::new(),
        mention_seen: HashSet::new(),
        locals: Vec::new(),
        opaque_strings: 0,
        field_annotation: false,
    };
    let file_id = w.cx.file_id.clone();
    w.docstring(root, None);
    let frame = Frame {
        caller: None,
        self_class: None,
        parent_def: None,
        parent_is_class: false,
        in_function: false,
        uses_scope: None,
        plain_scope: true,
    };
    w.visit_children(root, &file_id, frame, 0);
    w.finish();
}

/// Names bound by module-level `from … import` statements (aliases where given), known before
/// the walk so a mention above its import still counts. Only `from … import` names can become
/// `uses` edges; class- and function-level ones are added as the walk reaches them.
fn module_from_names(cx: &Ctx, root: TsNode) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let mut cur = node.walk();
        for child in node.named_children(&mut cur) {
            match child.kind() {
                "import_from_statement" => {
                    let mut c2 = child.walk();
                    for name in child.children_by_field_name("name", &mut c2) {
                        let bound = match name.kind() {
                            "aliased_import" => name.child_by_field_name("alias"),
                            _ => Some(name),
                        };
                        if let Some(b) = bound {
                            out.insert(cx.text(b).to_string());
                        }
                    }
                }
                "function_definition" | "class_definition" | "decorated_definition" => {}
                "if_statement"
                | "try_statement"
                | "with_statement"
                | "block"
                | "else_clause"
                | "elif_clause"
                | "except_clause"
                | "finally_clause"
                | "except_group_clause" => stack.push(child),
                _ => {}
            }
        }
    }
    out
}

/// An undecorated `def`/`class` statement directly in the module or a class body (not inside
/// `if`/`try` blocks).
fn plain_position(node: TsNode) -> bool {
    match node.parent() {
        Some(p) if p.kind() == "module" => true,
        Some(p) if p.kind() == "block" => {
            p.parent().is_some_and(|c| c.kind() == "class_definition")
        }
        _ => false,
    }
}

/// Ids of the plain functions and methods (undecorated, written directly in the module or in
/// the body of a plain class), with every name that produces each id. Only these clashes are
/// disambiguated (`Walker::disambiguate`).
fn plain_def_names(cx: &Ctx, root: TsNode) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut stack = vec![(root, cx.file_id.clone())];
    while let Some((body, prefix)) = stack.pop() {
        let mut cur = body.walk();
        for child in body.named_children(&mut cur) {
            let Some(name) = child
                .child_by_field_name("name")
                .map(|n| cx.text(n).to_string())
            else {
                continue;
            };
            let id = Ctx::child_id(&prefix, &name);
            match child.kind() {
                "function_definition" => out.entry(id).or_default().push(name),
                "class_definition" => {
                    if let Some(b) = child.child_by_field_name("body") {
                        stack.push((b, id));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Where a node sits: who calls, which class `self` is, what new definitions attach to.
#[derive(Clone, Copy)]
struct Frame {
    /// Nearest enclosing function (calls are attributed to it).
    caller: Option<u32>,
    /// Class whose methods `self` refers to.
    self_class: Option<u32>,
    /// Definition that new definitions attach to; `None` = the file.
    parent_def: Option<u32>,
    parent_is_class: bool,
    /// Inside a function body: classes are transparent, their methods become nested functions.
    in_function: bool,
    /// Top-level function or class that `uses` edges start from.
    uses_scope: Option<u32>,
    /// The module, or the body of a plain class (see `plain_def_names`).
    plain_scope: bool,
}

/// Names bound in one function scope.
#[derive(Default)]
struct Locals {
    names: HashSet<String>,
    /// Variable → class name as written (`x: Cls`, `x = Cls(...)`).
    types: HashMap<String, String>,
}

struct Walker<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    facts: PyFacts,
    from_names: HashSet<String>,
    /// (line, label, owning def; `None` = the module).
    docstrings: Vec<(u32, String, Option<u32>)>,
    /// Per def: (index of its node, index of its `contains`/`method` edge) in `cx.facts`.
    /// Names of the plain functions and methods (see `plain_def_names`) by id.
    plain_names: HashMap<String, Vec<String>>,
    /// (line, label).
    comments: Vec<(u32, String)>,
    mention_seen: HashSet<(u32, Option<u32>, String)>,
    locals: Vec<Locals>,
    /// Inside call arguments or `Annotated` metadata of an annotation: strings are values.
    opaque_strings: u32,
    /// Walking a class-level field annotation.
    field_annotation: bool,
}

impl<'a> Walker<'_, 'a> {
    fn text(&self, node: TsNode) -> &'a str {
        self.cx.text(node)
    }

    fn finish(mut self) {
        let file_id = self.cx.file_id.clone();
        let docs = std::mem::take(&mut self.docstrings);
        let comments = std::mem::take(&mut self.comments);
        let mut rationale = Vec::with_capacity(docs.len() + comments.len());
        for (line, label, owner) in docs {
            let owner = match owner {
                Some(d) => self.facts.defs[d as usize].id.clone(),
                None => file_id.clone(),
            };
            rationale.push((line, label, owner));
        }
        for (line, label) in comments {
            rationale.push((line, label, file_id.clone()));
        }
        for (line, label, _) in &rationale {
            let id = format!("{file_id}_rationale_{line}");
            self.cx.add_rationale(id, label.clone(), *line);
        }
        for (line, _, owner) in &rationale {
            let id = format!("{file_id}_rationale_{line}");
            self.cx.add_edge(&id, owner, "rationale_for", *line, None);
        }
        self.cx.facts.lang = LangFacts::Python(self.facts);
    }

    fn visit_children(&mut self, node: TsNode, parent_id: &str, f: Frame, depth: u32) {
        let mut cur = node.walk();
        for child in node.named_children(&mut cur) {
            self.visit(child, parent_id, f, depth + 1);
        }
    }

    fn visit(&mut self, node: TsNode, parent_id: &str, f: Frame, depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        match node.kind() {
            "comment" => self.comment(node),
            "function_definition" => self.function(node, &[], parent_id, f),
            "class_definition" => self.class(node, &[], parent_id, f),
            "decorated_definition" => {
                self.own_comments(node);
                let mut cur = node.walk();
                let decorators: Vec<TsNode> = node
                    .named_children(&mut cur)
                    .filter(|c| c.kind() == "decorator")
                    .collect();
                if let Some(def) = node.child_by_field_name("definition") {
                    match def.kind() {
                        "function_definition" => self.function(def, &decorators, parent_id, f),
                        "class_definition" => self.class(def, &decorators, parent_id, f),
                        _ => {}
                    }
                }
            }
            "import_statement" => self.import(node, f, false),
            "import_from_statement" => self.import(node, f, true),
            "future_import_statement" => {}
            "call" => {
                self.call(node, f);
                self.visit_children(node, parent_id, f, depth);
            }
            "return_statement" => {
                if let Some(v) = node.named_child(0) {
                    self.indirect_value(v, "return", f);
                }
                self.visit_children(node, parent_id, f, depth);
            }
            "assignment" => {
                self.assignment(node, f);
                self.visit_children(node, parent_id, f, depth);
            }
            "augmented_assignment" | "named_expression" => {
                let target = node
                    .child_by_field_name("left")
                    .or_else(|| node.child_by_field_name("name"));
                if let Some(t) = target {
                    self.bind_target(t);
                }
                self.visit_children(node, parent_id, f, depth);
            }
            "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression" => {
                // Comprehension variables are bound before the element expression runs.
                let mut cur = node.walk();
                for c in node.named_children(&mut cur) {
                    if c.kind() == "for_in_clause" {
                        if let Some(t) = c.child_by_field_name("left") {
                            self.bind_target(t);
                        }
                    }
                }
                self.visit_children(node, parent_id, f, depth);
            }
            "for_statement" | "for_in_clause" => {
                if let Some(t) = node.child_by_field_name("left") {
                    self.bind_target(t);
                }
                self.visit_children(node, parent_id, f, depth);
            }
            "as_pattern" => {
                if let Some(t) = node.child_by_field_name("alias") {
                    self.bind_target(t);
                }
                self.visit_children(node, parent_id, f, depth);
            }
            "list" | "tuple" | "set" => {
                let mut cur = node.walk();
                for el in node.named_children(&mut cur) {
                    self.indirect_value(el, "collection", f);
                }
                self.visit_children(node, parent_id, f, depth);
            }
            "dictionary" => {
                let mut cur = node.walk();
                for pair in node.named_children(&mut cur) {
                    if let Some(v) = pair.child_by_field_name("value") {
                        self.indirect_value(v, "collection", f);
                    }
                }
                self.visit_children(node, parent_id, f, depth);
            }
            "identifier" => self.mention(node, f),
            _ => self.visit_children(node, parent_id, f, depth),
        }
    }

    // ---- definitions ------------------------------------------------------------------------

    /// Plain functions and methods whose differently written names produce one id keep apart
    /// with `_<sha1(name)[:6]>`: a name that is not already in id form (`_get_x`, `Get_Y`), and
    /// a name with a same-letters, different-case twin (`get_y` beside `Get_Y`). Every other id
    /// clash (classes, nested or decorated functions, redefinitions) merges, the first
    /// definition winning (§4.5, Q5).
    fn disambiguate(&self, id: String, name: &str, plain: bool) -> String {
        if !plain {
            return id;
        }
        let Some(names) = self.plain_names.get(&id) else {
            return id;
        };
        if names.iter().all(|n| n == name) {
            return id;
        }
        let lower = name.to_lowercase();
        let case_twin = names.iter().any(|n| n != name && n.to_lowercase() == lower);
        if crate::extract::slug(name) == name && !case_twin {
            return id;
        }
        let digest = sha1_smol::Sha1::from(name.as_bytes()).digest().to_string();
        format!("{id}_{}", &digest[..6])
    }

    fn push_def(&mut self, id: String, name: &str, kind: DefKind, parent: Option<u32>) -> u32 {
        self.facts.defs.push(PyDef {
            id,
            name: name.to_string(),
            kind,
            parent,
        });
        (self.facts.defs.len() - 1) as u32
    }

    /// Comments tree-sitter attaches to a definition node itself rather than to its body (a
    /// comment before the first statement of a block).
    fn own_comments(&mut self, node: TsNode) {
        let mut cur = node.walk();
        for c in node.named_children(&mut cur) {
            if c.kind() == "comment" {
                self.comment(c);
            }
        }
    }

    fn function(&mut self, node: TsNode, decorators: &[TsNode], parent_id: &str, f: Frame) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        self.own_comments(node);
        let name = self.text(name_node).to_string();
        let line = Ctx::line(node);
        let is_method = f.parent_is_class && !f.in_function;
        let id = Ctx::child_id(parent_id, &name);
        let label = if is_method {
            format!(".{name}()")
        } else {
            format!("{name}()")
        };
        let plain = decorators.is_empty() && f.plain_scope && plain_position(node);
        let id = self.disambiguate(id, &name, plain);
        self.cx
            .add_def(id.clone(), label, line, vec![flag("_callable")]);
        let relation = if is_method { "method" } else { "contains" };
        self.cx.add_edge(parent_id, &id, relation, line, None);
        let kind = if is_method {
            DefKind::Method
        } else {
            DefKind::Function
        };
        let idx = self.push_def(id.clone(), &name, kind, f.parent_def);

        let uses_scope = f.uses_scope.or(Some(idx));
        // Decorators are evaluated in the enclosing scope; so are defaults and annotations, but
        // a top-level function's own annotations count as its mentions.
        for d in decorators {
            self.decorator(*d, idx, f, !f.in_function);
        }
        let outer = Frame { uses_scope, ..f };
        // Signatures of functions nested in functions record no `references` (§9.3).
        let signature_refs = !f.in_function;

        let mut locals = Locals::default();
        if let Some(params) = node.child_by_field_name("parameters") {
            let mut cur = params.walk();
            for p in params.named_children(&mut cur) {
                if p.kind() == "comment" {
                    self.comment(p);
                } else {
                    self.parameter(p, idx, outer, &mut locals, signature_refs);
                }
            }
        }
        if let Some(ret) = node.child_by_field_name("return_type") {
            if signature_refs {
                self.walk_type(ret, idx, RefKind::Return, Ctx::line(ret), 0);
            }
            self.visit(ret, parent_id, outer, 0);
        }

        let inner = Frame {
            caller: Some(idx),
            self_class: if is_method {
                f.parent_def
            } else {
                f.self_class
            },
            parent_def: Some(idx),
            parent_is_class: false,
            in_function: true,
            uses_scope,
            plain_scope: false,
        };
        if let Some(body) = node.child_by_field_name("body") {
            self.docstring(body, Some(idx));
            self.locals.push(locals);
            self.visit_children(body, &id, inner, 0);
            self.locals.pop();
        }
    }

    fn parameter(&mut self, p: TsNode, def: u32, outer: Frame, locals: &mut Locals, refs: bool) {
        let (name_node, ty, default) = match p.kind() {
            "identifier" => (Some(p), None, None),
            "typed_parameter" => {
                let mut cur = p.walk();
                let name = p
                    .named_children(&mut cur)
                    .find(|c| c.kind() != "type")
                    .map(|c| match c.kind() {
                        "list_splat_pattern" | "dictionary_splat_pattern" => {
                            c.named_child(0).unwrap_or(c)
                        }
                        _ => c,
                    });
                (name, p.child_by_field_name("type"), None)
            }
            "typed_default_parameter" => (
                p.child_by_field_name("name"),
                p.child_by_field_name("type"),
                p.child_by_field_name("value"),
            ),
            "default_parameter" => (
                p.child_by_field_name("name"),
                None,
                p.child_by_field_name("value"),
            ),
            "list_splat_pattern" | "dictionary_splat_pattern" => (p.named_child(0), None, None),
            _ => (None, None, None),
        };
        let pname = name_node
            .filter(|n| n.kind() == "identifier")
            .map(|n| self.text(n).to_string());
        if let Some(t) = ty {
            if refs {
                self.walk_type(t, def, RefKind::Param, Ctx::line(t), 0);
            }
            if let (Some(pn), Some(cls)) = (&pname, self.class_name_of_annotation(t)) {
                locals.types.insert(pn.clone(), cls);
            }
            self.visit(t, "", outer, 0);
        }
        if let Some(v) = default {
            self.visit(v, "", outer, 0);
        }
        if let Some(pn) = pname {
            locals.names.insert(pn);
        }
    }

    /// `x: Cls`, `x: mod.Cls` or `x: "Cls"` → the class name as written.
    fn class_name_of_annotation(&self, ty: TsNode) -> Option<String> {
        let inner = if ty.kind() == "type" {
            ty.named_child(0)?
        } else {
            ty
        };
        match inner.kind() {
            "identifier" | "attribute" => {
                let t = self.text(inner);
                is_dotted(t).then(|| t.to_string())
            }
            "string" => {
                let t = string_content(self.text(inner))?;
                is_dotted(t).then(|| t.to_string())
            }
            _ => None,
        }
    }

    fn class(&mut self, node: TsNode, decorators: &[TsNode], parent_id: &str, f: Frame) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        self.own_comments(node);
        let bases = node.child_by_field_name("superclasses");
        if f.in_function {
            // Classes inside functions are transparent (§9: their methods become nested
            // functions of the enclosing function).
            for d in decorators {
                self.visit(*d, parent_id, f, 0);
            }
            if let Some(b) = bases {
                self.visit(b, parent_id, f, 0);
            }
            if let Some(body) = node.child_by_field_name("body") {
                let inner = Frame {
                    self_class: None,
                    ..f
                };
                self.visit_children(body, parent_id, inner, 0);
            }
            return;
        }
        let name = self.text(name_node).to_string();
        let line = Ctx::line(node);
        let id = Ctx::child_id(parent_id, &name);
        let mut opaque = false;
        let mut base_names: Vec<(String, u32)> = Vec::new();
        if let Some(b) = bases {
            let mut cur = b.walk();
            for arg in b.named_children(&mut cur) {
                match arg.kind() {
                    "identifier" => base_names.push((self.text(arg).to_string(), line)),
                    "keyword_argument" | "comment" => {}
                    _ => opaque = true,
                }
            }
        }
        let mut attrs = Vec::new();
        if opaque {
            attrs.push((
                "metadata".to_string(),
                Attr::Json("{\"python_opaque_bases\":true}".into()),
            ));
        }
        attrs.push(flag("_callable"));
        attrs.push(flag("_callable_class"));
        let plain = decorators.is_empty() && f.plain_scope && plain_position(node);
        self.cx.add_def(id.clone(), name.clone(), line, attrs);
        self.cx.add_edge(parent_id, &id, "contains", line, None);
        let idx = self.push_def(id.clone(), &name, DefKind::Class, f.parent_def);
        for (base, l) in base_names {
            self.facts.refs.push(TypeRef {
                source: idx,
                kind: RefKind::Base,
                name: base,
                qual: None,
                line: l,
                resolved_only: false,
            });
        }
        let uses_scope = f.uses_scope.or(Some(idx));
        for d in decorators {
            self.decorator(*d, idx, f, true);
        }
        let outer = Frame { uses_scope, ..f };
        if let Some(b) = bases {
            self.visit(b, parent_id, outer, 0);
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.docstring(body, Some(idx));
            let inner = Frame {
                caller: None,
                self_class: Some(idx),
                parent_def: Some(idx),
                parent_is_class: true,
                in_function: false,
                uses_scope,
                plain_scope: plain,
            };
            self.visit_children(body, &id, inner, 0);
        }
    }

    fn decorator(&mut self, d: TsNode, def: u32, outer: Frame, record: bool) {
        let line = Ctx::line(d);
        if let Some(expr) = d.named_child(0).filter(|_| record) {
            let target = if expr.kind() == "call" {
                expr.child_by_field_name("function")
            } else {
                Some(expr)
            };
            if let Some(t) = target.filter(|t| matches!(t.kind(), "identifier" | "attribute")) {
                let text = self.text(t);
                if is_dotted(text) {
                    let (qual, name) = split_dotted(text);
                    if !EXCLUDED_DECORATORS.contains(name) {
                        self.facts.refs.push(TypeRef {
                            source: def,
                            kind: RefKind::Decorator,
                            name: name.to_string(),
                            qual: qual.map(str::to_string),
                            line,
                            resolved_only: false,
                        });
                    }
                }
            }
        }
        self.visit(d, "", outer, 0);
    }

    /// Module, class and function docstrings (§9.2): the first statement, when a string whose
    /// collapsed text is longer than 20 characters.
    fn docstring(&mut self, body: TsNode, owner: Option<u32>) {
        let mut cur = body.walk();
        let Some(first) = body
            .named_children(&mut cur)
            .find(|c| c.kind() != "comment")
        else {
            return;
        };
        if first.kind() != "expression_statement" || first.named_child_count() != 1 {
            return;
        }
        let Some(s) = first.named_child(0).filter(|s| s.kind() == "string") else {
            return;
        };
        let raw = self.text(s).trim_matches(|c| c == '"' || c == '\'');
        let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.chars().count() <= 20 {
            return;
        }
        self.docstrings
            .push((Ctx::line(s), rationale_label(&collapsed), owner));
    }

    /// `# NOTE: …` on its own line (§9.2).
    fn comment(&mut self, node: TsNode) {
        let text = self.text(node);
        let Some(rest) = text.strip_prefix("# ") else {
            return;
        };
        if !COMMENT_MARKERS
            .iter()
            .any(|m| rest.strip_prefix(m).is_some_and(|r| r.starts_with(':')))
        {
            return;
        }
        let start = node.start_byte();
        let line_start = self.cx.src[..start]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        if !self.cx.src[line_start..start]
            .iter()
            .all(|b| *b == b' ' || *b == b'\t')
        {
            return;
        }
        let label = rationale_label(text);
        self.comments.push((Ctx::line(node), label));
    }

    // ---- imports ----------------------------------------------------------------------------

    fn import(&mut self, node: TsNode, f: Frame, from: bool) {
        let line = Ctx::line(node);
        let scope = f.caller;
        let mut cur = node.walk();
        if !from {
            for name in node.children_by_field_name("name", &mut cur) {
                let (path, alias) = match name.kind() {
                    "aliased_import" => (
                        name.child_by_field_name("name")
                            .map(|n| self.text(n).to_string()),
                        name.child_by_field_name("alias")
                            .map(|n| self.text(n).to_string()),
                    ),
                    _ => (Some(self.text(name).to_string()), None),
                };
                if let Some(path) = path {
                    self.facts.imports.push(PyImport {
                        line,
                        scope,
                        kind: ImportKind::Module { path, alias },
                    });
                }
            }
            return;
        }
        let Some(module_node) = node.child_by_field_name("module_name") else {
            return;
        };
        let (level, module) = if module_node.kind() == "relative_import" {
            let mut level = 0;
            let mut module = String::new();
            let mut c2 = module_node.walk();
            for part in module_node.named_children(&mut c2) {
                match part.kind() {
                    "import_prefix" => {
                        level = self.text(part).chars().filter(|c| *c == '.').count() as u32
                    }
                    "dotted_name" => module = self.text(part).to_string(),
                    _ => {}
                }
            }
            (level, module)
        } else {
            (0, self.text(module_node).to_string())
        };
        let mut names = Vec::new();
        for name in node.children_by_field_name("name", &mut cur) {
            match name.kind() {
                "aliased_import" => {
                    if let (Some(n), Some(a)) = (
                        name.child_by_field_name("name"),
                        name.child_by_field_name("alias"),
                    ) {
                        names.push((self.text(n).to_string(), Some(self.text(a).to_string())));
                    }
                }
                _ => names.push((self.text(name).to_string(), None)),
            }
        }
        for (n, a) in &names {
            self.from_names
                .insert(a.as_deref().unwrap_or(n).to_string());
        }
        let mut c3 = node.walk();
        let star = node
            .named_children(&mut c3)
            .any(|c| c.kind() == "wildcard_import");
        self.facts.imports.push(PyImport {
            line,
            scope,
            kind: ImportKind::From {
                level,
                module,
                names,
                star,
            },
        });
    }

    // ---- bodies -----------------------------------------------------------------------------

    fn is_local(&self, name: &str) -> bool {
        self.locals.iter().any(|l| l.names.contains(name))
    }

    fn local_type(&self, name: &str) -> Option<&str> {
        self.locals
            .iter()
            .rev()
            .find_map(|l| l.types.get(name))
            .map(String::as_str)
    }

    fn bind_target(&mut self, t: TsNode) {
        let Some(top) = self.locals.last_mut() else {
            return;
        };
        let mut stack = vec![t];
        while let Some(n) = stack.pop() {
            match n.kind() {
                "identifier" => {
                    let name = std::str::from_utf8(&self.cx.src[n.byte_range()]).unwrap_or("");
                    top.names.insert(name.to_string());
                }
                "pattern_list"
                | "tuple_pattern"
                | "list_pattern"
                | "list_splat_pattern"
                | "as_pattern_target"
                | "parenthesized_expression"
                | "tuple"
                | "list" => {
                    let mut cur = n.walk();
                    stack.extend(n.named_children(&mut cur));
                }
                _ => {}
            }
        }
    }

    fn assignment(&mut self, node: TsNode, f: Frame) {
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");
        if let Some(r) = right {
            self.indirect_value(r, "assignment", f);
        }
        let Some(l) = left else {
            return;
        };
        if f.caller.is_none() {
            // A class-level annotation is a field of the class (`name: T`, `name: T = v`).
            if let (Some(class), true, Some(ty)) = (
                f.parent_def,
                f.parent_is_class && !f.in_function,
                node.child_by_field_name("type"),
            ) {
                self.field_annotation = true;
                self.walk_type(ty, class, RefKind::Field, Ctx::line(ty), 0);
                self.field_annotation = false;
            }
            return;
        }
        if l.kind() == "identifier" {
            let var = self.text(l).to_string();
            let class = right
                .filter(|r| r.kind() == "call")
                .and_then(|r| r.child_by_field_name("function"))
                .filter(|func| matches!(func.kind(), "identifier" | "attribute"))
                .map(|func| self.text(func).to_string())
                .filter(|t| is_dotted(t));
            if let Some(top) = self.locals.last_mut() {
                match class {
                    Some(c) => {
                        top.types.insert(var.clone(), c);
                    }
                    None => {
                        top.types.remove(&var);
                    }
                }
            }
        }
        self.bind_target(l);
    }

    /// A bare function name used as a value (`argument`, `return`, `collection`, `assignment`).
    fn indirect_value(&mut self, v: TsNode, context: &str, f: Frame) {
        // Module-level collections and assignments count, with the file as the source.
        let module_level = f.caller.is_none()
            && f.parent_def.is_none()
            && matches!(context, "collection" | "assignment");
        if f.caller.is_none() && !module_level {
            return;
        }
        let caller = f.caller;
        if v.kind() != "identifier" {
            return;
        }
        let name = self.text(v);
        if self.is_local(name) {
            return;
        }
        let name = name.to_string();
        self.facts.indirect.push(PyIndirect {
            caller,
            name,
            context: context.to_string(),
            line: Ctx::line(v),
        });
    }

    fn call(&mut self, node: TsNode, f: Frame) {
        let Some(caller) = f.caller else {
            return;
        };
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        let line = Ctx::line(node);
        let args = node.child_by_field_name("arguments");
        match func.kind() {
            "identifier" => {
                let name = self.text(func).to_string();
                if name == "getattr" {
                    self.getattr(args, caller, line);
                }
                if name != "super" && !self.is_local(&name) {
                    self.facts.calls.push(PyCall {
                        caller,
                        kind: CallKind::Name,
                        name,
                        line,
                    });
                }
            }
            "attribute" => {
                let (Some(obj), Some(attr)) = (
                    func.child_by_field_name("object"),
                    func.child_by_field_name("attribute"),
                ) else {
                    return;
                };
                let name = self.text(attr).to_string();
                let kind = match obj.kind() {
                    "identifier" => {
                        let recv = self.text(obj);
                        if recv == "self" {
                            f.self_class.map(CallKind::SelfMethod)
                        } else if let Some(t) = self.local_type(recv) {
                            Some(CallKind::Typed(t.to_string()))
                        } else if self.is_local(recv) {
                            None
                        } else {
                            Some(CallKind::Attr(recv.to_string()))
                        }
                    }
                    "call" => {
                        let is_super = obj
                            .child_by_field_name("function")
                            .is_some_and(|g| g.kind() == "identifier" && self.text(g) == "super");
                        if is_super {
                            f.self_class.map(CallKind::Super)
                        } else {
                            None
                        }
                    }
                    "attribute" => {
                        let recv = self.text(obj);
                        let first = first_component(recv);
                        (is_dotted(recv) && first != "self" && !self.is_local(first))
                            .then(|| CallKind::Attr(recv.to_string()))
                    }
                    _ => None,
                };
                if let Some(kind) = kind {
                    self.facts.calls.push(PyCall {
                        caller,
                        kind,
                        name,
                        line,
                    });
                }
            }
            _ => {}
        }
        if let Some(a) = args.filter(|a| a.kind() == "argument_list") {
            let mut cur = a.walk();
            for arg in a.named_children(&mut cur) {
                match arg.kind() {
                    "identifier" => self.indirect_value(arg, "argument", f),
                    "keyword_argument" => {
                        if let Some(v) = arg.child_by_field_name("value") {
                            self.indirect_value(v, "argument", f);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// `getattr(obj, "name")` names a function or method.
    fn getattr(&mut self, args: Option<TsNode>, caller: u32, line: u32) {
        let Some(a) = args else {
            return;
        };
        let mut cur = a.walk();
        let Some(s) = a.named_children(&mut cur).nth(1) else {
            return;
        };
        if s.kind() != "string" {
            return;
        }
        let Some(name) = string_content(self.text(s)) else {
            return;
        };
        if is_identifier(name) {
            let name = name.to_string();
            self.facts.indirect.push(PyIndirect {
                caller: Some(caller),
                name,
                context: "getattr".into(),
                line,
            });
        }
    }

    fn mention(&mut self, node: TsNode, f: Frame) {
        let Some(scope) = f.uses_scope else {
            return;
        };
        let name = self.text(node);
        if !self.from_names.contains(name) {
            return;
        }
        let name = name.to_string();
        if self.mention_seen.insert((scope, f.caller, name.clone())) {
            self.facts.mentions.push(PyMention {
                scope,
                at: f.caller,
                name,
                line: Ctx::line(node),
            });
        }
    }

    // ---- annotations ------------------------------------------------------------------------

    /// Record every name an annotation mentions (§9.3 `references`): the annotated type with
    /// `kind`, subscript and call arguments as `generic_arg`.
    fn walk_type(&mut self, node: TsNode, def: u32, kind: RefKind, line: u32, depth: u32) {
        if depth > 64 {
            return;
        }
        match node.kind() {
            "identifier" | "attribute" | "member_type" => {
                let text = self.text(node);
                if !is_dotted(text) {
                    return;
                }
                let (qual, name) = split_dotted(text);
                if EXCLUDED_TYPES.contains(name) {
                    return;
                }
                self.facts.refs.push(TypeRef {
                    source: def,
                    kind,
                    name: name.to_string(),
                    qual: qual.map(str::to_string),
                    line,
                    resolved_only: self.field_annotation,
                });
            }
            "subscript" => {
                let value = node.child_by_field_name("value");
                if let Some(v) = value {
                    self.walk_type(v, def, kind, line, depth + 1);
                }
                let head = value.map(|v| split_dotted(self.text(v)).1);
                let mut cur = node.walk();
                let args: Vec<TsNode> =
                    node.children_by_field_name("subscript", &mut cur).collect();
                self.type_args(head, &args, def, line, depth);
            }
            "generic_type" => {
                let mut cur = node.walk();
                let mut head = None;
                for c in node.named_children(&mut cur) {
                    if c.kind() == "type_parameter" {
                        let mut c2 = c.walk();
                        let args: Vec<TsNode> = c.named_children(&mut c2).collect();
                        self.type_args(head, &args, def, line, depth);
                    } else {
                        head = Some(split_dotted(self.text(c)).1);
                        self.walk_type(c, def, kind, line, depth + 1);
                    }
                }
            }
            "call" => {
                if let Some(func) = node.child_by_field_name("function") {
                    self.walk_type(func, def, kind, line, depth + 1);
                }
                if let Some(args) = node.child_by_field_name("arguments") {
                    // Call arguments are values: a string there is not a forward reference.
                    self.opaque_strings += 1;
                    let mut cur = args.walk();
                    for a in args.named_children(&mut cur) {
                        let v = if a.kind() == "keyword_argument" {
                            a.child_by_field_name("value")
                        } else {
                            Some(a)
                        };
                        if let Some(v) = v {
                            self.walk_type(v, def, RefKind::Generic, line, depth + 1);
                        }
                    }
                    self.opaque_strings -= 1;
                }
            }
            // A forward reference (`"User"`, `"list[Path | FileSlice]"`): Graphify skips it, but
            // it names the same types as the unquoted annotation.
            "string" if self.opaque_strings == 0 => {
                let text = self.text(node);
                if let Some(inner) = string_content(text).filter(|s| !s.contains(['"', '\''])) {
                    for (name, nested) in string_annotation_names(inner) {
                        let (qual, last) = split_dotted(name);
                        if EXCLUDED_TYPES.contains(last) {
                            continue;
                        }
                        self.facts.refs.push(TypeRef {
                            source: def,
                            kind: if nested { RefKind::Generic } else { kind },
                            name: last.to_string(),
                            qual: qual.map(str::to_string),
                            line,
                            resolved_only: true,
                        });
                    }
                }
            }
            "binary_operator" => {
                let is_union = node
                    .child_by_field_name("operator")
                    .is_some_and(|o| self.text(o) == "|");
                if is_union {
                    let mut cur = node.walk();
                    for c in node.named_children(&mut cur) {
                        self.walk_type(c, def, kind, line, depth + 1);
                    }
                }
            }
            "type"
            | "union_type"
            | "type_parameter"
            | "constrained_type"
            | "splat_type"
            | "list"
            | "tuple"
            | "parenthesized_expression"
            | "expression_list" => {
                let mut cur = node.walk();
                for c in node.named_children(&mut cur) {
                    self.walk_type(c, def, kind, line, depth + 1);
                }
            }
            _ => {}
        }
    }

    /// The arguments of a subscripted annotation `head[args]`, as `generic_arg`. `Literal[…]`
    /// holds values, not types; `Annotated[T, …]` holds a type and then metadata values.
    fn type_args(&mut self, head: Option<&str>, args: &[TsNode], def: u32, line: u32, depth: u32) {
        if head == Some("Literal") {
            return;
        }
        for (i, a) in args.iter().enumerate() {
            let metadata = head == Some("Annotated") && i > 0;
            // A field's metadata (validators, docs) names values, not the field's type.
            if metadata && self.field_annotation {
                continue;
            }
            self.opaque_strings += u32::from(metadata);
            self.walk_type(*a, def, RefKind::Generic, line, depth + 1);
            self.opaque_strings -= u32::from(metadata);
        }
    }
}

/// The dotted names in a string annotation, each with whether it sits inside brackets
/// (`"list[Path | FileSlice]"` → `list` top level, `Path` and `FileSlice` nested). Empty unless
/// the text is a plain type expression (names, `[]`, `()`, `,`, `|`, `...`), so prose such as
/// `"the user id"` yields nothing.
fn string_annotation_names(s: &str) -> Vec<(&str, bool)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut depth, mut after_name) = (0, 0u32, false);
    while i < b.len() {
        let c = b[i];
        if c == b'_' || c.is_ascii_alphabetic() {
            if after_name {
                return Vec::new();
            }
            let start = i;
            while i < b.len() && (b[i] == b'_' || b[i] == b'.' || b[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let name = &s[start..i];
            if !is_dotted(name) {
                return Vec::new();
            }
            out.push((name, depth > 0));
            after_name = true;
            continue;
        }
        match c {
            b'[' | b'(' => depth += 1,
            b']' | b')' => depth = depth.saturating_sub(1),
            b',' | b'|' => {}
            b'.' if s[i..].starts_with("...") => i += 2,
            b' ' | b'\t' | b'\n' => {
                i += 1;
                continue;
            }
            _ => return Vec::new(),
        }
        after_name = false;
        i += 1;
    }
    out
}

/// `a.b.C` → (`Some("a.b")`, `"C"`); `C` → (`None`, `"C"`).
pub fn split_dotted(text: &str) -> (Option<&str>, &str) {
    match text.rsplit_once('.') {
        Some((q, n)) => (Some(q), n),
        None => (None, text),
    }
}

pub fn first_component(text: &str) -> &str {
    text.split('.').next().unwrap_or(text)
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// A plain dotted name (`a`, `a.b.C`), no calls, subscripts or whitespace.
fn is_dotted(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(is_identifier)
}

/// The text of a simple string literal without prefix and quotes (`"Cls"` → `Cls`).
fn string_content(lit: &str) -> Option<&str> {
    let body = lit.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    for q in ["\"\"\"", "'''", "\"", "'"] {
        if let Some(inner) = body.strip_prefix(q).and_then(|b| b.strip_suffix(q)) {
            return Some(inner.trim());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_file;
    use crate::facts::{FileFacts, NodeKind};
    use crate::lang::Lang;

    fn run(src: &str) -> (FileFacts, PyFacts) {
        let facts = extract_file(Lang::Python, "pkg/m.py", src.as_bytes());
        let py = match &facts.lang {
            LangFacts::Python(p) => p.clone(),
            _ => panic!("no python facts"),
        };
        (facts, py)
    }

    fn edges(f: &FileFacts) -> Vec<String> {
        f.edges
            .iter()
            .map(|e| format!("{} -{}-> {} L{:?}", e.source, e.relation, e.target, e.loc))
            .collect()
    }

    #[test]
    fn methods_nested_functions_and_transparent_classes() {
        let (f, py) = run(
            "class A:\n    def m(self):\n        def inner():\n            pass\n\n    class B:\n        pass\n\n\ndef outer():\n    class Local:\n        def lm(self):\n            pass\n",
        );
        let ids: Vec<_> = f
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect();
        assert_eq!(
            ids,
            [
                ("pkg_m", "m.py"),
                ("pkg_m_a", "A"),
                ("pkg_m_a_m", ".m()"),
                ("pkg_m_a_m_inner", "inner()"),
                ("pkg_m_a_b", "B"),
                ("pkg_m_outer", "outer()"),
                ("pkg_m_outer_lm", "lm()"),
            ]
        );
        let e = edges(&f);
        assert!(e.contains(&"pkg_m_a -method-> pkg_m_a_m LLine(2)".to_string()));
        assert!(e.contains(&"pkg_m_a_m -contains-> pkg_m_a_m_inner LLine(3)".to_string()));
        assert!(e.contains(&"pkg_m_a -contains-> pkg_m_a_b LLine(6)".to_string()));
        assert!(e.contains(&"pkg_m_outer -contains-> pkg_m_outer_lm LLine(12)".to_string()));
        assert_eq!(py.defs[2].kind, DefKind::Function);
    }

    #[test]
    fn annotations_record_contexts_and_skip_builtins() {
        let (_, py) = run(
            "def f(a: Foo, b: Optional[Bar], *c: Baz, d: \"Str\" = None) -> dict[str, m.Qux] | None:\n    pass\n",
        );
        let refs: Vec<_> = py
            .refs
            .iter()
            .map(|r| (r.name.as_str(), r.qual.as_deref(), r.kind))
            .collect();
        assert_eq!(
            refs,
            [
                ("Foo", None, RefKind::Param),
                ("Bar", None, RefKind::Generic),
                ("Baz", None, RefKind::Param),
                ("Str", None, RefKind::Param),
                ("Qux", Some("m"), RefKind::Generic),
            ]
        );
    }

    #[test]
    fn string_annotations_name_types_but_not_values() {
        let (_, py) = run(
            "def f(a: \"list[Path | m.Slice]\", b: Literal[\"Zed\"], c: Annotated[\"Kind\", \"the doc\"], d: Q(\"Arg\")) -> \"tuple[A, ...]\":\n    pass\n",
        );
        let refs: Vec<_> = py.refs.iter().map(|r| (r.name.as_str(), r.kind)).collect();
        assert_eq!(
            refs,
            [
                ("Path", RefKind::Generic),
                ("Slice", RefKind::Generic),
                ("Kind", RefKind::Generic),
                ("Q", RefKind::Param),
                ("A", RefKind::Generic),
            ]
        );
        assert_eq!(string_annotation_names("the user id"), []);
        assert_eq!(string_annotation_names("a-b"), []);
        assert_eq!(
            string_annotation_names("Dict[str, x.Y]"),
            [("Dict", false), ("str", true), ("x.Y", true)]
        );
    }

    #[test]
    fn class_level_annotations_are_fields() {
        let (_, py) = run(
            "class C:\n    a: Foo\n    b: list[Bar] = []\n    c: Annotated[Baz, Field(check)] = None\n    def m(self):\n        x: Local = 1\n        self.y: Other = 2\n\n\ndef outer():\n    class Hidden:\n        z: Skip\n    Hidden()\n",
        );
        let refs: Vec<_> = py
            .refs
            .iter()
            .map(|r| {
                (
                    py.defs[r.source as usize].name.as_str(),
                    r.name.as_str(),
                    r.kind,
                    r.line,
                )
            })
            .collect();
        assert_eq!(
            refs,
            [
                ("C", "Foo", RefKind::Field, 2),
                ("C", "Bar", RefKind::Generic, 3),
                ("C", "Baz", RefKind::Generic, 4),
            ]
        );
    }

    #[test]
    fn opaque_bases_and_decorators() {
        let (f, py) = run(
            "@dataclass\n@register.thing(1)\nclass K(Base, abc.ABC, metaclass=M):\n    @property\n    def p(self):\n        pass\n",
        );
        let k = f.nodes.iter().find(|n| n.id == "pkg_m_k").unwrap();
        assert!(matches!(k.attr("metadata"), Some(Attr::Json(_))));
        assert_eq!(k.loc, crate::facts::Loc::Line(3));
        let refs: Vec<_> = py
            .refs
            .iter()
            .map(|r| (r.name.as_str(), r.kind, r.line))
            .collect();
        assert_eq!(
            refs,
            [("Base", RefKind::Base, 3), ("thing", RefKind::Decorator, 2)]
        );
    }

    #[test]
    fn rationale_docstrings_then_tagged_comments() {
        let (f, _) = run(
            "\"\"\"Module docstring long enough to keep.\"\"\"\n# NOTE: kept\nx = 1  # TODO: trailing\n#NOTE: no space\n# Note: wrong case\n\n\ndef f():\n    \"\"\"Short.\"\"\"\n    # WHY:inside\n    return 1\n",
        );
        let r: Vec<_> = f
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Rationale)
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect();
        assert_eq!(
            r,
            [
                ("pkg_m_rationale_1", "Module docstring long enough to keep."),
                ("pkg_m_rationale_2", "# NOTE: kept"),
                ("pkg_m_rationale_10", "# WHY:inside"),
            ]
        );
    }

    #[test]
    fn calls_receivers_and_shadowing() {
        let (_, py) = run(
            "class C:\n    def m(self, x: Other, cb):\n        self.n()\n        super().n()\n        x.go()\n        cb()\n        y = Thing()\n        y.run()\n        mod.f()\n        helper()\n        return helper\n",
        );
        let calls: Vec<_> = py
            .calls
            .iter()
            .map(|c| (format!("{:?}", c.kind), c.name.as_str()))
            .collect();
        assert_eq!(
            calls,
            [
                ("SelfMethod(0)".to_string(), "n"),
                ("Super(0)".to_string(), "n"),
                ("Typed(\"Other\")".to_string(), "go"),
                ("Name".to_string(), "Thing"),
                ("Typed(\"Thing\")".to_string(), "run"),
                ("Attr(\"mod\")".to_string(), "f"),
                ("Name".to_string(), "helper"),
            ]
        );
        assert_eq!(py.indirect.len(), 1);
        assert_eq!(py.indirect[0].context, "return");
    }
}

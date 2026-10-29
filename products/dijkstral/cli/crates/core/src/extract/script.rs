//! TypeScript, TSX, JavaScript and JSX (§10) extraction. See `docs/graphify-format.md`.
//!
//! One walk over the tree emits the file's definitions (pre-order), their `contains`, `method` and
//! `case_of` edges, then the calls and function references that resolve inside the file, then the
//! tagged-comment rationale. Everything that needs another file (imports, re-exports, imported
//! callees, heritage, type references, decorators, dynamic imports) is recorded in [`ScriptFacts`]
//! for `crate::resolve::script`, which owns module resolution.
//!
//! Quirks of the reference that are deliberately not reproduced (§15): member calls are not bound
//! to the last same-file method of that name (Q8): `this.m()` (and `const me = this; me.m()`)
//! binds to the enclosing class and `x.m()` only through a known receiver type; bare calls use
//! lexical scope and never reach methods; same-file id collisions keep the first definition (Q5).
//!
//! Observed behaviours kept for parity although they are not how a type checker would see the
//! code: class expressions and object literals passed to module-level code flatten their methods
//! into module-level functions (frappe-style `ns.X = class extends … {}` gets no nodes otherwise);
//! declarations inside module-level callbacks (`describe(() => { function f() {} })`) are
//! module-level, and their same-file calls belong to the file; receivers are typed file-wide by
//! parameter names (§10.4).

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, rationale_label, Ctx};
use crate::facts::{Confidence, Loc};

/// The caller index of the file itself (module-level callbacks).
const FILE: usize = 0;

/// Recursion bound for generated or minified inputs; deeper subtrees are skipped. Real code stays
/// under ~50 (excalidraw, erpnext); 256 keeps debug builds inside a 2 MiB worker stack.
const MAX_DEPTH: u32 = 256;

/// What a module-level definition is, for the cross-file pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefKind {
    Function,
    Class,
    Interface,
    TypeAlias,
    Enum,
    Namespace,
    Value,
}

impl DefKind {
    pub fn callable(self) -> bool {
        matches!(self, DefKind::Function | DefKind::Class)
    }
}

/// A module-level definition by its source name (namespaces are flattened into the module).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopDef {
    pub name: String,
    pub id: String,
    pub kind: DefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportKind {
    /// `import … from`, `import "…"`, `import x = require(…)`.
    Import,
    /// `export {…} from`.
    ReExport,
    /// `export * from`.
    ExportStar,
    /// `export * as ns from` (the namespace node is emitted per file).
    ExportStarAs,
    /// `require("…")`.
    Require,
    /// `import("…")` inside a function (`deferred`).
    Dynamic,
}

/// One binding of an import or re-export: `imported` is the exported name in the target module
/// (`default`, `*` for a namespace, or a name); `local` is the name in this file (the exported
/// alias for re-exports).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportName {
    pub imported: String,
    pub local: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Import {
    pub spec: String,
    pub line: u32,
    pub kind: ImportKind,
    /// The function that performs a `require`/`import()`; `None` for the file.
    pub owner: Option<String>,
    pub type_only: bool,
    pub names: Vec<ImportName>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DefaultExport {
    /// `export default class X` / `export default function f`.
    Def(String),
    /// `export default name` / `module.exports = name`.
    Name(String),
}

/// A class or interface with its own methods (by source name), for member-call resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassInfo {
    pub id: String,
    pub methods: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallKind {
    /// `name()`.
    Bare,
    /// `new Name()`.
    New,
    /// `<Name/>`.
    Jsx,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Receiver {
    /// `Name.m()` with `Name` not a same-file class (an imported class's static method).
    Name(String),
    /// `x.m()` where `x` was annotated with, or constructed as, this type name.
    Typed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefContext {
    Argument,
    Collection,
}

impl RefContext {
    pub fn as_str(self) -> &'static str {
        match self {
            RefContext::Argument => "argument",
            RefContext::Collection => "collection",
        }
    }
}

/// A call or function reference the file could not resolve by itself, in walk order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pending {
    Call {
        caller: String,
        /// The caller is a module-level function (its edges carry no score, as Graphify's).
        top: bool,
        /// The module-level function enclosing a nested caller: Graphify's import-scoped pass
        /// also attributes the nested function's imported calls to it.
        root: Option<String>,
        name: String,
        kind: CallKind,
        line: u32,
    },
    Member {
        caller: String,
        recv: Receiver,
        method: String,
        line: u32,
    },
    /// A function name passed as an argument or placed in a literal (`indirect_call`).
    Ref {
        /// `None` for the file (module-level code).
        owner: Option<String>,
        name: String,
        context: RefContext,
        line: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeContext {
    Field,
    Parameter,
    Return,
    GenericArg,
}

impl TypeContext {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeContext::Field => "field",
            TypeContext::Parameter => "parameter_type",
            TypeContext::Return => "return_type",
            TypeContext::GenericArg => "generic_arg",
        }
    }
}

/// A type name mentioned by a class member or interface member (`references`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeRef {
    pub source: String,
    pub name: String,
    pub context: TypeContext,
    pub line: u32,
}

/// `extends` (`inherits`) or `implements` from a class or interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heritage {
    pub source: String,
    pub name: String,
    pub implements: bool,
    pub line: u32,
}

/// `@name` / `@name(…)` on a class or method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decorator {
    pub source: String,
    pub name: String,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::script`) needs from one file. Ids are local
/// (before collision renaming).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptFacts(pub Box<ScriptData>);

impl std::ops::Deref for ScriptFacts {
    type Target = ScriptData;

    fn deref(&self) -> &ScriptData {
        &self.0
    }
}

/// The content of [`ScriptFacts`] (boxed so `LangFacts` stays small).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptData {
    /// Import-like statements in source order.
    pub imports: Vec<Import>,
    pub top: Vec<TopDef>,
    /// `export { local as exported }` without a source: (exported, local, line).
    pub local_exports: Vec<(String, String, u32)>,
    pub default_export: Option<DefaultExport>,
    pub classes: Vec<ClassInfo>,
    pub pending: Vec<Pending>,
    pub heritage: Vec<Heritage>,
    pub type_refs: Vec<TypeRef>,
    pub decorators: Vec<Decorator>,
    /// `import("…")` specifiers found anywhere in the source text (JSDoc types included).
    pub dynamic: Vec<String>,
    /// Member calls on typed receivers, as Graphify types them, for `metadata.unresolved_calls`
    /// (kept or not by the cross-file pass, which knows every type name).
    pub unresolved: Vec<UnresolvedCall>,
}

/// `x.callee()` in `caller` where Graphify types `x` as `ty` (§10.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnresolvedCall {
    pub caller: String,
    pub callee: String,
    pub ty: String,
    pub line: u32,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let mut w = Walker {
        cx,
        facts: ScriptData::default(),
        callers: Vec::new(),
        classes: Vec::new(),
        events: Vec::new(),
        file_typed: Vec::new(),
        graphify_vars: Vec::new(),
    };
    if let Some(tree) = tree {
        let st = St {
            caller: None,
            module: true,
            file_owner: true,
            exported: false,
            lazy: None,
            in_value: false,
            foreign_this: false,
        };
        w.scan_graphify_vars(tree.root_node());
        // Caller 0 is the file: calls in module-level callbacks (`describe(() => …)`).
        w.new_caller(w.cx.file_id.clone(), None, None, false);
        w.children(tree.root_node(), st, 0);
    }
    w.resolve_local();
    w.rationale();
    w.facts.dynamic = dynamic_imports(w.cx.src);
    let mut facts = w.facts;
    facts.classes = w
        .classes
        .into_iter()
        .filter(|c| !c.id.is_empty())
        .map(|c| ClassInfo {
            id: c.id,
            methods: c.methods,
        })
        .collect();
    cx.facts.lang = crate::facts::LangFacts::Script(ScriptFacts(Box::new(facts)));
}

/// A named function or method whose body is being walked: calls inside are attributed to it.
struct Caller {
    id: String,
    parent: Option<usize>,
    class: Option<usize>,
    /// Contained by the file node (module-level function).
    top: bool,
    /// Function declarations directly inside (by name).
    nested: Vec<(String, String)>,
    /// Locals with a known type: parameter annotations and `new X()` initializers.
    typed: Vec<(String, String)>,
    /// A module-level value (`const X = memo(() => …)`): nested declarations are transparent.
    value: bool,
    /// Plain-identifier parameters and locals: they hide functions passed by name.
    shadow: Vec<String>,
    /// Locals bound to `this` (`const me = this`).
    this_alias: Vec<String>,
    /// A class constructor: its `this.x = …` assignments rank above other methods' (§10.1).
    ctor: bool,
    /// A static method: its `this.x = …` assignments type static fields.
    static_method: bool,
    /// Own parameters with a receiver type (`p: Node`, `q?: Repo`, `r: Node | null`).
    params: Vec<(String, String)>,
    /// A function-valued class field (`handler = () => …`): its `this.x = …` types nothing.
    field_fn: bool,
}

struct Class {
    id: String,
    /// The `extends` name when it is a plain identifier.
    base: Option<String>,
    methods: Vec<(String, String)>,
    /// Field name → type name for `this.f.m()` (`_ts_fields`), final once the body is walked.
    fields: Vec<(String, String)>,
    /// Declared field types in source order (fields, parameter properties, accessors), with
    /// `static`.
    declared: Vec<(String, String, bool)>,
    /// `this.x = new X()` / `this.x = typedParam` in methods: (field, type, in the
    /// constructor, in a static method).
    assigned: Vec<(String, String, bool, bool)>,
    /// Members whose differently written names produce one id: raw name → final id (§10.1).
    renames: std::collections::HashMap<String, String>,
    /// `#n` and `n` both members: both nodes carry `_ts_public_twin`.
    public_twins: std::collections::HashSet<String>,
}

/// Walk state; `Copy` so recursion stays cheap.
#[derive(Clone, Copy)]
struct St {
    /// Nearest enclosing named function or method; `None` drops calls.
    caller: Option<usize>,
    /// At module level (program, namespace bodies, plain blocks between them).
    module: bool,
    /// Module-level code (or a callback inside it): function references belong to the file.
    file_owner: bool,
    /// Inside `export …`.
    exported: bool,
    /// A module-level value initialised by a call (`const X = memo(() => …)`): anonymous
    /// functions inside its initializer are attributed to it.
    lazy: Option<usize>,
    /// Inside a module-level value's literal initializer (`const X = { m() {} }`), which
    /// Graphify does not descend into: object-literal methods there are not functions.
    in_value: bool,
    /// Inside a non-arrow function expression of the caller: `this` is not the caller's.
    foreign_this: bool,
}

impl St {
    fn inner(self, caller: Option<usize>) -> St {
        St {
            caller,
            module: false,
            file_owner: caller.is_none() && self.file_owner,
            exported: false,
            lazy: None,
            in_value: caller.is_none() && self.in_value,
            foreign_this: false,
        }
    }

    /// Code Graphify does not descend into for module-level purposes (class expressions,
    /// anonymous default exports, object-literal methods without a node).
    fn sealed() -> St {
        St {
            in_value: true,
            ..St::opaque()
        }
    }

    /// Declarations here are module-level: the module itself, or a callback of module-level
    /// code (`describe(() => { function f() {} })`), as Graphify hoists them.
    fn hoist(self) -> bool {
        self.module
            || (self.caller.is_none() && self.lazy.is_none() && self.file_owner && !self.in_value)
    }

    /// Calls in callbacks of module-level code belong to the file (caller 0); direct
    /// module-level statements have no caller.
    fn caller_or_file(self) -> Option<usize> {
        match self.caller {
            Some(c) => Some(c),
            None if !self.module && self.lazy.is_none() && self.file_owner && !self.in_value => {
                Some(FILE)
            }
            None => None,
        }
    }

    fn opaque() -> St {
        St {
            caller: None,
            module: false,
            file_owner: false,
            exported: false,
            lazy: None,
            in_value: false,
            foreign_this: false,
        }
    }
}

enum Recv {
    This,
    /// `this.field.m()`.
    ThisField(String),
    Ident(String),
}

enum Event {
    Call {
        caller: usize,
        name: String,
        kind: CallKind,
        line: u32,
    },
    Member {
        caller: usize,
        recv: Recv,
        method: String,
        line: u32,
    },
    Ref {
        owner: Option<usize>,
        name: String,
        context: RefContext,
        line: u32,
    },
}

struct Walker<'a, 'b> {
    cx: &'b mut Ctx<'a>,
    facts: ScriptData,
    callers: Vec<Caller>,
    classes: Vec<Class>,
    events: Vec<Event>,
    /// `name: Type` annotations and module-level `new` initializers anywhere in the file.
    file_typed: Vec<(String, String)>,
    /// Graphify's receiver types: one file-wide name → type map, the last binding winning, from
    /// every parameter annotated with a plain type name (functions, callbacks, overloads, method
    /// signatures, function types; not rest or destructured ones) and every `x = new X()`
    /// declarator, at any depth (see [`Walker::scan_graphify_vars`]).
    graphify_vars: Vec<(String, String)>,
}

fn line(node: TsNode) -> u32 {
    Ctx::line(node)
}

/// The line of a declaration keyword: the first child that is not a decorator.
fn keyword_line(node: TsNode) -> u32 {
    let mut cur = node.walk();
    for c in node.children(&mut cur) {
        if c.kind() != "decorator" && c.kind() != "comment" {
            return line(c);
        }
    }
    line(node)
}

fn is_function_value(node: TsNode) -> bool {
    matches!(
        node.kind(),
        "arrow_function" | "function_expression" | "function" | "generator_function"
    )
}

/// An expression that is a statement directly in a method's body (not nested in a block or a
/// callback).
fn method_body_statement(expr: TsNode) -> bool {
    let block = expr
        .parent()
        .filter(|p| p.kind() == "expression_statement")
        .and_then(|s| s.parent())
        .filter(|b| b.kind() == "statement_block");
    let Some(owner) = block.and_then(|b| b.parent()) else {
        return false;
    };
    match owner.kind() {
        "method_definition" => true,
        "arrow_function" | "function_expression" => owner
            .parent()
            .is_some_and(|p| matches!(p.kind(), "public_field_definition" | "field_definition")),
        _ => false,
    }
}

fn named_children(node: TsNode) -> Vec<TsNode> {
    let mut cur = node.walk();
    node.named_children(&mut cur).collect()
}

fn has_token(node: TsNode, token: &str) -> bool {
    let mut cur = node.walk();
    let found = node
        .children(&mut cur)
        .any(|c| !c.is_named() && c.kind() == token);
    found
}

impl<'a> Walker<'a, '_> {
    fn text(&self, node: TsNode) -> &'a str {
        self.cx.text(node)
    }

    /// Unquoted value of a string or substitution-free template literal.
    fn string_value(&self, node: TsNode) -> Option<String> {
        match node.kind() {
            "string" => {
                let t = self.text(node);
                Some(
                    t.get(1..t.len().saturating_sub(1))
                        .unwrap_or("")
                        .to_string(),
                )
            }
            "template_string" => {
                let mut cur = node.walk();
                if node
                    .named_children(&mut cur)
                    .any(|c| c.kind() == "template_substitution")
                {
                    return None;
                }
                let t = self.text(node);
                Some(
                    t.get(1..t.len().saturating_sub(1))
                        .unwrap_or("")
                        .to_string(),
                )
            }
            _ => None,
        }
    }

    fn file_id(&self) -> String {
        self.cx.file_id.clone()
    }

    fn contains(&mut self, parent: &str, child: &str, line: u32, context: Option<&str>) {
        self.cx.add_edge(parent, child, "contains", line, context);
    }

    fn new_caller(
        &mut self,
        id: String,
        parent: Option<usize>,
        class: Option<usize>,
        top: bool,
    ) -> usize {
        self.callers.push(Caller {
            id,
            parent,
            class,
            top,
            nested: Vec::new(),
            typed: Vec::new(),
            value: false,
            shadow: Vec::new(),
            this_alias: Vec::new(),
            ctor: false,
            static_method: false,
            params: Vec::new(),
            field_fn: false,
        });
        self.callers.len() - 1
    }

    /// A module-level function node (`name()`, `_callable`) contained by the file.
    fn top_function(&mut self, name: &str, line: u32) -> usize {
        let id = Ctx::child_id(&self.cx.file_id, name);
        self.cx.add_def(
            id.clone(),
            format!("{name}()"),
            line,
            vec![flag("_callable")],
        );
        let file = self.file_id();
        self.contains(&file, &id, line, None);
        self.facts.top.push(TopDef {
            name: name.to_string(),
            id: id.clone(),
            kind: DefKind::Function,
        });
        self.new_caller(id, None, None, true)
    }

    fn top_def(&mut self, name: &str, label: String, line: u32, kind: DefKind) -> String {
        let id = Ctx::child_id(&self.cx.file_id, name);
        let flags = match kind {
            DefKind::Class | DefKind::Interface | DefKind::TypeAlias | DefKind::Enum => {
                vec![flag("_callable"), flag("_callable_class")]
            }
            DefKind::Function => vec![flag("_callable")],
            DefKind::Namespace | DefKind::Value => vec![],
        };
        self.cx.add_def(id.clone(), label, line, flags);
        let file = self.file_id();
        self.contains(&file, &id, line, None);
        self.facts.top.push(TopDef {
            name: name.to_string(),
            id: id.clone(),
            kind,
        });
        id
    }

    /// Walk a function-like body: parameters' types become typed locals of `caller`.
    fn function_body(&mut self, func: TsNode, caller: usize, st: St, depth: u32) {
        if let Some(params) = func.child_by_field_name("parameters") {
            // Graphify types receivers by the parameters of module-level functions and
            // methods only (not nested functions or callbacks).
            let own = self.callers[caller].top || self.callers[caller].class.is_some();
            self.params(params, Some(caller), own, st, depth);
        }
        if let Some(body) = func.child_by_field_name("body") {
            self.walk(body, st.inner(Some(caller)), depth + 1);
        }
    }

    /// Parameters: annotated names type their receivers (in `caller` and, as Graphify does,
    /// file-wide); default values are walked as part of the function.
    fn params(&mut self, params: TsNode, caller: Option<usize>, typed: bool, st: St, depth: u32) {
        for p in named_children(params) {
            if !matches!(p.kind(), "required_parameter" | "optional_parameter") {
                // JavaScript: `a`, `a = f()`, `{ a = f() }`.
                let ident = match p.kind() {
                    "identifier" => Some(p),
                    "assignment_pattern" => p
                        .child_by_field_name("left")
                        .filter(|l| l.kind() == "identifier"),
                    _ => None,
                };
                if let (Some(c), Some(i)) = (caller, ident) {
                    let name = self.text(i).to_string();
                    self.callers[c].shadow.push(name);
                }
                if matches!(p.kind(), "assignment_pattern" | "object_pattern") {
                    self.walk(p, st.inner(caller), depth + 1);
                }
                continue;
            }
            if let (Some(c), Some(i)) = (
                caller,
                p.child_by_field_name("pattern")
                    .filter(|i| i.kind() == "identifier"),
            ) {
                let name = self.text(i).to_string();
                self.callers[c].shadow.push(name);
            }
            if let Some(t) = p.child_by_field_name("type") {
                self.scan_annotation(t);
            }
            let pat = p.child_by_field_name("pattern");
            let ty = p
                .child_by_field_name("type")
                .and_then(|t| t.named_child(0))
                .and_then(|t| self.receiver_type(t));
            if let (true, Some(pat), Some(ty)) =
                (typed, pat.filter(|p| p.kind() == "identifier"), ty)
            {
                let name = self.text(pat).to_string();
                if let Some(c) = caller {
                    self.callers[c].typed.push((name.clone(), ty.clone()));
                    self.callers[c].params.push((name.clone(), ty.clone()));
                }
                self.file_typed.push((name, ty));
            }
            if let Some(v) = p.child_by_field_name("value") {
                self.walk(v, st.inner(caller), depth + 1);
            }
            if let Some(pat) = pat.filter(|p| p.kind() == "object_pattern") {
                self.walk(pat, st.inner(caller), depth + 1);
            }
        }
    }

    /// `name: Type` parameters inside a type (function types, method signatures): Graphify
    /// types receivers by these names anywhere in the file.
    fn scan_annotation(&mut self, ann: TsNode) {
        let mut stack = vec![ann];
        while let Some(n) = stack.pop() {
            let (name, ty) = match n.kind() {
                "required_parameter" | "optional_parameter" => (
                    n.child_by_field_name("pattern"),
                    n.child_by_field_name("type"),
                ),
                _ => (None, None),
            };
            if let (Some(name), Some(ty)) = (name, ty) {
                if matches!(name.kind(), "identifier" | "property_identifier") {
                    if let Some(t) = ty.named_child(0).and_then(|t| self.receiver_type(t)) {
                        self.file_typed.push((self.text(name).to_string(), t));
                    }
                }
            }
            stack.extend(named_children(n));
        }
    }

    /// The class a receiver annotation names: `Foo`, or `Foo | null | undefined`. Generic
    /// annotations (`Foo<T>`) do not type a receiver, as in Graphify.
    fn receiver_type(&self, ty: TsNode) -> Option<String> {
        let mut found = None;
        let mut stack = vec![ty];
        while let Some(n) = stack.pop() {
            match n.kind() {
                "type_identifier" if found.is_none() => found = Some(self.text(n).to_string()),
                "union_type" => stack.extend(named_children(n)),
                "literal_type" | "predefined_type"
                    if matches!(self.text(n), "null" | "undefined") => {}
                _ => return None,
            }
        }
        found
    }

    /// `Foo` or `Foo<…>` → `Foo`.
    fn simple_type_name(&self, ty: TsNode) -> Option<String> {
        match ty.kind() {
            "type_identifier" => Some(self.text(ty).to_string()),
            "generic_type" => {
                let n = ty.child_by_field_name("name")?;
                (n.kind() == "type_identifier").then(|| self.text(n).to_string())
            }
            _ => None,
        }
    }

    fn children(&mut self, node: TsNode, st: St, depth: u32) {
        let mut cur = node.walk();
        if !cur.goto_first_child() {
            return;
        }
        loop {
            let c = cur.node();
            if c.is_named() {
                self.walk(c, st, depth + 1);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    fn walk(&mut self, node: TsNode, st: St, depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        match node.kind() {
            "comment" => {}
            "import_statement" => self.import_statement(node),
            "export_statement" => self.export_statement(node, st, depth),
            "function_declaration" | "generator_function_declaration" => {
                self.function_decl(node, st, depth);
            }
            "class_declaration" | "abstract_class_declaration" => {
                self.class_decl(node, st, depth, &[]);
            }
            "interface_declaration" if st.hoist() => {
                self.interface_decl(node);
            }
            "type_alias_declaration" if st.hoist() => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    self.top_def(name, name.to_string(), line(node), DefKind::TypeAlias);
                }
                if let Some(v) = node.child_by_field_name("value") {
                    self.scan_annotation(v);
                }
            }
            "enum_declaration" if st.hoist() => self.enum_decl(node),
            "internal_module" | "module" if st.module => self.namespace(node, st, depth),
            "lexical_declaration" | "variable_declaration" => self.lexical(node, st, depth),
            "assignment_expression" if st.caller.is_some() => self.this_assignment(node, st, depth),
            "expression_statement" if st.module => self.module_statement(node, st, depth),
            "call_expression" => self.call(node, st, depth),
            "new_expression" => self.new_expr(node, st, depth),
            "jsx_opening_element" | "jsx_self_closing_element" => {
                self.jsx(node, st);
                self.children(node, st, depth);
            }
            "arrow_function" | "function_expression" | "function" | "generator_function" => {
                // Anonymous callbacks are transparent: their calls belong to the enclosing caller.
                let caller = st.caller.or(st.lazy);
                if let Some(params) = node.child_by_field_name("parameters") {
                    self.params(params, caller, false, st, depth);
                } else if let (Some(c), Some(p)) = (caller, node.child_by_field_name("parameter")) {
                    let name = self.text(p).to_string();
                    self.callers[c].shadow.push(name);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    let mut inner = st.inner(caller);
                    inner.foreign_this = st.foreign_this || node.kind() != "arrow_function";
                    self.walk(body, inner, depth + 1);
                }
            }
            // An object-literal method passed to module-level code (`expect.extend({ m() {} })`)
            // is a module-level function.
            "method_definition" if st.caller.is_none() && st.lazy.is_none() && !st.in_value => {
                match node.child_by_field_name("name") {
                    Some(n) => {
                        let name = self.text(n);
                        let caller = self.top_function(name, keyword_line(node));
                        self.function_body(node, caller, st, depth);
                    }
                    None => self.children(node, St::sealed(), depth),
                }
            }
            // Other object-literal methods, class expressions and static blocks own their calls
            // but have no node, so nothing inside them is attributed.
            "class" => self.class_expression(node, st, depth),
            "method_definition" | "class_static_block" => {
                self.children(node, St::sealed(), depth);
            }
            "object" => {
                self.collection(node, st);
                self.children(node, st, depth);
            }
            "array" => {
                self.collection(node, st);
                self.children(node, st, depth);
            }
            _ => self.children(node, st, depth),
        }
    }

    fn ref_owner(&self, st: St) -> Option<Option<usize>> {
        match st.caller {
            Some(c) => Some(Some(c)),
            None if st.file_owner => Some(None),
            None => None,
        }
    }

    /// Function names placed in an object or array literal (`indirect_call`, `collection`).
    fn collection(&mut self, node: TsNode, st: St) {
        let Some(owner) = self.ref_owner(st) else {
            return;
        };
        for c in named_children(node) {
            let ident = match c.kind() {
                "identifier" | "shorthand_property_identifier" => Some(c),
                "pair" => c
                    .child_by_field_name("value")
                    .filter(|v| v.kind() == "identifier"),
                _ => None,
            };
            if let Some(i) = ident {
                self.events.push(Event::Ref {
                    owner,
                    name: self.text(i).to_string(),
                    context: RefContext::Collection,
                    line: line(i),
                });
            }
        }
    }

    fn arguments(&mut self, args: TsNode, st: St, depth: u32) {
        let owner = self.ref_owner(st);
        for a in named_children(args) {
            if a.kind() == "identifier" {
                if let Some(owner) = owner {
                    self.events.push(Event::Ref {
                        owner,
                        name: self.text(a).to_string(),
                        context: RefContext::Argument,
                        line: line(a),
                    });
                }
            } else {
                self.walk(a, st, depth + 1);
            }
        }
    }

    fn import_statement(&mut self, node: TsNode) {
        let type_only = has_token(node, "type");
        let mut names = Vec::new();
        let mut spec = node
            .child_by_field_name("source")
            .and_then(|s| self.string_value(s));
        for c in named_children(node) {
            match c.kind() {
                "import_clause" => {
                    for part in named_children(c) {
                        match part.kind() {
                            "identifier" => names.push(ImportName {
                                imported: "default".into(),
                                local: self.text(part).to_string(),
                            }),
                            "namespace_import" => {
                                if let Some(id) = named_children(part)
                                    .into_iter()
                                    .find(|n| n.kind() == "identifier")
                                {
                                    names.push(ImportName {
                                        imported: "*".into(),
                                        local: self.text(id).to_string(),
                                    });
                                }
                            }
                            "named_imports" => {
                                for s in named_children(part) {
                                    if s.kind() != "import_specifier" {
                                        continue;
                                    }
                                    let Some(n) = s.child_by_field_name("name") else {
                                        continue;
                                    };
                                    let imported = self.name_text(n);
                                    let local = s
                                        .child_by_field_name("alias")
                                        .map_or_else(|| imported.clone(), |a| self.name_text(a));
                                    names.push(ImportName { imported, local });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "import_require_clause" => {
                    spec = c
                        .child_by_field_name("source")
                        .and_then(|s| self.string_value(s));
                    if let Some(id) = named_children(c)
                        .into_iter()
                        .find(|n| n.kind() == "identifier")
                    {
                        names.push(ImportName {
                            imported: "*".into(),
                            local: self.text(id).to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
        if let Some(spec) = spec {
            self.facts.imports.push(Import {
                spec,
                line: line(node),
                kind: ImportKind::Import,
                owner: None,
                type_only,
                names,
            });
        }
    }

    /// Identifier text, or a string's unquoted value (`import { "a-b" as x }`).
    fn name_text(&self, node: TsNode) -> String {
        if node.kind() == "string" {
            self.string_value(node).unwrap_or_default()
        } else {
            self.text(node).to_string()
        }
    }

    fn export_statement(&mut self, node: TsNode, st: St, depth: u32) {
        let is_default = has_token(node, "default");
        if let Some(src) = node.child_by_field_name("source") {
            let Some(spec) = self.string_value(src) else {
                return;
            };
            let ln = line(node);
            let mut kind = ImportKind::ExportStar;
            let mut names = Vec::new();
            for c in named_children(node) {
                match c.kind() {
                    "namespace_export" => {
                        if let Some(a) = named_children(c).into_iter().next() {
                            let alias = self.name_text(a);
                            let id = Ctx::child_id(&self.cx.file_id, &alias);
                            self.cx.add_def(id.clone(), alias.clone(), ln, vec![]);
                            let file = self.file_id();
                            self.contains(&file, &id, ln, Some("namespace_export"));
                            self.facts.top.push(TopDef {
                                name: alias.clone(),
                                id,
                                kind: DefKind::Namespace,
                            });
                            names.push(ImportName {
                                imported: "*".into(),
                                local: alias,
                            });
                        }
                        kind = ImportKind::ExportStarAs;
                    }
                    "export_clause" => {
                        kind = ImportKind::ReExport;
                        names = self.export_specifiers(c);
                    }
                    _ => {}
                }
            }
            self.facts.imports.push(Import {
                spec,
                line: ln,
                kind,
                owner: None,
                type_only: has_token(node, "type"),
                names,
            });
            return;
        }
        let decorators: Vec<TsNode> = named_children(node)
            .into_iter()
            .filter(|c| c.kind() == "decorator")
            .collect();
        let inner = St {
            exported: true,
            ..st
        };
        if let Some(decl) = node.child_by_field_name("declaration") {
            let id = match decl.kind() {
                "class_declaration" | "abstract_class_declaration" => {
                    self.class_decl(decl, inner, depth + 1, &decorators)
                }
                "function_declaration" | "generator_function_declaration" => {
                    self.function_decl(decl, inner, depth + 1)
                }
                _ => {
                    self.walk(decl, inner, depth + 1);
                    None
                }
            };
            if is_default {
                if let Some(id) = id {
                    self.facts.default_export = Some(DefaultExport::Def(id));
                }
            }
            return;
        }
        if let Some(value) = node.child_by_field_name("value") {
            if is_default && value.kind() == "identifier" {
                self.facts.default_export = Some(DefaultExport::Name(self.text(value).to_string()));
            } else if value.kind() == "class" && value.child_by_field_name("name").is_some() {
                let id = self.class_decl(value, inner, depth + 1, &decorators);
                if let (true, Some(id)) = (is_default, id) {
                    self.facts.default_export = Some(DefaultExport::Def(id));
                }
            } else if is_function_value(value) && value.child_by_field_name("name").is_some() {
                let id = self.function_decl(value, inner, depth + 1);
                if let (true, Some(id)) = (is_default, id) {
                    self.facts.default_export = Some(DefaultExport::Def(id));
                }
            } else if is_function_value(value) {
                // An anonymous default export has no node; nothing inside is attributed.
                self.walk(value, St::sealed(), depth + 1);
            } else {
                self.walk(value, inner, depth + 1);
            }
            return;
        }
        for c in named_children(node) {
            if c.kind() == "export_clause" {
                for n in self.export_specifiers(c) {
                    self.facts
                        .local_exports
                        .push((n.local, n.imported, line(node)));
                }
            }
        }
    }

    /// `{ a, b as c }` → (imported a, local a), (imported b, local c).
    fn export_specifiers(&self, clause: TsNode) -> Vec<ImportName> {
        named_children(clause)
            .into_iter()
            .filter(|s| s.kind() == "export_specifier")
            .filter_map(|s| {
                let imported = self.name_text(s.child_by_field_name("name")?);
                let local = s
                    .child_by_field_name("alias")
                    .map_or_else(|| imported.clone(), |a| self.name_text(a));
                Some(ImportName { imported, local })
            })
            .collect()
    }

    /// A function declaration; returns its id when a node was emitted.
    fn function_decl(&mut self, node: TsNode, st: St, depth: u32) -> Option<String> {
        let Some(name_node) = node.child_by_field_name("name") else {
            if let Some(body) = node.child_by_field_name("body") {
                self.walk(body, St::sealed(), depth + 1);
            }
            return None;
        };
        let name = self.text(name_node);
        let ln = line(node);
        // Declarations in module-level callbacks (`describe(() => { function f() {} })`) are
        // module-level functions.
        let caller = if st.hoist() {
            self.top_function(name, ln)
        } else if let Some(parent) = st.caller.filter(|&p| self.callers[p].value) {
            // Inside a value initializer: no node, the calls stay with the value.
            self.function_body(node, parent, st, depth);
            return None;
        } else if let Some(parent) = st.caller {
            let parent_id = self.callers[parent].id.clone();
            let id = Ctx::child_id(&parent_id, name);
            self.cx
                .add_def(id.clone(), format!("{name}()"), ln, vec![flag("_callable")]);
            self.contains(&parent_id, &id, ln, None);
            self.callers[parent]
                .nested
                .push((name.to_string(), id.clone()));
            self.new_caller(id, Some(parent), None, false)
        } else {
            if let Some(body) = node.child_by_field_name("body") {
                self.walk(body, St::sealed(), depth + 1);
            }
            return None;
        };
        self.function_body(node, caller, st, depth);
        Some(self.callers[caller].id.clone())
    }

    fn decorator(&mut self, source: &str, deco: TsNode) {
        let Some(expr) = deco.named_child(0) else {
            return;
        };
        let target = match expr.kind() {
            "identifier" => Some(expr),
            "call_expression" => expr
                .child_by_field_name("function")
                .filter(|f| f.kind() == "identifier"),
            _ => None,
        };
        if let Some(t) = target {
            self.facts.decorators.push(Decorator {
                source: source.to_string(),
                name: self.text(t).to_string(),
                line: line(deco),
            });
        }
    }

    fn class_decl(
        &mut self,
        node: TsNode,
        st: St,
        depth: u32,
        outer_decorators: &[TsNode],
    ) -> Option<String> {
        let name_node = node.child_by_field_name("name");
        let (Some(name_node), true) = (name_node, st.hoist()) else {
            if name_node.is_none() {
                // `export default class { … }`
                self.class_expression(node, st, depth);
                return None;
            }
            // Classes inside functions are not emitted.
            if let Some(body) = node.child_by_field_name("body") {
                self.children(body, St::sealed(), depth + 1);
            }
            return None;
        };
        let name = self.text(name_node);
        let ln = keyword_line(node);
        let id = self.top_def(name, name.to_string(), ln, DefKind::Class);
        let class = self.classes.len();
        self.classes.push(Class {
            id: id.clone(),
            base: None,
            methods: Vec::new(),
            fields: Vec::new(),
            declared: Vec::new(),
            assigned: Vec::new(),
            renames: Default::default(),
            public_twins: Default::default(),
        });
        for d in outer_decorators {
            self.decorator(&id, *d);
        }
        for c in named_children(node) {
            match c.kind() {
                "decorator" => self.decorator(&id, c),
                "class_heritage" => self.heritage(c, class, ln),
                _ => {}
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.member_renames(class, body);
            self.class_body(body, class, depth);
        }
        self.settle_fields(class);
        Some(id)
    }

    fn heritage(&mut self, node: TsNode, class: usize, ln: u32) {
        let mut found: Vec<(String, bool)> = Vec::new();
        for c in named_children(node) {
            match c.kind() {
                "extends_clause" => {
                    let mut cur = c.walk();
                    for v in c.children_by_field_name("value", &mut cur) {
                        if v.kind() == "identifier" {
                            found.push((self.text(v).to_string(), false));
                        }
                    }
                }
                "implements_clause" => {
                    for t in named_children(c) {
                        if let Some(n) = self.simple_type_name(t) {
                            found.push((n, true));
                        }
                    }
                }
                // JavaScript: `class_heritage` holds the base expression directly.
                "identifier" => found.push((self.text(c).to_string(), false)),
                _ => {}
            }
        }
        let source = self.classes[class].id.clone();
        for (name, implements) in found {
            if !implements && self.classes[class].base.is_none() {
                self.classes[class].base = Some(name.clone());
            }
            self.facts.heritage.push(Heritage {
                source: source.clone(),
                name,
                implements,
                line: ln,
            });
        }
    }

    /// A class expression (or anonymous class) in module-level code. Graphify flattens its
    /// methods into module-level functions (`m()`, contained by the file); this is kept for
    /// parity because it is how frappe-style code (`ns.Controller = class extends … {}`) gets
    /// any nodes at all. `this.m()` still binds within the class.
    fn class_expression(&mut self, node: TsNode, st: St, depth: u32) {
        let Some(body) = node.child_by_field_name("body").filter(|_| st.hoist()) else {
            self.children(node, St::sealed(), depth);
            return;
        };
        let class = self.classes.len();
        self.classes.push(Class {
            id: String::new(),
            base: None,
            methods: Vec::new(),
            fields: Vec::new(),
            declared: Vec::new(),
            assigned: Vec::new(),
            renames: Default::default(),
            public_twins: Default::default(),
        });
        for m in named_children(body) {
            let (name, func) = match m.kind() {
                "method_definition" => (m.child_by_field_name("name"), Some(m)),
                "public_field_definition" | "field_definition" => (
                    m.child_by_field_name("name")
                        .or_else(|| m.child_by_field_name("property")),
                    m.child_by_field_name("value")
                        .filter(|v| is_function_value(*v)),
                ),
                _ => (None, None),
            };
            let (Some(name), Some(func)) = (name, func) else {
                self.children(m, St::sealed(), depth + 1);
                continue;
            };
            let raw = self.text(name);
            let caller = self.top_function(raw, keyword_line(m));
            self.callers[caller].class = Some(class);
            let id = self.callers[caller].id.clone();
            if !self.classes[class].methods.iter().any(|(n, _)| n == raw) {
                self.classes[class].methods.push((raw.to_string(), id));
            }
            self.function_body(func, caller, St::opaque(), depth);
        }
    }

    /// `constructor(private svc: Svc)` declares a typed field.
    fn parameter_properties(&mut self, ctor: TsNode, class: usize) {
        let Some(params) = ctor.child_by_field_name("parameters") else {
            return;
        };
        for p in named_children(params) {
            let is_property = named_children(p)
                .iter()
                .any(|c| c.kind() == "accessibility_modifier")
                || has_token(p, "readonly");
            let pat = p
                .child_by_field_name("pattern")
                .filter(|n| n.kind() == "identifier");
            let ty = p
                .child_by_field_name("type")
                .and_then(|t| t.named_child(0))
                .and_then(|t| self.receiver_type(t));
            if let (true, Some(pat), Some(ty)) = (is_property, pat, ty) {
                let name = self.text(pat).to_string();
                self.classes[class].declared.push((name, ty, false));
            }
        }
    }

    fn class_body(&mut self, body: TsNode, class: usize, depth: u32) {
        let mut pending: Vec<TsNode> = Vec::new();
        for m in named_children(body) {
            match m.kind() {
                "decorator" => {
                    pending.push(m);
                    continue;
                }
                "method_definition" | "abstract_method_signature" => {
                    let Some(name) = m.child_by_field_name("name") else {
                        pending.clear();
                        continue;
                    };
                    let raw = self.text(name);
                    let id = self.method_node(class, raw, keyword_line(m));
                    for d in pending.drain(..) {
                        self.decorator(&id, d);
                    }
                    for d in named_children(m) {
                        if d.kind() == "decorator" {
                            self.decorator(&id, d);
                        }
                    }
                    self.signature_refs(&id, m, keyword_line(m));
                    if raw == "constructor" {
                        self.parameter_properties(m, class);
                    }
                    self.accessor_type(m, raw, class);
                    let caller = self.new_caller(id, None, Some(class), false);
                    self.callers[caller].ctor = raw == "constructor";
                    self.callers[caller].static_method = has_token(m, "static");
                    self.function_body(m, caller, St::opaque(), depth);
                }
                "public_field_definition" | "field_definition" => {
                    pending.clear();
                    let name = m
                        .child_by_field_name("name")
                        .or_else(|| m.child_by_field_name("property"));
                    let value = m.child_by_field_name("value");
                    match (name, value) {
                        (Some(name), Some(v)) if is_function_value(v) => {
                            let raw = self.text(name);
                            let id = self.method_node(class, raw, keyword_line(m));
                            // Arrow-function fields reference only their declared field type,
                            // which also types the field (`handler: EventListener = () => …`).
                            if let Some(t) = m.child_by_field_name("type") {
                                let source = self.classes[class].id.clone();
                                self.type_refs(&source, t, TypeContext::Field, line(m));
                                if let Some(ty) =
                                    t.named_child(0).and_then(|t| self.receiver_type(t))
                                {
                                    let is_static = has_token(m, "static");
                                    self.classes[class].declared.push((
                                        raw.to_string(),
                                        ty,
                                        is_static,
                                    ));
                                }
                            }
                            let caller = self.new_caller(id, None, Some(class), false);
                            self.callers[caller].field_fn = true;
                            self.function_body(v, caller, St::opaque(), depth);
                        }
                        _ => {
                            let annotation = m.child_by_field_name("type");
                            let declared = annotation
                                .and_then(|t| t.named_child(0))
                                .and_then(|t| self.receiver_type(t));
                            // An annotation decides alone (`s: Set<X> = new Set()` types nothing).
                            let constructed = value
                                .filter(|_| annotation.is_none())
                                .filter(|v| v.kind() == "new_expression")
                                .and_then(|v| v.child_by_field_name("constructor"))
                                .filter(|c| c.kind() == "identifier")
                                .map(|c| self.text(c).to_string());
                            if let (Some(n), Some(t)) = (name, declared.or(constructed)) {
                                let field = self.text(n).to_string();
                                self.file_typed.push((field.clone(), t.clone()));
                                let is_static = has_token(m, "static");
                                self.classes[class].declared.push((field, t, is_static));
                            }
                            if let Some(t) = m.child_by_field_name("type") {
                                let source = self.classes[class].id.clone();
                                self.type_refs(&source, t, TypeContext::Field, line(m));
                            }
                            if let Some(v) = value {
                                self.walk(v, St::opaque(), depth + 1);
                            }
                        }
                    }
                }
                "class_static_block" => {
                    pending.clear();
                    self.children(m, St::sealed(), depth + 1);
                }
                _ => pending.clear(),
            }
        }
    }

    /// Class and interface members whose differently written names produce one id (§10.1): when
    /// exactly one of them is public (not `_`- or `#`-prefixed) it keeps the id, every other one
    /// takes `_<sha1(raw name)[:6]>`; identical names still merge. `#n` beside `n` marks both
    /// `_ts_public_twin`.
    fn member_renames(&mut self, class: usize, body: TsNode) {
        let class_id = self.classes[class].id.clone();
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for m in named_children(body) {
            let name = match m.kind() {
                "method_definition" | "abstract_method_signature" | "method_signature" => {
                    m.child_by_field_name("name")
                }
                "public_field_definition" | "field_definition" => m
                    .child_by_field_name("value")
                    .filter(|v| is_function_value(*v))
                    .and_then(|_| {
                        m.child_by_field_name("name")
                            .or_else(|| m.child_by_field_name("property"))
                    }),
                _ => None,
            };
            let Some(n) = name else { continue };
            let raw = self.text(n).to_string();
            let id = Ctx::child_id(&class_id, &raw);
            match groups.iter_mut().find(|(g, _)| *g == id) {
                Some((_, names)) if !names.contains(&raw) => names.push(raw),
                Some(_) => {}
                None => groups.push((id, vec![raw])),
            }
        }
        for (id, names) in groups {
            if names.len() < 2 {
                continue;
            }
            let public: Vec<&String> = names
                .iter()
                .filter(|n| !n.starts_with('_') && !n.starts_with('#'))
                .collect();
            let keep = (public.len() == 1).then(|| public[0].clone());
            for n in &names {
                if let Some(plain) = n.strip_prefix('#') {
                    if names.iter().any(|o| o == plain) {
                        self.classes[class].public_twins.insert(n.clone());
                        self.classes[class].public_twins.insert(plain.to_string());
                    }
                }
            }
            for n in names {
                let final_id = if keep.as_ref() == Some(&n) {
                    id.clone()
                } else {
                    let digest = sha1_smol::Sha1::from(n.as_bytes()).digest().to_string();
                    format!("{id}_{}", &digest[..6])
                };
                self.classes[class].renames.insert(n, final_id);
            }
        }
    }

    fn method_node(&mut self, class: usize, raw: &str, ln: u32) -> String {
        self.method_node_with(class, raw, ln, vec![flag("_callable")])
    }

    fn method_node_with(
        &mut self,
        class: usize,
        raw: &str,
        ln: u32,
        flags: Vec<(String, crate::facts::Attr)>,
    ) -> String {
        let class_id = self.classes[class].id.clone();
        let id = self.classes[class]
            .renames
            .get(raw)
            .cloned()
            .unwrap_or_else(|| Ctx::child_id(&class_id, raw));
        let mut flags = flags;
        if self.classes[class].public_twins.contains(raw) {
            flags.push(flag("_ts_public_twin"));
        }
        self.cx.add_def(id.clone(), format!(".{raw}()"), ln, flags);
        self.cx.add_edge(&class_id, &id, "method", ln, None);
        if !self.classes[class].methods.iter().any(|(n, _)| n == raw) {
            self.classes[class]
                .methods
                .push((raw.to_string(), id.clone()));
        }
        id
    }

    /// Parameter and return type references of a method-like node.
    fn signature_refs(&mut self, source: &str, func: TsNode, ln: u32) {
        if let Some(params) = func.child_by_field_name("parameters") {
            for p in named_children(params) {
                if let Some(t) = p.child_by_field_name("type") {
                    self.type_refs(source, t, TypeContext::Parameter, ln);
                }
            }
        }
        if let Some(r) = func.child_by_field_name("return_type") {
            self.type_refs(source, r, TypeContext::Return, ln);
        }
    }

    /// Every type name in an annotation: `context` at the outer level, `generic_arg` inside
    /// type arguments. Qualified names (`ns.T`) are skipped.
    fn type_refs(&mut self, source: &str, ann: TsNode, context: TypeContext, ln: u32) {
        let mut stack = vec![(ann, context)];
        while let Some((n, ctx)) = stack.pop() {
            match n.kind() {
                "type_identifier" => self.facts.type_refs.push(TypeRef {
                    source: source.to_string(),
                    name: self.text(n).to_string(),
                    context: ctx,
                    line: ln,
                }),
                "nested_type_identifier" => {}
                // `typeof x` / `typeof x.y` names a value; Graphify references it too.
                "type_query" => {
                    let mut q = n.named_child(0);
                    while let Some(e) = q.filter(|e| e.kind() == "member_expression") {
                        q = e.child_by_field_name("object");
                    }
                    if let Some(i) = q.filter(|i| i.kind() == "identifier") {
                        self.facts.type_refs.push(TypeRef {
                            source: source.to_string(),
                            name: self.text(i).to_string(),
                            context: ctx,
                            line: ln,
                        });
                    }
                }
                _ => {
                    let inner = if n.kind() == "type_arguments" {
                        TypeContext::GenericArg
                    } else {
                        ctx
                    };
                    let kids = named_children(n);
                    for k in kids.into_iter().rev() {
                        stack.push((k, inner));
                    }
                }
            }
        }
    }

    fn interface_decl(&mut self, node: TsNode) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let ln = line(node);
        let id = self.top_def(name, name.to_string(), ln, DefKind::Interface);
        let class = self.classes.len();
        self.classes.push(Class {
            id: id.clone(),
            base: None,
            methods: Vec::new(),
            fields: Vec::new(),
            declared: Vec::new(),
            assigned: Vec::new(),
            renames: Default::default(),
            public_twins: Default::default(),
        });
        for c in named_children(node) {
            if c.kind() == "extends_type_clause" {
                for t in named_children(c) {
                    if let Some(n) = self.simple_type_name(t) {
                        self.facts.heritage.push(Heritage {
                            source: id.clone(),
                            name: n,
                            implements: false,
                            line: ln,
                        });
                    }
                }
            }
        }
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        self.scan_annotation(body);
        self.member_renames(class, body);
        for m in named_children(body) {
            match m.kind() {
                "method_signature" => {
                    if let Some(n) = m.child_by_field_name("name") {
                        let raw = self.text(n);
                        let mid = self.method_node(class, raw, line(m));
                        self.signature_refs(&mid, m, line(m));
                    }
                }
                "property_signature" => {
                    if let Some(t) = m.child_by_field_name("type") {
                        self.type_refs(&id, t, TypeContext::Field, line(m));
                    }
                }
                _ => {}
            }
        }
    }

    fn enum_decl(&mut self, node: TsNode) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let id = self.top_def(name, name.to_string(), line(node), DefKind::Enum);
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        for m in named_children(body) {
            let name_node = match m.kind() {
                "enum_assignment" => m.child_by_field_name("name"),
                "property_identifier" | "string" | "number" | "computed_property_name" => Some(m),
                _ => None,
            };
            let Some(n) = name_node else { continue };
            let member = self.name_text(n);
            let mid = Ctx::child_id(&id, &member);
            self.cx.add_def(mid.clone(), member, line(n), vec![]);
            self.cx.add_edge(&id, &mid, "case_of", line(n), None);
        }
    }

    fn namespace(&mut self, node: TsNode, st: St, depth: u32) {
        if let Some(n) = node.child_by_field_name("name") {
            let name = self.name_text(n);
            self.top_def(&name, name.clone(), line(node), DefKind::Namespace);
        }
        if let Some(body) = node.child_by_field_name("body") {
            // Namespace members are flattened into the module.
            self.children(
                body,
                St {
                    exported: false,
                    ..st
                },
                depth + 1,
            );
        }
    }

    /// `require("m")`, or `require("m").name` → (specifier, member name).
    fn require_call(&self, node: TsNode) -> Option<(String, Option<String>)> {
        match node.kind() {
            "call_expression" => {
                let f = node.child_by_field_name("function")?;
                if f.kind() != "identifier" || self.text(f) != "require" {
                    return None;
                }
                let arg = node.child_by_field_name("arguments")?.named_child(0)?;
                Some((self.string_value(arg)?, None))
            }
            "member_expression" => {
                let obj = node.child_by_field_name("object")?;
                let (spec, None) = self.require_call(obj)? else {
                    return None;
                };
                let prop = node.child_by_field_name("property")?;
                Some((spec, Some(self.text(prop).to_string())))
            }
            _ => None,
        }
    }

    fn lexical(&mut self, node: TsNode, st: St, depth: u32) {
        let is_var = node.kind() == "variable_declaration";
        let direct = st.module
            && !is_var
            && node.parent().is_some_and(|p| {
                matches!(p.kind(), "program" | "export_statement")
                    || (p.kind() == "statement_block"
                        && p.parent()
                            .is_some_and(|g| matches!(g.kind(), "internal_module" | "module")))
            });
        for d in named_children(node) {
            if d.kind() != "variable_declarator" {
                continue;
            }
            let Some(name_node) = d.child_by_field_name("name") else {
                continue;
            };
            let value = d.child_by_field_name("value");
            let ln = line(d);
            if let Some((spec, member)) = value.and_then(|v| self.require_call(v)) {
                let owner = st.caller.map(|c| self.callers[c].id.clone());
                let mut names = Vec::new();
                if owner.is_none() {
                    if let Some(m) = member {
                        names.push(ImportName {
                            imported: m.clone(),
                            local: m,
                        });
                    } else if name_node.kind() == "object_pattern" {
                        names = self.pattern_names(name_node);
                    } else if name_node.kind() == "identifier" {
                        names.push(ImportName {
                            imported: "*".into(),
                            local: self.text(name_node).to_string(),
                        });
                    }
                }
                self.facts.imports.push(Import {
                    spec,
                    line: ln,
                    kind: ImportKind::Require,
                    owner,
                    type_only: false,
                    names,
                });
                let is_call = value.is_some_and(|v| v.kind() == "call_expression");
                // A relative `require("./m")` binding is no node; a package's is (§10.1).
                let relative = value
                    .and_then(|v| v.child_by_field_name("arguments"))
                    .and_then(|a| a.named_child(0))
                    .is_some_and(|a| {
                        a.kind() == "string"
                            && self.string_value(a).is_some_and(|s| s.starts_with('.'))
                    });
                if direct && (is_call || st.exported) && !relative {
                    self.value_node(name_node, ln);
                }
                continue;
            }
            if direct && name_node.kind() == "identifier" {
                if let Some(v) = value.filter(|v| is_function_value(*v)) {
                    let name = self.text(name_node);
                    let caller = self.top_function(name, ln);
                    self.function_body(v, caller, st, depth);
                    continue;
                }
            }
            let value_emitted = direct
                && (st.exported
                    || value.is_some_and(|v| {
                        matches!(
                            v.kind(),
                            "object"
                                | "array"
                                | "new_expression"
                                | "call_expression"
                                | "as_expression"
                                | "satisfies_expression"
                        )
                    }));
            if direct {
                if value_emitted {
                    let id = self.value_node(name_node, ln);
                    if let Some(v) =
                        value.filter(|v| matches!(v.kind(), "new_expression" | "call_expression"))
                    {
                        if let Some(ctor) = v.child_by_field_name("constructor").filter(|c| {
                            c.kind() == "identifier" && name_node.kind() == "identifier"
                        }) {
                            let pair = (
                                self.text(name_node).to_string(),
                                self.text(ctor).to_string(),
                            );
                            self.file_typed.push(pair);
                        }
                        // Callbacks in the initializer (`memo(() => …)`, `register({ perform: … })`)
                        // are attributed to the value; the initializer call itself is not.
                        let lazy = self.new_caller(id, None, None, true);
                        self.callers[lazy].value = true;
                        let inner = St {
                            exported: false,
                            lazy: Some(lazy),
                            ..st
                        };
                        self.walk(v, inner, depth + 1);
                        continue;
                    }
                }
            } else if let (Some(c), "identifier") = (st.caller, name_node.kind()) {
                let local = self.text(name_node).to_string();
                if value.is_some_and(|v| v.kind() == "this") {
                    self.callers[c].this_alias.push(local.clone());
                }
                self.callers[c].shadow.push(local);
            }
            if let (false, Some(c), Some(v), "identifier") =
                (direct, st.caller, value, name_node.kind())
            {
                if v.kind() == "new_expression" {
                    if let Some(ctor) = v
                        .child_by_field_name("constructor")
                        .filter(|c| c.kind() == "identifier")
                    {
                        let ty = self.text(ctor).to_string();
                        let name = self.text(name_node).to_string();
                        self.callers[c].typed.push((name, ty));
                    }
                }
            }
            if let Some(v) = value {
                let inner = St {
                    exported: false,
                    in_value: direct && value_emitted,
                    ..st
                };
                self.walk(v, inner, depth + 1);
            }
        }
    }

    /// `{ a, b: c }` (destructured require) → imported a, b.
    fn pattern_names(&self, pat: TsNode) -> Vec<ImportName> {
        let mut out = Vec::new();
        for c in named_children(pat) {
            let (key, local) = match c.kind() {
                "shorthand_property_identifier_pattern" => (Some(c), Some(c)),
                "pair_pattern" => (
                    c.child_by_field_name("key"),
                    c.child_by_field_name("value")
                        .filter(|v| v.kind() == "identifier"),
                ),
                "object_assignment_pattern" => {
                    let l = c.child_by_field_name("left");
                    (l, l)
                }
                _ => (None, None),
            };
            if let Some(k) = key {
                let imported = self.name_text(k);
                let local = local.map_or_else(|| imported.clone(), |l| self.text(l).to_string());
                out.push(ImportName { imported, local });
            }
        }
        out
    }

    fn value_node(&mut self, name_node: TsNode, ln: u32) -> String {
        let label = self.text(name_node);
        let id = Ctx::child_id(&self.cx.file_id, label);
        self.cx.add_def(id.clone(), label.to_string(), ln, vec![]);
        let file = self.file_id();
        self.contains(&file, &id, ln, None);
        if name_node.kind() == "identifier" {
            self.facts.top.push(TopDef {
                name: label.to_string(),
                id: id.clone(),
                kind: DefKind::Value,
            });
        }
        id
    }

    fn top_kind(&self, name: &str) -> Option<&TopDef> {
        self.facts.top.iter().find(|d| d.name == name)
    }

    /// Module-level CommonJS assignments: `exports.x = fn`, `module.exports = {…}`,
    /// `X.prototype.m = fn`.
    fn module_statement(&mut self, node: TsNode, st: St, depth: u32) {
        let Some(expr) = node.named_child(0) else {
            return;
        };
        if expr.kind() != "assignment_expression" {
            self.children(node, st, depth);
            return;
        }
        let (Some(left), Some(right)) = (
            expr.child_by_field_name("left"),
            expr.child_by_field_name("right"),
        ) else {
            return;
        };
        let ln = line(node);
        if left.kind() == "member_expression" {
            let obj = left.child_by_field_name("object");
            let prop = left.child_by_field_name("property");
            if let (Some(obj), Some(prop)) = (obj, prop) {
                let obj_text = self.text(obj);
                let prop_text = self.text(prop);
                if obj_text == "exports" || obj_text == "module.exports" {
                    if is_function_value(right) {
                        let caller = self.top_function(prop_text, ln);
                        self.function_body(right, caller, st, depth);
                    } else {
                        self.walk(right, st, depth + 1);
                    }
                    return;
                }
                if obj_text == "module" && prop_text == "exports" {
                    self.module_exports(right, st, depth);
                    return;
                }
                if obj.kind() == "member_expression" && is_function_value(right) {
                    let owner = obj.child_by_field_name("object");
                    let proto = obj.child_by_field_name("property");
                    if let (Some(owner), Some(proto)) = (owner, proto) {
                        let owner_def = self
                            .top_kind(self.text(owner))
                            .filter(|_| self.text(proto) == "prototype")
                            .map(|d| d.id.clone());
                        if let Some(owner_id) = owner_def {
                            let id = Ctx::child_id(&owner_id, prop_text);
                            self.cx.add_def(
                                id.clone(),
                                format!(".{prop_text}()"),
                                ln,
                                vec![flag("_callable")],
                            );
                            self.cx.add_edge(&owner_id, &id, "method", ln, None);
                            let caller = self.new_caller(id, None, None, false);
                            self.function_body(right, caller, st, depth);
                            return;
                        }
                    }
                }
            }
        }
        self.children(node, st, depth);
    }

    /// `this.name = () => …` inside a method defines a method of the class (without
    /// `_callable`, as Graphify emits it).
    fn this_assignment(&mut self, node: TsNode, st: St, depth: u32) {
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");
        if let (Some(l), Some(r), Some(c), false) = (left, right, st.caller, st.foreign_this) {
            self.field_assignment(c, l, r);
        }
        let class = st.caller.and_then(|c| self.enclosing_class(c));
        if let (Some(left), Some(right), Some(class)) = (left, right, class) {
            let is_this = left.kind() == "member_expression"
                && left
                    .child_by_field_name("object")
                    .is_some_and(|o| o.kind() == "this")
                && method_body_statement(node);
            if let (true, true, Some(prop)) = (
                is_this,
                is_function_value(right),
                left.child_by_field_name("property"),
            ) {
                let raw = self.text(prop);
                let id = self.method_node_with(class, raw, line(node), vec![]);
                let caller = self.new_caller(id, None, Some(class), false);
                self.function_body(right, caller, St::opaque(), depth);
                return;
            }
        }
        self.children(node, st, depth);
    }

    /// `this.x = new X()` or `this.x = p` (`p` a typed parameter) in a method's own body types
    /// field `x` (§10.1 `_ts_fields`); other right-hand sides say nothing.
    fn field_assignment(&mut self, caller: usize, left: TsNode, right: TsNode) {
        let Some(class) = self.callers[caller]
            .class
            .filter(|_| !self.callers[caller].field_fn)
        else {
            return;
        };
        if left.kind() != "member_expression"
            || left
                .child_by_field_name("object")
                .is_none_or(|o| o.kind() != "this")
        {
            return;
        }
        let Some(prop) = left.child_by_field_name("property").filter(|p| {
            matches!(
                p.kind(),
                "property_identifier" | "private_property_identifier"
            )
        }) else {
            return;
        };
        let ty = match right.kind() {
            "new_expression" => right
                .child_by_field_name("constructor")
                .filter(|c| c.kind() == "identifier")
                .map(|c| self.text(c).to_string()),
            "identifier" => {
                let name = self.text(right);
                self.callers[caller]
                    .params
                    .iter()
                    .find(|(p, _)| p == name)
                    .map(|(_, t)| t.clone())
            }
            _ => None,
        };
        if let Some(ty) = ty {
            let c = &self.callers[caller];
            let entry = (self.text(prop).to_string(), ty, c.ctor, c.static_method);
            self.classes[class].assigned.push(entry);
        }
    }

    /// `get x(): T` and `set x(v: T)` declare field `x` of type `T`.
    fn accessor_type(&mut self, m: TsNode, raw: &str, class: usize) {
        let ty = if has_token(m, "get") {
            m.child_by_field_name("return_type")
        } else if has_token(m, "set") {
            m.child_by_field_name("parameters")
                .and_then(|ps| named_children(ps).into_iter().next())
                .and_then(|p| p.child_by_field_name("type"))
        } else {
            return;
        };
        if let Some(t) = ty
            .and_then(|t| t.named_child(0))
            .and_then(|t| self.receiver_type(t))
        {
            let is_static = has_token(m, "static");
            self.classes[class]
                .declared
                .push((raw.to_string(), t, is_static));
        }
    }

    /// Settle a class's field types once its body is walked: declarations first, then
    /// constructor assignments, then other methods' (a tier whose assignments disagree types
    /// nothing); static fields from static declarations and static methods. Writes
    /// `_ts_fields`/`_ts_static_fields` (sorted) on the class node.
    fn settle_fields(&mut self, class: usize) {
        let c = &self.classes[class];
        let tiers = |is_static: bool| -> std::collections::BTreeMap<String, String> {
            let mut out = std::collections::BTreeMap::new();
            for (f, t, st) in &c.declared {
                if *st == is_static {
                    out.entry(f.clone()).or_insert_with(|| t.clone());
                }
            }
            let ranks: &[Option<bool>] = if is_static {
                &[None]
            } else {
                &[Some(true), Some(false)]
            };
            for rank in ranks {
                let mut seen: std::collections::BTreeMap<&str, Option<&str>> =
                    std::collections::BTreeMap::new();
                for (f, t, ctor, st) in &c.assigned {
                    if *st != is_static || rank.is_some_and(|r| r != *ctor) {
                        continue;
                    }
                    let slot = seen.entry(f.as_str()).or_insert(Some(t.as_str()));
                    if slot.is_some_and(|prev| prev != t) {
                        *slot = None;
                    }
                }
                for (f, t) in seen {
                    if let (Some(t), false) = (t, out.contains_key(f)) {
                        out.insert(f.to_string(), t.to_string());
                    }
                }
            }
            out
        };
        let instance = tiers(false);
        let statics = tiers(true);
        let id = c.id.clone();
        self.classes[class].fields = instance
            .iter()
            .map(|(f, t)| (f.clone(), t.clone()))
            .collect();
        let mut attrs = Vec::new();
        for (key, map) in [("_ts_fields", &instance), ("_ts_static_fields", &statics)] {
            if !map.is_empty() {
                let json = serde_json::to_string(map).unwrap_or_default();
                attrs.push((key.to_string(), crate::facts::Attr::Json(json)));
            }
        }
        if attrs.is_empty() || id.is_empty() {
            return;
        }
        if let Some(n) = self.cx.facts.nodes.iter_mut().find(|n| n.id == id) {
            n.attrs.extend(attrs);
        }
    }

    fn module_exports(&mut self, right: TsNode, st: St, depth: u32) {
        match right.kind() {
            "identifier" => {
                self.facts.default_export = Some(DefaultExport::Name(self.text(right).to_string()));
            }
            "object" => {
                self.collection(right, st);
                for m in named_children(right) {
                    match m.kind() {
                        "method_definition" => {
                            let Some(n) = m.child_by_field_name("name") else {
                                continue;
                            };
                            let name = self.text(n);
                            let caller = self.top_function(name, line(m));
                            self.function_body(m, caller, st, depth);
                        }
                        "pair" => {
                            if let Some(v) = m.child_by_field_name("value") {
                                self.walk(v, st, depth + 1);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ if is_function_value(right) => self.walk(right, St::sealed(), depth + 1),
            _ => self.walk(right, st, depth + 1),
        }
    }

    fn call(&mut self, node: TsNode, st: St, depth: u32) {
        let func = node.child_by_field_name("function");
        let args = node.child_by_field_name("arguments");
        if let Some(f) = func {
            match f.kind() {
                // `require` counts only as a declaration's initializer (see `lexical`), as in
                // Graphify; elsewhere it is neither an import nor a call.
                "identifier" if self.text(f) == "require" => return,
                "import" => {
                    let spec = args
                        .and_then(|a| {
                            a.named_children(&mut a.walk())
                                .find(|c| c.kind() != "comment")
                        })
                        .and_then(|a| self.string_value(a));
                    if let (Some(c), Some(spec)) = (st.caller_or_file(), spec) {
                        self.facts.imports.push(Import {
                            spec,
                            line: line(node),
                            kind: ImportKind::Dynamic,
                            owner: (c != FILE).then(|| self.callers[c].id.clone()),
                            type_only: false,
                            names: Vec::new(),
                        });
                    }
                    return;
                }
                "identifier" => {
                    if let Some(c) = st.caller_or_file() {
                        self.events.push(Event::Call {
                            caller: c,
                            name: self.text(f).to_string(),
                            kind: CallKind::Bare,
                            line: line(node),
                        });
                    }
                }
                "member_expression" => {
                    let obj = f.child_by_field_name("object");
                    let prop = f.child_by_field_name("property");
                    if let (Some(c), Some(obj), Some(prop)) = (st.caller, obj, prop) {
                        let recv = match obj.kind() {
                            "this" => Some(Recv::This),
                            "member_expression" => obj
                                .child_by_field_name("object")
                                .filter(|o| o.kind() == "this")
                                .and(obj.child_by_field_name("property"))
                                .map(|p| Recv::ThisField(self.text(p).to_string())),
                            "identifier" => Some(Recv::Ident(self.text(obj).to_string())),
                            _ => None,
                        };
                        if let Some(recv) = recv {
                            self.events.push(Event::Member {
                                caller: c,
                                recv,
                                method: self.text(prop).to_string(),
                                line: line(node),
                            });
                        }
                    } else if let (Some(FILE), Some(obj), Some(prop)) =
                        (st.caller_or_file(), obj, prop)
                    {
                        // Member calls in module-level callbacks resolve nothing, but Graphify
                        // still records unresolved ones on the file node.
                        if obj.kind() == "identifier" {
                            let recv = Recv::Ident(self.text(obj).to_string());
                            let method = self.text(prop).to_string();
                            self.unresolved_candidate(FILE, &recv, &method, line(node));
                        }
                    }
                    if let Some(obj) = obj {
                        self.walk(obj, st, depth + 1);
                    }
                }
                _ => self.walk(f, st, depth + 1),
            }
        }
        if let Some(a) = args {
            self.arguments(a, st, depth);
        }
    }

    fn new_expr(&mut self, node: TsNode, st: St, depth: u32) {
        if let (Some(c), Some(ctor)) =
            (st.caller_or_file(), node.child_by_field_name("constructor"))
        {
            if ctor.kind() == "identifier" {
                self.events.push(Event::Call {
                    caller: c,
                    name: self.text(ctor).to_string(),
                    kind: CallKind::New,
                    line: line(node),
                });
            }
        }
        if let Some(a) = node.child_by_field_name("arguments") {
            self.arguments(a, st, depth);
        }
    }

    fn jsx(&mut self, node: TsNode, st: St) {
        let (Some(c), Some(name)) = (st.caller_or_file(), node.child_by_field_name("name")) else {
            return;
        };
        if name.kind() != "identifier" {
            return;
        }
        let tag = self.text(name);
        if tag.chars().next().is_some_and(char::is_uppercase) {
            self.events.push(Event::Call {
                caller: c,
                name: tag.to_string(),
                kind: CallKind::Jsx,
                line: line(node),
            });
        }
    }

    /// A function or class visible from `caller` by lexical scope: nested function declarations
    /// of the caller chain, then module-level functions and classes.
    fn lookup(&self, caller: Option<usize>, name: &str, for_call: bool) -> Option<String> {
        let mut c = caller;
        while let Some(i) = c {
            if let Some((_, id)) = self.callers[i].nested.iter().find(|(n, _)| n == name) {
                return Some(id.clone());
            }
            c = self.callers[i].parent;
        }
        self.facts
            .top
            .iter()
            .find(|d| {
                d.name == name
                    && (d.kind == DefKind::Function
                        || (for_call && matches!(d.kind, DefKind::Class | DefKind::Value)))
            })
            .map(|d| d.id.clone())
    }

    /// The module-level function that encloses a nested function `caller`.
    fn root_of(&self, caller: usize) -> Option<String> {
        let mut c = caller;
        while let Some(p) = self.callers[c].parent {
            c = p;
        }
        (c != caller && self.callers[c].top).then(|| self.callers[c].id.clone())
    }

    /// `name` was bound to `this` in `caller` or an enclosing caller.
    fn is_this_alias(&self, caller: usize, name: &str) -> bool {
        let mut c = Some(caller);
        while let Some(i) = c {
            if self.callers[i].this_alias.iter().any(|s| s == name) {
                return true;
            }
            c = self.callers[i].parent;
        }
        false
    }

    /// A parameter or local of `caller` (or an enclosing caller) named `name`.
    fn shadowed(&self, caller: usize, name: &str) -> bool {
        let mut c = Some(caller);
        while let Some(i) = c {
            if self.callers[i].shadow.iter().any(|s| s == name) {
                return true;
            }
            c = self.callers[i].parent;
        }
        false
    }

    /// A receiver name's type from annotations or `new` initializers anywhere in the file.
    fn file_type(&self, name: &str) -> Option<String> {
        self.file_typed
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
    }

    /// `x.method()` with `x: ty`: a same-file class's own method, or left for the cross-file
    /// pass.
    fn typed_target(
        &mut self,
        caller: usize,
        ty: String,
        method: String,
        line: u32,
    ) -> Option<String> {
        if let Some(k) = self.local_class(&ty) {
            return self.class_method(&k, &method);
        }
        self.facts.pending.push(Pending::Member {
            caller: self.callers[caller].id.clone(),
            recv: Receiver::Typed(ty),
            method,
            line,
        });
        None
    }

    /// A typed local of `caller` itself (enclosing functions' locals do not count, as in
    /// Graphify).
    fn typed(&self, caller: usize, name: &str) -> Option<String> {
        self.callers[caller]
            .typed
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
    }

    fn class_method_at(&self, class: usize, method: &str) -> Option<String> {
        self.classes[class]
            .methods
            .iter()
            .find(|(n, _)| n == method)
            .map(|(_, id)| id.clone())
    }

    fn class_method(&self, class_id: &str, method: &str) -> Option<String> {
        self.classes
            .iter()
            .find(|c| c.id == class_id)?
            .methods
            .iter()
            .find(|(n, _)| n == method)
            .map(|(_, id)| id.clone())
    }

    /// Same-file class by source name.
    fn local_class(&self, name: &str) -> Option<String> {
        self.facts
            .top
            .iter()
            .find(|d| d.name == name && matches!(d.kind, DefKind::Class | DefKind::Interface))
            .map(|d| d.id.clone())
    }

    /// Resolve recorded calls and references inside the file; keep the rest for the cross-file
    /// pass. Calls are one edge per (caller, callee) at the first call line.
    fn resolve_local(&mut self) {
        // One edge per (caller, callee) across calls and function references: the first wins.
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        let events = std::mem::take(&mut self.events);
        for ev in events {
            match ev {
                Event::Call {
                    caller,
                    name,
                    kind,
                    line,
                } => match self.lookup(Some(caller), &name, true) {
                    Some(target) => self.call_edge(caller, &target, line, &mut seen),
                    // File-level calls resolve only inside the file.
                    None if caller == FILE => {}
                    None => {
                        let root = self.root_of(caller);
                        let c = &self.callers[caller];
                        self.facts.pending.push(Pending::Call {
                            caller: c.id.clone(),
                            top: c.top,
                            root,
                            name,
                            kind,
                            line,
                        });
                    }
                },
                Event::Member {
                    caller,
                    recv,
                    method,
                    line,
                } => {
                    self.unresolved_candidate(caller, &recv, &method, line);
                    // A typed receiver's method is an INFERRED 0.85 call, never to the caller
                    // itself; `this.m()` and `Class.m()` are EXTRACTED.
                    let (target, typed) = match &recv {
                        Recv::This => (
                            self.enclosing_class(caller)
                                .and_then(|k| self.class_method_at(k, &method)),
                            false,
                        ),
                        Recv::ThisField(field) => {
                            let ty = self
                                .enclosing_class(caller)
                                .and_then(|k| {
                                    self.classes[k]
                                        .fields
                                        .iter()
                                        .find(|(f, _)| f == field)
                                        .map(|(_, t)| t.clone())
                                })
                                .or_else(|| self.file_type(field));
                            let Some(t) = ty else { continue };
                            (self.typed_target(caller, t, method, line), true)
                        }
                        Recv::Ident(n) if self.is_this_alias(caller, n) => (
                            self.enclosing_class(caller)
                                .and_then(|k| self.class_method_at(k, &method)),
                            false,
                        ),
                        Recv::Ident(n) => {
                            if let Some(t) = self.typed(caller, n) {
                                (self.typed_target(caller, t, method, line), true)
                            } else if let Some(k) = self.local_class(n) {
                                (self.class_method(&k, &method), false)
                            } else if let Some(t) = self.file_type(n) {
                                (self.typed_target(caller, t, method, line), true)
                            } else {
                                self.facts.pending.push(Pending::Member {
                                    caller: self.callers[caller].id.clone(),
                                    recv: Receiver::Name(n.clone()),
                                    method,
                                    line,
                                });
                                continue;
                            }
                        }
                    };
                    match target {
                        Some(t) if typed => {
                            if t != self.callers[caller].id {
                                self.inferred_call_edge(caller, &t, line, &mut seen);
                            }
                        }
                        Some(t) => self.call_edge(caller, &t, line, &mut seen),
                        None => {}
                    }
                }
                Event::Ref {
                    owner,
                    name,
                    context,
                    line,
                } => {
                    if owner.is_some_and(|o| self.shadowed(o, &name)) {
                        continue;
                    }
                    let owner_id =
                        owner.map_or_else(|| self.file_id(), |o| self.callers[o].id.clone());
                    match self.lookup(owner, &name, false) {
                        Some(target) => {
                            if seen.insert((owner_id.clone(), target.clone())) {
                                let mut e = self.cx.edge(
                                    &owner_id,
                                    &target,
                                    "indirect_call",
                                    Loc::Line(line),
                                    Some(context.as_str()),
                                );
                                e.confidence = Confidence::Inferred;
                                e.score = Some(0.85);
                                self.cx.facts.edges.push(e);
                            }
                        }
                        None => self.facts.pending.push(Pending::Ref {
                            owner: owner.map(|_| owner_id),
                            name,
                            context,
                            line,
                        }),
                    }
                }
            }
        }
    }

    fn enclosing_class(&self, caller: usize) -> Option<usize> {
        let mut c = Some(caller);
        while let Some(i) = c {
            if let Some(k) = self.callers[i].class {
                return Some(k);
            }
            c = self.callers[i].parent;
        }
        None
    }

    fn call_edge(
        &mut self,
        caller: usize,
        target: &str,
        line: u32,
        seen: &mut std::collections::HashSet<(String, String)>,
    ) {
        let src = self.callers[caller].id.clone();
        if seen.insert((src.clone(), target.to_string())) {
            self.cx.add_edge(&src, target, "calls", line, Some("call"));
        }
    }

    /// Collect [`Walker::graphify_vars`] in source order.
    fn scan_graphify_vars(&mut self, root: TsNode) {
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            match n.kind() {
                "required_parameter" | "optional_parameter" => {
                    let pat = n
                        .child_by_field_name("pattern")
                        .filter(|p| p.kind() == "identifier");
                    let ty = n
                        .child_by_field_name("type")
                        .and_then(|t| t.named_child(0))
                        .filter(|t| t.kind() == "type_identifier");
                    if let (Some(p), Some(t)) = (pat, ty) {
                        let pair = (self.text(p).to_string(), self.text(t).to_string());
                        self.graphify_vars.push(pair);
                    }
                }
                "variable_declarator" => {
                    let name = n
                        .child_by_field_name("name")
                        .filter(|p| p.kind() == "identifier");
                    let ctor = n
                        .child_by_field_name("value")
                        .filter(|v| v.kind() == "new_expression")
                        .and_then(|v| v.child_by_field_name("constructor"))
                        .filter(|c| c.kind() == "identifier");
                    if let (Some(name), Some(c)) = (name, ctor) {
                        let pair = (self.text(name).to_string(), self.text(c).to_string());
                        self.graphify_vars.push(pair);
                    }
                }
                _ => {}
            }
            let kids = named_children(n);
            stack.extend(kids.into_iter().rev());
        }
    }

    /// Record `x.m()` with the type Graphify gives `x` (its file-wide map, or the class's field
    /// types for `this.f`) for `metadata.unresolved_calls`.
    fn unresolved_candidate(&mut self, caller: usize, recv: &Recv, method: &str, line: u32) {
        let ty = match recv {
            Recv::This => None,
            // `this.f` outside any class (ES5 constructor functions) uses the file-wide map.
            Recv::ThisField(field) => match self.enclosing_class(caller) {
                Some(k) => self.classes[k]
                    .fields
                    .iter()
                    .find(|(f, _)| f == field)
                    .map(|(_, t)| t.clone()),
                None => self
                    .graphify_vars
                    .iter()
                    .rev()
                    .find(|(v, _)| v == field)
                    .map(|(_, t)| t.clone()),
            },
            Recv::Ident(n) if self.is_this_alias(caller, n) => None,
            Recv::Ident(n) => self
                .graphify_vars
                .iter()
                .rev()
                .find(|(v, _)| v == n)
                .map(|(_, t)| t.clone()),
        };
        if let Some(ty) = ty {
            self.facts.unresolved.push(UnresolvedCall {
                caller: self.callers[caller].id.clone(),
                callee: method.to_string(),
                ty,
                line,
            });
        }
    }

    fn inferred_call_edge(
        &mut self,
        caller: usize,
        target: &str,
        line: u32,
        seen: &mut std::collections::HashSet<(String, String)>,
    ) {
        let src = self.callers[caller].id.clone();
        if seen.insert((src.clone(), target.to_string())) {
            let mut e = self
                .cx
                .edge(&src, target, "calls", Loc::Line(line), Some("call"));
            e.confidence = Confidence::Inferred;
            e.score = Some(0.85);
            self.cx.facts.edges.push(e);
        }
    }

    /// `// TAG: …` and JSDoc `* TAG: …` lines (§10.5), always attached to the file.
    fn rationale(&mut self) {
        let Ok(src) = std::str::from_utf8(self.cx.src) else {
            return;
        };
        let file = self.file_id();
        for (i, raw) in src.lines().enumerate() {
            let t = raw.trim_start();
            let Some(rest) = t.strip_prefix("// ").or_else(|| t.strip_prefix("* ")) else {
                continue;
            };
            let tagged = TAGS.iter().any(|tag| {
                rest.strip_prefix(tag)
                    .is_some_and(|after| after.starts_with(':'))
            });
            if !tagged {
                continue;
            }
            let ln = i as u32 + 1;
            let id = format!("{file}_rationale_{ln}");
            self.cx.add_rationale(id.clone(), rationale_label(rest), ln);
            self.cx.add_edge(&id, &file, "rationale_for", ln, None);
        }
    }
}

const TAGS: [&str; 7] = [
    "NOTE",
    "WHY",
    "TODO",
    "FIXME",
    "HACK",
    "IMPORTANT",
    "RATIONALE",
];

/// Every `import('…')` with a literal specifier in the source text, skipping `//` comment lines,
/// in order and without duplicates. JSDoc and type-level `import()` count, as in Graphify.
pub fn dynamic_imports(src: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let needle = b"import(";
    let mut i = 0;
    while let Some(off) = find(&src[i..], needle) {
        let at = i + off;
        i = at + needle.len();
        if at > 0 {
            let prev = src[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' || prev == b'.' {
                continue;
            }
        }
        let line_start = src[..at]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |p| p + 1);
        if find(&src[line_start..at], b"//").is_some() {
            continue;
        }
        let mut j = i;
        while j < src.len() && (src[j] == b' ' || src[j] == b'\t') {
            j += 1;
        }
        let Some(&quote) = src.get(j) else { break };
        if !matches!(quote, b'\'' | b'"' | b'`') {
            continue;
        }
        let start = j + 1;
        let Some(len) = src[start..].iter().position(|&b| b == quote || b == b'\n') else {
            break;
        };
        let end = start + len;
        if src[end] != quote {
            continue;
        }
        let mut k = end + 1;
        while k < src.len() && (src[k] == b' ' || src[k] == b'\t') {
            k += 1;
        }
        if src.get(k) != Some(&b')') {
            continue;
        }
        let spec = String::from_utf8_lossy(&src[start..end]).into_owned();
        if spec.is_empty() || (quote == b'`' && spec.contains("${")) {
            continue;
        }
        if !out.contains(&spec) {
            out.push(spec);
        }
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_file;
    use crate::facts::{FileFacts, LangFacts};
    use crate::lang::Lang;

    fn run(lang: Lang, rel: &str, src: &str) -> (FileFacts, ScriptFacts) {
        let mut f = extract_file(lang, rel, src.as_bytes());
        let LangFacts::Script(s) = std::mem::take(&mut f.lang) else {
            panic!("no script facts")
        };
        (f, s)
    }

    fn edges(f: &FileFacts) -> Vec<String> {
        f.edges
            .iter()
            .map(|e| format!("{} -{}-> {}", e.source, e.relation, e.target))
            .collect()
    }

    #[test]
    fn dynamic_import_scan_matches_graphify() {
        let src = b"/** @type {import('./b').T} */\n// import('./c')\nconst s = \"import('./d')\";\nx.import('./z');\nimport(`./f`); import(\"./g\" + \"\"); import(`./${x}`);\n";
        assert_eq!(dynamic_imports(src), ["./b", "./d", "./f"]);
    }

    #[test]
    fn calls_use_lexical_scope_and_the_enclosing_class() {
        let (f, s) = run(
            Lang::TypeScript,
            "a.ts",
            "class K { m1() {} run() { this.m1(); other.m1(); console.log(1); } }\n\
             class K2 { m2() {} }\n\
             function dupName() { function h() {} h(); }\n\
             function dupName2() { function h() {} h(); }\n",
        );
        let e = edges(&f);
        assert!(e.contains(&"a_k_run -calls-> a_k_m1".to_string()));
        assert!(e.contains(&"a_dupname -calls-> a_dupname_h".to_string()));
        assert!(e.contains(&"a_dupname2 -calls-> a_dupname2_h".to_string()));
        assert_eq!(e.iter().filter(|x| x.contains("calls")).count(), 3);
        // Unknown receivers are left for the cross-file pass, never bound by method name.
        assert!(s
            .pending
            .iter()
            .any(|p| matches!(p, Pending::Member { method, .. } if method == "log")));
    }

    #[test]
    fn typed_receivers_resolve_same_file_classes() {
        let (f, _) = run(
            Lang::TypeScript,
            "a.ts",
            "class Svc { go() {} }\n\
             function f(s: Svc) { s.go(); }\n\
             function g() { const v = new Svc(); [1].map(() => v.go()); }\n",
        );
        let e = edges(&f);
        assert!(e.contains(&"a_f -calls-> a_svc_go".to_string()));
        assert!(e.contains(&"a_g -calls-> a_svc_go".to_string()));
        assert!(e.contains(&"a_g -calls-> a_svc".to_string()));
    }

    #[test]
    fn module_values_and_commonjs_definitions() {
        let (f, s) = run(
            Lang::JavaScript,
            "c.js",
            "exports.a = function () {};\nexports.c = 5;\n\
             module.exports = { g() { helper(); }, h: helper };\n\
             function helper() {}\n\
             const { p, q: r } = require(\"./d\");\nlet z = require(\"./d\").sub;\n\
             const fn = function () {};\nfn.prototype.method = function () {};\n\
             const obj = { m() {} };\nconst prim = 5;\nvar v = () => 1;\n",
        );
        let ids: Vec<&str> = f.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "c",
                "c_a",
                "c_g",
                "c_helper",
                // A relative `require` binding is no node (0.9.97).
                "c_fn",
                "c_fn_method",
                "c_obj"
            ]
        );
        let e = edges(&f);
        assert!(e.contains(&"c -indirect_call-> c_helper".to_string()));
        assert!(e.contains(&"c_g -calls-> c_helper".to_string()));
        assert!(e.contains(&"c_fn -method-> c_fn_method".to_string()));
        let names: Vec<Vec<&str>> = s
            .imports
            .iter()
            .map(|i| i.names.iter().map(|n| n.imported.as_str()).collect())
            .collect();
        assert_eq!(names, [vec!["p", "q"], vec!["sub"]]);
    }

    #[test]
    fn class_members_and_type_references() {
        let (f, s) = run(
            Lang::TypeScript,
            "u.ts",
            "@deco\nexport class A extends Base implements Shape {\n\
             field: Map<string, Opt>;\n\
             get gs() { return 1; }\n set gs(v) {}\n\
             handle = (x: Svc): void => {};\n\
             run(s: Svc | null): Promise<Opt[]> { return null as any; }\n}\n\
             interface I extends Shape { m(a: Svc): Opt; p: Kind }\n",
        );
        let labels: Vec<(&str, &str)> = f
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect();
        assert!(labels.contains(&("u_a_gs", ".gs()")));
        assert!(labels.contains(&("u_a_handle", ".handle()")));
        let refs: Vec<(&str, &str, &str)> = s
            .type_refs
            .iter()
            .map(|r| (r.source.as_str(), r.name.as_str(), r.context.as_str()))
            .collect();
        assert_eq!(
            refs,
            [
                ("u_a", "Map", "field"),
                ("u_a", "Opt", "generic_arg"),
                ("u_a_run", "Svc", "parameter_type"),
                ("u_a_run", "Promise", "return_type"),
                ("u_a_run", "Opt", "generic_arg"),
                ("u_i_m", "Svc", "parameter_type"),
                ("u_i_m", "Opt", "return_type"),
                ("u_i", "Kind", "field"),
            ]
        );
        assert_eq!(s.decorators.len(), 1);
        assert_eq!(s.decorators[0].line, 1);
        assert_eq!(s.heritage.len(), 3);
        assert_eq!(f.nodes[1].loc, Loc::Line(2));
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let src = format!(
            "function h() {{ return {}1{}; }}\n",
            "x(".repeat(5000),
            ")".repeat(5000)
        );
        let (f, _) = run(Lang::JavaScript, "d.js", &src);
        assert_eq!(f.nodes.len(), 2);
    }

    #[test]
    fn rationale_needs_exact_prefix_and_tag() {
        let (f, _) = run(
            Lang::JavaScript,
            "r.js",
            "// NOTE: kept\n//NOTE: no\n  * WHY: jsdoc\n// Note: no\nx(); // TODO: trailing\n",
        );
        let r: Vec<(&str, &str)> = f
            .nodes
            .iter()
            .filter(|n| n.file_type == "rationale")
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect();
        assert_eq!(
            r,
            [
                ("r_rationale_1", "NOTE: kept"),
                ("r_rationale_3", "WHY: jsdoc")
            ]
        );
    }
}

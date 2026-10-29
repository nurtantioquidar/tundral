//! Java (§13) extraction. See `docs/graphify-format.md`.
//!
//! One file yields its file node, every type declaration (`file_Name`, flat for nested types),
//! methods and constructors (`type_name`, overloads merged), enum constants, anonymous classes
//! (`method_object_Type_l<line>`), annotation stubs, and the in-file `contains`, `method`,
//! `case_of`, `references`, `inherits`, `implements` and `calls` edges.
//!
//! A type name that is neither declared in the file nor imported by name becomes a per-file stub
//! (`slug(Name)`, like `@Override`'s `override`) that the cross-file pass binds to a local class
//! when it finds one. Common JDK types (`String`, `List`, `Instant`…) produce nothing. A method
//! carries `_java_ret`, its declared return type's simple name, when every overload merged into
//! its id names the same non-builtin type.
//!
//! Targets that need other files are written as private virtual ids that `crate::resolve::java`
//! redirects (they never reach the output):
//!
//! - `@js:<path>|<Name>`: a supertype not imported by name → a local class or nothing.
//! - `@jf:<fqn>`: an explicitly imported type, supertype or annotation → the local class, else
//!   the shared stub `slug(fqn)`.
//! - `@ji:<fqn>|<external>`: a single-type import → the local class, else the `slug(fqn)` stub
//!   (`external` empty: the type is used in the file) or the external import node.
//!
//! Calls that cannot bind in the file are recorded in [`JavaFacts::calls`], with the receiver's
//! declared type when it is a parameter, local or field of the method's own class.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, slug, Ctx};

/// A declared class, interface, enum, record or annotation type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JClass {
    pub name: String,
    /// Local id.
    pub id: String,
    /// Package + enclosing types + name.
    pub fqn: String,
    /// Declares at least one constructor (`new X()` then also names `.X()`).
    pub has_ctor: bool,
    /// `extends` of a class, by simple name.
    pub superclass: Option<String>,
}

/// A method or constructor of a named type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JMethod {
    /// Local id of the owning type.
    pub owner: String,
    pub name: String,
    pub id: String,
    pub has_body: bool,
    pub ctor: bool,
    /// A method of an anonymous class: bindable from its own class only, never by name.
    pub anonymous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JCallKind {
    /// `m()` not bound in the file.
    Bare,
    /// `new X()` not bound in the file.
    New,
    /// `this.m()`.
    This,
    /// `super.m()`.
    Super,
    /// `Type.m()` with a capitalized identifier receiver.
    Static,
    /// `x.m()` on any other receiver; `receiver` holds `x`'s declared type when known.
    Member,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JCall {
    /// Local id of the calling method.
    pub caller: String,
    /// Local id of the type enclosing the caller (for `this`/`super`).
    pub owner: String,
    pub kind: JCallKind,
    pub name: String,
    /// Receiver type name for `Static`, and for `Member` when the receiver variable is typed.
    pub receiver: String,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::java`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JavaFacts {
    pub package: String,
    pub file_id: String,
    pub classes: Vec<JClass>,
    pub methods: Vec<JMethod>,
    /// Single-type imports: (simple name, FQN).
    pub imports: Vec<(String, String)>,
    /// `import static p.C.m;`: (FQN of `C`, line).
    pub static_imports: Vec<(String, u32)>,
    /// Simple names used as types in the file (fields, parameters, returns, supertypes…).
    pub typed: Vec<String>,
    /// Per-file annotation stubs: (local id, simple name).
    pub stubs: Vec<(String, String)>,
    pub calls: Vec<JCall>,
}

/// Prefix of the virtual ids above; see the module docs.
pub const VIRTUAL: &str = "@j";

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn is_type_decl(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration"
    )
}

/// Common JDK type names that never produce a reference or stub, imported or not (Graphify
/// stubs other JDK types such as `java.sql.Connection` like any library type).
pub(crate) fn is_common_jdk_type(name: &str) -> bool {
    COMMON_JDK_TYPES.binary_search(&name).is_ok()
}

/// Sorted. Probed name by name on Graphify 0.9.97 (§13.1: the same list decides references,
/// stubs and `_java_ret`); Graphify still stubs e.g. `ConcurrentLinkedQueue`, `ReentrantLock`,
/// `Pattern`, `Connection`, `URI`, `Charset`, `Lock`, `TimeoutException`.
const COMMON_JDK_TYPES: &[&str] = &[
    "Appendable",
    "ArithmeticException",
    "ArrayDeque",
    "ArrayIndexOutOfBoundsException",
    "ArrayList",
    "Arrays",
    "AssertionError",
    "AtomicBoolean",
    "AtomicInteger",
    "AtomicLong",
    "AtomicReference",
    "AutoCloseable",
    "BiConsumer",
    "BiFunction",
    "BiPredicate",
    "BigDecimal",
    "BigInteger",
    "BinaryOperator",
    "BitSet",
    "BlockingQueue",
    "Boolean",
    "BufferedReader",
    "BufferedWriter",
    "Byte",
    "ByteArrayInputStream",
    "ByteArrayOutputStream",
    "Calendar",
    "Callable",
    "CharSequence",
    "Character",
    "Class",
    "ClassCastException",
    "Clock",
    "CloneNotSupportedException",
    "Cloneable",
    "Closeable",
    "Collection",
    "Collections",
    "Collector",
    "Collectors",
    "Comparable",
    "Comparator",
    "CompletableFuture",
    "CompletionStage",
    "ConcurrentHashMap",
    "ConcurrentMap",
    "ConcurrentModificationException",
    "Consumer",
    "CopyOnWriteArrayList",
    "CountDownLatch",
    "CyclicBarrier",
    "Date",
    "DateTimeFormatter",
    "DayOfWeek",
    "Deque",
    "Double",
    "DoubleStream",
    "Duration",
    "Enum",
    "EnumMap",
    "EnumSet",
    "Error",
    "Exception",
    "Executor",
    "ExecutorService",
    "Executors",
    "File",
    "FileNotFoundException",
    "FileReader",
    "FileWriter",
    "Files",
    "Float",
    "Function",
    "Future",
    "HashMap",
    "HashSet",
    "Hashtable",
    "IOException",
    "IllegalArgumentException",
    "IllegalStateException",
    "IndexOutOfBoundsException",
    "InputStream",
    "InputStreamReader",
    "Instant",
    "IntFunction",
    "IntStream",
    "Integer",
    "InterruptedException",
    "Iterable",
    "Iterator",
    "LinkedHashMap",
    "LinkedHashSet",
    "LinkedList",
    "List",
    "ListIterator",
    "LocalDate",
    "LocalDateTime",
    "LocalTime",
    "Locale",
    "Long",
    "LongStream",
    "Map",
    "Math",
    "Month",
    "NavigableMap",
    "NavigableSet",
    "NoSuchElementException",
    "NullPointerException",
    "Number",
    "NumberFormatException",
    "Object",
    "Objects",
    "OffsetDateTime",
    "Optional",
    "OptionalDouble",
    "OptionalInt",
    "OptionalLong",
    "OutOfMemoryError",
    "OutputStream",
    "OutputStreamWriter",
    "Package",
    "Path",
    "Paths",
    "Period",
    "Predicate",
    "PrintStream",
    "PrintWriter",
    "PriorityQueue",
    "Process",
    "ProcessBuilder",
    "Properties",
    "Queue",
    "Random",
    "Readable",
    "Reader",
    "Record",
    "Runnable",
    "Runtime",
    "RuntimeException",
    "Scanner",
    "ScheduledExecutorService",
    "SecurityException",
    "Semaphore",
    "Serializable",
    "Set",
    "Short",
    "SortedMap",
    "SortedSet",
    "Spliterator",
    "Stack",
    "StackOverflowError",
    "Stream",
    "String",
    "StringBuffer",
    "StringBuilder",
    "StringJoiner",
    "StringTokenizer",
    "Supplier",
    "System",
    "Thread",
    "ThreadLocal",
    "Throwable",
    "TimeUnit",
    "ToDoubleFunction",
    "ToIntFunction",
    "ToLongFunction",
    "TreeMap",
    "TreeSet",
    "UUID",
    "UnaryOperator",
    "UncheckedIOException",
    "UnsupportedOperationException",
    "Vector",
    "Void",
    "Writer",
    "Year",
    "ZoneId",
    "ZoneOffset",
    "ZonedDateTime",
];

/// A pending call site, bound after the walk.
struct Site {
    caller: String,
    owner: String,
    kind: SiteKind,
    line: u32,
}

enum SiteKind {
    Bare(String),
    This(String),
    Super(String),
    Static(String, String),
    Member(String),
    /// `x.m()` where `x` is a parameter, local or field of declared type `T`: (T, m).
    Typed(String, String),
    /// A call on any other receiver expression: never bound.
    Skip,
    New(String),
    Anon(String),
}

struct Java<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    /// Type id → field name → declared type (simple name).
    fields: HashMap<String, HashMap<String, String>>,
    /// Simple name → local id of the first type declared with it in the file.
    classes: HashMap<String, String>,
    /// Type id → whether it declares a constructor.
    has_ctor: HashMap<String, bool>,
    /// (owner type id, method name) → (method id, has body).
    methods: HashMap<(String, String), (String, bool)>,
    /// Method name → first method with a body in the file.
    bodies: HashMap<String, String>,
    /// Single-type imports: simple name → FQN.
    imported: HashMap<String, String>,
    seen_nodes: HashSet<String>,
    /// Method id → the `_java_ret` its declarations agree on (`None` once any overload's return
    /// type differs or names no qualifying type).
    rets: HashMap<String, Option<String>>,
    stub_seen: HashSet<String>,
    typed: Vec<String>,
    typed_seen: HashSet<String>,
    /// Annotation names used in the file (their imports count as used even from the JDK).
    annotated: HashSet<String>,
    /// Plain (non-generic) local variable types.
    local_typed: HashSet<String>,
    /// (type id, simple name) of every `inherits`/`implements` edge.
    supers: HashSet<(String, String)>,
    sites: Vec<Site>,
    facts: JavaFacts,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let file_id = cx.file_id.clone();
    let mut j = Java {
        fields: HashMap::new(),
        classes: HashMap::new(),
        has_ctor: HashMap::new(),
        methods: HashMap::new(),
        bodies: HashMap::new(),
        imported: HashMap::new(),
        seen_nodes: HashSet::from([file_id.clone()]),
        rets: HashMap::new(),
        stub_seen: HashSet::new(),
        typed: Vec::new(),
        typed_seen: HashSet::new(),
        annotated: HashSet::new(),
        local_typed: HashSet::new(),
        supers: HashSet::new(),
        sites: Vec::new(),
        facts: JavaFacts {
            file_id,
            ..JavaFacts::default()
        },
        cx,
    };
    if let Some(tree) = tree {
        j.file(tree.root_node());
    }
    let Java {
        cx,
        mut facts,
        typed,
        rets,
        ..
    } = j;
    for node in &mut cx.facts.nodes {
        if let Some(Some(ret)) = rets.get(&node.id) {
            node.attrs
                .push(("_java_ret".into(), crate::facts::Attr::Str(ret.clone())));
        }
    }
    facts.typed = typed;
    cx.facts.lang = crate::facts::LangFacts::Java(facts);
}

impl<'a> Java<'_, 'a> {
    fn text(&self, node: TsNode) -> &'a str {
        self.cx.text(node)
    }

    fn file(&mut self, root: TsNode) {
        let mut imports = Vec::new();
        let mut types = Vec::new();
        for child in named_children(root) {
            match child.kind() {
                "package_declaration" => {
                    if let Some(name) = named_children(child)
                        .into_iter()
                        .find(|n| matches!(n.kind(), "scoped_identifier" | "identifier"))
                    {
                        self.facts.package = self.text(name).to_string();
                    }
                }
                "import_declaration" => imports.push(child),
                k if is_type_decl(k) => types.push(child),
                _ => {}
            }
        }
        for imp in &imports {
            self.read_import(*imp);
        }
        // Declarations first: in-file references bind to any type of the file, and in-file
        // calls to any method of it.
        let file_id = self.cx.file_id.clone();
        let package = self.facts.package.clone();
        for t in &types {
            self.declare(*t, &file_id, &package);
        }
        // Import edges come first in the file's edge list; whether an import is used as a type
        // is known only after the walk, so the walk's edges are moved behind them.
        let mark = self.cx.facts.edges.len();
        for t in &types {
            self.type_decl(*t, &file_id, &file_id, &mut Vec::new());
        }
        let walked: Vec<_> = self.cx.facts.edges.drain(mark..).collect();
        for imp in &imports {
            self.import_edge(*imp);
        }
        self.cx.facts.edges.extend(walked);
        self.bind_calls();
    }

    // ---- imports ----------------------------------------------------------------------------

    fn import_parts(&self, imp: TsNode) -> Option<(String, bool, bool)> {
        let path = named_children(imp)
            .into_iter()
            .find(|n| matches!(n.kind(), "scoped_identifier" | "identifier"))?;
        let mut is_static = false;
        let mut wildcard = false;
        let mut c = imp.walk();
        for ch in imp.children(&mut c) {
            match ch.kind() {
                "static" => is_static = true,
                "asterisk" => wildcard = true,
                _ => {}
            }
        }
        Some((self.text(path).to_string(), is_static, wildcard))
    }

    fn read_import(&mut self, imp: TsNode) {
        let Some((path, is_static, wildcard)) = self.import_parts(imp) else {
            return;
        };
        if !is_static && !wildcard {
            let simple = path.rsplit('.').next().unwrap_or(&path).to_string();
            self.imported.entry(simple.clone()).or_insert(path.clone());
            self.facts.imports.push((simple, path));
        }
    }

    /// One `imports` edge per declaration (§13.2). Static and on-demand imports target the
    /// external node of their last segment; a static member import also links its class in the
    /// cross-file pass.
    fn import_edge(&mut self, imp: TsNode) {
        let Some((path, is_static, wildcard)) = self.import_parts(imp) else {
            return;
        };
        let line = Ctx::line(imp);
        let file_id = self.cx.file_id.clone();
        let last = path.rsplit('.').next().unwrap_or(&path).to_string();
        if is_static || wildcard {
            if is_static {
                let class = if wildcard {
                    path.clone()
                } else {
                    path.rsplit_once('.')
                        .map_or(path.clone(), |(c, _)| c.to_string())
                };
                self.facts.static_imports.push((class, line));
            }
            let ext = slug(&last);
            if ext.is_empty() {
                return;
            }
            self.cx.add_external(ext.clone());
            self.cx
                .add_edge(&file_id, &ext, "imports", line, Some("import"));
            return;
        }
        // Used: an annotation, a declared type (fields, parameters, returns, supertypes), or a
        // plain local variable type that is also a static call receiver
        // (`Pipeline p = Pipeline.create()`); plain locals alone (`JsonNode n = …`) do not count.
        let static_receiver = self
            .sites
            .iter()
            .any(|s| matches!(&s.kind, SiteKind::Static(r, _) if *r == last));
        let used = self.annotated.contains(&last)
            || (!is_common_jdk_type(&last)
                && (self.typed_seen.contains(&last)
                    || (self.local_typed.contains(&last) && static_receiver)));
        let external = if used { String::new() } else { slug(&last) };
        let target = format!("{VIRTUAL}i:{path}|{external}");
        self.cx
            .add_edge(&file_id, &target, "imports", line, Some("import"));
    }

    // ---- declarations -------------------------------------------------------------------------

    /// Pre-pass: ids, names, constructors and methods of every type in the file.
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
        self.classes
            .entry(name.clone())
            .or_insert_with(|| id.clone());
        let superclass = decl
            .child_by_field_name("superclass")
            .and_then(|s| named_children(s).into_iter().next())
            .and_then(|t| self.base_name(t));
        let mut has_ctor = false;
        if let Some(body) = decl.child_by_field_name("body") {
            for member in self.members(body) {
                match member.kind() {
                    "field_declaration" => {
                        let Some(ty) = member
                            .child_by_field_name("type")
                            .and_then(|t| self.receiver_type(t))
                        else {
                            continue;
                        };
                        let mut c = member.walk();
                        let names: Vec<String> = member
                            .children_by_field_name("declarator", &mut c)
                            .filter_map(|d| d.child_by_field_name("name"))
                            .map(|n| self.text(n).to_string())
                            .collect();
                        let table = self.fields.entry(id.clone()).or_default();
                        for n in names {
                            table.entry(n).or_insert_with(|| ty.clone());
                        }
                    }
                    "method_declaration" | "constructor_declaration" => {
                        let ctor = member.kind() == "constructor_declaration";
                        has_ctor |= ctor;
                        let Some(mname) = member.child_by_field_name("name") else {
                            continue;
                        };
                        let mname = self.text(mname).to_string();
                        let mid = Ctx::child_id(&id, &mname);
                        let body = ctor || member.child_by_field_name("body").is_some();
                        self.methods
                            .entry((id.clone(), mname.clone()))
                            .and_modify(|e| e.1 |= body)
                            .or_insert((mid.clone(), body));
                        if body {
                            self.bodies.entry(mname.clone()).or_insert(mid.clone());
                        }
                        self.facts.methods.push(JMethod {
                            owner: id.clone(),
                            name: mname,
                            id: mid,
                            has_body: body,
                            ctor,
                            anonymous: false,
                        });
                    }
                    k if is_type_decl(k) => self.declare(member, file_id, &fqn),
                    _ => {}
                }
            }
        }
        *self.has_ctor.entry(id.clone()).or_insert(false) |= has_ctor;
        self.facts.classes.push(JClass {
            name,
            id,
            fqn,
            has_ctor,
            superclass,
        });
    }

    /// Members of a class/interface/enum/record/annotation body, enum body declarations
    /// flattened in.
    fn members<'t>(&self, body: TsNode<'t>) -> Vec<TsNode<'t>> {
        let mut out = Vec::new();
        for m in named_children(body) {
            if m.kind() == "enum_body_declarations" {
                out.extend(named_children(m));
            } else {
                out.push(m);
            }
        }
        out
    }

    fn put_def(
        &mut self,
        id: &str,
        label: String,
        line: u32,
        extra: Vec<(String, crate::facts::Attr)>,
    ) {
        if self.seen_nodes.insert(id.to_string()) {
            self.cx.add_def(id.to_string(), label, line, extra);
        }
    }

    /// A type declaration and everything in it. `parent` gets the `contains` edge.
    fn type_decl(&mut self, decl: TsNode, file_id: &str, parent: &str, scope: &mut Vec<String>) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(file_id, &name);
        let line = Ctx::line(decl);
        self.put_def(
            &id,
            name.clone(),
            line,
            vec![flag("_callable"), flag("_callable_class")],
        );
        self.cx.add_edge(parent, &id, "contains", line, None);

        let depth = scope.len();
        if let Some(tp) = decl.child_by_field_name("type_parameters") {
            self.type_params(tp, scope);
        }
        // Supertypes: local types only; their type arguments are references.
        if let Some(sup) = decl.child_by_field_name("superclass") {
            for t in named_children(sup) {
                self.supertype(&id, t, "inherits", line, scope);
            }
        }
        let interfaces = decl.child_by_field_name("interfaces").or_else(|| {
            named_children(decl)
                .into_iter()
                .find(|n| n.kind() == "extends_interfaces")
        });
        if let Some(list) = interfaces {
            let relation = if decl.kind() == "interface_declaration" {
                "inherits"
            } else {
                "implements"
            };
            for tl in named_children(list) {
                for t in named_children(tl) {
                    self.supertype(&id, t, relation, line, scope);
                }
            }
        }
        self.annotations(decl, &id, line);
        // Record components are fields.
        if decl.kind() == "record_declaration" {
            if let Some(params) = decl.child_by_field_name("parameters") {
                for p in named_children(params) {
                    if let Some(t) = p.child_by_field_name("type") {
                        let pl = Ctx::line(p);
                        self.type_refs(&id, t, Some("field"), pl, scope);
                    }
                }
            }
        }
        if let Some(body) = decl.child_by_field_name("body") {
            self.body(body, file_id, &id, scope);
        }
        scope.truncate(depth);
    }

    fn body(&mut self, body: TsNode, file_id: &str, owner: &str, scope: &mut Vec<String>) {
        for member in self.members(body) {
            match member.kind() {
                "field_declaration" => {
                    if let Some(t) = member.child_by_field_name("type") {
                        let line = Ctx::line(member);
                        self.type_refs(owner, t, Some("field"), line, scope);
                    }
                }
                "method_declaration" | "constructor_declaration" => {
                    self.method(member, owner, scope);
                }
                "enum_constant" => self.enum_constant(member, owner, scope),
                k if is_type_decl(k) => self.type_decl(member, file_id, owner, scope),
                _ => {}
            }
        }
    }

    fn enum_constant(&mut self, constant: TsNode, owner: &str, scope: &mut Vec<String>) {
        let Some(name) = constant.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(owner, &name);
        let line = Ctx::line(constant);
        self.put_def(&id, name, line, vec![]);
        self.cx.add_edge(owner, &id, "case_of", line, None);
        if let Some(body) = constant.child_by_field_name("body") {
            for m in self.members(body) {
                if matches!(m.kind(), "method_declaration" | "constructor_declaration") {
                    self.method(m, &id, scope);
                }
            }
        }
    }

    fn method(&mut self, decl: TsNode, owner: &str, scope: &mut Vec<String>) {
        let Some(name) = decl.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(owner, &name);
        let line = Ctx::line(decl);
        self.put_def(&id, format!(".{name}()"), line, vec![flag("_callable")]);
        self.cx.add_edge(owner, &id, "method", line, None);
        let depth = scope.len();
        if let Some(tp) = decl.child_by_field_name("type_parameters") {
            self.type_params(tp, scope);
        }
        // `_java_ret` (§13.1): kept only while every overload merged into this id names the
        // same type.
        let ret = decl
            .child_by_field_name("type")
            .and_then(|t| self.return_name(t, scope));
        match self.rets.get_mut(&id) {
            None => {
                self.rets.insert(id.clone(), ret);
            }
            Some(agreed) => {
                if *agreed != ret {
                    *agreed = None;
                }
            }
        }
        let mut locals: HashMap<String, String> = HashMap::new();
        if let Some(params) = decl.child_by_field_name("parameters") {
            for p in named_children(params) {
                if !matches!(p.kind(), "formal_parameter" | "spread_parameter") {
                    continue;
                }
                let ty = p.child_by_field_name("type").or_else(|| {
                    named_children(p)
                        .into_iter()
                        .find(|n| n.kind() != "modifiers")
                });
                if let Some(t) = ty {
                    if p.kind() == "formal_parameter" {
                        self.type_refs(&id, t, Some("parameter_type"), line, scope);
                    }
                    let name = p.child_by_field_name("name").or_else(|| {
                        named_children(p)
                            .into_iter()
                            .find(|n| n.kind() == "variable_declarator")
                            .and_then(|d| d.child_by_field_name("name"))
                    });
                    if let (Some(ty), Some(name)) = (self.receiver_type(t), name) {
                        locals.insert(self.text(name).to_string(), ty);
                    }
                }
            }
        }
        if let Some(t) = decl.child_by_field_name("type") {
            self.type_refs(&id, t, Some("return_type"), line, scope);
        }
        self.annotations(decl, &id, line);
        if let Some(body) = decl.child_by_field_name("body") {
            self.walk_body(body, &id, owner, scope, &mut locals);
        }
        scope.truncate(depth);
    }

    fn type_params(&self, list: TsNode, scope: &mut Vec<String>) {
        for p in named_children(list) {
            if p.kind() != "type_parameter" {
                continue;
            }
            if let Some(n) = named_children(p)
                .into_iter()
                .find(|n| matches!(n.kind(), "type_identifier" | "identifier"))
            {
                scope.push(self.text(n).to_string());
            }
        }
    }

    // ---- type references ----------------------------------------------------------------------

    /// The simple name a type node denotes: `Foo`, `a.b.Foo` → `Foo`, `Foo<T>` → `Foo`.
    fn base_name(&self, node: TsNode) -> Option<String> {
        match node.kind() {
            "type_identifier" | "identifier" => Some(self.text(node).to_string()),
            "scoped_type_identifier" | "scoped_identifier" => {
                let last = named_children(node).into_iter().last()?;
                self.base_name(last)
            }
            "generic_type" => self.base_name(named_children(node).into_iter().next()?),
            _ => None,
        }
    }

    /// A method's `_java_ret`: the simple name of its declared return type (`a.B` → `B`,
    /// `Box<B>` → `Box`), except arrays, primitives, `void`, type parameters in scope and the
    /// builtin types of [`COMMON_JDK_TYPES`].
    fn return_name(&self, t: TsNode, scope: &[String]) -> Option<String> {
        if !matches!(
            t.kind(),
            "type_identifier" | "scoped_type_identifier" | "generic_type"
        ) {
            return None;
        }
        let name = self.base_name(t)?;
        (!is_common_jdk_type(&name) && !scope.contains(&name)).then_some(name)
    }

    /// The declared type of a variable, as a simple name, when it can name a receiver
    /// (`Foo`, `a.Foo`, `Foo<T>`; not arrays, primitives or `var`).
    fn receiver_type(&self, t: TsNode) -> Option<String> {
        match t.kind() {
            // `var` stays as written, as Graphify records it (`receiver_type: "var"`).
            "type_identifier" | "scoped_type_identifier" | "generic_type" => self.base_name(t),
            _ => None,
        }
    }

    /// The declared type of `name` as seen from a method of `owner`: its locals, then the
    /// fields of `owner` itself (an outer class's fields are not consulted, as in Graphify).
    fn variable_type(
        &self,
        locals: &HashMap<String, String>,
        owner: &str,
        name: &str,
    ) -> Option<String> {
        if let Some(t) = locals.get(name) {
            return Some(t.clone());
        }
        self.fields.get(owner).and_then(|f| f.get(name)).cloned()
    }

    fn note_typed(&mut self, name: &str) {
        if self.typed_seen.insert(name.to_string()) {
            self.typed.push(name.to_string());
        }
    }

    /// Where a reference to type `name` (written `text`, e.g. `Outer.Inner`) points from this
    /// file, or `None` for a type parameter or a common JDK type.
    fn type_target(&mut self, name: &str, text: &str, scope: &[String]) -> Option<String> {
        if scope.iter().any(|s| s == name) {
            return None;
        }
        self.note_typed(name);
        if let Some(id) = self.classes.get(name) {
            return Some(id.clone());
        }
        if is_common_jdk_type(name) {
            return None;
        }
        if let Some(fqn) = self.imported.get(name) {
            return Some(format!("{VIRTUAL}f:{fqn}"));
        }
        // Not declared here nor imported by name: a per-file stub that the cross-file pass binds
        // to a local class when it finds one.
        let id = slug(text);
        if id.is_empty() {
            return None;
        }
        if self.seen_nodes.insert(id.clone()) {
            self.cx.add_stub(id.clone(), text.to_string());
        }
        if self.stub_seen.insert(id.clone()) {
            self.facts.stubs.push((id.clone(), name.to_string()));
        }
        Some(id)
    }

    fn type_refs(
        &mut self,
        source: &str,
        node: TsNode,
        context: Option<&str>,
        line: u32,
        scope: &[String],
    ) {
        match node.kind() {
            "type_identifier" | "scoped_type_identifier" => {
                if let Some(name) = self.base_name(node) {
                    if let Some(target) = self.type_target(&name, &name.clone(), scope) {
                        // A type's own supertype keeps its `inherits`/`implements` link whole
                        // in Graphify's build: a later reference to it adds nothing.
                        let supers = (source.to_string(), name.clone());
                        if target != source && !self.supers.contains(&supers) {
                            self.cx
                                .add_edge(source, &target, "references", line, context);
                        }
                    }
                }
            }
            "generic_type" => {
                let kids = named_children(node);
                if let Some(base) = kids.first() {
                    self.type_refs(source, *base, context, line, scope);
                }
                for args in kids.iter().skip(1) {
                    for a in named_children(*args) {
                        self.type_refs(source, a, Some("generic_arg"), line, scope);
                    }
                }
            }
            "array_type" => {
                if let Some(e) = node.child_by_field_name("element") {
                    self.type_refs(source, e, context, line, scope);
                }
            }
            "wildcard" | "annotated_type" | "type_list" => {
                for c in named_children(node) {
                    self.type_refs(source, c, context, line, scope);
                }
            }
            _ => {}
        }
    }

    /// `extends`/`implements` to a local type, plus `generic_arg` references for its arguments.
    fn supertype(
        &mut self,
        source: &str,
        node: TsNode,
        relation: &str,
        line: u32,
        scope: &[String],
    ) {
        if let Some(name) = self.base_name(node) {
            if !scope.contains(&name) {
                self.note_typed(&name);
                // A supertype imported by name links to its class or FQN stub; any other one
                // links only when it names a local class.
                let target = match self.classes.get(&name) {
                    Some(id) => Some(id.clone()),
                    None if is_common_jdk_type(&name) => None,
                    None => match self.imported.get(&name) {
                        Some(fqn) => Some(format!("{VIRTUAL}f:{fqn}")),
                        None => Some(format!("{VIRTUAL}s:{}|{name}", self.cx.rel)),
                    },
                };
                if let Some(target) = target.filter(|t| t != source) {
                    self.supers.insert((source.to_string(), name.clone()));
                    self.cx.add_edge(source, &target, relation, line, None);
                }
            }
        }
        if node.kind() == "generic_type" {
            for args in named_children(node).into_iter().skip(1) {
                for a in named_children(args) {
                    self.type_refs(source, a, Some("generic_arg"), line, scope);
                }
            }
        }
    }

    /// `references` with context `attribute` to every annotation of a declaration.
    fn annotations(&mut self, decl: TsNode, source: &str, line: u32) {
        let Some(mods) = named_children(decl)
            .into_iter()
            .find(|n| n.kind() == "modifiers")
        else {
            return;
        };
        for a in named_children(mods) {
            if !matches!(a.kind(), "marker_annotation" | "annotation") {
                continue;
            }
            let Some(name_node) = a.child_by_field_name("name") else {
                continue;
            };
            let Some(name) = self.base_name(name_node) else {
                continue;
            };
            // An unresolved annotation stub keeps the name as written (`Default.Integer`).
            let written = self.text(name_node).to_string();
            self.note_typed(&name);
            self.annotated.insert(name.clone());
            let target = if let Some(id) = self.classes.get(&name) {
                id.clone()
            } else if let Some(fqn) = self.imported.get(&name) {
                format!("{VIRTUAL}f:{fqn}")
            } else {
                let id = slug(&written);
                if id.is_empty() {
                    continue;
                }
                if self.seen_nodes.insert(id.clone()) {
                    self.cx.add_stub(id.clone(), written);
                }
                if self.stub_seen.insert(id.clone()) {
                    self.facts.stubs.push((id.clone(), name.clone()));
                }
                id
            };
            if target != source {
                self.cx
                    .add_edge(source, &target, "references", line, Some("attribute"));
            }
        }
    }

    // ---- bodies and calls ---------------------------------------------------------------------

    /// Calls in a method body, lambdas included; anonymous classes become nodes with their own
    /// methods. Local class declarations are skipped.
    fn walk_body(
        &mut self,
        body: TsNode,
        caller: &str,
        owner: &str,
        scope: &mut Vec<String>,
        locals: &mut HashMap<String, String>,
    ) {
        let mut stack = vec![body];
        while let Some(node) = stack.pop() {
            let line = Ctx::line(node);
            match node.kind() {
                "method_invocation" => {
                    if let Some(name) = node.child_by_field_name("name") {
                        let name = self.text(name).to_string();
                        // A variable receiver is typed by its declaration; an unknown
                        // capitalized identifier is taken for a type.
                        let kind = match node.child_by_field_name("object") {
                            None => SiteKind::Bare(name),
                            Some(o) if o.kind() == "this" => SiteKind::This(name),
                            Some(o) if o.kind() == "super" => SiteKind::Super(name),
                            Some(o) if o.kind() == "identifier" => {
                                let var = self.text(o);
                                // A common JDK type counts as unknown: `NAMES.contains()` on a
                                // `Set<String> NAMES` field is then a "static" call.
                                match self
                                    .variable_type(locals, owner, var)
                                    .filter(|t| !is_common_jdk_type(t))
                                {
                                    Some(t) => SiteKind::Typed(t, name),
                                    None if var.starts_with(char::is_uppercase) => {
                                        SiteKind::Static(var.to_string(), name)
                                    }
                                    None => SiteKind::Member(name),
                                }
                            }
                            // Chained calls and field accesses never bind by name.
                            Some(_) => SiteKind::Skip,
                        };
                        self.sites.push(Site {
                            caller: caller.to_string(),
                            owner: owner.to_string(),
                            kind,
                            line,
                        });
                    }
                }
                "object_creation_expression" => {
                    let ty = node.child_by_field_name("type");
                    let class_body = named_children(node)
                        .into_iter()
                        .find(|n| n.kind() == "class_body");
                    if let (Some(ty), Some(cb)) = (ty, class_body) {
                        let label = self.text(ty).to_string();
                        let id = Ctx::child_id(caller, &format!("object_{label}_l{line}"));
                        self.put_def(
                            &id,
                            label,
                            line,
                            vec![flag("_callable"), flag("_callable_class")],
                        );
                        self.cx.add_edge(caller, &id, "contains", line, None);
                        // The creation counts as a call only when the instance is stored
                        // (`Runnable r = new Runnable() {…}`), not when passed or returned.
                        let stored = node.parent().is_some_and(|p| {
                            matches!(p.kind(), "variable_declarator" | "expression_statement")
                        });
                        if stored {
                            self.sites.push(Site {
                                caller: caller.to_string(),
                                owner: owner.to_string(),
                                kind: SiteKind::Anon(id.clone()),
                                line,
                            });
                        }
                        // The anonymous class's own methods; the rest of the creation
                        // expression (arguments) still belongs to the caller.
                        for m in self.members(cb) {
                            if m.kind() != "method_declaration" {
                                continue;
                            }
                            if let Some(mname) = m.child_by_field_name("name") {
                                let mname = self.text(mname).to_string();
                                self.facts.methods.push(JMethod {
                                    owner: id.clone(),
                                    id: Ctx::child_id(&id, &mname),
                                    name: mname,
                                    has_body: m.child_by_field_name("body").is_some(),
                                    ctor: false,
                                    anonymous: true,
                                });
                            }
                            self.method(m, &id, scope);
                        }
                        if let Some(args) = node.child_by_field_name("arguments") {
                            stack.push(args);
                        }
                        continue;
                    }
                    if let Some(name) = ty.and_then(|t| self.base_name(t)) {
                        self.sites.push(Site {
                            caller: caller.to_string(),
                            owner: owner.to_string(),
                            kind: SiteKind::New(name),
                            line,
                        });
                    }
                }
                k if is_type_decl(k) => continue,
                // Locals type later receivers; a plain (non-generic) local type also counts as
                // a use of its import (no edge).
                "local_variable_declaration" | "resource" => {
                    let ty = node.child_by_field_name("type").or_else(|| {
                        named_children(node)
                            .into_iter()
                            .find(|n| n.kind() == "catch_type")
                            .and_then(|c| named_children(c).into_iter().next())
                    });
                    if let Some(t) = ty {
                        if node.kind() == "local_variable_declaration"
                            && matches!(t.kind(), "type_identifier" | "scoped_type_identifier")
                        {
                            if let Some(n) = self.base_name(t) {
                                self.local_typed.insert(n);
                            }
                        }
                        if let Some(rt) = self.receiver_type(t) {
                            let mut c = node.walk();
                            let mut names: Vec<String> = node
                                .children_by_field_name("declarator", &mut c)
                                .filter_map(|d| d.child_by_field_name("name"))
                                .map(|n| self.text(n).to_string())
                                .collect();
                            if let Some(n) = node.child_by_field_name("name") {
                                names.push(self.text(n).to_string());
                            }
                            for n in names {
                                locals.insert(n, rt.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
            let start = stack.len();
            let mut c = node.walk();
            stack.extend(node.named_children(&mut c));
            stack[start..].reverse();
        }
    }

    /// Bind call sites: bare calls and `new` to the file's own types first, the rest (and any
    /// miss) to the cross-file pass. In-file edges: one per (caller, callee), first site kept.
    fn bind_calls(&mut self) {
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut local: Vec<(String, String, u32)> = Vec::new();
        let sites = std::mem::take(&mut self.sites);
        for s in sites {
            let (kind, name, receiver) = match s.kind {
                SiteKind::Anon(id) => {
                    if seen.insert((s.caller.clone(), id.clone())) {
                        local.push((s.caller, id, s.line));
                    }
                    continue;
                }
                SiteKind::Bare(name) => {
                    // The enclosing type first (a body-less match drops the call), then any
                    // method with a body in the file.
                    match self.methods.get(&(s.owner.clone(), name.clone())) {
                        Some((id, true)) => {
                            let id = id.clone();
                            if seen.insert((s.caller.clone(), id.clone())) {
                                local.push((s.caller, id, s.line));
                            }
                            continue;
                        }
                        Some((_, false)) => continue,
                        None => {}
                    }
                    if let Some(id) = self.bodies.get(&name).cloned() {
                        if seen.insert((s.caller.clone(), id.clone())) {
                            local.push((s.caller, id, s.line));
                        }
                        continue;
                    }
                    (JCallKind::Bare, name, String::new())
                }
                SiteKind::New(name) => {
                    if let Some(class) = self.classes.get(&name).cloned() {
                        let target = if self.has_ctor.get(&class).copied().unwrap_or(false) {
                            Ctx::child_id(&class, &name)
                        } else {
                            class
                        };
                        if seen.insert((s.caller.clone(), target.clone())) {
                            local.push((s.caller, target, s.line));
                        }
                        continue;
                    }
                    (JCallKind::New, name, String::new())
                }
                SiteKind::This(name) => (JCallKind::This, name, String::new()),
                SiteKind::Super(name) => (JCallKind::Super, name, String::new()),
                SiteKind::Static(recv, name) => (JCallKind::Static, name, recv),
                SiteKind::Member(name) => (JCallKind::Member, name, String::new()),
                SiteKind::Typed(ty, name) => (JCallKind::Member, name, ty),
                SiteKind::Skip => continue,
            };
            self.facts.calls.push(JCall {
                caller: s.caller,
                owner: s.owner,
                kind,
                name,
                receiver,
                line: s.line,
            });
        }
        for (caller, callee, line) in local {
            self.cx
                .add_edge(&caller, &callee, "calls", line, Some("call"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_file;
    use crate::facts::{LangFacts, Loc};
    use crate::lang::Lang;

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
                        Loc::Line(n) => n,
                        _ => 0,
                    }
                )
            })
            .collect()
    }

    #[test]
    fn nested_types_are_flat_and_overloads_merge() {
        let f = extract_file(
            Lang::Java,
            "a/F.java",
            b"package a;\nclass F {\n  class Inner { void a() {} }\n  void m(int x) {}\n  void m(Inner i) {}\n}\nclass G {\n  class Inner { void b() {} }\n}\n",
        );
        let ids: Vec<_> = f.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "a_f",
                "a_f_f",
                "a_f_inner",
                "a_f_inner_a",
                "a_f_f_m",
                "a_f_g",
                "a_f_inner_b"
            ]
        );
        let e = edges(&f);
        assert!(e.contains(&"a_f_g contains a_f_inner - L8".to_string()));
        assert!(e.contains(&"a_f_f_m references a_f_inner parameter_type L5".to_string()));
        assert_eq!(
            e.iter().filter(|x| x.contains(" method a_f_f_m ")).count(),
            2
        );
    }

    #[test]
    fn imports_and_external_types_use_virtual_targets() {
        let f = extract_file(
            Lang::Java,
            "a/A.java",
            b"package a;\nimport b.B;\nimport java.time.Instant;\nimport static b.B.helper;\nimport c.*;\nclass A extends Base {\n  B b;\n  Instant t;\n  Other o;\n  @Override public void run() { helper(); B.make(); b.go(); new Other(); }\n}\n",
        );
        let e = edges(&f);
        assert!(e.contains(&"a_a imports @ji:b.B| import L2".to_string()));
        assert!(e.contains(&"a_a imports @ji:java.time.Instant|instant import L3".to_string()));
        assert!(e.contains(&"a_a imports helper import L4".to_string()));
        assert!(e.contains(&"a_a imports c import L5".to_string()));
        assert!(e.contains(&"a_a_a inherits @js:a/A.java|Base - L6".to_string()));
        assert!(e.contains(&"a_a_a references @jf:b.B field L7".to_string()));
        // A common JDK type gets no reference or stub.
        assert!(!e
            .iter()
            .any(|x| x.contains(" references ") && x.to_lowercase().contains("instant")));
        assert!(e.contains(&"a_a_a references other field L9".to_string()));
        assert!(e.contains(&"a_a_a_run references override attribute L10".to_string()));
        let LangFacts::Java(j) = &f.lang else {
            panic!()
        };
        let kinds: Vec<_> = j.calls.iter().map(|c| (c.kind, c.name.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (JCallKind::Bare, "helper"),
                (JCallKind::Static, "make"),
                (JCallKind::Member, "go"),
                (JCallKind::New, "Other")
            ]
        );
        assert_eq!(j.static_imports, [("b.B".to_string(), 4)]);
    }

    #[test]
    fn bare_calls_prefer_the_enclosing_type_and_skip_bodyless_matches() {
        let f = extract_file(
            Lang::Java,
            "q/X.java",
            b"package q;\nclass X { void a() { b(); c(); new Y(); new Z(); } void b() {} }\nclass Y { Y() {} void b() {} void c() { b(); } }\nclass Z {}\nabstract class W { abstract void abs(); void caller() { abs(); } }\n",
        );
        let e: Vec<_> = edges(&f)
            .into_iter()
            .filter(|x| x.contains(" calls "))
            .collect();
        assert_eq!(
            e,
            [
                "q_x_x_a calls q_x_x_b call L2",
                "q_x_x_a calls q_x_y_c call L2",
                "q_x_x_a calls q_x_y_y call L2",
                "q_x_x_a calls q_x_z call L2",
                "q_x_y_c calls q_x_y_b call L3"
            ]
        );
    }

    #[test]
    fn receivers_are_typed_by_params_locals_and_own_fields() {
        let f = extract_file(
            Lang::Java,
            "a/S.java",
            b"package a;\nimport java.util.*;\nclass S {\n  private static final Logger LOG = null;\n  private static final Set<String> NAMES = null;\n  private Repo repo;\n  void run(Client c, List<String> xs) {\n    var v = make();\n    Table t = c.table();\n    t.scan();\n    repo.save();\n    LOG.info(\"x\");\n    NAMES.contains(\"y\");\n    xs.add(\"z\");\n    v.go();\n    make().chained();\n    for (Item i : items()) { i.use(); }\n  }\n}\n",
        );
        let LangFacts::Java(j) = &f.lang else {
            panic!()
        };
        let calls: Vec<_> = j
            .calls
            .iter()
            .map(|c| (c.kind, c.receiver.as_str(), c.name.as_str()))
            .collect();
        assert_eq!(
            calls,
            [
                (JCallKind::Bare, "", "make"),
                (JCallKind::Member, "Client", "table"),
                (JCallKind::Member, "Table", "scan"),
                (JCallKind::Member, "Repo", "save"),
                (JCallKind::Member, "Logger", "info"),
                // A `Set` field is a common JDK type: its capitalized name reads as a type.
                (JCallKind::Static, "NAMES", "contains"),
                // A `List` parameter is skipped as untyped (lowercase receiver).
                (JCallKind::Member, "", "add"),
                (JCallKind::Member, "var", "go"),
                // `make().chained()`: the inner call only; a chained receiver never binds.
                (JCallKind::Bare, "", "make"),
                (JCallKind::Bare, "", "items"),
                // A for-each variable is untyped.
                (JCallKind::Member, "", "use"),
            ]
        );
    }

    #[test]
    fn anonymous_classes_and_enum_constants() {
        let f = extract_file(
            Lang::Java,
            "p/K.java",
            b"package p;\nclass K {\n  void apply() {\n    Runnable q = new Runnable() {\n      public void run() { apply(); }\n    };\n  }\n  enum Color { RED { int code() { return 1; } }, GREEN; }\n}\n",
        );
        let ids: Vec<_> = f.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"p_k_k_apply_object_runnable_l4"));
        assert!(ids.contains(&"p_k_k_apply_object_runnable_l4_run"));
        assert!(ids.contains(&"p_k_color_red_code"));
        let e = edges(&f);
        assert!(e.contains(&"p_k_k_apply contains p_k_k_apply_object_runnable_l4 - L4".to_string()));
        assert!(e.contains(&"p_k_k_apply calls p_k_k_apply_object_runnable_l4 call L4".to_string()));
        assert!(
            e.contains(&"p_k_k_apply_object_runnable_l4_run calls p_k_k_apply call L5".to_string())
        );
        assert!(e.contains(&"p_k_color case_of p_k_color_red - L8".to_string()));
        assert!(e.contains(&"p_k_color_red method p_k_color_red_code - L8".to_string()));
    }
}

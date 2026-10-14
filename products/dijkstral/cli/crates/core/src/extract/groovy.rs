//! Groovy and Gradle (§26) extraction. See `docs/graphify-format.md`.
//!
//! Files are parsed with tree-sitter-groovy (a Java-derived grammar that misses much of Groovy's
//! syntax; its error recovery is kept as is). One walk emits:
//!
//! - classes, interfaces and enums at any depth outside method bodies: id `file_<slug(Name)>`
//!   (flat for nested types), label `Name`, `_callable`, `_callable_class`, `contains` from the
//!   file or the enclosing type, at the declaration's first line (annotations included);
//! - methods and constructors of their bodies: `<type id>_<slug(name)>`, label `.name()`,
//!   `_callable`, `method` from the type (one edge per overload, one node); a method the grammar
//!   left outside any type (after an error) is a function of the file, `name()`; enum constants
//!   `<enum id>_<slug(NAME)>`, label `NAME`, `case_of`;
//! - `inherits` to the superclass (and an interface's `extends` list), `implements` to each
//!   interface, `references` (context `attribute`) to each annotation of the type. A target is
//!   the type of that name declared earlier in the file, else a per-file stub `slug(Name)` labelled
//!   with the last segment of the name (`b.Qual` → `Qual`, `Gen<T>` → `Gen`). Supertypes in a
//!   list of common JDK types produce nothing (annotations are not filtered);
//! - `imports` per import declaration to `slug(last segment)` (the member of a static import, the
//!   package of a wildcard, the type of an alias), written as a virtual target
//!   (`@groovy:<importer>|<id>`) that `crate::resolve::groovy` binds;
//! - `calls` from the enclosing method (closures included) by the invoked name, bound after the
//!   walk to the file's **last** method of that name (`x.reset()` → the last `.reset()`, quirk
//!   GR1); one edge per (caller, callee). Juxtaposed calls (`println "x"`), `new X()`, script
//!   statements, top-level `def` functions and the builtin-like names Lua skips produce nothing.
//!
//! A file with a string-named `def "…"()` (a Spock specification) is read line by line instead
//! (quirk GR2): `class Name` lines become classes without `_callable` flags, `def name(` and
//! `def "…"(` lines become methods of the last class (`.name()`, `"…"`), and `import` lines
//! become imports; nothing else.
//!
//! No rationale: comments and Groovydoc produce nothing. Unbound calls go to [`GroovyFacts`].

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::lua::is_denied_call;
use super::{flag, origin, slug, Ctx};
use crate::facts::{Confidence, Edge, LangFacts, Loc};

/// A call that did not bind in its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroovyCall {
    /// Local id of the calling method.
    pub caller: String,
    pub name: String,
    pub line: u32,
}

/// Facts the cross-file pass (`crate::resolve::groovy`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GroovyFacts {
    pub calls: Vec<GroovyCall>,
}

/// Sorted. Supertype names that produce no edge and no stub (probed one by one).
const DENIED_TYPES: &[&str] = &[
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
    "OptionalInt",
    "OutOfMemoryError",
    "OutputStream",
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
    "Supplier",
    "System",
    "Thread",
    "ThreadLocal",
    "Throwable",
    "TimeUnit",
    "ToIntFunction",
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
    "ZonedDateTime",
];

fn is_denied_type(name: &str) -> bool {
    DENIED_TYPES.binary_search(&name).is_ok()
}

struct Call {
    caller: String,
    name: String,
    line: u32,
}

struct Walker<'a, 'b> {
    cx: &'a mut Ctx<'b>,
    /// Types declared so far in the file, by name.
    types: HashMap<String, String>,
    /// Stubs already emitted.
    stubs: HashSet<String>,
    /// Methods by name, the last definition winning.
    methods: HashMap<String, String>,
    calls: Vec<Call>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(Vec::new());
    let source = std::str::from_utf8(cx.src).unwrap_or("");
    if source.lines().any(|l| spock_method(l).is_some()) {
        extract_lines(cx, source);
        cx.facts.lang = LangFacts::Groovy(GroovyFacts::default());
        return;
    }
    let mut w = Walker {
        cx,
        types: HashMap::new(),
        stubs: HashSet::new(),
        methods: HashMap::new(),
        calls: Vec::new(),
    };
    if let Some(tree) = tree {
        w.walk(tree.root_node(), None, None);
    }
    w.finish();
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

/// The last `.`-separated segment.
fn last_segment(text: &str) -> &str {
    text.rsplit('.').next().unwrap_or(text).trim()
}

impl Walker<'_, '_> {
    /// `owner`: the enclosing type's id; `caller`: the enclosing method's id.
    fn walk(&mut self, node: TsNode, owner: Option<&str>, caller: Option<&str>) {
        match node.kind() {
            "import_declaration" => self.import(node),
            "class_declaration" | "interface_declaration" | "enum_declaration"
                if caller.is_none() =>
            {
                self.declare_type(node, owner);
            }
            // A method outside any type body (error recovery): a function of the file, or a
            // method of the type whose body the error is in.
            "method_declaration" | "constructor_declaration" if caller.is_none() => {
                self.declare_method(node, owner);
            }
            "method_invocation" => {
                if let (Some(caller), Some(name)) = (caller, node.child_by_field_name("name")) {
                    let name = self.cx.text(name);
                    if !is_denied_call(name) {
                        self.calls.push(Call {
                            caller: caller.to_string(),
                            name: name.to_string(),
                            line: Ctx::line(node),
                        });
                    }
                }
                self.walk_children(node, owner, caller);
            }
            _ => self.walk_children(node, owner, caller),
        }
    }

    fn walk_children(&mut self, node: TsNode, owner: Option<&str>, caller: Option<&str>) {
        let mut c = node.walk();
        for child in node.children(&mut c) {
            self.walk(child, owner, caller);
        }
    }

    fn import(&mut self, node: TsNode) {
        let Some(path) = named_children(node)
            .into_iter()
            .find(|c| matches!(c.kind(), "scoped_identifier" | "identifier"))
        else {
            return;
        };
        let target = slug(last_segment(self.cx.text(path)));
        if target.is_empty() {
            return;
        }
        let (file, target) = (self.cx.file_id.clone(), virtual_target(self.cx, &target));
        self.cx
            .add_edge(&file, &target, "imports", Ctx::line(node), Some("import"));
    }

    fn declare_type(&mut self, node: TsNode, owner: Option<&str>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.cx.text(name).to_string();
        let id = Ctx::child_id(&self.cx.file_id, &name);
        let line = Ctx::line(node);
        self.cx.add_def(
            id.clone(),
            name.clone(),
            line,
            vec![flag("_callable"), flag("_callable_class")],
        );
        let parent = owner.map_or_else(|| self.cx.file_id.clone(), str::to_string);
        self.cx.add_edge(&parent, &id, "contains", line, None);
        self.types.insert(name, id.clone());

        let is_interface = node.kind() == "interface_declaration";
        let mut supers: Vec<(TsNode, &str)> = Vec::new();
        if let Some(sc) = node.child_by_field_name("superclass") {
            supers.extend(named_children(sc).into_iter().map(|t| (t, "inherits")));
        }
        let mut c = node.walk();
        for child in node.children(&mut c) {
            let relation = match child.kind() {
                "super_interfaces" => "implements",
                "extends_interfaces" if is_interface => "inherits",
                _ => continue,
            };
            for list in named_children(child) {
                supers.extend(named_children(list).into_iter().map(|t| (t, relation)));
            }
        }
        let mut linked: HashSet<(String, &str)> = HashSet::new();
        for (ty, relation) in supers {
            let Some(name) = self.type_name(ty) else {
                continue;
            };
            if is_denied_type(&name) {
                continue;
            }
            let target = self.target(&name);
            if linked.insert((target.clone(), relation)) {
                self.cx.add_edge(&id, &target, relation, line, None);
            }
        }
        if let Some(mods) = named_children(node)
            .into_iter()
            .find(|c| c.kind() == "modifiers")
        {
            for a in named_children(mods) {
                if !matches!(a.kind(), "marker_annotation" | "annotation") {
                    continue;
                }
                let Some(n) = a.child_by_field_name("name") else {
                    continue;
                };
                let name = last_segment(self.cx.text(n)).to_string();
                let target = self.target(&name);
                self.cx
                    .add_edge(&id, &target, "references", line, Some("attribute"));
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.body(body, &id);
        }
    }

    /// The type a supertype names: `Name`, `a.b.Name` and `Name<T>` give `Name`.
    fn type_name(&self, ty: TsNode) -> Option<String> {
        match ty.kind() {
            "type_identifier" => Some(self.cx.text(ty).to_string()),
            "scoped_type_identifier" => Some(last_segment(self.cx.text(ty)).to_string()),
            "generic_type" => named_children(ty)
                .into_iter()
                .find(|c| matches!(c.kind(), "type_identifier" | "scoped_type_identifier"))
                .and_then(|c| self.type_name(c)),
            _ => None,
        }
    }

    /// A type declared earlier in the file, else this file's stub for it.
    fn target(&mut self, name: &str) -> String {
        if let Some(id) = self.types.get(name) {
            return id.clone();
        }
        let id = slug(name);
        if self.stubs.insert(id.clone()) {
            self.cx.add_stub(id.clone(), name.to_string());
        }
        id
    }

    fn body(&mut self, body: TsNode, owner: &str) {
        let mut c = body.walk();
        for member in body.children(&mut c) {
            match member.kind() {
                "enum_constant" => {
                    if let Some(n) = member.child_by_field_name("name") {
                        let name = self.cx.text(n);
                        let id = Ctx::child_id(owner, name);
                        let line = Ctx::line(member);
                        self.cx
                            .add_def(id.clone(), name.to_string(), line, Vec::new());
                        self.cx.add_edge(owner, &id, "case_of", line, None);
                    }
                }
                "enum_body_declarations" => self.body(member, owner),
                "method_declaration" | "constructor_declaration" => {
                    self.declare_method(member, Some(owner));
                }
                _ => self.walk(member, Some(owner), None),
            }
        }
    }

    fn declare_method(&mut self, node: TsNode, owner: Option<&str>) {
        let Some(n) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.cx.text(n).to_string();
        let line = Ctx::line(node);
        let id = match owner {
            Some(owner) => {
                let id = Ctx::child_id(owner, &name);
                self.cx.add_def(
                    id.clone(),
                    format!(".{name}()"),
                    line,
                    vec![flag("_callable")],
                );
                self.cx.add_edge(owner, &id, "method", line, None);
                id
            }
            None => {
                let file = self.cx.file_id.clone();
                let id = Ctx::child_id(&file, &name);
                self.cx.add_def(
                    id.clone(),
                    format!("{name}()"),
                    line,
                    vec![flag("_callable")],
                );
                self.cx.add_edge(&file, &id, "contains", line, None);
                id
            }
        };
        self.methods.insert(name, id.clone());
        self.walk_children(node, owner, Some(&id));
    }

    fn finish(mut self) {
        let mut linked: HashSet<(String, String)> = HashSet::new();
        let mut facts = GroovyFacts::default();
        for c in std::mem::take(&mut self.calls) {
            match self.methods.get(&c.name) {
                Some(t) => {
                    if linked.insert((c.caller.clone(), t.clone())) {
                        let e = call_edge(self.cx, &c.caller, t, c.line);
                        self.cx.facts.edges.push(e);
                    }
                }
                None => facts.calls.push(GroovyCall {
                    caller: c.caller,
                    name: c.name,
                    line: c.line,
                }),
            }
        }
        self.cx.facts.lang = LangFacts::Groovy(facts);
    }
}

fn call_edge(cx: &Ctx, source: &str, target: &str, line: u32) -> Edge {
    Edge {
        source: source.into(),
        target: target.into(),
        relation: "calls".into(),
        confidence: Confidence::Extracted,
        score: None,
        context: Some("call".into()),
        source_file: cx.rel.into(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

/// `def "name"(` / `def 'name'(` at the start of a line: the name between the quotes.
fn spock_method(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("def")?;
    let rest = rest.strip_prefix(char::is_whitespace)?.trim_start();
    let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let body = &rest[1..];
    let end = body.find(quote)?;
    body[end + 1..]
        .trim_start()
        .starts_with('(')
        .then_some(&body[..end])
}

/// `def name(` at the start of a line.
fn def_method(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("def")?;
    let rest = rest.strip_prefix(char::is_whitespace)?.trim_start();
    let end = rest
        .char_indices()
        .find(|&(_, c)| !(c.is_alphanumeric() || c == '_'))
        .map_or(rest.len(), |(i, _)| i);
    (end > 0 && rest[end..].trim_start().starts_with('(')).then_some(&rest[..end])
}

/// `class Name` after optional annotations and modifiers at the start of a line.
fn class_line(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    loop {
        if let Some(r) = rest.strip_prefix('@') {
            let end = r
                .char_indices()
                .find(|&(_, c)| !(c.is_alphanumeric() || c == '_' || c == '.'))
                .map_or(r.len(), |(i, _)| i);
            let mut r = &r[end..];
            if r.starts_with('(') {
                r = &r[r.find(')')? + 1..];
            }
            rest = r.trim_start();
            continue;
        }
        let word_end = rest
            .char_indices()
            .find(|&(_, c)| c.is_whitespace())
            .map_or(rest.len(), |(i, _)| i);
        let word = &rest[..word_end];
        match word {
            "public" | "protected" | "private" | "abstract" | "final" | "static" => {
                rest = rest[word_end..].trim_start();
            }
            "class" => {
                if word_end == rest.len() {
                    return None;
                }
                let r = rest[word_end..].trim_start();
                let end = r
                    .char_indices()
                    .find(|&(_, c)| !(c.is_alphanumeric() || c == '_'))
                    .map_or(r.len(), |(i, _)| i);
                return (end > 0).then_some(&r[..end]);
            }
            _ => return None,
        }
    }
}

/// `import [static] a.b.C` at the start of a line: `slug(C)`.
fn import_line(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("import")?;
    let mut rest = rest.strip_prefix(char::is_whitespace)?.trim_start();
    if let Some(r) = rest.strip_prefix("static") {
        if r.starts_with(char::is_whitespace) {
            rest = r.trim_start();
        }
    }
    let end = rest
        .char_indices()
        .find(|&(_, c)| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .map_or(rest.len(), |(i, _)| i);
    let id = slug(last_segment(rest[..end].trim_end_matches('.')));
    (!id.is_empty()).then_some(id)
}

/// Prefix of the virtual `imports` targets: `@groovy:<importer rel>|<slug(name)>`.
pub const VIRTUAL: &str = "@groovy:";

fn virtual_target(cx: &Ctx, id: &str) -> String {
    format!("{VIRTUAL}{}|{id}", cx.rel)
}

/// The line-based reading of a Spock specification (quirk GR2).
fn extract_lines(cx: &mut Ctx, source: &str) {
    let file = cx.file_id.clone();
    let mut class: Option<String> = None;
    for (i, line) in source.lines().enumerate() {
        let n = i as u32 + 1;
        if let Some(target) = import_line(line) {
            let target = virtual_target(cx, &target);
            cx.add_edge(&file, &target, "imports", n, Some("import"));
        } else if let Some(name) = class_line(line) {
            let id = Ctx::child_id(&file, name);
            cx.add_def(id.clone(), name.to_string(), n, Vec::new());
            cx.add_edge(&file, &id, "contains", n, None);
            class = Some(id);
        } else if let Some(owner) = &class {
            let (name, label) = if let Some(name) = spock_method(line) {
                (name, format!("\"{name}\""))
            } else if let Some(name) = def_method(line) {
                (name, format!(".{name}()"))
            } else {
                continue;
            };
            let id = Ctx::child_id(owner, name);
            cx.add_def(id.clone(), label, n, Vec::new());
            cx.add_edge(owner, &id, "method", n, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spock_lines() {
        assert_eq!(
            spock_method("  def \"does a thing\"() {"),
            Some("does a thing")
        );
        assert_eq!(spock_method("def 'single'() { }"), Some("single"));
        assert_eq!(spock_method("def x() {}"), None);
        assert_eq!(def_method("    def   spaced  ( ) { }"), Some("spaced"));
        assert_eq!(def_method("def String typed() { }"), None);
        assert_eq!(def_method("private def priv() { }"), None);
        assert_eq!(class_line("@Anno class F8 {}"), Some("F8"));
        assert_eq!(class_line("public final class F9<T> {}"), Some("F9"));
        assert_eq!(class_line("// class Fake {}"), None);
        assert_eq!(class_line("class"), None);
        assert_eq!(import_line("import s.Other as O"), Some("other".into()));
        assert_eq!(import_line("import static a.B.c"), Some("c".into()));
        assert_eq!(import_line("import java.util.*"), Some("util".into()));
    }
}

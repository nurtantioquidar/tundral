//! Cross-file pass for Python (§9). See `docs/graphify-format.md`.
//!
//! Every name the extractor recorded is resolved here through one set of scoping rules: the
//! enclosing definitions, then the file's top level, then import bindings (following
//! re-exports through `__init__.py` and other modules), then the heuristics Graphify applies to
//! free names (modules the file imports from, then a unique definition anywhere). Ids stored in
//! `PyFacts` were recorded before collision renaming and are translated with `View::id`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::python::{
    first_component, split_dotted, CallKind, DefKind, ImportKind, PyFacts, RefKind,
};
use crate::extract::{external_node, origin, slug};
use crate::facts::{Confidence, Edge, LangFacts, Loc, Node, NodeKind};
use crate::lang::Family;

/// Quirk Q12 for external modules: Graphify emits `calls` to an external module node only from
/// top-level functions and only for `alias.f()` on an `import x [as y]` binding. Calls through
/// a local module (`models.validate()`) are always resolved (the quirk drops them).
const EXTERNAL_CALLS_TOP_LEVEL_ONLY: bool = true;

/// Builtins never resolved through the free-name heuristics (a local definition still wins).
const BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "anext",
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
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
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
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__import__",
];

#[derive(Debug, Clone)]
enum ModRef {
    Local(usize),
    External(String),
}

#[derive(Debug, Clone)]
enum Binding {
    /// `import a.b.c` binds `a` to module `a`; `import a.b.c as x` binds `x` to `a.b.c`.
    Module(String),
    /// `from M import name [as alias]`.
    From { module: ModRef, name: String },
}

/// Outcome of resolving a type-like name.
enum Lookup {
    Found((usize, u32)),
    /// Unresolved: becomes a stub node.
    NotFound,
    /// Names something that has no node (a module, a module-level variable): no edge.
    Drop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Target {
    Def(usize, u32),
    Module(usize),
    /// Re-exported from outside the repository.
    Ext,
}

/// What a bound name refers to.
#[derive(Debug, Clone)]
enum Bound {
    Target(Target),
    /// An external module (`import json` → `json`).
    ExtModule(String),
    /// A name imported from an external module.
    ExtSym,
    /// Imported from a local module that does not define it.
    Missing,
}

struct PyFile<'a> {
    py: &'a PyFacts,
    rel: &'a str,
    /// Final file node id.
    file_id: String,
    /// Dotted module name (`pkg.models`, `pkg` for `pkg/__init__.py`).
    dotted: String,
    /// Package directory components (where `.` relative imports start).
    dir: Vec<&'a str>,
    is_init: bool,
    /// Final id of every def.
    ids: Vec<String>,
    top: HashMap<&'a str, u32>,
    children: HashMap<u32, HashMap<&'a str, u32>>,
    /// First class with a given name anywhere in the file (nested classes).
    classes: HashMap<&'a str, u32>,
    mod_bindings: HashMap<&'a str, Binding>,
    fn_bindings: HashMap<u32, HashMap<&'a str, Binding>>,
    stars: Vec<usize>,
    /// Local modules this file imports from (`from M import …`, any scope), sorted.
    from_modules: Vec<usize>,
    /// Every def by name.
    names: HashMap<&'a str, Vec<u32>>,
}

/// Which definitions a repository-wide unique-name lookup considers.
#[derive(Clone, Copy)]
enum Uniq {
    Any = 0,
    Class = 1,
    NonMethod = 2,
}

struct Resolver<'a> {
    views: &'a [View<'a>],
    files: Vec<PyFile<'a>>,
    modules: HashMap<String, usize>,
    /// Per name, the definition unique in the repository for each `Uniq` filter.
    uniq: HashMap<&'a str, [Option<(usize, u32)>; 3]>,
    memo: RefCell<HashMap<(usize, String), Option<Target>>>,
    /// Resolved base classes per class.
    bases: HashMap<(usize, u32), Vec<(usize, u32)>>,
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut r = Resolver::new(files, index);
    if r.files.is_empty() {
        return Resolution::default();
    }
    let mut out = Out::default();
    r.imports(&mut out);
    r.refs(&mut out, index);
    let called = r.calls(&mut out);
    r.indirect(&mut out, &called);
    r.uses(&mut out);
    Resolution {
        nodes: out.nodes,
        edges: out.edges,
        ..Resolution::default()
    }
}

#[derive(Default)]
struct Out {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    externals: HashSet<String>,
}

impl Out {
    fn external(&mut self, dotted: &str) -> Option<String> {
        let id = slug(dotted);
        if id.is_empty() {
            return None;
        }
        if self.externals.insert(id.clone()) {
            self.nodes.push(external_node(id.clone()));
        }
        Some(id)
    }
}

#[allow(clippy::too_many_arguments)]
fn edge(
    source: &str,
    target: &str,
    relation: &str,
    file: &str,
    line: u32,
    context: Option<&str>,
    confidence: Confidence,
    score: Option<f64>,
) -> Edge {
    Edge {
        source: source.into(),
        target: target.into(),
        relation: relation.into(),
        confidence,
        score,
        context: context.map(str::to_string),
        source_file: file.into(),
        loc: Loc::Line(line),
        weight: Some(if relation == "uses" { 0.8 } else { 1.0 }),
        attrs: vec![origin()],
    }
}

/// `pkg/models.py` → `pkg.models`; `pkg/__init__.py` → `pkg`; extensionless scripts keep
/// their name.
fn dotted_of(rel: &str) -> (String, bool) {
    let stem = match rel.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("py") && !stem.ends_with('/') => stem,
        _ => rel,
    };
    let (stem, is_init) = if stem == "__init__" {
        ("", true)
    } else if let Some(pkg) = stem.strip_suffix("/__init__") {
        (pkg, true)
    } else {
        (stem, false)
    };
    (stem.replace('/', "."), is_init)
}

impl<'a> Resolver<'a> {
    fn new(views: &'a [View<'a>], index: &Index) -> Resolver<'a> {
        let mut files = Vec::new();
        for v in views.iter().filter(|v| v.family() == Family::Python) {
            let LangFacts::Python(py) = &v.facts.lang else {
                continue;
            };
            let rel = v.file.rel.as_str();
            let (dotted, is_init) = dotted_of(rel);
            let mut dir: Vec<&str> = rel.split('/').collect();
            dir.pop();
            let file_id = index
                .file_ids
                .get(rel)
                .cloned()
                .unwrap_or_else(|| crate::ids::file_id(rel));
            let ids = py.defs.iter().map(|d| v.id(&d.id).to_string()).collect();
            let mut top = HashMap::new();
            let mut children: HashMap<u32, HashMap<&str, u32>> = HashMap::new();
            let mut classes = HashMap::new();
            let mut names: HashMap<&str, Vec<u32>> = HashMap::new();
            for (i, d) in py.defs.iter().enumerate() {
                let i = i as u32;
                names.entry(d.name.as_str()).or_default().push(i);
                match d.parent {
                    None => top.entry(d.name.as_str()).or_insert(i),
                    Some(p) => children
                        .entry(p)
                        .or_default()
                        .entry(d.name.as_str())
                        .or_insert(i),
                };
                if d.kind == DefKind::Class {
                    classes.entry(d.name.as_str()).or_insert(i);
                }
            }
            files.push(PyFile {
                py,
                rel,
                file_id,
                dotted,
                dir,
                is_init,
                ids,
                top,
                children,
                classes,
                mod_bindings: HashMap::new(),
                fn_bindings: HashMap::new(),
                stars: Vec::new(),
                from_modules: Vec::new(),
                names,
            });
        }

        let mut modules: HashMap<String, usize> = HashMap::new();
        for (i, f) in files.iter().enumerate() {
            // A package (`x/__init__.py`) wins over a same-named module file (`x.py`).
            match modules.get(&f.dotted) {
                Some(&j) if files[j].is_init || !f.is_init => {}
                _ => {
                    modules.insert(f.dotted.clone(), i);
                }
            }
        }

        // Unique-name tables: a name maps to a definition when exactly one distinct id carries
        // it (redefinitions in one file share an id).
        let mut uniq: HashMap<&str, [Option<(usize, u32)>; 3]> = HashMap::new();
        let mut ambiguous: HashSet<(&str, usize)> = HashSet::new();
        for (fi, f) in files.iter().enumerate() {
            for (di, d) in f.py.defs.iter().enumerate() {
                let filters = [true, d.kind == DefKind::Class, d.kind != DefKind::Method];
                let slot = uniq.entry(d.name.as_str()).or_insert([None; 3]);
                for (k, keep) in filters.into_iter().enumerate() {
                    if !keep || ambiguous.contains(&(d.name.as_str(), k)) {
                        continue;
                    }
                    match slot[k] {
                        None => slot[k] = Some((fi, di as u32)),
                        Some((f0, d0)) if files[f0].ids[d0 as usize] == f.ids[di] => {}
                        Some(_) => {
                            slot[k] = None;
                            ambiguous.insert((d.name.as_str(), k));
                        }
                    }
                }
            }
        }

        let mut r = Resolver {
            views,
            files,
            modules,
            uniq,
            memo: RefCell::new(HashMap::new()),
            bases: HashMap::new(),
        };
        r.bind_imports();
        r
    }

    fn bind_imports(&mut self) {
        for fi in 0..self.files.len() {
            let mut mod_bindings = HashMap::new();
            let mut fn_bindings: HashMap<u32, HashMap<&str, Binding>> = HashMap::new();
            let mut stars = Vec::new();
            let mut from_modules = Vec::new();
            let f = &self.files[fi];
            for imp in &f.py.imports {
                let mut bind = |name: &'a str, b: Binding| match imp.scope {
                    None => {
                        mod_bindings.insert(name, b);
                    }
                    Some(s) => {
                        fn_bindings.entry(s).or_default().insert(name, b);
                    }
                };
                match &imp.kind {
                    ImportKind::Module { path, alias } => match alias {
                        Some(a) => bind(a.as_str(), Binding::Module(path.clone())),
                        None => {
                            let first = first_component(path);
                            bind(first, Binding::Module(first.to_string()));
                        }
                    },
                    ImportKind::From {
                        level,
                        module,
                        names,
                        star,
                    } => {
                        let m = self.import_module(fi, *level, module);
                        if let ModRef::Local(mi) = m {
                            from_modules.push(mi);
                            if *star && imp.scope.is_none() && mi != fi {
                                stars.push(mi);
                            }
                        }
                        for (n, a) in names {
                            let bound = a.as_deref().unwrap_or(n);
                            bind(
                                bound,
                                Binding::From {
                                    module: m.clone(),
                                    name: n.clone(),
                                },
                            );
                        }
                    }
                }
            }
            let f = &mut self.files[fi];
            f.mod_bindings = mod_bindings;
            f.fn_bindings = fn_bindings;
            f.stars = stars;
            from_modules.sort_unstable();
            from_modules.dedup();
            f.from_modules = from_modules;
        }
    }

    // ---- modules ----------------------------------------------------------------------------

    /// An absolute module path: from the scan root, or under `src/`.
    fn module(&self, dotted: &str) -> Option<usize> {
        self.modules
            .get(dotted)
            .or_else(|| self.modules.get(&format!("src.{dotted}")))
            .copied()
    }

    fn import_module(&self, fi: usize, level: u32, module: &str) -> ModRef {
        if level == 0 {
            return match self.module(module) {
                Some(m) => ModRef::Local(m),
                None => ModRef::External(module.to_string()),
            };
        }
        let dir = &self.files[fi].dir;
        let up = (level - 1) as usize;
        let base = &dir[..dir.len().saturating_sub(up)];
        let mut dotted = base.join(".");
        if !module.is_empty() {
            if !dotted.is_empty() {
                dotted.push('.');
            }
            dotted.push_str(module);
        }
        match self.modules.get(&dotted) {
            Some(&m) => ModRef::Local(m),
            None => ModRef::External(dotted),
        }
    }

    fn submodule(&self, m: usize, name: &str) -> Option<Target> {
        let f = &self.files[m];
        if !f.is_init {
            return None;
        }
        let dotted = if f.dotted.is_empty() {
            name.to_string()
        } else {
            format!("{}.{name}", f.dotted)
        };
        self.modules.get(&dotted).map(|&s| Target::Module(s))
    }

    /// What module `m` exposes as `name`: its own top-level definition, a name it imports
    /// (re-exports, followed), a star-imported definition, or a submodule of a package.
    fn lookup(&self, m: usize, name: &str) -> Option<Target> {
        self.lookup_depth(m, name, 0)
    }

    fn lookup_depth(&self, m: usize, name: &str, depth: u8) -> Option<Target> {
        if depth > 8 {
            return None;
        }
        let key = (m, name.to_string());
        if let Some(hit) = self.memo.borrow().get(&key) {
            return *hit;
        }
        // Guard against import cycles while this lookup is in progress.
        self.memo.borrow_mut().insert(key.clone(), None);
        let found = self.lookup_uncached(m, name, depth);
        self.memo.borrow_mut().insert(key, found);
        found
    }

    fn lookup_uncached(&self, m: usize, name: &str, depth: u8) -> Option<Target> {
        let f = &self.files[m];
        if let Some(&d) = f.top.get(name) {
            return Some(Target::Def(m, d));
        }
        if let Some(b) = f.mod_bindings.get(name) {
            match b {
                Binding::From {
                    module: ModRef::Local(m2),
                    name: n2,
                } if *m2 != m => {
                    if let Some(t) = self
                        .lookup_depth(*m2, n2, depth + 1)
                        .or_else(|| self.submodule(*m2, n2))
                    {
                        return Some(t);
                    }
                }
                Binding::From {
                    module: ModRef::Local(_),
                    name: n2,
                } => {
                    if let Some(t) = self.submodule(m, n2) {
                        return Some(t);
                    }
                }
                Binding::Module(dotted) => {
                    return Some(match self.module(dotted) {
                        Some(m2) => Target::Module(m2),
                        None => Target::Ext,
                    });
                }
                Binding::From {
                    module: ModRef::External(_),
                    ..
                } => return Some(Target::Ext),
            }
        }
        for &s in &f.stars {
            if let Some(t @ Target::Def(..)) = self.lookup_depth(s, name, depth + 1) {
                return Some(t);
            }
        }
        self.submodule(m, name)
    }

    fn bound(&self, b: &Binding) -> Bound {
        match b {
            Binding::Module(dotted) => match self.module(dotted) {
                Some(m) => Bound::Target(Target::Module(m)),
                None => Bound::ExtModule(dotted.clone()),
            },
            Binding::From {
                module: ModRef::Local(m),
                name,
            } => match self.lookup(*m, name).or_else(|| self.submodule(*m, name)) {
                Some(Target::Ext) => Bound::ExtSym,
                Some(t) => Bound::Target(t),
                None => Bound::Missing,
            },
            Binding::From {
                module: ModRef::External(_),
                ..
            } => Bound::ExtSym,
        }
    }

    // ---- scopes -----------------------------------------------------------------------------

    fn def(&self, fi: usize, d: u32) -> &crate::extract::python::PyDef {
        &self.files[fi].py.defs[d as usize]
    }

    /// A definition visible from `scope` by name: nested definitions of enclosing functions,
    /// then the file's top level.
    fn local(&self, fi: usize, scope: Option<u32>, name: &str) -> Option<u32> {
        let f = &self.files[fi];
        let mut s = scope;
        while let Some(d) = s {
            let def = &f.py.defs[d as usize];
            if def.kind != DefKind::Class {
                if let Some(&c) = f.children.get(&d).and_then(|m| m.get(name)) {
                    return Some(c);
                }
            }
            s = def.parent;
        }
        f.top.get(name).copied()
    }

    fn binding(&self, fi: usize, scope: Option<u32>, name: &str) -> Option<&Binding> {
        let f = &self.files[fi];
        let mut s = scope;
        while let Some(d) = s {
            if let Some(b) = f.fn_bindings.get(&d).and_then(|m| m.get(name)) {
                return Some(b);
            }
            s = f.py.defs[d as usize].parent;
        }
        f.mod_bindings.get(name)
    }

    /// The definition named `name` that is unique in the repository among `kind`.
    fn unique(&self, name: &str, kind: Uniq) -> Option<(usize, u32)> {
        self.uniq.get(name).and_then(|u| u[kind as usize])
    }

    /// The definition named `name` that is unique among the modules file `fi` imports from.
    fn unique_in_from_modules(&self, fi: usize, name: &str) -> Option<(usize, u32)> {
        let mut found: Option<(usize, u32)> = None;
        for &m in &self.files[fi].from_modules {
            for &d in self.files[m].names.get(name).into_iter().flatten() {
                match found {
                    None => found = Some((m, d)),
                    Some(t) if self.id(t.0, t.1) == self.id(m, d) => {}
                    Some(_) => return None,
                }
            }
        }
        found
    }

    fn id(&self, fi: usize, d: u32) -> &str {
        &self.files[fi].ids[d as usize]
    }

    /// Method (or nested class) `name` of class `c`, searching base classes depth-first.
    fn method(&self, class: (usize, u32), name: &str, with_bases: bool) -> Option<(usize, u32)> {
        let mut stack = vec![class];
        let mut seen = HashSet::new();
        while let Some((fi, c)) = stack.pop() {
            if !seen.insert((fi, c)) {
                continue;
            }
            if let Some(&m) = self.files[fi].children.get(&c).and_then(|m| m.get(name)) {
                return Some((fi, m));
            }
            if with_bases {
                if let Some(bs) = self.bases.get(&(fi, c)) {
                    stack.extend(bs.iter().rev().copied());
                }
            }
        }
        None
    }

    // ---- imports ----------------------------------------------------------------------------

    fn imports(&self, out: &mut Out) {
        // File → module edges, in statement order (module level only).
        for (fi, f) in self.files.iter().enumerate() {
            for imp in f.py.imports.iter().filter(|i| i.scope.is_none()) {
                let (target, relation) = match &imp.kind {
                    ImportKind::Module { path, .. } => match self.module(path) {
                        Some(m) => (Some(self.files[m].file_id.clone()), "imports"),
                        None => (out.external(path), "imports"),
                    },
                    ImportKind::From { level, module, .. } => {
                        match self.import_module(fi, *level, module) {
                            ModRef::Local(m) if m == fi => (None, "imports_from"),
                            ModRef::Local(m) => {
                                (Some(self.files[m].file_id.clone()), "imports_from")
                            }
                            ModRef::External(d) => (out.external(&d), "imports_from"),
                        }
                    }
                };
                if let Some(t) = target {
                    out.edges.push(edge(
                        &f.file_id,
                        &t,
                        relation,
                        f.rel,
                        imp.line,
                        Some("import"),
                        Confidence::Extracted,
                        None,
                    ));
                }
            }
        }
        // `__init__.py` re-exports.
        for (fi, f) in self.files.iter().enumerate().filter(|(_, f)| f.is_init) {
            for imp in f.py.imports.iter().filter(|i| i.scope.is_none()) {
                if let ImportKind::From { level, module, .. } = &imp.kind {
                    if let ModRef::Local(m) = self.import_module(fi, *level, module) {
                        if m != fi {
                            out.edges.push(edge(
                                &f.file_id,
                                &self.files[m].file_id,
                                "re_exports",
                                f.rel,
                                imp.line,
                                Some("export"),
                                Confidence::Extracted,
                                None,
                            ));
                        }
                    }
                }
            }
        }
        // Imported symbols, following re-exports.
        for (fi, f) in self.files.iter().enumerate() {
            for imp in &f.py.imports {
                let ImportKind::From {
                    level,
                    module,
                    names,
                    ..
                } = &imp.kind
                else {
                    continue;
                };
                let ModRef::Local(m) = self.import_module(fi, *level, module) else {
                    continue;
                };
                for (n, _) in names {
                    if let Some(Target::Def(f2, d)) = self.lookup(m, n) {
                        if f2 != fi {
                            out.edges.push(edge(
                                &f.file_id,
                                self.id(f2, d),
                                "imports",
                                f.rel,
                                imp.line,
                                Some("import"),
                                Confidence::Extracted,
                                Some(1.0),
                            ));
                        }
                    }
                }
            }
        }
        // `from pkg import submodule`.
        for (fi, f) in self.files.iter().enumerate() {
            for imp in f.py.imports.iter().filter(|i| i.scope.is_none()) {
                let ImportKind::From {
                    level,
                    module,
                    names,
                    ..
                } = &imp.kind
                else {
                    continue;
                };
                let m = self.import_module(fi, *level, module);
                for (n, _) in names {
                    let sub = match &m {
                        ModRef::Local(m) => match self.lookup(*m, n) {
                            Some(Target::Module(s)) => Some(s),
                            Some(Target::Def(..) | Target::Ext) => None,
                            None => self.submodule(*m, n).and_then(|t| match t {
                                Target::Module(s) => Some(s),
                                Target::Def(..) | Target::Ext => None,
                            }),
                        },
                        ModRef::External(d) if !d.is_empty() => {
                            let dotted = format!("{d}.{n}");
                            if *level == 0 {
                                self.module(&dotted)
                            } else {
                                self.modules.get(&dotted).copied()
                            }
                        }
                        ModRef::External(_) => None,
                    };
                    if let Some(s) = sub.filter(|s| *s != fi) {
                        out.edges.push(edge(
                            &f.file_id,
                            &self.files[s].file_id,
                            "imports_from",
                            f.rel,
                            imp.line,
                            Some("submodule_import"),
                            Confidence::Extracted,
                            Some(1.0),
                        ));
                    }
                }
            }
        }
    }

    // ---- types, bases, decorators -----------------------------------------------------------

    /// Resolve a type-like name (annotation, base, decorator) seen in `scope`.
    fn resolve_type(
        &self,
        fi: usize,
        scope: Option<u32>,
        name: &str,
        qual: Option<&str>,
    ) -> Lookup {
        let f = &self.files[fi];
        let Some(q) = qual else {
            if let Some(d) = self.local(fi, scope, name) {
                return Lookup::Found((fi, d));
            }
            if let Some(b) = self.binding(fi, scope, name) {
                return match self.bound(b) {
                    Bound::Target(Target::Def(f2, d)) => Lookup::Found((f2, d)),
                    // Imported from outside the repository, or a local module's variable
                    // (`TypeVar`, alias): a stub.
                    Bound::ExtSym | Bound::Missing | Bound::Target(Target::Ext) => Lookup::NotFound,
                    // A module used as a type: no node.
                    Bound::Target(Target::Module(_)) | Bound::ExtModule(_) => Lookup::Drop,
                };
            }
            if let Some(&c) = f.classes.get(name) {
                return Lookup::Found((fi, c));
            }
            for &s in &f.stars {
                if let Some(Target::Def(f2, d)) = self.lookup(s, name) {
                    return Lookup::Found((f2, d));
                }
            }
            return self
                .unique(name, Uniq::Class)
                .or_else(|| self.unique(name, Uniq::NonMethod))
                .map_or(Lookup::NotFound, Lookup::Found);
        };
        match self.receiver(fi, scope, q) {
            Some(Target::Module(m)) => match self.lookup(m, name) {
                Some(Target::Def(f2, d)) => Lookup::Found((f2, d)),
                Some(Target::Module(_)) => Lookup::Drop,
                Some(Target::Ext) | None => Lookup::NotFound,
            },
            Some(Target::Def(f2, c)) if self.def(f2, c).kind == DefKind::Class => self
                .method((f2, c), name, false)
                .map_or(Lookup::NotFound, Lookup::Found),
            _ => Lookup::NotFound,
        }
    }

    /// A dotted receiver (`mod`, `pkg.sub`, `Cls`) as a local module or definition.
    fn receiver(&self, fi: usize, scope: Option<u32>, dotted: &str) -> Option<Target> {
        let mut parts = dotted.split('.');
        let first = parts.next()?;
        let mut cur = if let Some(d) = self.local(fi, scope, first) {
            Target::Def(fi, d)
        } else {
            match self.bound(self.binding(fi, scope, first)?) {
                Bound::Target(t) => t,
                _ => return None,
            }
        };
        for part in parts {
            cur = match cur {
                Target::Module(m) => self.lookup(m, part)?,
                Target::Def(f2, c) if self.def(f2, c).kind == DefKind::Class => {
                    let (f3, d) = self.method((f2, c), part, false)?;
                    Target::Def(f3, d)
                }
                Target::Def(..) | Target::Ext => return None,
            };
        }
        Some(cur)
    }

    fn refs(&mut self, out: &mut Out, index: &Index) {
        enum To {
            Id(String),
            Stub(usize, String),
        }
        struct Pending {
            file: usize,
            source: String,
            to: To,
            relation: &'static str,
            context: Option<&'static str>,
            line: u32,
        }
        let mut pending: Vec<Pending> = Vec::new();
        let mut stub_labels: HashMap<(usize, String), String> = HashMap::new();
        let mut bases: HashMap<(usize, u32), Vec<(usize, u32)>> = HashMap::new();
        for (fi, f) in self.files.iter().enumerate() {
            for r in &f.py.refs {
                let scope = f.py.defs[r.source as usize].parent;
                let target = self.resolve_type(fi, scope, &r.name, r.qual.as_deref());
                let (relation, context) = match r.kind {
                    RefKind::Base => ("inherits", None),
                    RefKind::Param => ("references", Some("parameter_type")),
                    RefKind::Return => ("references", Some("return_type")),
                    RefKind::Generic => ("references", Some("generic_arg")),
                    RefKind::Decorator => ("references", Some("decorator")),
                };
                let to = match target {
                    Lookup::Drop => continue,
                    Lookup::Found((f2, d)) => {
                        if r.kind == RefKind::Base && self.def(f2, d).kind == DefKind::Class {
                            bases.entry((fi, r.source)).or_default().push((f2, d));
                        }
                        To::Id(self.id(f2, d).to_string())
                    }
                    Lookup::NotFound => {
                        let s = slug(&r.name);
                        if s.is_empty() {
                            continue;
                        }
                        stub_labels
                            .entry((fi, s.clone()))
                            .or_insert_with(|| r.name.clone());
                        To::Stub(fi, s)
                    }
                };
                pending.push(Pending {
                    file: fi,
                    source: f.ids[r.source as usize].clone(),
                    to,
                    relation,
                    context,
                    line: r.line,
                });
            }
        }
        self.bases = bases;

        // Stub ids follow the collision rule (§4.5): an id emitted by two files, or clashing
        // with another node, is prefixed with the emitting file's path slug.
        let mut emitters: HashMap<&str, usize> = HashMap::new();
        for (_, s) in stub_labels.keys() {
            *emitters.entry(s.as_str()).or_default() += 1;
        }
        let taken = |id: &str| {
            index
                .get(self.views, id)
                .is_some_and(|n| n.kind != NodeKind::External)
        };
        let mut renamed: HashMap<(usize, String), String> = HashMap::new();
        let mut used: HashMap<String, usize> = HashMap::new();
        let mut keys: Vec<&(usize, String)> = stub_labels.keys().collect();
        keys.sort();
        for key in &keys {
            let (fi, s) = key;
            let id = if emitters[s.as_str()] > 1 || taken(s) {
                format!("{}_{s}", slug(self.files[*fi].rel))
            } else {
                s.clone()
            };
            *used.entry(id.clone()).or_default() += 1;
            renamed.insert((*fi, s.clone()), id);
        }
        for (key, id) in renamed.iter_mut() {
            if id != &key.1 && (used[id.as_str()] > 1 || taken(id)) {
                let digest = sha1_smol::Sha1::from(self.files[key.0].rel.as_bytes())
                    .digest()
                    .to_string();
                *id = format!("{id}_{}", &digest[..6]);
            }
        }
        let mut emitted: HashSet<String> = HashSet::new();
        for Pending {
            file: fi,
            source,
            to,
            relation,
            context,
            line,
        } in pending
        {
            let target = match to {
                To::Id(id) => id,
                To::Stub(sfi, s) => {
                    let id = renamed[&(sfi, s.clone())].clone();
                    if emitted.insert(id.clone()) {
                        out.nodes.push(Node {
                            id: id.clone(),
                            label: stub_labels[&(sfi, s)].clone(),
                            kind: NodeKind::Stub,
                            file_type: "code".into(),
                            source_file: String::new(),
                            loc: Loc::Empty,
                            attrs: vec![origin()],
                        });
                    }
                    id
                }
            };
            // An `inherits` link carries its score so a folded `uses` cannot lower it (§8.2).
            let score = (relation == "inherits").then_some(1.0);
            out.edges.push(edge(
                &source,
                &target,
                relation,
                self.files[fi].rel,
                line,
                context,
                Confidence::Extracted,
                score,
            ));
        }
    }

    // ---- calls ------------------------------------------------------------------------------

    /// A free name used in `scope`: (target, confidence, score).
    fn free_name(
        &self,
        fi: usize,
        scope: Option<u32>,
        name: &str,
    ) -> Option<((usize, u32), Confidence, Option<f64>)> {
        if let Some(d) = self.local(fi, scope, name) {
            return Some(((fi, d), Confidence::Extracted, None));
        }
        if let Some(b) = self.binding(fi, scope, name) {
            return match self.bound(b) {
                Bound::Target(Target::Def(f2, d)) => {
                    Some(((f2, d), Confidence::Extracted, (f2 != fi).then_some(1.0)))
                }
                // Imported from outside the repository: at best a unique-name guess (a sibling
                // script module imported as a top-level name).
                Bound::ExtSym if !BUILTINS.contains(&name) => self
                    .unique(name, Uniq::Any)
                    .map(|t| (t, Confidence::Inferred, Some(0.85))),
                _ => None,
            };
        }
        if BUILTINS.contains(&name) {
            return None;
        }
        if let Some(t) = self.unique_in_from_modules(fi, name) {
            return Some((t, Confidence::Extracted, Some(1.0)));
        }
        self.unique(name, Uniq::Any)
            .map(|t| (t, Confidence::Inferred, Some(0.85)))
    }

    /// Emits `calls` and returns the first call line per (file, caller, target).
    fn calls(&self, out: &mut Out) -> HashMap<(usize, u32, String), u32> {
        let mut first: HashMap<(usize, u32, String), u32> = HashMap::new();
        for (fi, f) in self.files.iter().enumerate() {
            for c in &f.py.calls {
                let target: Option<(String, Confidence, Option<f64>)> = match &c.kind {
                    CallKind::Name => self
                        .free_name(fi, Some(c.caller), &c.name)
                        .map(|(t, conf, score)| (self.id(t.0, t.1).to_string(), conf, score)),
                    CallKind::SelfMethod(class) => {
                        self.method((fi, *class), &c.name, true).map(|(f2, d)| {
                            (
                                self.id(f2, d).to_string(),
                                Confidence::Extracted,
                                (f2 != fi).then_some(1.0),
                            )
                        })
                    }
                    CallKind::Super(class) => self
                        .bases
                        .get(&(fi, *class))
                        .into_iter()
                        .flatten()
                        .find_map(|&b| self.method(b, &c.name, true))
                        .map(|(f2, d)| {
                            (
                                self.id(f2, d).to_string(),
                                Confidence::Extracted,
                                (f2 != fi).then_some(1.0),
                            )
                        }),
                    CallKind::Typed(class) => {
                        let (q, n) = split_dotted(class);
                        let found = match self.resolve_type(fi, Some(c.caller), n, q) {
                            Lookup::Found(t) => Some(t),
                            _ => None,
                        };
                        found
                            .filter(|&(f2, d)| self.def(f2, d).kind == DefKind::Class)
                            .and_then(|cls| self.method(cls, &c.name, false))
                            .filter(|&(f2, d)| self.def(f2, d).kind == DefKind::Method)
                            .map(|(f2, d)| {
                                (self.id(f2, d).to_string(), Confidence::Inferred, Some(0.85))
                            })
                    }
                    CallKind::Attr(recv) => self.attr_call(fi, c.caller, recv, &c.name, out),
                };
                let Some((target, conf, score)) = target else {
                    continue;
                };
                if first.contains_key(&(fi, c.caller, target.clone())) {
                    continue;
                }
                first.insert((fi, c.caller, target.clone()), c.line);
                out.edges.push(edge(
                    &f.ids[c.caller as usize],
                    &target,
                    "calls",
                    f.rel,
                    c.line,
                    Some("call"),
                    conf,
                    score,
                ));
            }
        }
        first
    }

    /// `recv.name()` where `recv` is a class, a local module or an external module.
    fn attr_call(
        &self,
        fi: usize,
        caller: u32,
        recv: &str,
        name: &str,
        out: &mut Out,
    ) -> Option<(String, Confidence, Option<f64>)> {
        let first = first_component(recv);
        if self.local(fi, Some(caller), first).is_none() {
            if let Some(b) = self.binding(fi, Some(caller), first) {
                if let Bound::ExtModule(dotted) = self.bound(b) {
                    let def = self.def(fi, caller);
                    let top_level = def.parent.is_none() && def.kind == DefKind::Function;
                    // Only through a module-level `import x [as y]`.
                    let module_binding = self.files[fi]
                        .mod_bindings
                        .get(first)
                        .is_some_and(|m| std::ptr::eq(m, b));
                    if EXTERNAL_CALLS_TOP_LEVEL_ONLY
                        && (!top_level || recv != first || !module_binding)
                    {
                        return None;
                    }
                    let id = out.external(&dotted)?;
                    return Some((id, Confidence::Extracted, Some(1.0)));
                }
            }
        }
        let (f2, d) = match self.receiver(fi, Some(caller), recv)? {
            Target::Module(m) => match self.lookup(m, name)? {
                Target::Def(f2, d) => (f2, d),
                Target::Module(_) | Target::Ext => return None,
            },
            Target::Def(f2, c) if self.def(f2, c).kind == DefKind::Class => {
                self.method((f2, c), name, true)?
            }
            Target::Def(..) | Target::Ext => return None,
        };
        Some((self.id(f2, d).to_string(), Confidence::Extracted, Some(1.0)))
    }

    /// A function used as a value; skipped when the same caller already called it directly.
    fn indirect(&self, out: &mut Out, called: &HashMap<(usize, u32, String), u32>) {
        for (fi, f) in self.files.iter().enumerate() {
            let mut seen: HashSet<(Option<u32>, String)> = HashSet::new();
            for s in &f.py.indirect {
                let Some(((f2, d), _, _)) = self.free_name(fi, s.caller, &s.name) else {
                    continue;
                };
                if self.def(f2, d).kind == DefKind::Class {
                    continue;
                }
                let target = self.id(f2, d).to_string();
                if s.caller.is_some_and(|c| {
                    called
                        .get(&(fi, c, target.clone()))
                        .is_some_and(|&l| l <= s.line)
                }) {
                    continue;
                }
                if !seen.insert((s.caller, target.clone())) {
                    continue;
                }
                let source = match s.caller {
                    Some(c) => &f.ids[c as usize],
                    None => &f.file_id,
                };
                out.edges.push(edge(
                    source,
                    &target,
                    "indirect_call",
                    f.rel,
                    s.line,
                    Some(&s.context),
                    Confidence::Inferred,
                    Some(0.85),
                ));
            }
        }
    }

    // ---- uses -------------------------------------------------------------------------------

    /// A top-level function or class mentioning a class imported by name from the module that
    /// defines it (re-exports are not followed).
    fn uses(&self, out: &mut Out) {
        for (fi, f) in self.files.iter().enumerate() {
            let mut seen: HashSet<(u32, String)> = HashSet::new();
            for m in &f.py.mentions {
                let Some(Binding::From {
                    module: ModRef::Local(m2),
                    name,
                }) = self.binding(fi, m.at, &m.name)
                else {
                    continue;
                };
                if *m2 == fi {
                    continue;
                }
                let Some(&c) = self.files[*m2].top.get(name.as_str()) else {
                    continue;
                };
                if self.def(*m2, c).kind != DefKind::Class {
                    continue;
                }
                let target = self.id(*m2, c).to_string();
                if !seen.insert((m.scope, target.clone())) {
                    continue;
                }
                out.edges.push(edge(
                    &f.ids[m.scope as usize],
                    &target,
                    "uses",
                    f.rel,
                    m.line,
                    None,
                    Confidence::Inferred,
                    Some(0.95),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dotted_of;

    #[test]
    fn module_names_from_paths() {
        assert_eq!(dotted_of("pkg/models.py"), ("pkg.models".into(), false));
        assert_eq!(dotted_of("pkg/__init__.py"), ("pkg".into(), true));
        assert_eq!(dotted_of("__init__.py"), ("".into(), true));
        assert_eq!(
            dotted_of("benchsuite/benchsuite"),
            ("benchsuite.benchsuite".into(), false)
        );
    }
}

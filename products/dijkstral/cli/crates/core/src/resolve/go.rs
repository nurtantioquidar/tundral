//! Cross-file pass for Go (§12). See `docs/graphify-format.md`.
//!
//! - Module resolution: every `go.mod` contributes `module path → directory`; an import path
//!   resolves to a directory by the longest module prefix, and it is local only when that directory
//!   holds Go files. Local imports link the importing file to every file node of the package
//!   directory (`_test.go` files included, quirk G4); anything else links to `go_pkg_<path>`.
//! - Type stubs: an unqualified type not declared in its file binds to the unique type of that name
//!   in the same directory, else stays a stub; a qualified `q.T` binds to the unique type `T` of the
//!   imported directory, else becomes `go_type_<path>_<T>`.
//! - Calls: `pkg.F()` binds to the unique callable `F` of the imported directory (EXTRACTED 1.0); a
//!   bare `F()` not defined in its file binds to the unique `F` among the other files of its
//!   directory, else to the unique `F` in the repository (INFERRED 0.85).
//! - Files at the scan root: their `@root` type and method ids take the root directory's name.
//! - Method calls on typed receivers (dijkstral-native, G5 not reproduced for them): a receiver's
//!   symbolic type (`crate::extract::go::GoTy`, local type inference) evaluates against the
//!   package's declared types, results and fields to a named non-interface type, whose method
//!   of that name gets an EXTRACTED `calls` edge with score 1.0 when in-file resolution chose
//!   another target or none. Interfaces, embedded (promoted) methods and anything a declaration
//!   does not fix resolve nothing.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::{Index, Resolution, View};
use crate::extract::go::{GoCallee, GoFacts, GoField, GoReturn, GoTy, GoWritten, ROOT_DIR};
use crate::extract::{external_node, origin, slug};
use crate::facts::{Confidence, Edge, LangFacts, Loc, Node, NodeKind};
use crate::lang::Lang;

/// The directory part of a relative path (`""` at the root).
fn parent(rel: &str) -> &str {
    rel.rfind('/').map_or("", |i| &rel[..i])
}

fn go_facts<'v>(v: &'v View) -> Option<&'v GoFacts> {
    match &v.facts.lang {
        LangFacts::Go(g) => Some(g),
        _ => None,
    }
}

/// The scan root's directory name, from any file's absolute and relative paths.
fn root_name(v: &View) -> String {
    let depth = v.file.rel.split('/').count();
    let root = v.file.abs.ancestors().nth(depth).unwrap_or(Path::new(""));
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// (module path, module directory), longest path first.
struct Modules(Vec<(String, String)>);

impl Modules {
    /// The directory an import path names, if it is under a known module.
    fn dir(&self, path: &str) -> Option<String> {
        for (module, dir) in &self.0 {
            let rest = if path == module {
                ""
            } else if let Some(rest) = path
                .strip_prefix(module.as_str())
                .and_then(|r| r.strip_prefix('/'))
            {
                rest
            } else {
                continue;
            };
            return Some(match (dir.is_empty(), rest.is_empty()) {
                (true, _) => rest.to_string(),
                (false, true) => dir.clone(),
                (false, false) => format!("{dir}/{rest}"),
            });
        }
        None
    }
}

struct EdgeSpec<'s> {
    relation: &'s str,
    rel: &'s str,
    line: u32,
    context: &'s str,
    confidence: Confidence,
    score: Option<f64>,
}

fn edge(source: String, target: String, s: &EdgeSpec) -> Edge {
    Edge {
        source,
        target,
        relation: s.relation.into(),
        confidence: s.confidence,
        score: s.score,
        context: Some(s.context.into()),
        source_file: s.rel.into(),
        loc: Loc::Line(s.line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

fn stub_node(id: String, label: String) -> Node {
    Node {
        id,
        label,
        kind: NodeKind::Stub,
        file_type: "code".into(),
        source_file: String::new(),
        loc: Loc::Empty,
        attrs: vec![origin()],
    }
}

/// Exactly one distinct candidate.
fn unique(c: Option<&Vec<String>>) -> Option<&String> {
    match c {
        Some(v) if v.len() == 1 => v.first(),
        _ => None,
    }
}

fn push_unique(v: &mut Vec<String>, id: String) {
    if !v.contains(&id) {
        v.push(id);
    }
}

/// The name a definition answers to in a call: `.M()` → `M`, `F()` → `F`, `T` → `T`.
fn call_name(label: &str) -> &str {
    let l = label.strip_prefix('.').unwrap_or(label);
    l.strip_suffix("()").unwrap_or(l)
}

/// Path proximity of a candidate to the caller's file: the number of leading directories they
/// share, then (negated) how far below that common directory the candidate sits.
fn proximity(from: &str, to: &str) -> (usize, isize) {
    let (a, b) = (parent(from), parent(to));
    let parts = |s: &'_ str| s.split('/').filter(|p| !p.is_empty()).count();
    let common = a
        .split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y && !x.is_empty())
        .count();
    (common, -((parts(b) - common) as isize))
}

/// The candidate nearest to the caller's file (see [`proximity`]), if no other candidate is as
/// near. Graphify binds a bare cross-file call by name this way (INFERRED 0.85); measured on
/// kubernetes it reproduces 98% of its choices. Candidates that share no directory with the
/// caller are only taken when there is exactly one.
fn nearest<'c>(from: &str, caller: &str, cands: &'c [(String, &str)]) -> Option<&'c str> {
    let mut best: Option<((usize, isize), &str)> = None;
    let (mut tied, mut count) = (false, 0);
    for (id, rel) in cands {
        if id == caller {
            continue;
        }
        count += 1;
        let p = proximity(from, rel);
        match best {
            Some((bp, _)) if p < bp => {}
            Some((bp, _)) if p == bp => tied = true,
            _ => {
                best = Some((p, id));
                tied = false;
            }
        }
    }
    let (p, id) = best?;
    (!tied && (p.0 > 0 || count == 1)).then_some(id)
}

/// Directory → name → distinct ids.
type Table<'t> = HashMap<&'t str, HashMap<&'t str, Vec<String>>>;

fn lookup(table: &Table, dir: &str, name: &str) -> Option<String> {
    unique(table.get(dir).and_then(|m| m.get(name))).cloned()
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let go: Vec<(&View, &GoFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Go)
        .filter_map(|v| go_facts(v).map(|g| (v, g)))
        .collect();
    if go.is_empty() {
        return res;
    }

    let mut modules: Vec<(String, String)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::GoMod)
        .filter_map(|v| {
            let m = go_facts(v)?.module.clone()?;
            Some((m, parent(&v.file.rel).to_string()))
        })
        .collect();
    modules.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.cmp(b)));
    let modules = Modules(modules);

    // Root-level files: `@root` ids take the scan root's directory name.
    let root_slug = go
        .iter()
        .find(|(v, _)| !v.file.rel.contains('/'))
        .map(|(v, _)| slug(&root_name(v)));
    let fix = |id: &str| -> String {
        match &root_slug {
            Some(r) if id.contains(ROOT_DIR) => id.replace(ROOT_DIR, r),
            _ => id.to_string(),
        }
    };
    for (v, _) in go.iter().filter(|(v, _)| !v.file.rel.contains('/')) {
        for n in v.facts.nodes.iter().filter(|n| n.id.contains(ROOT_DIR)) {
            let mut moved = n.clone();
            moved.id = fix(&n.id);
            res.redirects.push((n.id.clone(), moved.id.clone()));
            res.nodes.push(moved);
        }
    }
    let fid = |v: &View, local: &str| fix(v.id(local));

    // Per-directory tables.
    let mut dir_files: HashMap<&str, Vec<String>> = HashMap::new();
    let mut types: Table = HashMap::new();
    // Qualified-call candidates: every definition of the directory (receiver nodes included).
    let mut callables: Table = HashMap::new();
    for (v, g) in &go {
        let dir = parent(&v.file.rel);
        dir_files.entry(dir).or_default().push(fid(v, &g.file_id));
        for (name, id) in &g.types {
            let slot = types.entry(dir).or_default().entry(name.as_str());
            push_unique(slot.or_default(), fid(v, id));
        }
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
            let slot = callables.entry(dir).or_default().entry(call_name(&n.label));
            push_unique(slot.or_default(), fix(&n.id));
        }
    }
    // Every definition by the name it answers to: the label index for stubs that did not bind
    // in their file (type-like labels and function names; methods never match) and the
    // candidates of cross-file bare calls (any kind, chosen by path proximity).
    let mut type_labels: HashMap<&str, Vec<String>> = HashMap::new();
    let mut func_labels: HashMap<&str, Vec<String>> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<(String, &str)>> = HashMap::new();
    for (v, _) in &go {
        let rel = v.file.rel.as_str();
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
            let id = fix(&n.id);
            by_name
                .entry(call_name(&n.label))
                .or_default()
                .push((id.clone(), rel));
            if n.label.starts_with('.') {
                continue;
            }
            match n.label.strip_suffix("()") {
                Some(name) => push_unique(func_labels.entry(name).or_default(), id),
                None => push_unique(type_labels.entry(&n.label).or_default(), id),
            }
        }
    }
    let by_label = |key: &str| -> Option<String> {
        unique(type_labels.get(key))
            .or_else(|| unique(func_labels.get(key)))
            .cloned()
    };
    let mut local_dirs: HashMap<String, Option<String>> = HashMap::new();
    let mut local_dir = |path: &str| -> Option<String> {
        if let Some(d) = local_dirs.get(path) {
            return d.clone();
        }
        let d = modules
            .dir(path)
            .filter(|d| dir_files.contains_key(d.as_str()));
        local_dirs.insert(path.to_string(), d.clone());
        d
    };
    let is_stub = |id: &str| {
        index
            .get(files, id)
            .is_some_and(|n| n.kind == NodeKind::Stub)
    };

    // Declarations for local type inference, by package directory.
    let dirs: Vec<&str> = go.iter().map(|(v, _)| parent(&v.file.rel)).collect();
    let quals: Vec<HashMap<&str, Option<&str>>> = go
        .iter()
        .map(|(_, g)| {
            let mut m = HashMap::new();
            for imp in &g.imports {
                let q = match imp.alias.as_deref() {
                    Some("." | "_") => None,
                    Some(a) => Some(a),
                    None => imp.path.rsplit('/').next(),
                };
                if let Some(q) = q {
                    m.entry(q).or_insert_with(|| {
                        local_dir(&imp.path)
                            .and_then(|d| dir_files.get_key_value(d.as_str()).map(|(k, _)| *k))
                    });
                }
            }
            m
        })
        .collect();
    let decls = Decls::new(&go, dirs, quals, &types, fid, fix);

    let mut externals: HashSet<String> = HashSet::new();
    let mut go_types: HashSet<String> = HashSet::new();
    for (fi, (v, g)) in go.iter().enumerate() {
        let rel = v.file.rel.as_str();
        let file_id = fid(v, &g.file_id);
        let mut qualifiers: HashMap<&str, &str> = HashMap::new();
        for imp in &g.imports {
            let q = match imp.alias.as_deref() {
                Some("." | "_") => None,
                Some(a) => Some(a),
                None => imp.path.rsplit('/').next(),
            };
            if let Some(q) = q {
                qualifiers.entry(q).or_insert(imp.path.as_str());
            }
        }

        // Imports.
        let mut seen: HashSet<String> = HashSet::new();
        for imp in &g.imports {
            let spec = EdgeSpec {
                relation: "imports_from",
                rel,
                line: imp.line,
                context: "import",
                confidence: Confidence::Extracted,
                score: None,
            };
            match local_dir(&imp.path) {
                Some(d) => {
                    for target in &dir_files[d.as_str()] {
                        if *target != file_id && seen.insert(target.clone()) {
                            res.edges.push(edge(file_id.clone(), target.clone(), &spec));
                        }
                    }
                }
                None => {
                    // One edge per spec: `_ "x"` then `y "x"` folds to the later line.
                    let target = format!("go_pkg_{}", slug(&imp.path));
                    if externals.insert(target.clone()) {
                        res.nodes.push(external_node(target.clone()));
                    }
                    res.edges.push(edge(file_id.clone(), target, &spec));
                }
            }
        }

        // Type stubs.
        for (local, name) in &g.bare_stubs {
            let stub = v.id(local);
            if !is_stub(stub) {
                continue;
            }
            if let Some(target) = by_label(name) {
                res.redirects.push((stub.to_string(), target));
            }
        }
        for q in &g.qual_stubs {
            let stub = v.id(&q.id);
            if !is_stub(stub) {
                continue;
            }
            // Through an import: the package's type, else a type labelled like `path.Name` with
            // the dots dropped (`bytes.Reader` → `bytesReader`), else the `go_type_` stub. A
            // qualifier that is not an import binds by label (`x.Alias` → `xAlias()`) or stays.
            let path = qualifiers.get(q.qualifier.as_str()).copied();
            let found = match path {
                Some(p) => {
                    let key = format!("{p}{}", q.name).replace('.', "");
                    local_dir(p)
                        .and_then(|d| lookup(&types, &d, &q.name))
                        .or_else(|| unique(type_labels.get(key.as_str())).cloned())
                }
                None => by_label(&format!("{}{}", q.qualifier, q.name)),
            };
            let target = match found {
                Some(t) => t,
                None => {
                    let Some(path) = path else { continue };
                    let id = format!("go_type_{}_{}", slug(path), slug(&q.name));
                    if go_types.insert(id.clone()) {
                        res.nodes
                            .push(stub_node(id.clone(), format!("{path}.{}", q.name)));
                    }
                    id
                }
            };
            res.redirects.push((stub.to_string(), target));
        }

        // Calls.
        let mut called: HashSet<(String, String)> = HashSet::new();
        for c in &g.calls {
            let caller = fid(v, &c.caller);
            let (target, confidence, score) = match &c.path {
                Some(path) => {
                    let Some(d) = local_dir(path) else { continue };
                    // A method is never `pkg.M`: when the name is also a method of the
                    // package, its unique function decides (dijkstral-native).
                    let found = lookup(&callables, &d, &c.name).or_else(|| {
                        unique(decls.funcs.get(&(d.as_str(), c.name.as_str()))).cloned()
                    });
                    match found {
                        Some(t) => (t, Confidence::Extracted, Some(1.0)),
                        None => continue,
                    }
                }
                None => {
                    let cands = by_name.get(c.name.as_str()).map_or(&[][..], Vec::as_slice);
                    match nearest(rel, &caller, cands) {
                        Some(t) => (t.to_string(), Confidence::Inferred, Some(0.85)),
                        None => continue,
                    }
                }
            };
            if target == caller || !called.insert((caller.clone(), target.clone())) {
                continue;
            }
            let spec = EdgeSpec {
                relation: "calls",
                rel,
                line: c.line,
                context: "call",
                confidence,
                score,
            };
            res.edges.push(edge(caller, target, &spec));
        }

        // Method calls on typed receivers.
        for c in &g.method_calls {
            let Some(target) = decls.method_call(fi, c.recv, &c.name) else {
                continue;
            };
            // Only from the caller's own node (not another file's definition of its id).
            if index
                .get(files, v.id(&c.caller))
                .is_none_or(|n| n.source_file != v.file.rel)
            {
                continue;
            }
            let caller = fid(v, &c.caller);
            if c.in_file.as_deref().is_some_and(|t| fid(v, t) == target)
                || target == caller
                || !called.insert((caller.clone(), target.clone()))
            {
                continue;
            }
            let spec = EdgeSpec {
                relation: "calls",
                rel,
                line: c.line,
                context: "call",
                confidence: Confidence::Extracted,
                score: Some(1.0),
            };
            res.edges.push(edge(caller, target, &spec));
        }
    }
    res
}

/// A named type: (package directory, name).
type Named<'t> = (&'t str, &'t str);

/// A struct's fields: (declaring Go file, field).
type Fields<'t> = Vec<(usize, &'t GoField)>;

/// What a selector `x.name` reaches on a named type.
enum Member<'t> {
    Method(String),
    /// A field of a named type, written in Go file `.0`.
    Field(usize, &'t GoWritten),
    /// A field whose type is not a named type.
    Opaque,
}

/// The declarations local type inference evaluates against.
struct Decls<'t> {
    /// Per Go file: its package directory.
    dirs: Vec<&'t str>,
    /// Per Go file: import qualifier → the repository directory it names (`None` outside).
    quals: Vec<HashMap<&'t str, Option<&'t str>>>,
    /// Per Go file: its symbolic types.
    tys: Vec<&'t [GoTy]>,
    types: &'t Table<'t>,
    interfaces: HashSet<Named<'t>>,
    /// (directory, receiver type, method) → method ids.
    methods: HashMap<(&'t str, &'t str, &'t str), Vec<String>>,
    /// (directory, function) → function ids.
    funcs: HashMap<(&'t str, &'t str), Vec<String>>,
    /// Struct → (number of declarations, fields of every declaration).
    structs: HashMap<Named<'t>, (usize, Fields<'t>)>,
    /// Function or method id → (declaring file, results), every definition.
    returns: HashMap<String, Vec<(usize, &'t GoReturn)>>,
    /// (directory, package-level variable) → (declaring file, its type), every declaration.
    vars: HashMap<Named<'t>, Vec<(usize, u32)>>,
    memo: RefCell<HashMap<(usize, u32), Option<Named<'t>>>>,
}

impl<'t> Decls<'t> {
    fn new(
        go: &'t [(&'t View<'t>, &'t GoFacts)],
        dirs: Vec<&'t str>,
        quals: Vec<HashMap<&'t str, Option<&'t str>>>,
        types: &'t Table<'t>,
        fid: impl Fn(&View, &str) -> String,
        fix: impl Fn(&str) -> String,
    ) -> Self {
        let mut d = Decls {
            dirs,
            quals,
            tys: go.iter().map(|(_, g)| g.tys.as_slice()).collect(),
            types,
            interfaces: HashSet::new(),
            methods: HashMap::new(),
            funcs: HashMap::new(),
            structs: HashMap::new(),
            returns: HashMap::new(),
            vars: HashMap::new(),
            memo: RefCell::new(HashMap::new()),
        };
        for (fi, (v, g)) in go.iter().enumerate() {
            let dir = d.dirs[fi];
            for name in &g.interfaces {
                d.interfaces.insert((dir, name));
            }
            for (ty, name, id) in &g.methods {
                push_unique(d.methods.entry((dir, ty, name)).or_default(), fid(v, id));
            }
            for name in &g.structs {
                d.structs.entry((dir, name)).or_default().0 += 1;
            }
            for f in &g.fields {
                d.structs
                    .entry((dir, &f.owner))
                    .or_default()
                    .1
                    .push((fi, f));
            }
            for r in &g.returns {
                d.returns.entry(fid(v, &r.id)).or_default().push((fi, r));
            }
            for (name, ty) in &g.vars {
                d.vars.entry((dir, name)).or_default().push((fi, *ty));
            }
            for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
                if let Some(name) = n.label.strip_suffix("()").filter(|l| !l.starts_with('.')) {
                    push_unique(d.funcs.entry((dir, name)).or_default(), fix(&n.id));
                }
            }
        }
        d
    }

    /// The method `recv.name(..)` calls.
    fn method_call(&self, f: usize, recv: u32, name: &str) -> Option<String> {
        let (d, t) = self.eval(f, recv)?;
        match self.member(d, t, name)? {
            Member::Method(id) => Some(id),
            _ => None,
        }
    }

    /// What `x.name` selects on a value of type `(d, t)`: its own method or field, else the
    /// one promoted from the shallowest level of embedded fields that has the name, when that
    /// level has exactly one. A level holding a type whose members are not all known (an
    /// interface, a type from outside the repository, a non-struct or twice-declared type)
    /// decides nothing.
    fn member(&self, d: &'t str, t: &'t str, name: &str) -> Option<Member<'t>> {
        let mut level: Vec<Named<'t>> = vec![(d, t)];
        let mut seen: HashSet<Named<'t>> = HashSet::new();
        for _ in 0..8 {
            let mut found = None;
            let mut count = 0;
            let mut unknown = false;
            let mut next = Vec::new();
            for &(d, t) in &level {
                if let Some(ids) = self.methods.get(&(d, t, name)) {
                    count += ids.len();
                    found = Some(Member::Method(ids[0].clone()));
                }
                match self.structs.get(&(d, t)) {
                    Some((1, fields)) => {
                        for &(g, f) in fields {
                            if f.name == name {
                                count += 1;
                                found = Some(match &f.ty {
                                    Some(w) => Member::Field(g, w),
                                    None => Member::Opaque,
                                });
                            }
                            if f.embedded {
                                match f.ty.as_ref().and_then(|w| self.written(g, w)) {
                                    Some(e) if seen.insert(e) => next.push(e),
                                    Some(_) => {}
                                    None => unknown = true,
                                }
                            }
                        }
                    }
                    _ => unknown = true,
                }
            }
            match count {
                0 if !unknown && !next.is_empty() => level = next,
                1 => return found,
                _ => return None,
            }
        }
        None
    }

    fn eval(&self, f: usize, i: u32) -> Option<Named<'t>> {
        if let Some(v) = self.memo.borrow().get(&(f, i)) {
            return *v;
        }
        // Entries only point at earlier ones and variables cannot initialise each other in
        // Go; the marker guards malformed input.
        self.memo.borrow_mut().insert((f, i), None);
        let v = self.eval_uncached(f, i);
        self.memo.borrow_mut().insert((f, i), v);
        v
    }

    fn eval_uncached(&self, f: usize, i: u32) -> Option<Named<'t>> {
        match self.tys.get(f)?.get(i as usize)? {
            GoTy::Written(w) => self.written(f, w),
            GoTy::Call { callee, index } => match callee {
                GoCallee::Bare(name) => self.call_in(self.dirs[f], name, *index),
                GoCallee::Qual(q, name) => {
                    let d = (*self.quals[f].get(q.as_str())?)?;
                    self.call_in(d, name, *index)
                }
                GoCallee::Method(r, name) => {
                    let (d, t) = self.eval(f, *r)?;
                    match self.member(d, t, name)? {
                        Member::Method(m) => self.returned(&m, *index),
                        _ => None,
                    }
                }
            },
            GoTy::Field { recv, name } => {
                let (d, t) = self.eval(f, *recv)?;
                match self.member(d, t, name)? {
                    Member::Field(g, w) => self.written(g, w),
                    _ => None,
                }
            }
            GoTy::PkgVar(name) => self.var(self.dirs[f], name),
            GoTy::QualVar(q, name) => {
                let d = (*self.quals[f].get(q.as_str())?)?;
                self.var(d, name)
            }
        }
    }

    /// The type of package-level variable `name` of directory `d` (declared once).
    fn var(&self, d: &'t str, name: &str) -> Option<Named<'t>> {
        match self.vars.get(&(d, name)).map(Vec::as_slice) {
            Some(&[(g, i)]) => self.eval(g, i),
            _ => None,
        }
    }

    /// The non-interface named type `w` denotes in Go file `f`.
    fn written(&self, f: usize, w: &GoWritten) -> Option<Named<'t>> {
        let d = match &w.qual {
            None => self.dirs[f],
            Some(q) => (*self.quals[f].get(q.as_str())?)?,
        };
        let (&name, _) = self.types.get(d)?.get_key_value(w.name.as_str())?;
        (!self.interfaces.contains(&(d, name))).then_some((d, name))
    }

    /// Result `index` of `name(..)` in package directory `d`: a function's declared result,
    /// or the type a conversion `T(x)` names.
    fn call_in(&self, d: &'t str, name: &str, index: u32) -> Option<Named<'t>> {
        if let Some((&t, _)) = self.types.get(d).and_then(|m| m.get_key_value(name)) {
            return (index == 0 && !self.interfaces.contains(&(d, t))).then_some((d, t));
        }
        let id = unique(self.funcs.get(&(d, name)))?;
        self.returned(id, index)
    }

    /// Result `index` of function or method `id` (every definition must agree).
    fn returned(&self, id: &str, index: u32) -> Option<Named<'t>> {
        let rets = self.returns.get(id)?;
        let pick = |(g, r): &(usize, &'t GoReturn)| {
            let w = r.results.get(index as usize)?.as_ref()?;
            self.written(*g, w)
        };
        let first = pick(&rets[0])?;
        rets[1..]
            .iter()
            .all(|r| pick(r) == Some(first))
            .then_some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_paths_resolve_by_longest_module_prefix() {
        let m = Modules(vec![
            ("ex.com/m/sub".into(), "sub".into()),
            ("ex.com/m".into(), String::new()),
            ("k8s.io/api".into(), "staging/src/k8s.io/api".into()),
        ]);
        assert_eq!(m.dir("ex.com/m/p"), Some("p".into()));
        assert_eq!(m.dir("ex.com/m"), Some(String::new()));
        assert_eq!(m.dir("ex.com/m/sub/x"), Some("sub/x".into()));
        assert_eq!(
            m.dir("k8s.io/api/core/v1"),
            Some("staging/src/k8s.io/api/core/v1".into())
        );
        assert_eq!(m.dir("ex.com/mx"), None);
        assert_eq!(m.dir("fmt"), None);
    }
}

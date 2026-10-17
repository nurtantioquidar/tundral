//! Cross-file pass for Rust (§11). See `docs/graphify-format.md`.
//!
//! 1. External nodes for `use` leaves (appended after every per-file node, so a type-reference
//!    stub with the same id wins, §11.2 rule 1).
//! 2. Type-stub resolution (§11.2 rule 3): a stub whose label names exactly one Rust struct, enum
//!    or trait (or, when no type has that name, exactly one other Rust definition: fn, const,
//!    variant, macro, impl owner) is deleted and redirected to it. Import edges that target the
//!    same id follow (rule 4).
//! 3. Bare calls left unresolved in-file (§11.5): among the Rust definitions labelled `name()`,
//!    `.name()` or `Name`, the one closest to the caller (longest shared directory prefix, then
//!    same directory, then same file) gets an INFERRED `calls` or `references` (`constructor`)
//!    edge with score 0.85. A tie for closest resolves nothing.
//! 4. `self.name()` inside an impl whose file has no method `name`: when the impl's type (its
//!    owner's `_rust_impl_key` name, else its label) names exactly one declared struct, enum or
//!    trait, the unique method `name` among all impls of that type name gets an EXTRACTED `calls`
//!    edge with score 1.0 (`impl Atlas` split across `lib.rs` and `rank.rs`).
//!
//! Deliberate departures from Graphify 0.9.86 (Q7 not reproduced), through the module tree in
//! [`modules`]:
//!
//! - Every `use` leaf that resolves into the repository (function bodies included) adds a
//!   file → file `imports_from` edge to the file defining it (`use crate::flags::HiArgs` →
//!   `flags/hiargs.rs` through the `pub use` in `flags/mod.rs`). The per-leaf edges to the
//!   `slug(leaf)` external stubs stay as Graphify emits them.
//! - A type stub named by one of its file's imports resolves through that import: to the
//!   definition it names, or not at all when the import leads outside the repository
//!   (`use termcolor::Color` never becomes another crate's `Color`). Only names no import
//!   explains fall back to the unique-label rule (step 2).
//!
//! Calls (Q23 not reproduced), each an EXTRACTED edge with score 1.0, only from a caller whose
//! definition is the node the build keeps for its id, and only where in-file resolution chose
//! another target or none:
//!
//! - A bare call to a name the file imports (`use … as` aliases, globs, the calling function's
//!   own `use` first) goes where the import leads, denylisted names included (`use crate::build;
//!   build()`); an import from outside the repository leaves the name unresolved. Step 3's
//!   INFERRED edge stays as it is when the import confirms it.
//! - `a::b::f(..)` resolves `a::b` through the module tree, crates included (`grep::cli::
//!   resolve_binary` through the `grep` facade's `pub extern crate grep_cli as cli`), then `f` in
//!   the value namespace of that module: a function (`calls`) or tuple struct (`references`,
//!   constructor).
//! - `T::f(..)` resolves `T` to a declared struct or enum (through `type T = …;` aliases of a
//!   path), then takes the unique method `f` over
//!   every impl of it in the repository (an impl's owner node stands for the type its path
//!   resolves to), else its variant `f` (constructor). A trait (`Trait::f(x)`) dispatches on the
//!   type of `x` and resolves nothing.
//! - `self.a.b.m(..)` follows the declared types of the fields (`struct S { a: crate::A }`,
//!   through `&`, `Box`, `Arc` and `Rc`, generic parameters never) from the impl's type to the
//!   unique method `m` of the last one, unless a wrapper answers `m` itself (`clone`, `fmt`…).
//!   Receivers other than `self` field chains need local type inference and are left alone.

mod modules;

use std::collections::{HashMap, HashSet};

use self::modules::{Modules, Res};
use super::{Index, Resolution, View};
use crate::extract::rust::{FieldCall, PathCall, RsAlias, RsField, RsUse};
use crate::extract::{external_node, origin};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc, NodeKind};
use crate::lang::Family;

const INFERRED_SCORE: f64 = 0.85;

/// One definition a name can resolve to.
#[derive(Clone, Copy)]
struct Target<'a> {
    id: &'a str,
    /// A function or method (`calls`) rather than a type, variant or const (`constructor`).
    is_call: bool,
    /// Path of the defining file.
    path: &'a str,
}

/// Distinct ids per name, in first-seen order.
#[derive(Default)]
struct Candidates<'a>(HashMap<&'a str, Vec<Target<'a>>>);

impl<'a> Candidates<'a> {
    fn add(&mut self, key: &'a str, t: Target<'a>) {
        let v = self.0.entry(key).or_default();
        if !v.iter().any(|x| x.id == t.id) {
            v.push(t);
        }
    }

    fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    fn unique(&self, key: &str) -> Option<Target<'a>> {
        match self.0.get(key) {
            Some(v) if v.len() == 1 => Some(v[0]),
            _ => None,
        }
    }

    /// The single candidate nearest to `from`, if one is strictly nearer than the rest.
    fn closest(&self, key: &str, from: &str) -> Option<Target<'a>> {
        let v = self.0.get(key)?;
        if v.len() == 1 {
            return Some(v[0]);
        }
        let mut best: Option<((usize, bool, bool), Target<'a>)> = None;
        let mut tied = false;
        for t in v {
            let score = proximity(from, t.path);
            match &best {
                Some((s, _)) if score < *s => {}
                Some((s, _)) if score == *s => tied = true,
                _ => {
                    best = Some((score, *t));
                    tied = false;
                }
            }
        }
        if tied {
            None
        } else {
            best.map(|(_, t)| t)
        }
    }
}

/// `Pair/1` (an owner's `_rust_impl_key`) → `Pair`; a plain owner label is already the name.
fn type_name(key: &str) -> &str {
    key.split_once('/').map_or(key, |(name, _)| name)
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// (shared leading directory components, same directory, same file).
fn proximity(from: &str, to: &str) -> (usize, bool, bool) {
    let (a, b) = (dir_of(from), dir_of(to));
    let shared = a
        .split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y && !x.is_empty())
        .count();
    (shared, a == b, from == to)
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut out = Resolution::default();
    let rust: Vec<&View> = files
        .iter()
        .filter(|v| v.family() == Family::Rust)
        .collect();
    if rust.is_empty() {
        return out;
    }

    // Definitions by name, first occurrence of each id only (the build keeps the first node).
    // Stubs prefer declared types (struct/enum/trait nodes) over any other definition.
    let mut type_targets = Candidates::default();
    let mut stub_targets = Candidates::default();
    let mut call_targets = Candidates::default();
    let mut seen: HashSet<&str> = HashSet::new();
    for v in &rust {
        let path = v.file.rel.as_str();
        for n in &v.facts.nodes {
            if n.kind != NodeKind::Def || !seen.insert(n.id.as_str()) {
                continue;
            }
            let label = n.label.as_str();
            let t = |is_call| Target {
                id: n.id.as_str(),
                is_call,
                path,
            };
            if n.attr("_rust_declaration_count").is_some() {
                type_targets.add(label.strip_suffix("()").unwrap_or(label), t(false));
            }
            if let Some(name) = label.strip_prefix('.') {
                if let Some(name) = name.strip_suffix("()") {
                    call_targets.add(name, t(true));
                }
            } else if let Some(name) = label.strip_suffix("()") {
                stub_targets.add(name, t(false));
                call_targets.add(name, t(true));
            } else if let Some(name) = label.strip_suffix('!') {
                stub_targets.add(name, t(false));
            } else {
                stub_targets.add(label, t(false));
                call_targets.add(label, t(false));
            }
        }
    }

    let modules = Modules::new(files);
    // Per Rust file (module order), definitions by bare label: declared types first.
    let defs: Vec<HashMap<&str, &str>> = modules
        .views
        .iter()
        .map(|v| {
            let mut m: HashMap<&str, &str> = HashMap::new();
            for pass in [true, false] {
                for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
                    if (n.attr("_rust_declaration_count").is_some()) != pass {
                        continue;
                    }
                    let label = n.label.as_str();
                    if label.starts_with('.') {
                        continue;
                    }
                    let name = label
                        .strip_suffix("()")
                        .or_else(|| label.strip_suffix('!'))
                        .unwrap_or(label);
                    m.entry(name).or_insert(n.id.as_str());
                }
            }
            m
        })
        .collect();

    // What each `use` leaf names, by the id of the stub its Graphify edge targets
    // (`slug(last segment)`): a stub redirected through one file's import drags every other
    // file's leaf edge with that id along (§11.2 rule 4).
    let mut leaf_targets: HashMap<String, Vec<(usize, Option<&str>)>> = HashMap::new();
    for fi in 0..modules.views.len() {
        // Only leaves Graphify emits an edge for: not in function bodies, not aliased.
        for u in modules.uses(fi).iter().filter(|u| !u.in_fn) {
            let Some(last) = u.path.last() else { continue };
            if u.bound.as_deref().is_some_and(|b| b != last) {
                continue;
            }
            // A glob over an enum (`use self::Kind::*`) names the enum like a plain leaf.
            let target = match modules.resolve_use(fi, u) {
                Some(Res::Item(t, name)) => defs[t].get(name).copied(),
                _ => None,
            };
            leaf_targets
                .entry(crate::extract::slug(last))
                .or_default()
                .push((fi, target));
        }
    }

    let mut externals: HashSet<&str> = HashSet::new();
    let mut redirected: HashSet<&str> = HashSet::new();
    for (fi, v) in rust.iter().enumerate() {
        let LangFacts::Rust(facts) = &v.facts.lang else {
            continue;
        };
        for id in &facts.externals {
            if externals.insert(id) {
                out.nodes.push(external_node(id.clone()));
            }
        }
        for n in &v.facts.nodes {
            if n.kind != NodeKind::Stub || redirected.contains(n.id.as_str()) {
                continue;
            }
            // Only a stub that is the node the build keeps for this id may be replaced.
            if index.get(files, &n.id).map(|k| k.kind) != Some(NodeKind::Stub) {
                continue;
            }
            let target = match imported_name(&modules, fi, &n.label, &defs) {
                // Another file's leaf with this id would follow the redirect somewhere else.
                Some(Some(t))
                    if leaf_targets
                        .get(&n.id)
                        .is_some_and(|v| v.iter().any(|&(f, x)| f != fi && x != Some(t))) =>
                {
                    None
                }
                Some(found) => found,
                None if type_targets.has(&n.label) => type_targets.unique(&n.label).map(|t| t.id),
                None => stub_targets.unique(&n.label).map(|t| t.id),
            };
            if let Some(t) = target.filter(|t| *t != n.id) {
                redirected.insert(&n.id);
                out.redirects.push((n.id.clone(), t.to_string()));
            }
        }
    }

    // Methods by (owner type key, name): the key is the owner's `_rust_impl_key`, else its label.
    let mut type_methods: HashMap<(&str, &str), Vec<&str>> = HashMap::new();
    for v in &rust {
        for e in v.facts.edges.iter().filter(|e| e.relation == "method") {
            let (Some(owner), Some(method)) =
                (index.get(files, &e.source), index.get(files, &e.target))
            else {
                continue;
            };
            let Some(name) = method
                .label
                .strip_prefix('.')
                .and_then(|l| l.strip_suffix("()"))
            else {
                continue;
            };
            let key = match owner.attr("_rust_impl_key") {
                Some(Attr::Str(k)) => k.as_str(),
                _ => owner.label.as_str(),
            };
            let ids = type_methods.entry((type_name(key), name)).or_default();
            if !ids.contains(&method.id.as_str()) {
                ids.push(&method.id);
            }
        }
    }

    // File → file imports through the module tree.
    for (fi, v) in modules.views.iter().enumerate() {
        let Some(source) = index.file_ids.get(&v.file.rel) else {
            continue;
        };
        let mut seen: HashSet<usize> = HashSet::new();
        for u in modules.uses(fi) {
            let Some(t) = modules.resolve_use(fi, u).and_then(Res::file) else {
                continue;
            };
            if t == fi || !seen.insert(t) {
                continue;
            }
            let Some(target) = index.file_ids.get(&modules.views[t].file.rel) else {
                continue;
            };
            out.edges.push(Edge {
                source: source.clone(),
                target: target.clone(),
                relation: "imports_from".into(),
                confidence: Confidence::Extracted,
                score: Some(1.0),
                context: Some("import".into()),
                source_file: v.file.rel.clone(),
                loc: Loc::Line(u.line),
                weight: Some(1.0),
                attrs: vec![origin()],
            });
        }
    }

    let types = Types::new(files, index, &modules);
    for (fi, v) in rust.iter().enumerate() {
        let LangFacts::Rust(facts) = &v.facts.lang else {
            continue;
        };
        let mut done: HashSet<(&str, &str)> = HashSet::new();
        for call in &facts.self_calls {
            // The impl's type must name exactly one declared struct/enum/trait in the repo.
            let ty = type_name(&call.owner_key);
            if !matches!(type_targets.0.get(ty).map(Vec::len), Some(1)) {
                continue;
            }
            let Some([target]) = type_methods
                .get(&(ty, call.name.as_str()))
                .map(Vec::as_slice)
            else {
                continue;
            };
            let caller = v.id(&call.caller);
            if caller == *target || !done.insert((caller, target)) {
                continue;
            }
            out.edges.push(Edge {
                source: caller.to_string(),
                target: target.to_string(),
                relation: "calls".into(),
                confidence: Confidence::Extracted,
                score: Some(1.0),
                context: Some("call".into()),
                source_file: v.file.rel.clone(),
                loc: Loc::Line(call.line),
                weight: Some(1.0),
                attrs: vec![origin()],
            });
        }
        for call in &facts.calls {
            // A name the file imports means what the import names (nothing when it leads outside
            // the repository); only names no import explains fall back to the closest definition.
            let fallback = || {
                (!call.common)
                    .then(|| call_targets.closest(&call.name, &v.file.rel))
                    .flatten()
            };
            let uses = modules.uses_in(fi, &call.caller);
            let (target, is_call, inferred) = match imported(&modules, fi, &call.name, &uses, true)
            {
                Some(Some((t, item))) => match types.callable(t, item) {
                    Some((id, is_call)) => match fallback() {
                        // The edge Graphify infers, confirmed: keep its shape.
                        Some(t) if t.id == id => (id, t.is_call, true),
                        _ if call.kept => (id, is_call, false),
                        _ => continue,
                    },
                    None => continue,
                },
                Some(None) => continue,
                None => match fallback() {
                    Some(t) => (t.id, t.is_call, true),
                    None => continue,
                },
            };
            let caller = v.id(&call.caller);
            if caller == target || !done.insert((caller, target)) {
                continue;
            }
            out.edges
                .push(call_edge(v, caller, target, is_call, inferred, call.line));
        }
        // Path-qualified calls through the module tree, and `self.field.m()` through the fields'
        // declared types (dijkstral-native): only where in-file resolution chose another target
        // or none.
        let path_targets = facts.path_calls.iter().filter_map(|c| {
            let (t, is_call) = types.path_call(&modules, fi, c)?;
            Some((c.caller.as_str(), c.in_file.as_deref(), t, is_call, c.line))
        });
        let field_targets = facts.field_calls.iter().filter_map(|c| {
            let t = types.field_call(&modules, fi, c)?;
            Some((c.caller.as_str(), c.in_file.as_deref(), t, true, c.line))
        });
        let mut resolved: Vec<_> = path_targets.chain(field_targets).collect();
        resolved.sort_by_key(|r| r.4);
        for (caller, in_file, target, is_call, line) in resolved {
            let caller = v.id(caller);
            if in_file.is_some_and(|t| v.id(t) == target)
                || caller == target
                || !done.insert((caller, target))
            {
                continue;
            }
            out.edges
                .push(call_edge(v, caller, target, is_call, false, line));
        }
    }
    out
}

/// A cross-file `calls` (or `references` constructor) edge: INFERRED 0.85 for the bare-name
/// fallback, EXTRACTED 1.0 when the module tree or a declared type decided it.
fn call_edge(
    v: &View,
    caller: &str,
    target: &str,
    is_call: bool,
    inferred: bool,
    line: u32,
) -> Edge {
    let (relation, context) = if is_call {
        ("calls", "call")
    } else {
        ("references", "constructor")
    };
    let (confidence, score) = if inferred {
        (Confidence::Inferred, INFERRED_SCORE)
    } else {
        (Confidence::Extracted, 1.0)
    };
    Edge {
        source: caller.to_string(),
        target: target.to_string(),
        relation: relation.into(),
        confidence,
        score: Some(score),
        context: Some(context.into()),
        source_file: v.file.rel.clone(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

/// The definition a name used in Rust file `f` refers to through the file's own imports: an
/// explicit `use` binding the name, else a glob import that brings it in. `Some(None)` when the
/// import leads outside the repository or to something that is not a node; `None` when no
/// import explains the name.
fn imported_name<'d>(
    modules: &Modules,
    f: usize,
    name: &str,
    defs: &[HashMap<&str, &'d str>],
) -> Option<Option<&'d str>> {
    let uses: Vec<&RsUse> = modules.uses(f).iter().collect();
    Some(imported(modules, f, name, &uses, false)?.and_then(|(t, n)| defs[t].get(n).copied()))
}

/// The item a name used in Rust file `f` names through the imports `uses` (with `in_body`, the
/// calling function's own `use`s come first): (defining file, its name there). `Some(None)` when
/// the import leads outside the repository or to a module; `None` when no import explains the
/// name.
fn imported<'v>(
    modules: &Modules<'v, '_>,
    f: usize,
    name: &str,
    uses: &[&'v RsUse],
    in_body: bool,
) -> Option<Option<(usize, &'v str)>> {
    let binds = |u: &&&RsUse| !u.glob && u.bound.as_deref() == Some(name);
    let own = in_body
        .then(|| uses.iter().find(|u| u.in_fn && binds(u)))
        .flatten();
    let explicit = own.or_else(|| uses.iter().filter(binds).min_by_key(|u| u.depth));
    if let Some(u) = explicit {
        return match modules.resolve_use(f, u)? {
            Res::Item(t, n) => Some(Some((t, n))),
            Res::File(_) | Res::External => Some(None),
        };
    }
    for u in uses.iter().filter(|u| u.glob) {
        if let Some(Res::File(g)) = modules.resolve_use(f, u) {
            if g == f {
                continue;
            }
            match modules.lookup(g, name, false) {
                Some(Res::Item(t, n)) => return Some(Some((t, n))),
                Some(Res::File(_) | Res::External) => return Some(None),
                None => {}
            }
        }
    }
    None
}

/// Methods a field's wrapper (`Box`, `Arc`, `Rc`) or its std trait impls answer before the
/// wrapped type's own (`self.inner.clone()` clones the `Arc`).
const WRAPPER_METHODS: &[&str] = &[
    "as_mut",
    "as_ptr",
    "as_ref",
    "borrow",
    "borrow_mut",
    "call",
    "clone",
    "clone_from",
    "cmp",
    "deref",
    "deref_mut",
    "downcast",
    "drop",
    "eq",
    "flush",
    "fmt",
    "ge",
    "get_mut",
    "gt",
    "hash",
    "into",
    "le",
    "lt",
    "ne",
    "next",
    "partial_cmp",
    "poll",
    "read",
    "to_owned",
    "to_string",
    "write",
];

/// Declared types, their methods, variants and fields, in the repository's node space, for
/// path-qualified and field-typed calls (dijkstral-native).
struct Types<'v> {
    /// Per Rust file (module order): free functions by name (`name()` nodes).
    fns: Vec<HashMap<&'v str, &'v str>>,
    /// Per Rust file: declared struct/enum/trait nodes by name.
    types: Vec<HashMap<&'v str, &'v str>>,
    /// (type id, method name) → distinct method ids, over every impl of the type.
    methods: HashMap<(&'v str, &'v str), Vec<&'v str>>,
    /// (enum id, variant label) → variant id.
    variants: HashMap<(&'v str, &'v str), &'v str>,
    /// (struct id, field name) → (declaring file, field), every declaration.
    fields: HashMap<(&'v str, &'v str), Vec<(usize, &'v RsField)>>,
    /// Trait ids: `Trait::m(x)` dispatches on `x`, whose type is unknown here.
    traits: HashSet<&'v str>,
    /// (file, alias name) → `type` alias of a path.
    aliases: HashMap<(usize, &'v str), &'v RsAlias>,
}

impl<'v> Types<'v> {
    fn new(files: &'v [View], index: &Index, modules: &Modules<'v, '_>) -> Self {
        let n = modules.views.len();
        let mut fns: Vec<HashMap<&str, &str>> = vec![HashMap::new(); n];
        let mut types: Vec<HashMap<&str, &str>> = vec![HashMap::new(); n];
        for (fi, v) in modules.views.iter().enumerate() {
            for node in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
                // Only the node the build keeps for an id is a target.
                if !index
                    .get(files, &node.id)
                    .is_some_and(|k| std::ptr::eq(k, node))
                {
                    continue;
                }
                let label = node.label.as_str();
                if node.attr("_rust_declaration_count").is_some() {
                    types[fi].entry(label).or_insert(node.id.as_str());
                } else if let Some(name) = label.strip_suffix("()") {
                    if !name.starts_with('.') {
                        fns[fi].entry(name).or_insert(node.id.as_str());
                    }
                }
            }
        }
        // The type an impl owner node stands for: itself when declared, else its impl's path.
        let mut owners: HashMap<&str, &str> = HashMap::new();
        for (fi, v) in modules.views.iter().enumerate() {
            let LangFacts::Rust(facts) = &v.facts.lang else {
                continue;
            };
            for imp in &facts.impls {
                if let Some(Res::Item(t, name)) = modules.resolve_qualifier(fi, imp.depth, &imp.ty)
                {
                    if let Some(&ty) = types[t].get(name) {
                        owners.entry(v.id(&imp.owner)).or_insert(ty);
                    }
                }
            }
        }
        let mut methods: HashMap<(&str, &str), Vec<&str>> = HashMap::new();
        let mut variants: HashMap<(&str, &str), &str> = HashMap::new();
        for v in &modules.views {
            for e in &v.facts.edges {
                let (Some(owner), Some(member)) =
                    (index.get(files, &e.source), index.get(files, &e.target))
                else {
                    continue;
                };
                match e.relation.as_str() {
                    "method" => {
                        let Some(name) = member
                            .label
                            .strip_prefix('.')
                            .and_then(|l| l.strip_suffix("()"))
                        else {
                            continue;
                        };
                        let ty = if owner.attr("_rust_declaration_count").is_some() {
                            owner.id.as_str()
                        } else {
                            match owners.get(owner.id.as_str()) {
                                Some(t) => t,
                                None => continue,
                            }
                        };
                        let ids = methods.entry((ty, name)).or_default();
                        if !ids.contains(&member.id.as_str()) {
                            ids.push(&member.id);
                        }
                    }
                    "case_of" => {
                        variants
                            .entry((owner.id.as_str(), member.label.as_str()))
                            .or_insert(&member.id);
                    }
                    _ => {}
                }
            }
        }
        let mut fields: HashMap<(&str, &str), Vec<(usize, &RsField)>> = HashMap::new();
        let mut traits = HashSet::new();
        let mut aliases = HashMap::new();
        for (fi, v) in modules.views.iter().enumerate() {
            let LangFacts::Rust(facts) = &v.facts.lang else {
                continue;
            };
            traits.extend(
                facts
                    .traits
                    .iter()
                    .filter_map(|t| types[fi].get(t.as_str())),
            );
            for a in &facts.aliases {
                aliases.entry((fi, a.name.as_str())).or_insert(a);
            }
            for f in &facts.fields {
                if let Some(&owner) = types[fi].get(f.owner.as_str()) {
                    fields
                        .entry((owner, f.name.as_str()))
                        .or_default()
                        .push((fi, f));
                }
            }
        }
        Types {
            fns,
            types,
            methods,
            variants,
            fields,
            traits,
            aliases,
        }
    }

    /// The declared type a resolved path names, through `type` aliases (a few hops deep).
    fn type_of(&self, modules: &Modules<'v, '_>, r: Option<Res<'_>>) -> Option<&'v str> {
        let (mut t, name) = match r? {
            Res::Item(t, name) => (t, name),
            _ => return None,
        };
        if let Some(&id) = self.types[t].get(name) {
            return Some(id);
        }
        let mut alias = *self.aliases.get(&(t, name))?;
        for _ in 0..4 {
            match modules.resolve_qualifier(t, alias.depth, &alias.ty)? {
                Res::Item(t2, n2) => {
                    if let Some(&id) = self.types[t2].get(n2) {
                        return Some(id);
                    }
                    t = t2;
                    alias = self.aliases.get(&(t2, n2))?;
                }
                _ => return None,
            }
        }
        None
    }

    fn method(&self, ty: &'v str, name: &str) -> Option<&'v str> {
        match self.methods.get(&(ty, name)).map(Vec::as_slice) {
            Some([m]) => Some(m),
            _ => None,
        }
    }

    /// What `name(..)` reaches as an item `item` of file `t`: a function (`calls`) or a tuple
    /// struct (`references`, constructor).
    fn callable(&self, t: usize, item: &str) -> Option<(&'v str, bool)> {
        if let Some(&id) = self.fns[t].get(item) {
            return Some((id, true));
        }
        self.types[t].get(item).map(|&id| (id, false))
    }

    /// The target of `qual::name(..)` written in file `f`: a function of the module `qual`
    /// names, a method or variant of the type it names.
    fn path_call(
        &self,
        modules: &Modules<'v, '_>,
        f: usize,
        c: &'v PathCall,
    ) -> Option<(&'v str, bool)> {
        // A first segment bound by a `use` in the calling function's body, else by one in the
        // inline module around the call (`mod tests { use … }`; sibling modules must agree).
        let first = c.qual.first().map(String::as_str);
        let binds = |u: &&RsUse| !u.glob && u.bound.as_deref() == first;
        let uses = modules.uses(f);
        let mut local = uses
            .iter()
            .filter(binds)
            .find(|u| u.func.as_deref() == Some(c.caller.as_str()));
        if local.is_none() && c.depth > 0 {
            let mut inner = uses
                .iter()
                .filter(binds)
                .filter(|u| !u.in_fn && u.depth == c.depth);
            local = inner.next();
            if local.is_some_and(|u| inner.any(|o| o.path != u.path)) {
                return None;
            }
        }
        match local {
            Some(u) => {
                let path: Vec<String> = u.path.iter().chain(&c.qual[1..]).cloned().collect();
                self.below(
                    modules,
                    modules.resolve_qualifier(f, u.depth, &path)?,
                    &c.name,
                )
            }
            None => self.below(
                modules,
                modules.resolve_qualifier(f, c.depth, &c.qual)?,
                &c.name,
            ),
        }
    }

    /// `name` below what a qualifier resolved to: a function or tuple struct of a module, a
    /// method or variant of a type.
    fn below(
        &self,
        modules: &Modules<'v, '_>,
        qual: Res<'_>,
        name: &str,
    ) -> Option<(&'v str, bool)> {
        match qual {
            Res::File(m) => match modules.lookup_value(m, name)? {
                Res::Item(t, item) => self.callable(t, item),
                _ => None,
            },
            r @ Res::Item(..) => {
                let ty = self.type_of(modules, Some(r))?;
                if self.traits.contains(ty) {
                    return None;
                }
                if let Some(m) = self.method(ty, name) {
                    return Some((m, true));
                }
                self.variants.get(&(ty, name)).map(|&id| (id, false))
            }
            Res::External => None,
        }
    }

    /// The method `self.<fields>.name(..)` calls, following each field's declared type.
    fn field_call(&self, modules: &Modules<'v, '_>, f: usize, c: &'v FieldCall) -> Option<&'v str> {
        let mut ty = self.type_of(
            modules,
            modules.resolve_qualifier(f, c.depth, std::slice::from_ref(&c.self_ty)),
        )?;
        let mut wrapped = false;
        for field in &c.fields {
            let decls = self.fields.get(&(ty, field.as_str()))?;
            let (g, first) = decls[0];
            if decls[1..]
                .iter()
                .any(|(_, d)| d.ty != first.ty || d.wrapped != first.wrapped)
            {
                return None;
            }
            ty = self.type_of(
                modules,
                modules.resolve_qualifier(g, first.depth, &first.ty),
            )?;
            wrapped = first.wrapped;
        }
        if wrapped && WRAPPER_METHODS.contains(&c.name.as_str()) {
            return None;
        }
        self.method(ty, &c.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t<'a>(id: &'a str, path: &'a str) -> Target<'a> {
        Target {
            id,
            is_call: true,
            path,
        }
    }

    #[test]
    fn closest_candidate_wins_and_ties_resolve_nothing() {
        let mut c = Candidates::default();
        // Same directory beats a deeper directory with the same shared prefix.
        c.add("p2", t("deep", "src/sub/deep/g.rs"));
        c.add("p2", t("near", "src/h.rs"));
        assert_eq!(c.closest("p2", "src/a.rs").map(|t| t.id), Some("near"));
        // A longer shared prefix beats a shorter one.
        c.add("q1", t("x", "src/a/x.rs"));
        c.add("q1", t("e", "src/a/b/d/e.rs"));
        assert_eq!(c.closest("q1", "src/a/b/c.rs").map(|t| t.id), Some("e"));
        // Two sibling directories tie; nothing shared ties too.
        c.add("m", t("s1", "src/sub/e.rs"));
        c.add("m", t("s2", "src/sub2/f.rs"));
        assert!(c.closest("m", "src/a.rs").is_none());
        c.add("f", t("f1", "x/y/c.rs"));
        c.add("f", t("f2", "x/d.rs"));
        assert!(c.closest("f", "src/a.rs").is_none());
        // The same file beats the same directory.
        c.add("s", t("same", "src/a.rs"));
        c.add("s", t("dir", "src/j.rs"));
        assert_eq!(c.closest("s", "src/a.rs").map(|t| t.id), Some("same"));
    }

    /// Extract and resolve a small Rust repository; returns `source -relation-> target [conf]`.
    fn build(files: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
        use crate::discover::SourceFile;
        use crate::extract::extract_file;
        use crate::lang::Lang;
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|(rel, _)| SourceFile {
                rel: rel.to_string(),
                abs: rel.into(),
                lang: Lang::Rust,
            })
            .collect();
        let input = sources
            .iter()
            .zip(files)
            .map(|(sf, (rel, src))| (sf, extract_file(Lang::Rust, rel, src.as_bytes())))
            .collect();
        let g = crate::resolve::resolve(input);
        let nodes = g.nodes.iter().map(|n| n.id.clone()).collect();
        let edges = g
            .edges
            .iter()
            .map(|e| {
                format!(
                    "{} -{}-> {} [{}]",
                    e.source,
                    e.relation,
                    e.target,
                    e.confidence.as_str()
                )
            })
            .collect();
        (nodes, edges)
    }

    #[test]
    fn stubs_imports_and_calls_resolve_across_files() {
        let (nodes, edges) = build(&[
            (
                "src/a.rs",
                "use crate::b::Thing;\nfn f(t: Thing, r: Round) -> Shared { helper(); Meters(1); }\n\
                 impl Atlas { fn rank(&self) { self.files(); } }\n",
            ),
            (
                "src/b.rs",
                "pub struct Thing;\npub enum E { Round }\npub struct Meters(u8);\n\
                 pub fn helper() {}\npub struct Atlas;\nimpl Atlas { fn files(&self) {} }\n",
            ),
            // `Shared` is declared twice: its stub stays. `Round` the struct beats `E::Round`.
            ("src/c.rs", "pub struct Shared;\npub struct Round;\n"),
            ("src/d/e.rs", "pub struct Shared;\npub fn helper() {}\n"),
        ]);
        let has = |e: &str| edges.iter().any(|x| x == e);
        // The import of a name used as a type in that file follows the resolved stub.
        assert!(has("src_a -imports_from-> src_b_thing [EXTRACTED]"));
        assert!(has("src_a_f -references-> src_b_thing [EXTRACTED]"));
        assert!(has("src_a_f -references-> src_c_round [EXTRACTED]"));
        assert!(has("src_a_f -references-> shared [EXTRACTED]"));
        assert!(nodes.iter().any(|n| n == "shared"));
        assert!(!nodes.iter().any(|n| n == "thing"));
        // `helper` exists in src/b.rs and src/d/e.rs: the same directory wins.
        assert!(has("src_a_f -calls-> src_b_helper [INFERRED]"));
        assert!(has("src_a_f -references-> src_b_meters [INFERRED]"));
        assert!(has(
            "src_a_atlas_rank -calls-> src_b_atlas_files [EXTRACTED]"
        ));
    }

    #[test]
    fn scoped_imports_field_types_and_kept_callers() {
        let (_, edges) = build(&[
            ("Cargo.toml", "[package]\nname = \"p\"\n"),
            ("src/lib.rs", "mod a;\nmod b;\nmod c;\n"),
            (
                "src/b.rs",
                "pub struct Comma;\nimpl Comma { pub fn new() -> Comma { Comma } }\n\
                 pub struct Colon;\nimpl Colon { pub fn new() -> Colon { Colon } }\n\
                 pub struct Inner;\nimpl Inner {\n    pub fn run(&self) {}\n    \
                 pub fn clone(&self) -> Inner { Inner }\n}\npub fn helper() {}\n\
                 pub mod ansi {\n    pub fn render() {}\n}\npub type Sep2 = Comma;\n",
            ),
            (
                "src/c.rs",
                // An inline module is flattened into its file; a `type` alias names its type.
                "use crate::b::ansi;\npub fn paint() { ansi::render(); crate::b::Sep2::new(); }\n",
            ),
            (
                "src/a.rs",
                // A `use` in a function body binds only there.
                "fn comma() { use crate::b::Comma as Sep; Sep::new(); }\n\
                 fn colon() { use crate::b::Colon as Sep; Sep::new(); }\n\
                 pub struct Holder(Box<crate::b::Inner>);\n\
                 impl Holder {\n    fn go(&self) { self.0.run(); self.0.clone(); }\n}\n\
                 // `fn welcome` shares the struct's id: its calls would start from the struct.\n\
                 pub struct Welcome;\nfn welcome() { crate::b::helper(); }\n\
                 #[cfg(test)]\nmod tests {\n    use crate::b::Comma;\n    use ext::helper;\n    \
                 fn t() { Comma::new(); helper(); }\n}\n",
            ),
        ]);
        let has = |e: &str| edges.iter().any(|x| x == e);
        assert!(
            has("src_a_comma -calls-> src_b_comma_new [EXTRACTED]"),
            "{edges:#?}"
        );
        assert!(
            has("src_a_colon -calls-> src_b_colon_new [EXTRACTED]"),
            "{edges:#?}"
        );
        assert!(
            has("src_a_t -calls-> src_b_comma_new [EXTRACTED]"),
            "{edges:#?}"
        );
        assert!(
            has("src_a_holder_go -calls-> src_b_inner_run [EXTRACTED]"),
            "{edges:#?}"
        );
        assert!(
            has("src_c_paint -calls-> src_b_render [EXTRACTED]"),
            "{edges:#?}"
        );
        assert!(
            has("src_c_paint -calls-> src_b_comma_new [EXTRACTED]"),
            "{edges:#?}"
        );
        for unwanted in [
            // `Box<Inner>::clone` clones the box.
            "src_a_holder_go -calls-> src_b_inner_clone",
            // `helper` comes from outside the repository: no fallback to a same-named fn.
            "src_a_t -calls-> src_b_helper",
            "src_a_welcome -calls->",
        ] {
            assert!(!edges.iter().any(|x| x.starts_with(unwanted)), "{edges:#?}");
        }
    }
}

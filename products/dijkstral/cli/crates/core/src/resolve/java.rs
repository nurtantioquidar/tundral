//! Cross-file pass for Java (§13). See `docs/graphify-format.md`.
//!
//! - Virtual targets written by the extractor (`crate::extract::java`) are redirected: a
//!   supertype not imported by name binds to the local class it names (same package, then the
//!   unique class of that name) or is dropped; an explicitly imported name binds to the local
//!   class with that FQN or to the shared stub `slug(fqn)`; a single-type import binds to the
//!   local class, else that stub or the external import node.
//! - Per-file type and annotation stubs (`other`, `override`) bind to a local class when one is
//!   found the same way, else stay.
//! - `import static p.C.m` also links the file to class `p.C` (EXTRACTED 1.0, no context).
//! - Calls: `Type.m()` on the unique class `Type`, `this.m()`, `super.m()` (inherited methods
//!   included) and `new X()` through the import of a type the file uses are EXTRACTED 1.0. A
//!   bare call the file could not bind goes to its own anonymous class (EXTRACTED 1.0), an
//!   inherited method or the only method with a body of that name (INFERRED 0.85). `x.m()` binds
//!   INFERRED 0.85 through `x`'s declared type when that names one local class, and never
//!   when the type is unknown. Unbound `new X()` binds INFERRED 0.85 to `X` when it is the only
//!   class or constructor of that name and the file does not use `X` as a type.
//! - `metadata.unresolved_calls`: `Type.m()`, or a typed `x.m()` (common JDK types excepted),
//!   whose type names no local class; one entry per (callee, type) and caller.
//!
//! Deliberate departures from Graphify (quirk Q11, see `docs/graphify-format.md` §13.4): an
//! import of a local class always targets the class (J3), a wildcard import keeps its external
//! node (J1), and a static import links only the class its FQN names (J2).

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::java::{is_common_jdk_type, JCallKind, JavaFacts, VIRTUAL};
use crate::extract::{external_node, origin, slug};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc, Node, NodeKind};
use crate::lang::Lang;

/// Redirect target for references that resolve to nothing: the edge becomes dangling and the
/// build drops it.
const DROP: &str = "@drop";

fn java_facts<'v>(v: &'v View) -> Option<&'v JavaFacts> {
    match &v.facts.lang {
        LangFacts::Java(j) => Some(j),
        _ => None,
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

/// A cross-file edge; `rel` and `line` locate the source construct.
fn edge(
    source: String,
    target: String,
    relation: &str,
    (rel, line): (&str, u32),
    context: Option<&str>,
    confidence: Confidence,
    score: Option<f64>,
) -> Edge {
    Edge {
        source,
        target,
        relation: relation.into(),
        confidence,
        score,
        context: context.map(str::to_string),
        source_file: rel.into(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

fn one<T: Clone>(v: Option<&Vec<T>>) -> Option<T> {
    match v {
        Some(v) if v.len() == 1 => v.first().cloned(),
        _ => None,
    }
}

/// Every class of the repository.
struct Classes {
    by_fqn: HashMap<String, String>,
    by_name: HashMap<String, Vec<String>>,
    /// (package, simple name) → ids.
    by_package: HashMap<(String, String), Vec<String>>,
}

impl Classes {
    /// The class a simple type name denotes from a file: its explicit import, then its
    /// package, then the only class of that name.
    fn resolve(&self, j: &JavaFacts, imports: &HashMap<&str, &str>, name: &str) -> Option<String> {
        if let Some(fqn) = imports.get(name) {
            return self.by_fqn.get(*fqn).cloned();
        }
        one(self.by_package.get(&(j.package.clone(), name.to_string())))
            .or_else(|| one(self.by_name.get(name)))
    }
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let java: Vec<(&View, &JavaFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Java)
        .filter_map(|v| java_facts(v).map(|j| (v, j)))
        .collect();
    if java.is_empty() {
        return res;
    }

    // Class and method tables (final ids).
    let mut classes = Classes {
        by_fqn: HashMap::new(),
        by_name: HashMap::new(),
        by_package: HashMap::new(),
    };
    let mut superclass: HashMap<String, (usize, String)> = HashMap::new();
    // (class id, method name) → method id; name → methods with a body (constructors included).
    let mut members: HashMap<(String, String), String> = HashMap::new();
    let mut bodies: HashMap<&str, Vec<String>> = HashMap::new();
    let mut ctors: HashSet<String> = HashSet::new();
    // Every named type (anonymous classes are not in it).
    let mut named: HashSet<String> = HashSet::new();
    for (fi, (v, j)) in java.iter().enumerate() {
        for c in &j.classes {
            let id = v.id(&c.id).to_string();
            classes
                .by_fqn
                .entry(c.fqn.clone())
                .or_insert_with(|| id.clone());
            push_unique(classes.by_name.entry(c.name.clone()).or_default(), &id);
            push_unique(
                classes
                    .by_package
                    .entry((j.package.clone(), c.name.clone()))
                    .or_default(),
                &id,
            );
            if let Some(s) = &c.superclass {
                superclass.entry(id.clone()).or_insert((fi, s.clone()));
            }
            if c.has_ctor {
                ctors.insert(id.clone());
            }
            named.insert(id.clone());
        }
        for m in &j.methods {
            let owner = v.id(&m.owner).to_string();
            let id = v.id(&m.id).to_string();
            members.entry((owner, m.name.clone())).or_insert(id.clone());
            if m.has_body && !m.anonymous {
                push_unique(bodies.entry(m.name.as_str()).or_default(), &id);
            }
        }
    }
    // Virtual targets and annotation stubs.
    let mut redirected: HashSet<String> = HashSet::new();
    let mut fqn_stubs: HashSet<String> = HashSet::new();
    let mut externals: HashSet<String> = HashSet::new();
    let by_rel: HashMap<&str, usize> = java
        .iter()
        .enumerate()
        .map(|(i, (v, _))| (v.file.rel.as_str(), i))
        .collect();
    for (v, j) in &java {
        let imports = imports_of(j);
        for e in &v.facts.edges {
            let t = e.target.as_str();
            if !t.starts_with(VIRTUAL) || !redirected.insert(t.to_string()) {
                continue;
            }
            let body = &t[VIRTUAL.len() + 2..];
            let target = match &t[VIRTUAL.len()..VIRTUAL.len() + 1] {
                // `@js:<path>|<Name>` (a fully qualified `@js:<fqn>` is accepted too)
                "s" => match body.split_once('|') {
                    Some((rel, name)) => {
                        let (_, owner) = java[by_rel[rel]];
                        classes.resolve(owner, &imports_of(owner), name)
                    }
                    None => classes.by_fqn.get(body).cloned(),
                }
                .unwrap_or_else(|| DROP.to_string()),
                // `@jf:<fqn>`
                "f" => classes.by_fqn.get(body).cloned().unwrap_or_else(|| {
                    let id = slug(body);
                    if fqn_stubs.insert(id.clone()) {
                        res.nodes.push(stub_node(id.clone(), body.to_string()));
                    }
                    id
                }),
                // `@ji:<fqn>|<external>`
                "i" => {
                    let (fqn, ext) = body.split_once('|').unwrap_or((body, ""));
                    match classes.by_fqn.get(fqn) {
                        Some(c) => c.clone(),
                        None if ext.is_empty() => {
                            let id = slug(fqn);
                            if fqn_stubs.insert(id.clone()) {
                                res.nodes.push(stub_node(id.clone(), fqn.to_string()));
                            }
                            id
                        }
                        None => {
                            if externals.insert(ext.to_string()) {
                                res.nodes.push(external_node(ext.to_string()));
                            }
                            ext.to_string()
                        }
                    }
                }
                _ => DROP.to_string(),
            };
            res.redirects.push((t.to_string(), target));
        }
        for (local, name) in &j.stubs {
            let stub = v.id(local);
            if index
                .get(files, stub)
                .is_some_and(|n| n.kind == NodeKind::Stub)
            {
                if let Some(c) = classes.resolve(j, &imports, name) {
                    res.redirects.push((stub.to_string(), c));
                }
            }
        }
    }

    // Static imports link their class; then calls.
    for (v, j) in &java {
        let rel = v.file.rel.as_str();
        let file_id = v.id(&j.file_id).to_string();
        let imports = imports_of(j);
        let mut linked: HashSet<String> = v
            .facts
            .edges
            .iter()
            .filter(|e| e.relation == "imports" && e.source == file_id)
            .map(|e| {
                let t = e.target.as_str();
                if t.starts_with(VIRTUAL) {
                    let fqn = t[VIRTUAL.len() + 2..].split('|').next().unwrap_or("");
                    classes.by_fqn.get(fqn).cloned().unwrap_or_default()
                } else {
                    t.to_string()
                }
            })
            .collect();
        for (class, line) in &j.static_imports {
            if let Some(c) = classes.by_fqn.get(class) {
                if linked.insert(c.clone()) {
                    res.edges.push(edge(
                        file_id.clone(),
                        c.clone(),
                        "imports",
                        (rel, *line),
                        None,
                        Confidence::Extracted,
                        Some(1.0),
                    ));
                }
            }
        }

        let mut called: HashSet<(String, String)> = v
            .facts
            .edges
            .iter()
            .filter(|e| e.relation == "calls")
            .map(|e| (e.source.clone(), e.target.clone()))
            .collect();
        let typed: HashSet<&str> = j.typed.iter().map(String::as_str).collect();
        // A class's method, or the nearest superclass's (`user.id()` → `Model.id()`).
        let find_member = |class: &str, name: &str| -> Option<String> {
            let mut cur = class.to_string();
            for _ in 0..8 {
                if let Some(m) = members.get(&(cur.clone(), name.to_string())) {
                    return Some(m.clone());
                }
                let (fi, sup) = superclass.get(&cur)?;
                let (_, sj) = java[*fi];
                cur = classes.resolve(sj, &imports_of(sj), sup)?;
            }
            None
        };
        // `metadata.unresolved_calls`, in source order: a `Type.m()` or a typed `x.m()` whose
        // type names no local class (common JDK receivers excepted for typed variables).
        let mut unresolved: Vec<(String, Vec<String>)> = Vec::new();
        let mut recorded: HashSet<(String, String, String)> = HashSet::new();
        for c in &j.calls {
            let record = match c.kind {
                JCallKind::Static => true,
                JCallKind::Member => !c.receiver.is_empty() && !is_common_jdk_type(&c.receiver),
                _ => false,
            };
            if !record || classes.by_name.contains_key(&c.receiver) {
                continue;
            }
            // One entry per (callee, receiver type) and caller, first site kept.
            let caller = v.id(&c.caller).to_string();
            if !recorded.insert((caller.clone(), c.name.clone(), c.receiver.clone())) {
                continue;
            }
            let entry = format!(
                "{{\"callee\":{},\"receiver_type\":{},\"lang\":\"java\",\"line\":\"L{}\"}}",
                json_str(&c.name),
                json_str(&c.receiver),
                c.line
            );
            match unresolved.iter_mut().find(|(id, _)| *id == caller) {
                Some((_, list)) => list.push(entry),
                None => unresolved.push((caller, vec![entry])),
            }
        }
        // Graphify's order: unbound bare calls and `new`, then `this`/`super`/`Type.`, then
        // other member calls.
        for phase in 0..3 {
            for c in &j.calls {
                let p = match c.kind {
                    JCallKind::Bare | JCallKind::New => 0,
                    JCallKind::This | JCallKind::Super | JCallKind::Static => 1,
                    JCallKind::Member => 2,
                };
                if p != phase {
                    continue;
                }
                let caller = v.id(&c.caller).to_string();
                let owner = v.id(&c.owner).to_string();
                let found: Option<(String, Confidence, f64)> = match c.kind {
                    // A typed receiver binds through its class (body or not, inherited too); a
                    // receiver of unknown type never binds.
                    JCallKind::Member => match classes.by_name.get(&c.receiver) {
                        Some(cs) if cs.len() == 1 => {
                            find_member(&cs[0], &c.name).map(|t| (t, Confidence::Inferred, 0.85))
                        }
                        _ => None,
                    },
                    // A bare call the file could not bind: its own (anonymous) class's method
                    // (EXTRACTED 1.0), an inherited one, or the only method with a body of
                    // that name (both INFERRED 0.85).
                    JCallKind::Bare => match members.get(&(owner.clone(), c.name.clone())) {
                        Some(t) => Some((t.clone(), Confidence::Extracted, 1.0)),
                        None => find_member(&owner, &c.name)
                            .or_else(|| one(bodies.get(c.name.as_str())))
                            .map(|t| (t, Confidence::Inferred, 0.85)),
                    },
                    JCallKind::This => {
                        find_member(&owner, &c.name).map(|t| (t, Confidence::Extracted, 1.0))
                    }
                    JCallKind::Super => superclass
                        .get(&owner)
                        .and_then(|(fi, name)| {
                            let (_, sj) = java[*fi];
                            classes.resolve(sj, &imports_of(sj), name)
                        })
                        .and_then(|s| members.get(&(s, c.name.clone())).cloned())
                        .map(|t| (t, Confidence::Extracted, 1.0)),
                    JCallKind::Static => match classes.by_name.get(&c.receiver) {
                        Some(cs) if cs.len() == 1 => members
                            .get(&(cs[0].clone(), c.name.clone()))
                            .map(|t| (t.clone(), Confidence::Extracted, 1.0)),
                        _ => None,
                    },
                    // `new X()` where the file uses `X` as a type binds through its import (or not
                    // at all); otherwise to the only class `X` without a constructor.
                    JCallKind::New => {
                        if typed.contains(c.name.as_str()) {
                            imports.get(c.name.as_str()).and_then(|fqn| {
                                classes
                                    .by_fqn
                                    .get(*fqn)
                                    .map(|t| (t.clone(), Confidence::Extracted, 1.0))
                            })
                        } else {
                            let cands = classes.by_name.get(&c.name).map_or(0, Vec::len)
                                + bodies.get(c.name.as_str()).map_or(0, Vec::len);
                            one(classes.by_name.get(&c.name))
                                .filter(|k| cands == 1 && !ctors.contains(k))
                                .map(|t| (t, Confidence::Inferred, 0.85))
                        }
                    }
                };
                let Some((target, confidence, score)) = found else {
                    continue;
                };
                if !called.insert((caller.clone(), target.clone())) {
                    continue;
                }
                res.edges.push(edge(
                    caller,
                    target,
                    "calls",
                    (rel, c.line),
                    Some("call"),
                    confidence,
                    Some(score),
                ));
            }
        }
        for (id, entries) in unresolved {
            let json = format!("{{\"unresolved_calls\":[{}]}}", entries.join(","));
            res.attrs.push((id, "metadata".into(), Attr::Json(json)));
        }
    }
    res
}

/// A file's single-type imports: simple name → FQN (first wins).
fn imports_of(j: &JavaFacts) -> HashMap<&str, &str> {
    let mut m = HashMap::new();
    for (simple, fqn) in &j.imports {
        m.entry(simple.as_str()).or_insert(fqn.as_str());
    }
    m
}

fn push_unique(v: &mut Vec<String>, id: &str) {
    if !v.iter().any(|x| x == id) {
        v.push(id.to_string());
    }
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

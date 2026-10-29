//! Cross-file pass for Swift (§23). See `docs/graphify-format.md`.
//!
//! - Module nodes: `import X` targets the shared node `slug(X)` (`file_type: code`,
//!   `type: module`), whose `source_file`/`source_location` come from the importing file with the
//!   smallest basename.
//! - An extension node merges into the one declaration of its name; an extension the declaring
//!   file writes itself shares the declaration's id and makes that id an extension too, so it is
//!   then no merge target.
//! - Per-file and shared (supertype) stubs bind to the one type, declaration or extension, with
//!   their name; unbound shared stubs become one node.
//! - Calls the file could not bind (bare calls of every file first, then member calls): a bare
//!   call binds INFERRED 0.85 to the definition of that name (function, member, property, case
//!   or type) nearest to the caller's file, unless a tie; calls through a name the file uses as
//!   an undeclared type are dropped. `x.m()` with `x` in the file's variable table (or produced
//!   by `Type.f()` returning a plain type) binds INFERRED 0.85 to that type's member `m`, else
//!   references the type (context `call`); `Type.m()` does the same EXTRACTED 1.0. The type must
//!   be the one class-like node of that label and a declaration or in the caller's file. A
//!   receiver type no type of the repository has is recorded once per (caller, callee, type) in
//!   the caller's `metadata.unresolved_calls`, builtin-like types excepted. A caller already
//!   linked to the target gets no new edge.
//! - `dispatches_to`: a protocol links each requirement to every conforming type's member of the
//!   same name (context `call`, at the conforming type's declaration), INFERRED 0.85 with one
//!   conforming type, 0.75 with two, 0.65 with more.

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::origin;
use crate::extract::slug;
use crate::extract::swift::{is_denied, SwiftFacts};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc, Node, NodeKind};
use crate::lang::Lang;

fn swift_facts<'v>(v: &'v View) -> Option<&'v SwiftFacts> {
    match &v.facts.lang {
        LangFacts::Swift(s) => Some(s),
        _ => None,
    }
}

fn edge(
    source: String,
    target: String,
    relation: &str,
    rel: &str,
    line: u32,
    confidence: Confidence,
    score: f64,
) -> Edge {
    Edge {
        source,
        target,
        relation: relation.into(),
        confidence,
        score: Some(score),
        context: Some("call".into()),
        source_file: rel.into(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

fn push_unique(v: &mut Vec<String>, id: &str) {
    if !v.iter().any(|x| x == id) {
        v.push(id.to_string());
    }
}

/// `.name()`, `name()`, `.name` → `name`.
fn member_name(label: &str) -> &str {
    let l = label.strip_prefix('.').unwrap_or(label);
    l.strip_suffix("()").unwrap_or(l)
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let swift: Vec<(&View, &SwiftFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Swift)
        .filter_map(|v| swift_facts(v).map(|s| (v, s)))
        .collect();
    if swift.is_empty() {
        return res;
    }

    // Module nodes: the importer with the smallest basename provides the location.
    let mut modules: HashMap<String, (String, String, &str, u32)> = HashMap::new();
    let mut module_order: Vec<String> = Vec::new();
    for (v, s) in &swift {
        let rel = v.file.rel.as_str();
        let base = rel.rsplit('/').next().unwrap_or(rel).to_string();
        for (path, line) in &s.imports {
            let id = slug(path);
            match modules.get_mut(&id) {
                Some(cur) => {
                    if (base.as_str(), rel) < (cur.1.as_str(), cur.2) {
                        *cur = (path.clone(), base.clone(), rel, *line);
                    }
                }
                None => {
                    module_order.push(id.clone());
                    modules.insert(id, (path.clone(), base.clone(), rel, *line));
                }
            }
        }
    }
    for id in module_order {
        if index.contains(&id) {
            continue;
        }
        let (label, _, rel, line) = &modules[&id];
        res.nodes.push(Node {
            id,
            label: label.clone(),
            kind: NodeKind::Def,
            file_type: "code".into(),
            source_file: (*rel).into(),
            loc: Loc::Line(*line),
            attrs: vec![("type".into(), Attr::Str("module".into())), origin()],
        });
    }

    // Types: declarations and extensions, by label.
    let mut ext_ids: HashSet<String> = HashSet::new();
    for (v, s) in &swift {
        for e in &s.extensions {
            ext_ids.insert(v.id(e).to_string());
        }
    }
    let mut decl_by_label: HashMap<&str, Vec<String>> = HashMap::new();
    let mut all_by_label: HashMap<&str, Vec<String>> = HashMap::new();
    let mut label_of: HashMap<&str, &str> = HashMap::new();
    let mut file_of: HashMap<&str, &str> = HashMap::new();
    for (v, _) in &swift {
        for n in &v.facts.nodes {
            if n.kind == NodeKind::Def || n.kind == NodeKind::File {
                file_of.entry(n.id.as_str()).or_insert(v.file.rel.as_str());
            }
            label_of.entry(n.id.as_str()).or_insert(n.label.as_str());
            if n.kind != NodeKind::Def || n.attr("_callable_class").is_none() {
                continue;
            }
            push_unique(all_by_label.entry(n.label.as_str()).or_default(), &n.id);
            if !ext_ids.contains(&n.id) {
                push_unique(decl_by_label.entry(n.label.as_str()).or_default(), &n.id);
            }
        }
    }
    let mut canon: HashMap<String, String> = HashMap::new();
    let mut merged: HashSet<String> = HashSet::new();
    for (v, s) in &swift {
        for e in &s.extensions {
            let id = v.id(e).to_string();
            if !merged.insert(id.clone()) {
                continue;
            }
            let Some(label) = label_of.get(id.as_str()) else {
                continue;
            };
            if let Some(d) = decl_by_label.get(label) {
                if d.len() == 1 && d[0] != id {
                    canon.insert(id.clone(), d[0].clone());
                    res.redirects.push((id, d[0].clone()));
                }
            }
        }
    }
    let canonical = |id: &str, canon: &HashMap<String, String>| -> String {
        canon.get(id).cloned().unwrap_or_else(|| id.to_string())
    };
    let mut class_by_label: HashMap<&str, Vec<String>> = HashMap::new();
    for (label, ids) in &all_by_label {
        let mut c = Vec::new();
        for id in ids {
            push_unique(&mut c, &canonical(id, &canon));
        }
        class_by_label.insert(label, c);
    }
    let unique_class = |label: &str| -> Option<&String> {
        match class_by_label.get(label) {
            Some(c) if c.len() == 1 => c.first(),
            _ => None,
        }
    };
    // Per-file stubs bind to the one type with their name.
    for (v, s) in &swift {
        for st in &s.stubs {
            let id = v.id(st).to_string();
            if canon.contains_key(&id)
                || index.get(files, &id).map(|n| n.kind) != Some(NodeKind::Stub)
            {
                continue;
            }
            let Some(label) = label_of.get(id.as_str()) else {
                continue;
            };
            if let Some(t) = unique_class(label) {
                canon.insert(id.clone(), t.clone());
                res.redirects.push((id, t.clone()));
            }
        }
    }
    // Supertype stubs are shared: bound the same way, else one node for every file.
    let mut shared: HashSet<&str> = HashSet::new();
    for (_, s) in &swift {
        for (id, label) in &s.global_stubs {
            if !shared.insert(id.as_str()) || canon.contains_key(id) {
                continue;
            }
            if let Some(t) = unique_class(label) {
                canon.insert(id.clone(), t.clone());
                res.redirects.push((id.clone(), t.clone()));
            } else if !index.contains(id) {
                res.nodes.push(Node {
                    id: id.clone(),
                    label: label.clone(),
                    kind: NodeKind::Stub,
                    file_type: "code".into(),
                    source_file: String::new(),
                    loc: Loc::Empty,
                    attrs: vec![origin()],
                });
                label_of.insert(id.as_str(), label.as_str());
            }
        }
    }

    // Members, plain return types, implements edges and existing links (canonical ids).
    let mut members: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut plain_return: HashMap<String, String> = HashMap::new();
    let mut implements: Vec<(String, String, &str, u32)> = Vec::new();
    let mut linked: HashSet<(String, String)> = HashSet::new();
    for (v, _) in &swift {
        for e in &v.facts.edges {
            let s = canonical(&e.source, &canon);
            let t = canonical(&e.target, &canon);
            match e.relation.as_str() {
                "method" => {
                    if let Some(label) = label_of.get(e.target.as_str()) {
                        let list = members.entry(s.clone()).or_default();
                        let name = member_name(label).to_string();
                        if !list.iter().any(|(n, id)| *n == name && *id == t) {
                            list.push((name, t.clone()));
                        }
                    }
                }
                "references" => {
                    if e.attrs.iter().any(|(k, _)| k == "metadata") {
                        plain_return.entry(s.clone()).or_insert(t.clone());
                    }
                }
                "implements" => {
                    if let Loc::Line(l) = e.loc {
                        implements.push((s.clone(), t.clone(), v.file.rel.as_str(), l));
                    }
                }
                _ => {}
            }
            linked.insert((s, t));
        }
    }
    let member_of = |ty: &str, name: &str| -> Option<String> {
        members
            .get(ty)?
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| id.clone())
    };
    // Declared types (not extensions): only they bind typed receivers and `Type.m()`.
    let mut decl_ids: HashSet<String> = HashSet::new();
    for (v, s) in &swift {
        for d in &s.decls {
            decl_ids.insert(canonical(v.id(d), &canon));
        }
    }
    // Receiver types bind to the one class-like node with that label when it is a declared type
    // or lies in the caller's own file (a type known only by extensions elsewhere does not).
    let mut decl_ids: HashSet<String> = HashSet::new();
    for (v, s) in &swift {
        for d in &s.decls {
            decl_ids.insert(canonical(v.id(d), &canon));
        }
    }
    let receiver_class = |rel: &str, label: &str| -> Option<String> {
        one(class_by_label.get(label))
            .filter(|id| decl_ids.contains(id) || file_of.get(id.as_str()) == Some(&rel))
    };
    // Every definition by name for bare calls (functions, methods, properties, cases, types),
    // with its file for path proximity.
    let mut by_name: HashMap<&str, Vec<(String, &str)>> = HashMap::new();
    let mut line_of: HashMap<&str, u32> = HashMap::new();
    for (v, _) in &swift {
        for n in &v.facts.nodes {
            if n.kind != NodeKind::Def {
                continue;
            }
            if let Loc::Line(l) = n.loc {
                line_of.entry(n.id.as_str()).or_insert(l);
            }
            let id = canonical(&n.id, &canon);
            let list = by_name.entry(member_name(&n.label)).or_default();
            if !list.iter().any(|(c, _)| *c == id) {
                let rel = file_of
                    .get(id.as_str())
                    .copied()
                    .unwrap_or(v.file.rel.as_str());
                list.push((id, rel));
            }
        }
    }

    // Calls: bare calls of every file, then member calls.
    let mut unresolved: Vec<(String, Vec<String>)> = Vec::new();
    let mut recorded: HashSet<(String, String, String)> = HashSet::new();
    for phase in 0..2 {
        for (v, s) in &swift {
            let rel = v.file.rel.as_str();
            let table: HashMap<&str, &str> = s
                .table
                .iter()
                .map(|(k, t)| (k.as_str(), t.as_str()))
                .collect();
            let factory: HashMap<&str, (&str, &str)> = s
                .factory
                .iter()
                .map(|(k, t, m)| (k.as_str(), (t.as_str(), m.as_str())))
                .collect();
            for c in &s.calls {
                if c.member != (phase == 1) {
                    continue;
                }
                let caller = canonical(v.id(&c.caller), &canon);
                let found: Option<(String, &str, Confidence, f64)> = if !c.member {
                    if c.via_stub {
                        None
                    } else {
                        by_name
                            .get(c.callee.as_str())
                            .and_then(|cands| nearest(rel, cands))
                            .map(|t| (t, "calls", Confidence::Inferred, 0.85))
                    }
                } else {
                    let Some(r) = c.receiver.as_deref() else {
                        continue;
                    };
                    let typed: Option<String> = table.get(r).map(|t| t.to_string()).or_else(|| {
                        let (ty, m) = factory.get(r)?;
                        let cls = receiver_class(rel, ty)?;
                        let f = member_of(&cls, m)?;
                        let ret = plain_return.get(&f)?;
                        label_of.get(ret.as_str()).map(|l| l.to_string())
                    });
                    let (ty, confidence, score) = match typed {
                        Some(t) => (t, Confidence::Inferred, 0.85),
                        None if r.chars().next().is_some_and(char::is_uppercase) => {
                            (r.to_string(), Confidence::Extracted, 1.0)
                        }
                        None => continue,
                    };
                    if class_by_label.contains_key(ty.as_str()) {
                        receiver_class(rel, ty.as_str()).map(|cls| {
                            match member_of(&cls, &c.callee) {
                                Some(m) => (m, "calls", confidence, score),
                                None => (cls, "references", confidence, score),
                            }
                        })
                    } else if is_denied(&ty)
                        || RECEIVER_DENY.contains(&ty.as_str())
                        || !recorded.insert((caller.clone(), c.callee.clone(), ty.clone()))
                    {
                        None
                    } else {
                        let entry = format!(
                            "{{\"callee\":{},\"receiver_type\":{},\"lang\":\"swift\",\"line\":\"L{}\",\"file\":{}}}",
                            json_str(&c.callee),
                            json_str(&ty),
                            c.line,
                            json_str(rel)
                        );
                        match unresolved.iter_mut().find(|(id, _)| *id == caller) {
                            Some((_, list)) => list.push(entry),
                            None => unresolved.push((caller.clone(), vec![entry])),
                        }
                        None
                    }
                };
                let Some((target, relation, confidence, score)) = found else {
                    continue;
                };
                if !linked.insert((caller.clone(), target.clone())) {
                    continue;
                }
                res.edges.push(edge(
                    caller, target, relation, rel, c.line, confidence, score,
                ));
            }
        }
    }
    for (id, entries) in unresolved {
        let json = format!("{{\"unresolved_calls\":[{}]}}", entries.join(","));
        res.attrs.push((id, "metadata".into(), Attr::Json(json)));
    }

    // A protocol dispatches each requirement to every conforming type's member of the same
    // name, at that type's declaration: INFERRED 0.85 with one conforming type, 0.75 with two,
    // 0.65 with more.
    let mut conformers: Vec<(&String, Vec<&String>)> = Vec::new();
    for (ty, proto, _, _) in &implements {
        match conformers.iter_mut().find(|(p, _)| *p == proto) {
            Some((_, list)) => {
                if !list.contains(&ty) {
                    list.push(ty);
                }
            }
            None => conformers.push((proto, vec![ty])),
        }
    }
    let owner_of: HashMap<&str, &String> = members
        .iter()
        .flat_map(|(o, l)| l.iter().map(move |(_, id)| (id.as_str(), o)))
        .collect();
    let mut dispatched: HashSet<(String, String)> = HashSet::new();
    for (proto, types) in &conformers {
        let Some(reqs) = members.get(*proto) else {
            continue;
        };
        let score = match types.len() {
            1 => 0.85,
            2 => 0.75,
            _ => 0.65,
        };
        for (name, req) in reqs {
            for ty in types {
                let Some(imp) = member_of(ty, name) else {
                    continue;
                };
                if imp == *req || !dispatched.insert((req.clone(), imp.clone())) {
                    continue;
                }
                let Some(owner) = owner_of.get(imp.as_str()) else {
                    continue;
                };
                let (Some(rel), Some(line)) =
                    (file_of.get(owner.as_str()), line_of.get(owner.as_str()))
                else {
                    continue;
                };
                res.edges.push(edge(
                    req.clone(),
                    imp,
                    "dispatches_to",
                    rel,
                    *line,
                    Confidence::Inferred,
                    score,
                ));
            }
        }
    }
    res
}

/// Receiver types never recorded in `unresolved_calls`, besides the call denylist.
const RECEIVER_DENY: &[&str] = &["Locale", "Sendable"];

fn one(v: Option<&Vec<String>>) -> Option<String> {
    match v {
        Some(v) if v.len() == 1 => v.first().cloned(),
        _ => None,
    }
}

/// (shared leading directories, same directory, same file) of a candidate's file to the
/// caller's.
fn proximity(from: &str, to: &str) -> (usize, bool, bool) {
    let dir = |p: &str| p.rsplit_once('/').map_or("", |(d, _)| d).to_string();
    let (a, b) = (dir(from), dir(to));
    let shared = a
        .split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y && !x.is_empty())
        .count();
    (shared, a == b, from == to)
}

/// The candidate nearest to the caller's file, if no other is as near.
fn nearest(from: &str, cands: &[(String, &str)]) -> Option<String> {
    let mut best: Option<((usize, bool, bool), &String)> = None;
    let mut tied = false;
    for (id, rel) in cands {
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
    if tied {
        None
    } else {
        best.map(|(_, id)| id.clone())
    }
}

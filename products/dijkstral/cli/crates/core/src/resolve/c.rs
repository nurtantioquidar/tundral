//! Cross-file pass for C and C++ (§24, §25). See `docs/graphify-format.md`.
//!
//! - `#include "p"` links to the indexed file at `p` relative to the including file; anything
//!   else (`<p>`, a missing or macro path) links to the external `slug(stem of p)`, unless exactly
//!   one C/C++ file in the repository has that stem, which then takes its place (§8.1).
//! - Type stubs bind to the unique C/C++ definition labelled like them (classes, enumerators,
//!   fields); an ambiguous or unknown name stays a stub.
//! - Bare calls with no definition of that name in their file bind to the nearest C/C++
//!   definition with that name by path proximity (INFERRED 0.85).
//! - `x.m()` through `x`'s declared type binds to member `m` of the unique class of that name
//!   (INFERRED 0.85); `Q::m()` binds to member `m` of the unique class `Q`, else to the unique
//!   out-of-line definition `Q::m()` (EXTRACTED 1.0); a `Q` that names no class falls back to the
//!   file's own definition named like the rest of the call.
//! - `metadata.unresolved_calls` (C++ only): typed calls whose type names no class, and
//!   qualified calls with a capitalised scope that names no class; one entry per (callee, type)
//!   and caller.

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::c::{call_name, CCallKind, CFacts};
use crate::extract::{external_node, origin, slug};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc, NodeKind};
use crate::lang::Lang;

fn c_facts<'v>(v: &'v View) -> Option<&'v CFacts> {
    match &v.facts.lang {
        LangFacts::C(c) => Some(c),
        _ => None,
    }
}

/// The directory part of a relative path (`""` at the root).
fn parent(rel: &str) -> &str {
    rel.rfind('/').map_or("", |i| &rel[..i])
}

/// `dir/../a/./b.h` → `a/b.h`; `None` when the path leaves the scan root.
fn join(dir: &str, path: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The file name without its final extension.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// `.h` / `.hpp`.
fn is_header(rel: &str) -> bool {
    let ext = rel.rsplit_once('.').map_or("", |(_, e)| e);
    ext.eq_ignore_ascii_case("h") || ext.eq_ignore_ascii_case("hpp")
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

/// The candidate nearest to the caller's file, if no other candidate is as near; candidates that
/// share no directory with the caller are only taken when there is exactly one.
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

fn unique(v: Option<&Vec<String>>) -> Option<&String> {
    match v {
        Some(v) if v.len() == 1 => v.first(),
        _ => None,
    }
}

fn push_unique(v: &mut Vec<String>, id: String) {
    if !v.contains(&id) {
        v.push(id);
    }
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

struct Spec<'s> {
    relation: &'s str,
    context: &'s str,
    rel: &'s str,
    line: u32,
    confidence: Confidence,
    score: Option<f64>,
}

fn edge(source: String, target: String, s: &Spec) -> Edge {
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

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let views: Vec<(&View, &CFacts)> = files
        .iter()
        .filter(|v| matches!(v.file.lang, Lang::C | Lang::Cpp))
        .filter_map(|v| c_facts(v).map(|c| (v, c)))
        .collect();
    if views.is_empty() {
        return res;
    }

    // Stems of every C/C++ file node, for the external-to-file remap.
    let mut stems: HashMap<&str, Vec<&str>> = HashMap::new();
    for v in files {
        stems
            .entry(stem(&v.file.rel))
            .or_default()
            .push(v.file.rel.as_str());
    }

    // Definitions by label (stubs), by call name (bare calls) and out-of-line `Q::m()`; classes
    // by label with their members.
    let mut by_label: HashMap<&str, Vec<String>> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<(String, &str)>> = HashMap::new();
    let mut out_of_line: HashMap<&str, Vec<String>> = HashMap::new();
    let mut classes: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
    for (vi, (v, c)) in views.iter().enumerate() {
        let rel = v.file.rel.as_str();
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
            let name = call_name(&n.label);
            by_name.entry(name).or_default().push((n.id.clone(), rel));
            if n.label.ends_with("()") {
                if name.contains("::") {
                    push_unique(out_of_line.entry(name).or_default(), n.id.clone());
                }
            } else {
                push_unique(by_label.entry(&n.label).or_default(), n.id.clone());
            }
        }
        for (ci, class) in c.classes.iter().enumerate() {
            classes
                .entry(class.label.as_str())
                .or_default()
                .push((vi, ci));
        }
    }
    // The unique class labelled `name` and its member `member`.
    let class_member = |name: &str, member: &str| -> (bool, Option<String>) {
        // A templated scope (`formatter<std::tm, Char>::parse`) is never looked up as a class.
        let Some(cands) = classes.get(name).filter(|_| !name.contains('<')) else {
            return (false, None);
        };
        if cands.len() != 1 {
            return (true, None);
        }
        let (vi, ci) = cands[0];
        let (v, c) = views[vi];
        let found = c.classes[ci]
            .members
            .iter()
            .find(|(m, _)| m == member)
            .map(|(_, id)| v.id(id).to_string());
        (true, found)
    };
    let is_stub = |id: &str| {
        index
            .get(files, id)
            .is_some_and(|n| n.kind == NodeKind::Stub)
    };

    // An unresolved include whose external id is a type stub several files share (so each got
    // its own prefix) lands on the stub of the first header by path that has it: `#include
    // "jv.h"` from `tests/` → `src_jv_h_jv`, `<vector>` → `test_gtest_gmock_gmock_h_vector`.
    let mut emitters: HashMap<&str, &View> = HashMap::new();
    for (v, c) in &views {
        if !is_header(&v.file.rel) {
            continue;
        }
        for (local, _) in &c.stubs {
            if v.id(local) != local {
                emitters.entry(local.as_str()).or_insert(v);
            }
        }
    }
    let renamed_elsewhere =
        |id: &str| -> Option<String> { emitters.get(id).map(|v| v.id(id).to_string()) };
    // Otherwise a definition labelled exactly like it (a class, field or enumerator `x`, a
    // function `x()`; not a method) takes it when it is the only one: `<ostream>` → class
    // `ostream`, `"fmt/printf.h"` → function `printf()`. JSON and YAML keys never do.
    let mut def_labels: HashMap<&str, Vec<&str>> = HashMap::new();
    for v in files
        .iter()
        .filter(|v| !matches!(v.file.lang, Lang::Json | Lang::Yaml))
    {
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
            if n.label.starts_with('.') {
                continue;
            }
            let key = n.label.strip_suffix("()").unwrap_or(&n.label);
            let ids = def_labels.entry(key).or_default();
            if !ids.contains(&n.id.as_str()) {
                ids.push(n.id.as_str());
            }
        }
    }
    let by_def_label = |id: &str| -> Option<String> {
        match def_labels.get(id).map(Vec::as_slice) {
            Some([only]) => Some(only.to_string()),
            _ => None,
        }
    };

    // Includes, then stubs.
    let mut externals: HashSet<String> = HashSet::new();
    for (v, c) in &views {
        let rel = v.file.rel.as_str();
        let file_id = v.id(&c.file_id).to_string();
        for inc in &c.includes {
            let local = inc
                .quoted
                .then(|| join(parent(rel), &inc.path))
                .flatten()
                .and_then(|p| index.file_ids.get(&p).cloned());
            let target = match local {
                Some(t) => t,
                None => {
                    let id = slug(stem(&inc.path));
                    let own = v.id(&id);
                    if let Some(t) = renamed_elsewhere(&id) {
                        t
                    } else if own != id
                        || v.facts
                            .nodes
                            .iter()
                            .any(|n| n.id == own && n.kind != NodeKind::External)
                    {
                        // Graphify writes the edge to the external id inside the file's own pass,
                        // so a node of the file with that id (a type stub `Shape` next to
                        // `#include "shape.h"`) captures it, renaming and binding included.
                        own.to_string()
                    } else if index.contains(&id) {
                        id
                    } else if let Some(t) = by_def_label(&id) {
                        t
                    } else if let Some(t) = match stems.get(id.as_str()).map(Vec::as_slice) {
                        Some([only]) => index.file_ids.get(*only).cloned(),
                        _ => None,
                    } {
                        // §8.1: the only file with that stem replaces the external.
                        t
                    } else {
                        if externals.insert(id.clone()) {
                            res.nodes.push(external_node(id.clone()));
                        }
                        id
                    }
                }
            };
            let spec = Spec {
                relation: "imports",
                context: "import",
                rel,
                line: inc.line,
                confidence: Confidence::Extracted,
                score: None,
            };
            res.edges.push(edge(file_id.clone(), target, &spec));
        }
        for (local, label) in &c.stubs {
            let stub = v.id(local);
            if !is_stub(stub) {
                continue;
            }
            if let Some(target) = unique(by_label.get(label.as_str())) {
                if target != stub {
                    res.redirects.push((stub.to_string(), target.clone()));
                }
            }
        }
    }

    // An out-of-line definition `K::a()` whose id is also a method `.a()` of another file (a
    // header and its implementation file share a directory and stem, so their ids coincide)
    // is not renamed apart: it merges into the method node, which keeps its own fields and
    // gains `definition_file` / `definition_location` (the first definition's).
    for (v, c) in &views {
        for (local, line) in &c.out_of_line {
            let def = v.id(local);
            if def == local {
                continue;
            }
            let others: Vec<&View> = views
                .iter()
                .map(|(o, _)| *o)
                .filter(|o| o.file.rel != v.file.rel && o.id(local) != local)
                .collect();
            let [m] = others.as_slice() else {
                continue;
            };
            let method = m.id(local);
            let Some(node) = m
                .facts
                .nodes
                .iter()
                .find(|n| n.id == method && n.label.starts_with('.'))
            else {
                continue;
            };
            if res.redirects.iter().any(|(from, _)| from == method) {
                res.redirects.push((def.to_string(), local.clone()));
                continue;
            }
            let mut moved = node.clone();
            moved.id = local.clone();
            moved
                .attrs
                .push(("definition_file".into(), Attr::Str(v.file.rel.clone())));
            moved
                .attrs
                .push(("definition_location".into(), Attr::Str(format!("L{line}"))));
            res.nodes.push(moved);
            res.redirects.push((method.to_string(), local.clone()));
            res.redirects.push((def.to_string(), local.clone()));
        }
    }

    // Calls: bare calls first, then typed and qualified ones, each in file and source order.
    let mut called: HashSet<(String, String)> = HashSet::new();
    for (v, c) in &views {
        let rel = v.file.rel.as_str();
        // Graphify adds a file's bare cross-file calls in callee-name order.
        let mut bare: Vec<_> = c
            .calls
            .iter()
            .filter(|c| c.kind == CCallKind::Bare)
            .collect();
        bare.sort_by(|a, b| a.name.cmp(&b.name));
        for call in bare {
            let caller = v.id(&call.caller).to_string();
            let cands = by_name
                .get(call.name.as_str())
                .map_or(&[][..], Vec::as_slice);
            let Some(target) = nearest(rel, &caller, cands) else {
                continue;
            };
            if called.insert((caller.clone(), target.to_string())) {
                let spec = Spec {
                    relation: "calls",
                    context: "call",
                    rel,
                    line: call.line,
                    confidence: Confidence::Inferred,
                    score: Some(0.85),
                };
                res.edges.push(edge(caller, target.to_string(), &spec));
            }
        }
    }
    for (v, c) in &views {
        let rel = v.file.rel.as_str();
        let mut unresolved: Vec<(String, Vec<String>)> = Vec::new();
        let mut recorded: HashSet<(String, String, String)> = HashSet::new();
        for call in c.calls.iter().filter(|c| c.kind != CCallKind::Bare) {
            let caller = v.id(&call.caller).to_string();
            let (known, member) = class_member(&call.receiver, &call.name);
            let found = match call.kind {
                CCallKind::Typed => member.map(|t| (t, Confidence::Inferred, Some(0.85))),
                _ => member
                    .or_else(|| {
                        let key = format!("{}::{}", call.receiver, call.name);
                        unique(out_of_line.get(key.as_str())).cloned()
                    })
                    .map(|t| (t, Confidence::Extracted, Some(1.0)))
                    .or_else(|| {
                        (!known)
                            .then_some(call.fallback.as_ref())
                            .flatten()
                            .map(|t| (v.id(t).to_string(), Confidence::Extracted, None))
                    }),
            };
            let record = !known
                && v.file.lang == Lang::Cpp
                && (call.kind == CCallKind::Typed
                    || call.receiver.chars().next().is_some_and(char::is_uppercase));
            if record && recorded.insert((caller.clone(), call.name.clone(), call.receiver.clone()))
            {
                let entry = format!(
                    "{{\"callee\":{},\"receiver_type\":{},\"lang\":\"cpp\",\"line\":\"L{}\"}}",
                    json_str(&call.name),
                    json_str(&call.receiver),
                    call.line
                );
                match unresolved.iter_mut().find(|(id, _)| *id == caller) {
                    Some((_, list)) => list.push(entry),
                    None => unresolved.push((caller.clone(), vec![entry])),
                }
            }
            let Some((target, confidence, score)) = found else {
                continue;
            };
            // Through a class, a method never calls itself (`S::r()` inside `S::r`).
            if score.is_some() && target == caller {
                continue;
            }
            if called.insert((caller.clone(), target.clone())) {
                let spec = Spec {
                    relation: "calls",
                    context: "call",
                    rel,
                    line: call.line,
                    confidence,
                    score,
                };
                res.edges.push(edge(caller, target, &spec));
            }
        }
        for (id, entries) in unresolved {
            let json = format!("{{\"unresolved_calls\":[{}]}}", entries.join(","));
            res.attrs.push((id, "metadata".into(), Attr::Json(json)));
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_paths_join_relative_to_the_including_directory() {
        assert_eq!(
            join("src", "../include/util.h"),
            Some("include/util.h".into())
        );
        assert_eq!(join("", "include/shape.h"), Some("include/shape.h".into()));
        assert_eq!(join("a", "./b/../c.h"), Some("a/c.h".into()));
        assert_eq!(join("", "../x.h"), None);
    }

    #[test]
    fn stems_drop_the_final_extension() {
        assert_eq!(stem("sys/types.h"), "types");
        assert_eq!(stem("cmath"), "cmath");
        assert_eq!(stem("a.b.hpp"), "a.b");
    }

    #[test]
    fn nearest_prefers_the_closest_directory() {
        let cands = vec![
            ("b_three_shared".to_string(), "b/three.h"),
            ("c_shared_shared".to_string(), "c/shared.c"),
        ];
        assert_eq!(nearest("b/three.c", "x", &cands), Some("b_three_shared"));
        assert_eq!(nearest("a/one.c", "x", &cands), None);
    }
}

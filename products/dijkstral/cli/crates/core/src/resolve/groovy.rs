//! Cross-file pass for Groovy (§26). See `docs/graphify-format.md`.
//!
//! - A type stub (`extends`, `implements`, annotation) binds to the class labelled with its name
//!   when exactly one class of **any** language carries that label and it is a Groovy, Java,
//!   Kotlin or Scala class; otherwise the stub stays (`PyOnly` and `Twice` stay stubs).
//! - An import (`@groovy:<importer>|<id>`, `id` = `slug(Name)`) links, in order: the importer's
//!   own stub of that id; the only Java class named `Name` that Java code uses as a type (Java
//!   quirk J3, which Graphify applies through the shared stub id); any node with that id; the only
//!   file node whose id equals it or ends with `_<it>` (§8.1) when it is a Groovy, Java, Kotlin or
//!   Scala file (another language drops the edge); else the external node `id`.
//! - A call that did not bind in its file binds by name to the nearest method or function of a
//!   Groovy, Java, Kotlin or Scala file labelled `.name()` or `name()` (`resolve::php::nearest`;
//!   two in the nearest file, such as a class's and its interface's `.count()`, bind nothing):
//!   INFERRED 0.85, one edge per (caller, callee).

use std::collections::{HashMap, HashSet};

use super::php::{cross_edge, nearest};
use super::{Index, Resolution, View};
use crate::extract::groovy::{GroovyFacts, VIRTUAL};
use crate::extract::{external_node, slug};
use crate::facts::{Attr, Confidence, LangFacts, NodeKind};
use crate::lang::Lang;

const DROP: &str = "@drop";

/// The languages Graphify lets Groovy bind to.
fn jvm(lang: Lang) -> bool {
    matches!(lang, Lang::Groovy | Lang::Java | Lang::Kotlin | Lang::Scala)
}

fn groovy_facts<'v>(v: &'v View) -> Option<&'v GroovyFacts> {
    match &v.facts.lang {
        LangFacts::Groovy(g) => Some(g),
        _ => None,
    }
}

fn is_class(n: &crate::facts::Node) -> bool {
    n.kind == NodeKind::Def && matches!(n.attr("_callable_class"), Some(Attr::Bool(true)))
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let groovy: Vec<(&View, &GroovyFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Groovy)
        .filter_map(|v| groovy_facts(v).map(|g| (v, g)))
        .collect();
    if groovy.is_empty() {
        return res;
    }

    // Classes of every language by label, with whether they may be bound to.
    let mut classes: HashMap<&str, Vec<(&str, bool)>> = HashMap::new();
    for v in files {
        for n in v.facts.nodes.iter().filter(|n| is_class(n)) {
            classes
                .entry(n.label.as_str())
                .or_default()
                .push((n.id.as_str(), jvm(v.file.lang)));
        }
    }
    let mut redirected: HashSet<&str> = HashSet::new();
    for (v, _) in &groovy {
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Stub) {
            if let Some([(id, true)]) = classes.get(n.label.as_str()).map(Vec::as_slice) {
                if redirected.insert(n.id.as_str()) {
                    res.redirects.push((n.id.clone(), id.to_string()));
                }
            }
        }
    }

    let file_nodes: Vec<(&str, Lang)> = files
        .iter()
        .filter_map(|v| {
            let n = v.facts.nodes.iter().find(|n| n.kind == NodeKind::File)?;
            Some((n.id.as_str(), v.file.lang))
        })
        .collect();
    // Java classes by slug of their label, and the names Java code uses as types (Java quirk J3:
    // Graphify merges such a type's stub, which other languages' imports also name, into the
    // class).
    let mut java_classes: HashMap<String, Vec<(&str, &str)>> = HashMap::new();
    let mut java_typed: HashSet<&str> = HashSet::new();
    for v in files.iter().filter(|v| v.file.lang == Lang::Java) {
        for n in v.facts.nodes.iter().filter(|n| is_class(n)) {
            java_classes
                .entry(slug(&n.label))
                .or_default()
                .push((n.id.as_str(), n.label.as_str()));
        }
        if let LangFacts::Java(j) = &v.facts.lang {
            java_typed.extend(j.typed.iter().map(String::as_str));
        }
    }
    let mut done: HashSet<&str> = HashSet::new();
    let mut externals: HashSet<&str> = HashSet::new();
    for (v, _) in &groovy {
        for e in v.facts.edges.iter().filter(|e| e.relation == "imports") {
            let t = e.target.as_str();
            let Some(body) = t.strip_prefix(VIRTUAL) else {
                continue;
            };
            if !done.insert(t) {
                continue;
            }
            let id = body.split_once('|').map_or(body, |(_, id)| id);
            let local = v.id(id);
            let target = if v
                .facts
                .nodes
                .iter()
                .any(|n| n.kind == NodeKind::Stub && n.id == local)
            {
                local.to_string()
            } else if let Some([(class, _)]) = java_classes
                .get(id)
                .map(|cs| {
                    cs.iter()
                        .filter(|(_, label)| java_typed.contains(label))
                        .copied()
                        .collect::<Vec<_>>()
                })
                .as_deref()
            {
                class.to_string()
            } else if index.contains(id) {
                id.to_string()
            } else {
                match remap(id, &file_nodes) {
                    Some((file, lang)) if jvm(lang) => file.to_string(),
                    Some(_) => DROP.to_string(),
                    None => {
                        if externals.insert(id) {
                            res.nodes.push(external_node(id.to_string()));
                        }
                        id.to_string()
                    }
                }
            };
            res.redirects.push((t.to_string(), target));
        }
    }

    // Methods and functions of JVM files by name.
    let mut by_name: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    for v in files.iter().filter(|v| jvm(v.file.lang)) {
        for n in &v.facts.nodes {
            // Methods of line-read Spock files count too, although they carry no `_callable`.
            if n.kind != NodeKind::Def || is_class(n) {
                continue;
            }
            let Some(name) = n.label.strip_suffix("()") else {
                continue;
            };
            let name = name.strip_prefix('.').unwrap_or(name);
            by_name
                .entry(name)
                .or_default()
                .push((n.id.clone(), v.file.rel.clone()));
        }
    }
    let mut linked: HashSet<(String, String)> = HashSet::new();
    for (v, g) in &groovy {
        let rel = v.file.rel.as_str();
        for c in &g.calls {
            let Some(target) = by_name
                .get(c.name.as_str())
                .and_then(|cands| nearest(rel, cands))
            else {
                continue;
            };
            let caller = v.id(&c.caller).to_string();
            if linked.insert((caller.clone(), target.clone())) {
                res.edges.push(cross_edge(
                    caller,
                    target,
                    "calls",
                    (rel, c.line),
                    Some("call"),
                    Confidence::Inferred,
                    0.85,
                ));
            }
        }
    }
    res
}

/// The only file node whose id is `id` or ends with `_<id>`.
fn remap<'f>(id: &str, files: &[(&'f str, Lang)]) -> Option<(&'f str, Lang)> {
    let suffix = format!("_{id}");
    let mut hit = None;
    for &(fid, lang) in files {
        if fid == id || fid.ends_with(&suffix) {
            if hit.is_some() {
                return None;
            }
            hit = Some((fid, lang));
        }
    }
    hit
}

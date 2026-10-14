//! Cross-file pass for PowerShell (§41). See `docs/graphify-format.md`.
//!
//! - A type stub binds to the only PowerShell class or enum with that name, unless that
//!   definition's id was renamed by the collision rule (§4.5), when the stub stays.
//! - An import's external is replaced by the only file of the repository, in any language,
//!   whose stem slugs to the same id (`. .\Utils.ps1` → `worked/httpx/raw/utils.py`); with none
//!   or several, the external stays.
//! - A command its file did not bind goes to the function or method of that exact
//!   (case-sensitive) name nearest to the caller's file (`resolve::php::nearest`; INFERRED 0.85,
//!   context `call`), once per pair.

use std::collections::{HashMap, HashSet};

use super::php::{cross_edge, nearest};
use super::{Index, Resolution, View};
use crate::extract::external_node;
use crate::extract::powershell::{stem, PsFacts};
use crate::extract::slug;
use crate::facts::{Confidence, LangFacts};

fn ps_facts<'v>(v: &'v View) -> Option<&'v PsFacts> {
    match &v.facts.lang {
        LangFacts::PowerShell(p) => Some(p),
        _ => None,
    }
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let ps: Vec<(&View, &PsFacts)> = files
        .iter()
        .filter_map(|v| ps_facts(v).map(|p| (v, p)))
        .collect();
    if ps.is_empty() {
        return res;
    }

    // Type stubs.
    let mut types: HashMap<&str, Vec<(String, bool)>> = HashMap::new();
    for (v, p) in &ps {
        for (name, local) in &p.types {
            let id = v.id(local);
            let list = types.entry(name.as_str()).or_default();
            if !list.iter().any(|(x, _)| x == id) {
                list.push((id.to_string(), id == local));
            }
        }
    }
    for (v, p) in &ps {
        for (local, name) in &p.stubs {
            if let Some([(target, true)]) = types.get(name.as_str()).map(Vec::as_slice) {
                res.redirects
                    .push((v.id(local).to_string(), target.clone()));
            }
        }
    }

    // Imports: the only same-stem file, else the external.
    let mut stems: HashMap<String, Vec<&str>> = HashMap::new();
    for v in files {
        if let Some(id) = index.file_ids.get(&v.file.rel) {
            stems
                .entry(slug(&stem(&v.file.rel)))
                .or_default()
                .push(id.as_str());
        }
    }
    let mut done: HashSet<&str> = HashSet::new();
    for (_, p) in &ps {
        for ext in &p.imports {
            if !done.insert(ext.as_str()) {
                continue;
            }
            let Some([file]) = stems.get(ext).map(Vec::as_slice) else {
                res.nodes.push(external_node(ext.clone()));
                continue;
            };
            let imports_itself = ps.iter().any(|(v, p)| {
                index.file_ids.get(&v.file.rel).map(String::as_str) == Some(*file)
                    && p.imports.contains(ext)
            });
            if !imports_itself {
                res.redirects.push((ext.clone(), file.to_string()));
                continue;
            }
            // The file's own import would become a self-loop, which Graphify drops: leave every
            // edge to the external dangling (dropped by the build) and re-point the others here.
            for (v, _) in &ps {
                for e in &v.facts.edges {
                    if e.relation == "imports_from" && e.target == *ext && e.source != *file {
                        let mut moved = e.clone();
                        moved.target = file.to_string();
                        res.edges.push(moved);
                    }
                }
            }
        }
    }

    // Commands across files.
    let mut callables: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    for (v, p) in &ps {
        for (name, local) in &p.callables {
            callables
                .entry(name.as_str())
                .or_default()
                .push((v.id(local).to_string(), v.file.rel.clone()));
        }
    }
    for (v, p) in &ps {
        let mut linked: HashSet<(&str, String)> = HashSet::new();
        for (caller, name, line) in &p.calls {
            let Some(cands) = callables.get(name.as_str()) else {
                continue;
            };
            let Some(target) = nearest(&v.file.rel, cands) else {
                continue;
            };
            let src = v.id(caller);
            if linked.insert((src, target.clone())) {
                res.edges.push(cross_edge(
                    src.to_string(),
                    target,
                    "calls",
                    (&v.file.rel, *line),
                    Some("call"),
                    Confidence::Inferred,
                    0.85,
                ));
            }
        }
    }
    res
}

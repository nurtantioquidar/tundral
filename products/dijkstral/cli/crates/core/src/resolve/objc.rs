//! Cross-file pass for Objective-C (§34). See `docs/graphify-format.md`.
//!
//! - **Header/implementation merge.** A definition id emitted by an Objective-C header and by the
//!   `.m`/`.mm` file with the same directory and stem (both prefix ids with the same file id) is
//!   not renamed apart: the header's node stays and gains `definition_file` /
//!   `definition_location` (the implementation's first node of that id). Edges the
//!   implementation file adds from a merged definition are dropped when the header or an earlier
//!   edge of the implementation already has the same (source, relation, target id as written),
//!   so the header's lines win; the extractor gave each of them an alias target, which is
//!   redirected to the real target only when kept.
//! - **Stubs** bind to the only Objective-C class, protocol or enum with their name.
//! - **Imports.** `#import "p"` names the file at `p` relative to the importing file when it is
//!   indexed, else `slug(stem of p)`; that id lands on the importing file's own node with that id,
//!   else the node that has it, else the first file (by path) that emitted it before collision
//!   renaming (a header's renamed stub, the header of a header/implementation pair), else the
//!   only file with that stem, else an external node.
//! - **Messages.** A receiver typed as a class (`self`, `super`, a capitalised name: EXTRACTED
//!   1.0; a declared variable, instance variable or property: INFERRED 0.85) whose name is the
//!   label of exactly one class (case-insensitive) calls that class's method of the selector, or
//!   else `references` the class (context `call`). A pair the file already links is skipped, and
//!   a method never calls itself.

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::objc::{ObjcFacts, ObjcRecv};
use crate::extract::{external_node, origin, slug, Ctx};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc, NodeKind};
use crate::ids::file_id;
use crate::lang::{Family, Lang};

fn objc_facts<'v>(v: &'v View) -> Option<&'v ObjcFacts> {
    match &v.facts.lang {
        LangFacts::ObjC(f) => Some(f),
        _ => None,
    }
}

/// The directory part of a relative path (`""` at the root).
fn parent(rel: &str) -> &str {
    rel.rfind('/').map_or("", |i| &rel[..i])
}

/// The file name without its final extension (Python's `Path.stem`).
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
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

fn is_implementation(rel: &str) -> bool {
    rel.ends_with(".m") || rel.ends_with(".mm")
}

/// A cross-file edge with context `context` at `rel:line`, `(confidence, score)` as given.
fn edge(
    source: String,
    target: String,
    relation: &str,
    context: Option<&str>,
    (rel, line): (&str, u32),
    (confidence, score): (Confidence, f64),
) -> Edge {
    Edge {
        source,
        target,
        relation: relation.into(),
        confidence,
        score: Some(score),
        context: context.map(str::to_string),
        source_file: rel.into(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let views: Vec<(&View, &ObjcFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::ObjC)
        .filter_map(|v| objc_facts(v).map(|f| (v, f)))
        .collect();
    if views.is_empty() {
        return res;
    }

    // Header/implementation pairs and their merged definitions.
    let mut headers: HashMap<(&str, &str), usize> = HashMap::new();
    for (i, (v, _)) in views.iter().enumerate() {
        if v.file.rel.ends_with(".h") {
            headers
                .entry((parent(&v.file.rel), stem(&v.file.rel)))
                .or_insert(i);
        }
    }
    // Implementation view → (header view, merged local ids).
    let mut pairs: HashMap<usize, (usize, HashSet<String>)> = HashMap::new();
    // Renamed id → merged (local) id.
    let mut merged: HashMap<String, String> = HashMap::new();
    let mut merged_ids: HashSet<String> = HashSet::new();
    for (mi, (mv, mf)) in views.iter().enumerate() {
        if !is_implementation(&mv.file.rel) {
            continue;
        }
        let Some(&hi) = headers.get(&(parent(&mv.file.rel), stem(&mv.file.rel))) else {
            continue;
        };
        let (hv, hf) = views[hi];
        let header_defs: HashSet<&str> = hf.defs.iter().map(|(id, _)| id.as_str()).collect();
        let mut here = HashSet::new();
        for (local, line) in &mf.defs {
            if !header_defs.contains(local.as_str()) || merged_ids.contains(local) {
                continue;
            }
            let (hx, mx) = (hv.id(local), mv.id(local));
            if hx == local || mx == local {
                continue;
            }
            let Some(node) = index.get(files, hx) else {
                continue;
            };
            let mut node = node.clone();
            node.id = local.clone();
            node.attrs
                .push(("definition_file".into(), Attr::Str(mv.file.rel.clone())));
            node.attrs
                .push(("definition_location".into(), Attr::Str(format!("L{line}"))));
            res.nodes.push(node);
            res.redirects.push((hx.to_string(), local.clone()));
            res.redirects.push((mx.to_string(), local.clone()));
            merged.insert(hx.to_string(), local.clone());
            merged.insert(mx.to_string(), local.clone());
            merged_ids.insert(local.clone());
            here.insert(local.clone());
        }
        pairs.insert(mi, (hi, here));
    }
    let fin = |v: &View, local: &str| -> String {
        let id = v.id(local);
        merged.get(id).cloned().unwrap_or_else(|| id.to_string())
    };

    // A stub named like one of Swift's supertype stubs (which its cross-file pass creates) is
    // renamed as if Swift had emitted it too: `slug(path)_id`.
    let swift_globals: HashSet<&str> = files
        .iter()
        .filter_map(|w| match &w.facts.lang {
            LangFacts::Swift(s) => Some(s.global_stubs.iter().map(|(id, _)| id.as_str())),
            _ => None,
        })
        .flatten()
        .collect();
    let mut over: HashMap<(usize, &str), String> = HashMap::new();
    for (vi, (v, f)) in views.iter().enumerate() {
        for (local, _) in &f.stubs {
            if v.id(local) != local || !swift_globals.contains(local.as_str()) {
                continue;
            }
            if let Some(node) = index.get(files, local) {
                let renamed = format!("{}_{}", slug(&v.file.rel), local);
                let mut node = node.clone();
                node.id = renamed.clone();
                res.nodes.push(node);
                over.insert((vi, local.as_str()), renamed);
            }
        }
    }
    let fin_at = |vi: usize, v: &View, local: &str| -> String {
        over.get(&(vi, local))
            .cloned()
            .unwrap_or_else(|| fin(v, local))
    };

    // Stubs bind to the only class, protocol or enum with their name.
    let mut by_label: HashMap<&str, Vec<String>> = HashMap::new();
    for (v, _) in &views {
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
            if n.label.starts_with('-') || n.label.starts_with('+') {
                continue;
            }
            let name = n
                .label
                .strip_prefix('<')
                .and_then(|l| l.strip_suffix('>'))
                .unwrap_or(&n.label);
            let id = merged.get(&n.id).cloned().unwrap_or_else(|| n.id.clone());
            let ids = by_label.entry(name).or_default();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let mut bound: HashMap<String, String> = HashMap::new();
    for (vi, (v, f)) in views.iter().enumerate() {
        for (local, label) in &f.stubs {
            if !index
                .get(files, v.id(local))
                .is_some_and(|n| n.kind == NodeKind::Stub)
            {
                continue;
            }
            let stub = fin_at(vi, v, local);
            if let Some([target]) = by_label.get(label.as_str()).map(Vec::as_slice) {
                if *target != stub {
                    res.redirects.push((stub.clone(), target.clone()));
                    bound.insert(stub, target.clone());
                }
            }
        }
    }

    // Aliased edges of implementation files. A merged definition's edges are dropped when the
    // header or an earlier edge of the file has the same (source, relation, target as written);
    // and once any definition merged, every Objective-C edge keeps the first of its (source,
    // target, relation) instead of the last.
    let any_merge = !merged_ids.is_empty();
    let final_of = |id: &str| -> String {
        let id = merged.get(id).map_or(id, String::as_str);
        bound.get(id).cloned().unwrap_or_else(|| id.to_string())
    };
    // A property reference on a pair that `inherits`/`implements` elsewhere (a class extension
    // in the implementation file, its superclass in the header) is absorbed without a trace.
    let mut typed_pairs: HashSet<(String, String)> = HashSet::new();
    for (vi, (v, f)) in views.iter().enumerate() {
        let aliases: HashMap<&str, &str> = f
            .aliases
            .iter()
            .map(|a| (a.alias.as_str(), a.target.as_str()))
            .collect();
        for e in v
            .facts
            .edges
            .iter()
            .filter(|e| e.relation == "inherits" || e.relation == "implements")
        {
            let target = match aliases.get(e.target.as_str()) {
                Some(t) => fin_at(vi, v, t),
                None => e.target.clone(),
            };
            typed_pairs.insert((final_of(&e.source), final_of(&target)));
        }
    }
    let mut first: HashSet<(String, String, String)> = HashSet::new();
    for (mi, (mv, mf)) in views.iter().enumerate() {
        let none = HashSet::new();
        let (header, here) = match pairs.get(&mi) {
            Some((hi, here)) => (Some(views[*hi].0), here),
            None => (None, &none),
        };
        let mut seen: HashSet<(String, String, String)> = HashSet::new();
        if let Some((hv, hf)) = header.filter(|_| !here.is_empty()).map(|hv| {
            let hf = objc_facts(hv).expect("Objective-C view");
            (hv, hf)
        }) {
            let haliases: HashMap<&str, usize> = hf
                .aliases
                .iter()
                .enumerate()
                .map(|(i, a)| (a.alias.as_str(), i))
                .collect();
            for e in &hv.facts.edges {
                if let Some(&i) = haliases.get(e.target.as_str()) {
                    let a = &hf.aliases[i];
                    if here.contains(&a.source) {
                        seen.insert((a.source.clone(), a.relation.clone(), a.target.clone()));
                    }
                }
            }
        }
        let aliases: HashMap<&str, usize> = mf
            .aliases
            .iter()
            .enumerate()
            .map(|(i, a)| (a.alias.as_str(), i))
            .collect();
        for e in &mv.facts.edges {
            let Some(&i) = aliases.get(e.target.as_str()) else {
                if any_merge {
                    first.insert((final_of(&e.source), final_of(&e.target), e.relation.clone()));
                }
                continue;
            };
            let a = &mf.aliases[i];
            if here.contains(&a.source)
                && !seen.insert((a.source.clone(), a.relation.clone(), a.target.clone()))
            {
                continue;
            }
            let target = fin_at(mi, mv, &a.target);
            if e.relation == "references"
                && e.context.as_deref() == Some("field")
                && typed_pairs.contains(&(final_of(&fin(mv, &a.source)), final_of(&target)))
            {
                continue;
            }
            if any_merge {
                let source = final_of(&fin(mv, &a.source));
                let target = final_of(&target);
                // Self-loops are dropped too.
                if source == target || !first.insert((source, target, a.relation.clone())) {
                    continue;
                }
            }
            res.redirects.push((a.alias.clone(), target));
        }
    }

    // Imports.
    // §8.1: an external named like the stem of exactly one file (case-insensitive) is that file.
    let mut stems: HashMap<String, Vec<&str>> = HashMap::new();
    for v in files {
        stems
            .entry(slug(stem(&v.file.rel)))
            .or_default()
            .push(v.file.rel.as_str());
    }
    // (id, whether the path named an indexed file relative to the importing one).
    let import_target = |v: &View, imp: &crate::extract::objc::ObjcImport| -> (String, bool) {
        imp.quoted
            .then(|| join(parent(&v.file.rel), &imp.path))
            .flatten()
            .filter(|p| index.file_ids.contains_key(p))
            .map_or_else(|| (slug(stem(&imp.path)), false), |p| (file_id(&p), true))
    };
    // An import landing on another language's node (Swift's `import Foundation` module) is
    // dropped: the language of the first file that emitted an id, for the ids imports name.
    let mut wanted: HashSet<String> = HashSet::new();
    for (v, f) in &views {
        for imp in &f.imports {
            let (local, _) = import_target(v, imp);
            if index.contains(&local) {
                wanted.insert(local);
            }
        }
    }
    let mut emitter: HashMap<String, Family> = HashMap::new();
    for w in files {
        for n in &w.facts.nodes {
            if wanted.contains(&n.id) && !emitter.contains_key(&n.id) {
                emitter.insert(n.id.clone(), w.family());
            }
        }
    }
    // Swift's module nodes are created by its own cross-file pass: `slug(import path)`.
    let swift_modules: HashSet<String> = files
        .iter()
        .filter_map(|w| match &w.facts.lang {
            LangFacts::Swift(s) => Some(s.imports.iter().map(|(p, _)| slug(p))),
            _ => None,
        })
        .flatten()
        .collect();
    let mut externals: HashSet<String> = HashSet::new();
    for (v, f) in &views {
        let rel = v.file.rel.as_str();
        let source = v.id(&f.file_id).to_string();
        for imp in &f.imports {
            let (local, relative) = import_target(v, imp);
            let foreign = match emitter.get(&local) {
                Some(f) => !matches!(f, Family::ObjC | Family::C),
                None => !index.contains(&local) && swift_modules.contains(&local),
            };
            if foreign {
                continue;
            }
            let target = if index.contains(&local) || merged_ids.contains(&local) {
                local
            } else if let Some(w) = files
                .iter()
                .find(|w| w.file.rel.ends_with(".h") && w.id(&local) != local)
            {
                fin(w, &local)
            } else if !relative
                && v.id(&local) != local
                && v.facts
                    .nodes
                    .iter()
                    .any(|n| n.id == v.id(&local) && n.kind != NodeKind::File)
            {
                // The importing file's own node with that id (a stub of the imported class).
                fin(v, &local)
            } else if let Some(t) = match stems.get(local.as_str()).map(Vec::as_slice) {
                Some([only]) => index.file_ids.get(*only).cloned(),
                _ => None,
            } {
                t
            } else {
                if externals.insert(local.clone()) {
                    res.nodes.push(external_node(local.clone()));
                }
                local
            };
            res.edges.push(edge(
                source.clone(),
                target,
                "imports",
                Some("import"),
                (rel, imp.line),
                (Confidence::Extracted, 1.0),
            ));
        }
    }

    // Messages to typed receivers.
    let mut classes: HashMap<String, Vec<String>> = HashMap::new();
    let mut defs: HashSet<String> = HashSet::new();
    for (v, f) in &views {
        for c in &f.classes {
            let id = fin(v, &c.id);
            let ids = classes.entry(c.name.to_lowercase()).or_default();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        for (local, _) in &f.defs {
            defs.insert(fin(v, local));
        }
    }
    let mut linked: HashSet<(String, String)> = HashSet::new();
    for (v, f) in &views {
        for (caller, target) in &f.linked {
            let target = fin(v, target);
            let target = bound.get(&target).cloned().unwrap_or(target);
            linked.insert((fin(v, caller), target));
        }
    }
    // Properties, instance variables and superclass of every class name, over all its
    // declarations.
    type Members<'m> = Vec<(&'m str, &'m str)>;
    let mut hierarchy: HashMap<&str, (Members, Members, Option<&str>)> = HashMap::new();
    for (_, f) in &views {
        for c in &f.classes {
            let entry = hierarchy.entry(c.name.as_str()).or_default();
            for (p, t) in &c.props {
                entry.0.push((p.as_str(), t.as_str()));
            }
            for (p, t) in &c.ivars {
                entry.1.push((p.as_str(), t.as_str()));
            }
            if entry.2.is_none() {
                entry.2 = c.superclass.as_deref();
            }
        }
    }
    // The declared class of a property (`ivar` false) or instance variable on the class or its
    // nearest superclass that has it.
    let member_type = |class: &str, name: &str, ivar: bool| -> Option<&str> {
        let mut cur = class;
        for _ in 0..16 {
            let (props, ivars, sup) = hierarchy.get(cur)?;
            let list = if ivar { ivars } else { props };
            if let Some((_, t)) = list.iter().find(|(p, _)| *p == name) {
                return Some(t);
            }
            cur = (*sup)?;
        }
        None
    };
    for (v, f) in &views {
        let rel = v.file.rel.as_str();
        for send in &f.sends {
            let caller = fin(v, &send.caller);
            let (name, confidence, score) = match &send.recv {
                ObjcRecv::Class(n) => (n.as_str(), Confidence::Extracted, 1.0),
                ObjcRecv::Typed(t) => (t.as_str(), Confidence::Inferred, 0.85),
                ObjcRecv::Prop { class, prop } => match member_type(class, prop, false) {
                    Some(t) => (t, Confidence::Inferred, 0.85),
                    None => continue,
                },
                ObjcRecv::Ivar { class, name } => match member_type(class, name, true) {
                    Some(t) => (t, Confidence::Inferred, 0.85),
                    None => continue,
                },
            };
            let Some([class]) = classes.get(&name.to_lowercase()).map(Vec::as_slice) else {
                continue;
            };
            let method = Ctx::child_id(class, &send.selector);
            let (target, relation) = if defs.contains(&method) {
                if method == caller {
                    continue;
                }
                (method, "calls")
            } else {
                (class.clone(), "references")
            };
            if linked.insert((caller.clone(), target.clone())) {
                res.edges.push(edge(
                    caller,
                    target,
                    relation,
                    Some("call"),
                    (rel, send.line),
                    (confidence, score),
                ));
            }
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_and_joins() {
        assert_eq!(stem("UIKit/UIKit.h"), "UIKit");
        assert_eq!(stem("Foundation.NSString"), "Foundation");
        assert_eq!(stem("x"), "x");
        assert_eq!(join("a/b", "../c.h").as_deref(), Some("a/c.h"));
        assert_eq!(join("", "../c.h"), None);
    }
}

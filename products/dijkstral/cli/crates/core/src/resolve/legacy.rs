//! Cross-file pass for the grammar-less languages of §44.
//!
//! - Pascal `uses X` imports the file whose `unit X` it names, else the external `slug(X)`.
//! - A base class not declared in the file binds to the type of that name in a used unit; the
//!   file's own node for it is dropped when nothing else points at it.
//! - A method's call that the file could not resolve walks the caller's base classes (in the
//!   file, then in used units) and calls the first method of that name (EXTRACTED 1.0).
//! - An `.lpk` unit named like exactly one Pascal file in the package's directory or below takes
//!   that file's id as the package file emits it (`<package file>_<pas file id>`, or the Pascal
//!   file's path with its extension, `pascal_widgets_pas`, when the ids coincide); otherwise it
//!   stays `slug(unit)`.

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::external_node;
use crate::extract::legacy::{LegacyFacts, PascalFacts};
use crate::extract::slug;
use crate::facts::{Confidence, Edge, LangFacts, Loc, NodeKind};
use crate::lang::Lang;

const DROP: &str = "@drop";

fn legacy<'v>(v: &'v View) -> Option<&'v LegacyFacts> {
    match &v.facts.lang {
        LangFacts::Legacy(f) => Some(f),
        _ => None,
    }
}

pub fn resolve(files: &[View], _index: &Index) -> Resolution {
    let mut res = Resolution::default();
    lpk_units(files, &mut res);
    pascal(files, &mut res);
    res
}

fn lpk_units(files: &[View], res: &mut Resolution) {
    let pascal: Vec<&View> = files
        .iter()
        .filter(|v| {
            let base = v.file.rel.rsplit('/').next().unwrap_or("");
            let ext = base.rsplit_once('.').map_or("", |(_, e)| e);
            v.file.lang == Lang::Pascal
                && (ext.eq_ignore_ascii_case("pas") || ext.eq_ignore_ascii_case("pp"))
        })
        .collect();
    for v in files.iter().filter(|v| v.file.lang == Lang::Lpk) {
        let Some(f) = legacy(v) else {
            continue;
        };
        // Units bind only to Pascal files in the package's directory or below.
        let under = v.file.rel.rsplit_once('/').map(|(d, _)| format!("{d}/"));
        let mut stems: HashMap<String, Vec<&View>> = HashMap::new();
        for p in &pascal {
            if under
                .as_ref()
                .is_some_and(|d| !p.file.rel.starts_with(d.as_str()))
            {
                continue;
            }
            let base = p.file.rel.rsplit('/').next().unwrap_or("");
            let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
            stems.entry(stem.to_lowercase()).or_default().push(p);
        }
        let own = crate::ids::file_id(&v.file.rel);
        for (local, name, target) in &f.units {
            let node = v.id(local).to_string();
            let Some([pas]) = stems.get(&name.to_lowercase()).map(Vec::as_slice) else {
                res.redirects.push((target.clone(), node));
                continue;
            };
            if *local != own {
                res.redirects.push((node, DROP.into()));
            }
            let pas_id = crate::ids::file_id(&pas.file.rel);
            // A Pascal file whose id is the package file's own (same directory and stem) is
            // named by its path with the extension instead (`pascal_widgets_pas`).
            let id = if pas_id == own {
                slug(&pas.file.rel)
            } else {
                format!("{}_{pas_id}", slug(&v.file.rel))
            };
            res.nodes.push(crate::facts::Node {
                id: id.clone(),
                label: name.clone(),
                kind: NodeKind::Def,
                file_type: "code".into(),
                source_file: v.file.rel.clone(),
                loc: Loc::Line(1),
                attrs: vec![crate::extract::origin()],
            });
            res.redirects.push((target.clone(), id));
        }
        if let Some((pkg, target)) = &f.package {
            res.redirects.push((target.clone(), v.id(pkg).to_string()));
        }
    }
}

struct Unit<'v> {
    view: &'v View<'v>,
    facts: &'v PascalFacts,
    file_id: String,
}

fn pascal(files: &[View], res: &mut Resolution) {
    let units: Vec<Unit> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Pascal)
        .filter_map(|v| {
            let facts = legacy(v)?.pascal.as_ref()?;
            let file_id = v
                .facts
                .nodes
                .iter()
                .find(|n| n.kind == NodeKind::File)?
                .id
                .clone();
            Some(Unit {
                view: v,
                facts,
                file_id,
            })
        })
        .collect();
    if units.is_empty() {
        return;
    }
    let by_name: HashMap<String, usize> = units
        .iter()
        .enumerate()
        .filter(|(_, u)| !u.facts.unit.is_empty())
        .rev()
        .map(|(i, u)| (u.facts.unit.to_ascii_lowercase(), i))
        .collect();
    let mut externals: HashSet<String> = HashSet::new();
    // The units a file uses, by index.
    let used = |u: &Unit| -> Vec<usize> {
        u.facts
            .uses
            .iter()
            .filter_map(|(n, _)| by_name.get(&n.to_ascii_lowercase()).copied())
            .collect()
    };
    // A type by name as seen from unit `u`: its own, else a used unit's. (unit index, type index)
    let find_type = |u: usize, name: &str| -> Option<(usize, usize)> {
        let own = units[u]
            .facts
            .types
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name));
        if let Some(t) = own {
            return Some((u, t));
        }
        for w in used(&units[u]) {
            if let Some(t) = units[w]
                .facts
                .types
                .iter()
                .position(|t| t.name.eq_ignore_ascii_case(name))
            {
                return Some((w, t));
            }
        }
        None
    };
    for (ui, u) in units.iter().enumerate() {
        let v = u.view;
        for (name, target) in &u.facts.uses {
            let to = match by_name.get(&name.to_ascii_lowercase()) {
                Some(&w) if w != ui => units[w].file_id.clone(),
                _ => {
                    let id = slug(name);
                    if externals.insert(id.clone()) {
                        res.nodes.push(external_node(id.clone()));
                    }
                    id
                }
            };
            res.redirects.push((target.clone(), to));
        }
        // Base classes declared in used units.
        let mut kept: HashSet<String> = v
            .facts
            .edges
            .iter()
            .filter(|e| {
                !e.target
                    .starts_with(crate::extract::legacy::pascal::VIRTUAL)
            })
            .map(|e| e.target.clone())
            .collect();
        let mut fallbacks: Vec<String> = Vec::new();
        for b in &u.facts.bases {
            let node = v.id(&b.node).to_string();
            let found = used(u).into_iter().find_map(|w| {
                units[w]
                    .facts
                    .types
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(&b.name))
                    .map(|t| units[w].view.id(&t.id).to_string())
            });
            match found {
                Some(t) => {
                    res.redirects.push((b.target.clone(), t));
                    fallbacks.push(node);
                }
                None => {
                    kept.insert(node.clone());
                    res.redirects.push((b.target.clone(), node));
                }
            }
        }
        for n in fallbacks {
            if !kept.contains(&n) {
                kept.insert(n.clone());
                res.redirects.push((n, DROP.into()));
            }
        }
        // Calls into base classes.
        let mut done: HashSet<(String, String)> = HashSet::new();
        for c in &u.facts.pending {
            let Some(start) = find_type(ui, &c.class) else {
                continue;
            };
            let mut queue = vec![start];
            let mut seen = HashSet::new();
            let mut hit = None;
            while let Some((w, t)) = queue.pop() {
                if !seen.insert((w, t)) || seen.len() > 64 {
                    continue;
                }
                let ty = &units[w].facts.types[t];
                if (w, t) != start {
                    if let Some((_, id)) = ty
                        .methods
                        .iter()
                        .find(|(m, _)| m.eq_ignore_ascii_case(&c.name))
                    {
                        hit = Some(units[w].view.id(id).to_string());
                        break;
                    }
                }
                for b in ty.bases.iter().rev() {
                    if let Some(next) = find_type(w, b) {
                        queue.push(next);
                    }
                }
            }
            let caller = v.id(&c.caller).to_string();
            if let Some(target) = hit.filter(|t| *t != caller) {
                if done.insert((caller.clone(), target.clone())) {
                    res.edges.push(Edge {
                        source: caller,
                        target,
                        relation: "calls".into(),
                        confidence: Confidence::Extracted,
                        score: Some(1.0),
                        context: Some("call".into()),
                        source_file: v.file.rel.clone(),
                        loc: Loc::Line(c.line),
                        weight: Some(1.0),
                        attrs: vec![crate::extract::origin()],
                    });
                }
            }
        }
    }
}

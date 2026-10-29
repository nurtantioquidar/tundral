//! Cross-file pass for Dart (§27). See `docs/graphify-format.md`.
//!
//! - Part-file definitions (`@dartpart:<library id>_<name>`) get Graphify's absolute id: the slug
//!   of the canonical scan root, `_`, then the library-relative id (quirk DA1).
//! - An anonymous extension's id gets the slug of its file's canonical absolute path without the
//!   extension, then `_anonymous_extension` (quirk DA3).
//! - A referenced type node (`inherits`, `implements`, `references` …) binds to the only Dart
//!   definition labelled with its name (a trailing `?` ignored); otherwise it stays. Annotation
//!   nodes never bind.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{Index, Resolution, View};
use crate::extract::dart::{DartFacts, ANON, PART};
use crate::extract::slug;
use crate::facts::{LangFacts, NodeKind};
use crate::lang::Lang;

fn dart_facts<'v>(v: &'v View) -> Option<&'v DartFacts> {
    match &v.facts.lang {
        LangFacts::Dart(d) => Some(d),
        _ => None,
    }
}

/// The canonical scan root: `abs` without the components of `rel`.
fn root_of(v: &View) -> Option<PathBuf> {
    let depth = v.file.rel.split('/').count();
    let root = v.file.abs.ancestors().nth(depth).map(Path::to_path_buf)?;
    Some(std::fs::canonicalize(&root).unwrap_or(root))
}

pub fn resolve(files: &[View], _index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let dart: Vec<(&View, &DartFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Dart)
        .filter_map(|v| dart_facts(v).map(|d| (v, d)))
        .collect();
    let Some(root) = dart.first().and_then(|(v, _)| root_of(v)) else {
        return res;
    };
    let root_slug = slug(&root.to_string_lossy());

    // Final ids of part-file definitions.
    let mut moved: HashMap<String, String> = HashMap::new();
    for (v, _) in &dart {
        for n in &v.facts.nodes {
            // Renamed on collision (`<part path>_@dartpart:…`) or not.
            if !n.id.contains(PART) || moved.contains_key(&n.id) {
                continue;
            }
            let id = if root_slug.is_empty() {
                n.id.replace(PART, "")
            } else {
                n.id.replace(PART, &format!("{root_slug}_"))
            };
            moved.insert(n.id.clone(), id.clone());
            let mut node = n.clone();
            node.id = id.clone();
            res.nodes.push(node);
            res.redirects.push((n.id.clone(), id));
        }
        // Anonymous extensions.
        let anon = v
            .facts
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Def && n.id.ends_with(ANON));
        if let Some(n) = anon {
            let abs = std::fs::canonicalize(&v.file.abs).unwrap_or_else(|_| v.file.abs.clone());
            let stem = abs.with_extension("");
            let local = slug(&format!("{}_anonymous_extension", stem.to_string_lossy()));
            let base = n.id.strip_suffix(ANON).unwrap_or(&n.id);
            let base = moved
                .get(&n.id)
                .and_then(|m| m.strip_suffix(ANON))
                .unwrap_or(base);
            let id = format!("{base}{local}");
            let mut node = n.clone();
            node.id = id.clone();
            res.nodes.push(node);
            res.redirects.push((
                moved.get(&n.id).cloned().unwrap_or_else(|| n.id.clone()),
                id,
            ));
        }
    }

    // Definitions by label.
    let mut defs: HashMap<&str, Vec<String>> = HashMap::new();
    for (v, _) in &dart {
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
            let id = moved.get(&n.id).cloned().unwrap_or_else(|| n.id.clone());
            let ids = defs.entry(n.label.as_str()).or_default();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let mut done: HashSet<&str> = HashSet::new();
    for (_, d) in &dart {
        for (id, label) in &d.types {
            if !done.insert(id.as_str()) {
                continue;
            }
            if let Some([target]) = defs.get(label.trim_end_matches('?')).map(Vec::as_slice) {
                if target != id {
                    res.redirects.push((id.clone(), target.clone()));
                }
            }
        }
    }
    res
}

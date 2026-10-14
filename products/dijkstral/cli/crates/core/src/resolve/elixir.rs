//! Cross-file pass for Elixir (§29). See `docs/graphify-format.md`.
//!
//! - **Imports.** Each `alias`/`import`/`use`/`require` target `E` (`slug(name as written)`)
//!   becomes, in order: the only top-level module whose `slug(label)` is `E`, when it lives in
//!   another file; an existing node whose id is `E`; the only module or file whose id is a
//!   path-anchored suffix match (`slug(path suffix)_local` for a module, `slug(path suffix)` for a
//!   file: `q_r` for module `R` of `lib/q.ex`, `w_v` for `lib/w/v.ex`), when in another file,
//!   added after the others and only where the importing file has no import edge to it yet (an
//!   own-file match drops the edge); else the external node `E`.
//! - **`Mod.f`** calls the function labelled `f()` of the only top-level module labelled `Mod`
//!   (EXTRACTED 1.0); nested modules are not reachable by name from other files.
//! - **Bare `f`** calls the only function labelled `f()` in the files defining the caller's
//!   `import`/`use` modules (INFERRED 0.85).
//!
//! Each (caller, target) pair is linked once, first call wins; self-calls are dropped.

use std::collections::{HashMap, HashSet};

use super::php::cross_edge;
use super::{Index, Resolution, View};
use crate::extract::elixir::{ElixirFacts, ExCallKind};
use crate::extract::{external_node, origin, slug};
use crate::facts::{Confidence, Edge, LangFacts, Loc};
use crate::ids::file_id;
use crate::lang::Lang;

fn elixir_facts<'v>(v: &'v View) -> Option<&'v ElixirFacts> {
    match &v.facts.lang {
        LangFacts::Elixir(e) => Some(e),
        _ => None,
    }
}

/// `lib/w/v.ex` → `v`, `w_v`, `lib_w_v` (slugged path suffixes without the extension).
fn path_keys(rel: &str) -> Vec<String> {
    let stem = match rel.rsplit_once('.') {
        Some((s, ext)) if !ext.contains('/') => s,
        _ => rel,
    };
    let mut out = Vec::new();
    let mut start = stem.len();
    loop {
        let piece = &stem[start..];
        if !piece.is_empty() {
            let k = slug(piece);
            if !k.is_empty() && !out.contains(&k) {
                out.push(k);
            }
        }
        match stem[..start.saturating_sub(1)].rfind('/') {
            Some(i) if start > 0 => start = i + 1,
            _ if start > 0 => start = 0,
            _ => break,
        }
    }
    out
}

fn import_edge(source: &str, target: String, rel: &str, line: u32) -> Edge {
    Edge {
        source: source.to_string(),
        target,
        relation: "imports".into(),
        confidence: Confidence::Extracted,
        score: None,
        context: Some("import".into()),
        source_file: rel.into(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let ex: Vec<(usize, &View, &ElixirFacts)> = files
        .iter()
        .enumerate()
        .filter(|(_, v)| v.file.lang == Lang::Elixir)
        .filter_map(|(i, v)| elixir_facts(v).map(|e| (i, v, e)))
        .collect();
    if ex.is_empty() {
        return res;
    }

    // Top-level modules by label and by slug(label); every module's files by label; functions
    // by (owner, name) and by (file, name); anchored suffix keys of modules and files.
    let mut top_by_label: HashMap<&str, Vec<String>> = HashMap::new();
    let mut top_by_slug: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    let mut module_files: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut fn_by_owner: HashMap<(String, &str), String> = HashMap::new();
    let mut fns_by_file: HashMap<(usize, &str), Vec<String>> = HashMap::new();
    let mut suffix: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    let mut file_node: HashMap<usize, String> = HashMap::new();
    for &(fi, v, e) in &ex {
        let fid = file_id(&v.file.rel);
        let file_final = v.id(&fid).to_string();
        let keys = path_keys(&v.file.rel);
        for k in &keys {
            suffix
                .entry(k.clone())
                .or_default()
                .push((file_final.clone(), fi));
        }
        file_node.insert(fi, file_final);
        for m in &e.modules {
            let id = v.id(&m.id).to_string();
            if m.top {
                top_by_label
                    .entry(m.label.as_str())
                    .or_default()
                    .push(id.clone());
                top_by_slug
                    .entry(slug(&m.label))
                    .or_default()
                    .push((id.clone(), fi));
            }
            let files_of = module_files.entry(m.label.as_str()).or_default();
            if !files_of.contains(&fi) {
                files_of.push(fi);
            }
            if let Some(local) = m.id.strip_prefix(&format!("{fid}_")) {
                for k in &keys {
                    suffix
                        .entry(format!("{k}_{local}"))
                        .or_default()
                        .push((id.clone(), fi));
                }
            }
        }
        for (owner, name, local) in &e.functions {
            let id = v.id(local).to_string();
            if !owner.is_empty() {
                fn_by_owner
                    .entry((v.id(owner).to_string(), name.as_str()))
                    .or_insert_with(|| id.clone());
            }
            let list = fns_by_file.entry((fi, name.as_str())).or_default();
            if !list.contains(&id) {
                list.push(id);
            }
        }
    }
    let unique_top = |label: &str| match top_by_label.get(label).map(Vec::as_slice) {
        Some([only]) => Some(only.clone()),
        _ => None,
    };

    let mut externals: HashSet<String> = HashSet::new();
    let mut deferred: Vec<Edge> = Vec::new();
    let mut calls: Vec<Edge> = Vec::new();
    for &(fi, v, e) in &ex {
        let rel = v.file.rel.as_str();
        let src = file_node[&fi].clone();

        // Imports.
        let mut linked: HashSet<String> = HashSet::new();
        let mut late: Vec<(String, u32)> = Vec::new();
        for (ext, line) in &e.imports {
            let line = *line;
            let target = match top_by_slug.get(ext).map(Vec::as_slice) {
                Some([(id, f)]) => (*f != fi).then(|| id.clone()),
                Some(_) => None,
                None => None,
            };
            let ambiguous_top = top_by_slug.get(ext).is_some_and(|v| !v.is_empty());
            if let Some(t) = target {
                linked.insert(t.clone());
                res.edges.push(import_edge(&src, t, rel, line));
                continue;
            }
            if index.contains(ext) {
                linked.insert(ext.clone());
                res.edges.push(import_edge(&src, ext.clone(), rel, line));
                continue;
            }
            if let Some([(id, f)]) = suffix.get(ext).map(Vec::as_slice) {
                if *f != fi {
                    late.push((id.clone(), line));
                }
                continue;
            }
            let _ = ambiguous_top;
            if externals.insert(ext.clone()) {
                res.nodes.push(external_node(ext.clone()));
            }
            res.edges.push(import_edge(&src, ext.clone(), rel, line));
        }
        for (t, line) in late {
            if linked.insert(t.clone()) {
                deferred.push(import_edge(&src, t, rel, line));
            }
        }

        // Calls.
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for c in &e.calls {
            let caller = v.id(&c.caller).to_string();
            let found = match &c.kind {
                ExCallKind::Member { module, name } => unique_top(module)
                    .and_then(|m| fn_by_owner.get(&(m, name.as_str())).cloned())
                    .map(|t| (t, Confidence::Extracted, 1.0)),
                ExCallKind::Bare { name, scope } => {
                    let mut in_files: Vec<usize> = Vec::new();
                    for m in scope {
                        for &f in module_files.get(m.as_str()).into_iter().flatten() {
                            if !in_files.contains(&f) {
                                in_files.push(f);
                            }
                        }
                    }
                    let mut cands: Vec<&String> = Vec::new();
                    for f in in_files {
                        for id in fns_by_file.get(&(f, name.as_str())).into_iter().flatten() {
                            if !cands.contains(&id) {
                                cands.push(id);
                            }
                        }
                    }
                    match cands.as_slice() {
                        [only] => Some(((*only).clone(), Confidence::Inferred, 0.85)),
                        _ => None,
                    }
                }
            };
            let Some((target, conf, score)) = found else {
                continue;
            };
            if target == caller || !seen.insert((caller.clone(), target.clone())) {
                continue;
            }
            calls.push(cross_edge(
                caller,
                target,
                "calls",
                (rel, c.line),
                Some("call"),
                conf,
                score,
            ));
        }
    }
    res.edges.extend(deferred);
    // Graphify adds cross-file calls in target order (observable in surprise ties: `f → B.c`
    // before `f → B.y` whatever the call order).
    calls.sort_by(|a, b| (&a.target, &a.source).cmp(&(&b.target, &b.source)));
    res.edges.extend(calls);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_keys_are_anchored_suffixes() {
        assert_eq!(path_keys("lib/w/v.ex"), ["v", "w_v", "lib_w_v"]);
        assert_eq!(path_keys("mix.exs"), ["mix"]);
        assert_eq!(path_keys("config/config.exs"), ["config", "config_config"]);
    }
}

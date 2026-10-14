//! Cross-file pass for Lua and Luau (§28). See `docs/graphify-format.md`.
//!
//! - `require` targets (`@lua:<importer>|<module>`, written by `crate::extract::lua`): the module
//!   with `.` read as `/` is looked up on disk from the importer's directory up to the root, as
//!   `<dir>/<path>.lua`, then `<dir>/<path>/init.lua`, nearest directory first. A hit links that
//!   file (or nothing, when the file is not indexed). Otherwise the target is the external node
//!   `slug(module)`, unless exactly one file node's id equals it or ends with `_<it>` (Graphify's
//!   build-stage remap, §8.1): a Lua file is linked instead, the importer itself or a file of
//!   another language drops the edge.
//! - A call that did not bind in its file and names a file the caller's file requires
//!   (`lexer.lua(…)` with `require "pl.lexer"`) links that file, EXTRACTED 1.0. Otherwise it binds
//!   by its name as written (`f`, `M.f`) to the nearest Lua function labelled so, or Lua file of
//!   that name (`resolve::php::nearest`; a tie binds nothing): INFERRED 0.85. One edge per
//!   (caller, callee).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::php::{cross_edge, nearest};
use super::{Index, Resolution, View};
use crate::extract::lua::{LuaFacts, VIRTUAL};
use crate::extract::{external_node, slug};
use crate::facts::{Confidence, LangFacts, NodeKind};
use crate::lang::Lang;

const DROP: &str = "@drop";

fn lua_facts<'v>(v: &'v View) -> Option<&'v LuaFacts> {
    match &v.facts.lang {
        LangFacts::Lua(l) => Some(l),
        _ => None,
    }
}

/// The scan root: `abs` without the components of `rel`.
fn root_of(v: &View) -> Option<PathBuf> {
    let depth = v.file.rel.split('/').count();
    v.file.abs.ancestors().nth(depth).map(Path::to_path_buf)
}

fn parent(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(d, _)| d)
}

/// `<dir>/<path>.lua`, then `<dir>/<path>/init.lua`, for `dir` and each of its ancestors.
fn candidates(importer: &str, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut dir = parent(importer);
    loop {
        let base = if dir.is_empty() {
            path.to_string()
        } else {
            format!("{dir}/{path}")
        };
        out.push(format!("{base}.lua"));
        out.push(format!("{base}/init.lua"));
        if dir.is_empty() {
            break;
        }
        dir = parent(dir);
    }
    out
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let lua: Vec<(&View, &LuaFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Lua)
        .filter_map(|v| lua_facts(v).map(|l| (v, l)))
        .collect();
    let Some(root) = lua.first().and_then(|(v, _)| root_of(v)) else {
        return res;
    };

    // Every file node with its language, for the external-to-file remap.
    let file_nodes: Vec<(&str, &str, Lang)> = files
        .iter()
        .filter_map(|v| {
            let n = v.facts.nodes.iter().find(|n| n.kind == NodeKind::File)?;
            Some((n.id.as_str(), v.file.rel.as_str(), v.file.lang))
        })
        .collect();

    let mut bound: HashMap<&str, String> = HashMap::new();
    let mut externals: HashSet<String> = HashSet::new();
    // Files each Lua file imports, by file node id.
    let mut imported: Vec<HashSet<String>> = vec![HashSet::new(); lua.len()];
    for (fi, (v, _)) in lua.iter().enumerate() {
        for e in &v.facts.edges {
            let t = e.target.as_str();
            let Some(body) = t.strip_prefix(VIRTUAL) else {
                continue;
            };
            if let Some(target) = bound.get(t) {
                imported[fi].insert(target.clone());
                continue;
            }
            let (importer, module) = body.split_once('|').unwrap_or(("", body));
            let target = bind_require(importer, module, &root, index).unwrap_or_else(|| {
                let id = slug(module);
                if id.is_empty() {
                    return DROP.to_string();
                }
                match remap(&id, &file_nodes) {
                    Some((file, rel, lang)) => {
                        if rel == importer || lang != Lang::Lua {
                            DROP.to_string()
                        } else {
                            file.to_string()
                        }
                    }
                    None => {
                        if externals.insert(id.clone()) {
                            res.nodes.push(external_node(id.clone()));
                        }
                        id
                    }
                }
            });
            imported[fi].insert(target.clone());
            bound.insert(t, target.clone());
            res.redirects.push((t.to_string(), target));
        }
    }

    // Lua functions and files by label (functions without `()`), for calls across files.
    let mut by_label: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    let mut file_labels: HashMap<&str, &str> = HashMap::new();
    for (v, _) in &lua {
        for n in &v.facts.nodes {
            let label = match n.kind {
                NodeKind::Def => n.label.strip_suffix("()").unwrap_or(&n.label),
                NodeKind::File => {
                    file_labels.insert(n.id.as_str(), n.label.as_str());
                    n.label.as_str()
                }
                _ => continue,
            };
            by_label
                .entry(label)
                .or_default()
                .push((n.id.clone(), v.file.rel.clone()));
        }
    }
    let mut linked: HashSet<(String, String)> = HashSet::new();
    for (fi, (v, l)) in lua.iter().enumerate() {
        let rel = v.file.rel.as_str();
        // Required files by name: `lexer.lua(…)` with `require "pl.lexer"` links that file.
        let mut required: Vec<&String> = imported[fi]
            .iter()
            .filter(|t| file_labels.contains_key(t.as_str()))
            .collect();
        required.sort();
        for c in &l.calls {
            let file = required
                .iter()
                .find(|t| file_labels[t.as_str()] == c.callee)
                .map(|t| (*t).clone());
            let (target, confidence, score) = match file {
                Some(t) => (t, Confidence::Extracted, 1.0),
                None => {
                    let Some(t) = by_label
                        .get(c.callee.as_str())
                        .and_then(|cands| nearest(rel, cands))
                    else {
                        continue;
                    };
                    (t, Confidence::Inferred, 0.85)
                }
            };
            let caller = v.id(&c.caller).to_string();
            if linked.insert((caller.clone(), target.clone())) {
                res.edges.push(cross_edge(
                    caller,
                    target,
                    "calls",
                    (rel, c.line),
                    Some("call"),
                    confidence,
                    score,
                ));
            }
        }
    }
    res
}

/// The file a `require` names on disk, searched from the importer's directory upwards; `DROP`
/// when that file exists but is not indexed.
fn bind_require(importer: &str, module: &str, root: &Path, index: &Index) -> Option<String> {
    let path = module.replace('.', "/");
    if path.starts_with('/') {
        return None;
    }
    for cand in candidates(importer, &path) {
        // Path semantics: `pl.` names `pl/`, whose `init.lua` is `pl//init.lua`.
        let Some(cand) = normalize(&cand) else {
            continue;
        };
        if root.join(&cand).is_file() {
            return Some(
                index
                    .file_ids
                    .get(&cand)
                    .cloned()
                    .unwrap_or_else(|| DROP.to_string()),
            );
        }
    }
    None
}

/// `a//b/./c/../d` → `a/b/d`; `None` when it leaves the root.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The only file node whose id is `id` or ends with `_<id>`.
fn remap<'f>(id: &str, files: &[(&'f str, &'f str, Lang)]) -> Option<(&'f str, &'f str, Lang)> {
    let suffix = format!("_{id}");
    let mut hit = None;
    for &(fid, rel, lang) in files {
        if fid == id || fid.ends_with(&suffix) {
            if hit.is_some() {
                return None;
            }
            hit = Some((fid, rel, lang));
        }
    }
    hit
}

#[cfg(test)]
mod tests {
    use super::candidates;

    #[test]
    fn require_candidates_go_up_from_the_importer() {
        assert_eq!(
            candidates("a/b/r.lua", "x/y"),
            [
                "a/b/x/y.lua",
                "a/b/x/y/init.lua",
                "a/x/y.lua",
                "a/x/y/init.lua",
                "x/y.lua",
                "x/y/init.lua"
            ]
        );
        assert_eq!(candidates("r.lua", "m"), ["m.lua", "m/init.lua"]);
        assert_eq!(
            super::normalize("lua/pl//init.lua").as_deref(),
            Some("lua/pl/init.lua")
        );
        assert_eq!(super::normalize("a/../../x.lua"), None);
    }
}

//! Cross-file pass for Zig (§33). See `docs/graphify-format.md`.
//!
//! - An `@import` target id (see `crate::extract::zig`) lands on the node with that id (a file at
//!   the root: `"util.zig"` → file `util`; any node does), else on the only file whose stem slugs
//!   to it (§8.1; in any language: `@import("xev")` → `include/xev.h`), else on the external
//!   node of that id. An import that lands on the importing file is dropped.
//! - A bare call that binds nothing in its file binds to the Zig definitions answering to that
//!   name (functions, methods, containers, enum and error members; the caller's own file
//!   included): EXTRACTED 1.0 when exactly one of them is in a file the caller's file imports
//!   directly (its id is an import target), otherwise INFERRED 0.85 to the nearest by path (most
//!   shared leading directories; a tie binds nothing; a candidate sharing no directory binds only
//!   from the same directory or alone). Context `call`; one edge per (caller, callee).

use std::collections::{HashMap, HashSet};

use super::kotlin::{call_name, edge, DROP};
use super::{Index, Resolution, View};
use crate::extract::external_node;
use crate::extract::slug;
use crate::extract::zig::{virtual_target, ZigFacts};
use crate::facts::{Confidence, LangFacts, NodeKind};
use crate::lang::Lang;

fn zig_facts<'v>(v: &'v View) -> Option<&'v ZigFacts> {
    match &v.facts.lang {
        LangFacts::Zig(z) => Some(z),
        _ => None,
    }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Shared leading directories, then whether the candidate is in the caller's directory.
fn proximity(from: &str, to: &str) -> (usize, bool) {
    let (a, b) = (parent(from), parent(to));
    let common = a
        .split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y && !x.is_empty())
        .count();
    (common, a == b)
}

/// The nearest candidate, if no other is as near; one that shares no directory with the caller
/// binds only from the caller's own directory (the root) or when it is the only candidate.
fn nearest<'c>(
    from: &str,
    caller: &str,
    cands: &'c [(String, usize)],
    rels: &[&str],
) -> Option<&'c str> {
    let mut best: Option<((usize, bool), &str)> = None;
    let (mut tied, mut count) = (false, 0);
    for (id, f) in cands {
        if id == caller {
            continue;
        }
        count += 1;
        let p = proximity(from, rels[*f]);
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
    (!tied && (p.0 > 0 || p.1 || count == 1)).then_some(id)
}

/// `slug` of a file name up to its last dot.
fn stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    slug(name.rsplit_once('.').map_or(name, |(s, _)| s))
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let zig: Vec<(usize, &View, &ZigFacts)> = files
        .iter()
        .enumerate()
        .filter(|(_, v)| v.file.lang == Lang::Zig)
        .filter_map(|(i, v)| zig_facts(v).map(|z| (i, v, z)))
        .collect();
    if zig.is_empty() {
        return res;
    }
    let rels: Vec<&str> = files.iter().map(|v| v.file.rel.as_str()).collect();

    // Imports.
    let mut stems: HashMap<String, Vec<String>> = HashMap::new();
    for v in files {
        if let Some(n) = v.facts.nodes.iter().find(|n| n.kind == NodeKind::File) {
            stems
                .entry(stem(&v.file.rel))
                .or_default()
                .push(n.id.clone());
        }
    }
    let mut targets: HashMap<&str, String> = HashMap::new();
    for (_, v, z) in &zig {
        let own = v.id(&z.file_id);
        let mut done: HashSet<&str> = HashSet::new();
        for id in &z.imports {
            if !done.insert(id.as_str()) {
                continue;
            }
            let target = targets.entry(id.as_str()).or_insert_with(|| {
                if index.contains(id) {
                    id.clone()
                } else {
                    // Unlike Python's or Kotlin's imports, a file of any language takes it.
                    match stems.get(id).map(Vec::as_slice) {
                        Some([file]) => file.clone(),
                        _ => {
                            res.nodes.push(external_node(id.clone()));
                            id.clone()
                        }
                    }
                }
            });
            // An import that lands on the importing file itself is dropped.
            let target = if target == own {
                DROP.to_string()
            } else {
                target.clone()
            };
            res.redirects
                .push((virtual_target(id, &v.file.rel), target));
        }
    }

    // Calls: Zig definitions by call name, with the index of their file.
    let mut by_name: HashMap<&str, Vec<(String, usize)>> = HashMap::new();
    for (fi, v, _) in &zig {
        // Only the node the build keeps for an id (the first) answers to its name.
        let mut seen: HashSet<&str> = HashSet::new();
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
            if !seen.insert(n.id.as_str()) {
                continue;
            }
            let c = by_name.entry(call_name(&n.label)).or_default();
            if !c.iter().any(|(id, _)| *id == n.id) {
                c.push((n.id.clone(), *fi));
            }
        }
    }
    let file_ids: HashMap<usize, &str> = zig
        .iter()
        .filter_map(|(fi, v, _)| {
            let n = v.facts.nodes.iter().find(|n| n.kind == NodeKind::File)?;
            Some((*fi, n.id.as_str()))
        })
        .collect();
    let mut called: HashSet<(String, String)> = HashSet::new();
    for (_, v, z) in &zig {
        let rel = v.file.rel.as_str();
        let imported: HashSet<&str> = z
            .imports
            .iter()
            .filter(|id| {
                index
                    .get(files, id)
                    .is_some_and(|n| n.kind == NodeKind::File)
            })
            .map(String::as_str)
            .collect();
        for call in &z.calls {
            let caller = v.id(&call.caller).to_string();
            let Some(cands) = by_name.get(call.name.as_str()) else {
                continue;
            };
            let direct: Vec<&String> = cands
                .iter()
                .filter(|(id, f)| {
                    *id != caller && file_ids.get(f).is_some_and(|fid| imported.contains(fid))
                })
                .map(|(id, _)| id)
                .collect();
            let (target, confidence, score) = match direct.as_slice() {
                [only] => ((*only).clone(), Confidence::Extracted, 1.0),
                _ => match nearest(rel, &caller, cands, &rels) {
                    Some(t) => (t.to_string(), Confidence::Inferred, 0.85),
                    None => continue,
                },
            };
            if called.insert((caller.clone(), target.clone())) {
                res.edges
                    .push(edge(caller, target, (rel, call.line), confidence, score));
            }
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_files_share_the_root_directory() {
        let rels = ["main.zig", "c.zig", "y/h.zig", "x/a.zig", "y/z/a.zig"];
        let cands = |fs: &[usize]| -> Vec<(String, usize)> {
            fs.iter().map(|&f| (format!("n{f}"), f)).collect()
        };
        assert_eq!(nearest("main.zig", "m", &cands(&[1, 2]), &rels), Some("n1"));
        assert_eq!(nearest("main.zig", "m", &cands(&[3, 4]), &rels), None);
        assert_eq!(nearest("main.zig", "m", &cands(&[4]), &rels), Some("n4"));
        assert_eq!(nearest("y/q.zig", "m", &cands(&[1, 2]), &rels), Some("n2"));
    }

    #[test]
    fn stems_drop_the_last_extension() {
        assert_eq!(stem("sub/deep.zig"), "deep");
        assert_eq!(stem("x.y.zig"), "x_y");
    }
}

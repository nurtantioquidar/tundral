//! `extract --cargo`: crate → crate dependencies of a Cargo workspace, read from its manifests
//! without running `cargo` (`docs/graphify-inputs.md` §2).
//!
//! The manifest is the root's `Cargo.toml`, else the first `*/Cargo.toml` or `*/*/Cargo.toml` in
//! path order. Its crates are its own `[package]` and its `[workspace] members` (globs expanded,
//! `exclude` honoured); a dependency is an edge when a crate's `[dependencies]` names another of
//! them, by key or by `package`. Each crate becomes `crate:<name>`, which the graph merges into the
//! node with the same label and source file (the `Cargo.toml` package node of the code pass);
//! each dependency becomes a `crate_depends_on` link unless the pair is already linked.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::facts::{Confidence, Edge, Loc, Node, NodeKind};
use crate::graph::Graph;

/// A crate of the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Crate {
    pub name: String,
    /// Its `Cargo.toml`, relative to the scan root.
    pub manifest: String,
}

/// The crate graph of a workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Workspace {
    /// The manifest the workspace was read from; `None` when there is none.
    pub manifest: Option<String>,
    /// Sorted by name.
    pub crates: Vec<Crate>,
    /// `(dependent, dependency)` indices into `crates`, sorted by their names.
    pub deps: Vec<(usize, usize)>,
}

impl Workspace {
    pub fn fingerprint(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Read the workspace under `root`.
pub fn scan(root: &Path) -> Workspace {
    let Some(manifest) = find_manifest(root) else {
        return Workspace::default();
    };
    let Some(top) = read_toml(&root.join(&manifest)) else {
        return Workspace {
            manifest: Some(manifest),
            ..Workspace::default()
        };
    };
    let dir = parent(&manifest);
    // name → (manifest, parsed table), first manifest wins on a repeated name.
    let mut found: BTreeMap<String, (String, toml::Table)> = BTreeMap::new();
    let mut add = |rel: String, table: toml::Table| {
        if let Some(name) = package_name(&table) {
            found.entry(name).or_insert((rel, table));
        }
    };
    if package_name(&top).is_some() {
        add(manifest.clone(), top.clone());
    }
    if let Some(ws) = top.get("workspace").and_then(toml::Value::as_table) {
        let strings = |key: &str| -> Vec<String> {
            ws.get(key)
                .and_then(toml::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let excluded: HashSet<String> = strings("exclude")
            .iter()
            .filter_map(|e| join(&dir, e))
            .collect();
        for pattern in strings("members") {
            for member in expand(root, &dir, &pattern) {
                if excluded.contains(&member) {
                    continue;
                }
                let rel = if member.is_empty() {
                    "Cargo.toml".to_string()
                } else {
                    format!("{member}/Cargo.toml")
                };
                if rel == manifest {
                    continue;
                }
                if let Some(table) = read_toml(&root.join(&rel)) {
                    add(rel, table);
                }
            }
        }
    }
    let crates: Vec<Crate> = found
        .iter()
        .map(|(name, (manifest, _))| Crate {
            name: name.clone(),
            manifest: manifest.clone(),
        })
        .collect();
    let index: HashMap<&str, usize> = crates
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.as_str(), i))
        .collect();
    let mut deps = Vec::new();
    for (i, (_, table)) in found.values().enumerate() {
        let mut targets: Vec<usize> = dependencies(table)
            .filter_map(|name| index.get(name.as_str()).copied())
            // Cargo rejects a package that depends on itself (quirk C2b).
            .filter(|&t| t != i)
            .collect();
        targets.sort_by(|&a, &b| crates[a].name.cmp(&crates[b].name));
        targets.dedup();
        deps.extend(targets.into_iter().map(|t| (i, t)));
    }
    Workspace {
        manifest: Some(manifest),
        crates,
        deps,
    }
}

/// Add the crates and their dependencies to `graph`; returns `(crates, dependencies)`.
pub fn apply(graph: &mut Graph, ws: &Workspace) -> (usize, usize) {
    // Each crate stands for the first node with its label and source file (rule C5).
    let mut by_label: HashMap<(&str, &str), usize> = HashMap::new();
    for (i, n) in graph.nodes.iter().enumerate() {
        by_label
            .entry((n.label.as_str(), n.source_file.as_str()))
            .or_insert(i);
    }
    let ids: Vec<String> = ws
        .crates
        .iter()
        .map(|c| {
            by_label
                .get(&(c.name.as_str(), c.manifest.as_str()))
                .map(|&i| graph.nodes[i].id.clone())
                .unwrap_or_else(|| format!("crate:{}", c.name))
        })
        .collect();
    drop(by_label);
    let existing: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let missing: Vec<Node> = ws
        .crates
        .iter()
        .zip(&ids)
        .filter(|(_, id)| !existing.contains(id.as_str()))
        .map(|(c, id)| Node {
            id: id.clone(),
            label: c.name.clone(),
            kind: NodeKind::Def,
            file_type: "concept".into(),
            source_file: c.manifest.clone(),
            loc: Loc::Line(1),
            attrs: Vec::new(),
        })
        .collect();
    drop(existing);
    graph.nodes.extend(missing);

    // A pair that already has a link keeps it (rule C6).
    let pair = |a: &str, b: &str| -> (String, String) {
        if a <= b {
            (a.to_string(), b.to_string())
        } else {
            (b.to_string(), a.to_string())
        }
    };
    let mut linked: HashSet<(String, String)> = graph
        .edges
        .iter()
        .map(|e| pair(&e.source, &e.target))
        .collect();
    for &(s, t) in &ws.deps {
        if !linked.insert(pair(&ids[s], &ids[t])) {
            continue;
        }
        graph.edges.push(Edge {
            source: ids[s].clone(),
            target: ids[t].clone(),
            relation: "crate_depends_on".into(),
            confidence: Confidence::Extracted,
            score: None,
            context: Some("cargo_dependency".into()),
            source_file: ws.crates[s].manifest.clone(),
            loc: Loc::Line(1),
            weight: Some(1.0),
            attrs: Vec::new(),
        });
    }
    (ws.crates.len(), ws.deps.len())
}

/// `Cargo.toml` at the root, else the first `*/Cargo.toml` or `*/*/Cargo.toml` by path
/// components (rule C1; hidden and ignored directories included).
fn find_manifest(root: &Path) -> Option<String> {
    if root.join("Cargo.toml").is_file() {
        return Some("Cargo.toml".into());
    }
    let mut found: Vec<Vec<String>> = Vec::new();
    for a in subdirs(root) {
        if root.join(&a).join("Cargo.toml").is_file() {
            found.push(vec![a.clone(), "Cargo.toml".into()]);
        }
        for b in subdirs(&root.join(&a)) {
            if root.join(&a).join(&b).join("Cargo.toml").is_file() {
                found.push(vec![a.clone(), b, "Cargo.toml".into()]);
            }
        }
    }
    found.sort();
    found.into_iter().next().map(|parts| parts.join("/"))
}

fn subdirs(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    out.sort();
    out
}

fn read_toml(path: &Path) -> Option<toml::Table> {
    fs::read_to_string(path).ok()?.parse::<toml::Table>().ok()
}

fn package_name(t: &toml::Table) -> Option<String> {
    t.get("package")?
        .as_table()?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

/// The package names a manifest's `[dependencies]` refer to (rule C3: not dev, build or
/// target-specific dependencies).
fn dependencies(t: &toml::Table) -> impl Iterator<Item = String> + '_ {
    t.get("dependencies")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .map(|(key, spec)| {
            spec.as_table()
                .and_then(|s| s.get("package"))
                .and_then(toml::Value::as_str)
                .unwrap_or(key)
                .to_string()
        })
}

fn parent(rel: &str) -> String {
    rel.rfind('/')
        .map_or(String::new(), |i| rel[..i].to_string())
}

/// `dir/rel` normalised (`.` and `..` resolved); `None` when it leaves the root.
fn join(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in rel.split('/') {
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

/// Directories (relative to the root) a `members` pattern names: `*` and `?` match within one
/// path segment.
fn expand(root: &Path, dir: &str, pattern: &str) -> Vec<String> {
    let Some(pattern) = join("", pattern) else {
        return Vec::new();
    };
    let Some(base) = join(dir, "") else {
        return Vec::new();
    };
    let mut current = vec![base];
    for seg in pattern.split('/').filter(|s| !s.is_empty()) {
        let mut next = Vec::new();
        for c in &current {
            if seg == ".." {
                if let Some(up) = join(c, "..") {
                    next.push(up);
                }
            } else if seg.contains(['*', '?']) {
                for d in subdirs(&root.join(c)) {
                    if glob_match(seg, &d) {
                        next.push(join(c, &d).unwrap_or_default());
                    }
                }
            } else if root.join(c).join(seg).is_dir() {
                next.push(join(c, seg).unwrap_or_default());
            }
        }
        current = next;
    }
    current.sort();
    current.dedup();
    current
}

fn glob_match(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, files: &[(&str, &str)]) {
        for (rel, text) in files {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
    }

    fn edges(ws: &Workspace) -> Vec<(String, String)> {
        ws.deps
            .iter()
            .map(|&(s, t)| (ws.crates[s].name.clone(), ws.crates[t].name.clone()))
            .collect()
    }

    #[test]
    fn dependency_kinds_order_and_self() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            &[
                (
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"omega\", \"zeta\", \"beta\", \"kappa\", \"mu\", \"nu\"]\n",
                ),
                (
                    "omega/Cargo.toml",
                    "[package]\nname = \"omega\"\n[dependencies]\nzeta = \"0.1\"\nbeta = { path = \"../beta\" }\nomega = { path = \".\" }\nserde = \"1\"\n[target.'cfg(unix)'.dependencies]\nkappa = { path = \"../kappa\" }\n[build-dependencies]\nmu = { path = \"../mu\" }\n[dev-dependencies]\nnu = { path = \"../nu\" }\n",
                ),
                (
                    "zeta/Cargo.toml",
                    "[package]\nname = \"zeta\"\n[dependencies]\nmu = \"1\"\nbeta = \"1\"\n",
                ),
                ("beta/Cargo.toml", "[package]\nname = \"beta\"\n"),
                ("kappa/Cargo.toml", "[package]\nname = \"kappa\"\n"),
                ("mu/Cargo.toml", "[package]\nname = \"mu\"\n"),
                ("nu/Cargo.toml", "[package]\nname = \"nu\"\n"),
            ],
        );
        let ws = scan(root);
        let names: Vec<&str> = ws.crates.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["beta", "kappa", "mu", "nu", "omega", "zeta"]);
        let e = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            edges(&ws),
            [
                e("omega", "beta"),
                e("omega", "zeta"),
                e("zeta", "beta"),
                e("zeta", "mu")
            ]
        );
    }

    #[test]
    fn manifest_lookup_globs_exclude_and_renames() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            &[
                ("zz/Cargo.toml", "[workspace]\nmembers = [\"m1\"]\n"),
                ("zz/m1/Cargo.toml", "[package]\nname = \"m1\"\n"),
                ("aa/deep/Cargo.toml", "[package]\nname = \"deepone\"\n"),
                ("bb/Cargo.toml", "[package]\nname = \"bbone\"\n"),
            ],
        );
        let ws = scan(root);
        assert_eq!(ws.manifest.as_deref(), Some("aa/deep/Cargo.toml"));
        assert_eq!(ws.crates.len(), 1);

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            &[
                (
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"crates/*\", \"missing\"]\nexclude = [\"crates/skip\"]\n",
                ),
                (
                    "crates/one/Cargo.toml",
                    "[package]\nname = \"one\"\n[dependencies]\ntwo-lib = { package = \"two\", path = \"../two\" }\nskip = { path = \"../skip\" }\n",
                ),
                ("crates/two/Cargo.toml", "[package]\nname = \"two\"\n"),
                ("crates/skip/Cargo.toml", "[package]\nname = \"skip\"\n"),
                ("crates/noname/Cargo.toml", "[package]\nversion = \"1.0.0\"\n"),
                ("crates/README.md", "# crates\n"),
            ],
        );
        let ws = scan(root);
        assert_eq!(ws.manifest.as_deref(), Some("Cargo.toml"));
        let names: Vec<&str> = ws.crates.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["one", "two"]);
        assert_eq!(edges(&ws), [("one".to_string(), "two".to_string())]);
        assert_eq!(ws.crates[0].manifest, "crates/one/Cargo.toml");

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(scan(dir.path()), Workspace::default());
    }

    #[test]
    fn globs() {
        assert!(glob_match("*", "a"));
        assert!(glob_match("cr?tes-*", "crates-x"));
        assert!(!glob_match("a*", "ba"));
    }
}

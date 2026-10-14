//! Cross-file pass for PHP (§19). See `docs/graphify-format.md`.
//!
//! - `@pi:<fqn>` (a class import, or a type named through one) binds to the class with that
//!   FQN, else to the shared stub `slug(fqn)` labelled with the FQN.
//! - `@ps:<name>` (a supertype or trait that is not imported) binds to the only class of that
//!   name, else to the shared stub `slug(name)`.
//! - A per-file stub (a type, `X::CONST` or `X::$p` naming no class of its file) binds to the
//!   only class, interface, trait or enum of that name in the repository.
//! - A bare call `f()` the file could not bind goes to the function or method of that name
//!   nearest to the caller's file ([`nearest`]; INFERRED 0.85).
//! - `new X` / `X::m()` (any qualification, last segment) bind to class `X`: through a
//!   non-aliased import of `X` (EXTRACTED 1.0), else, unless the file writes `X` as a type, to
//!   the nearest class of that name (INFERRED 0.85). Aliased names bind nothing.
//! - Per-file stubs reserved for collision renaming but never linked are removed.
//! - A cross-file call is dropped when the caller already links to its target in the file
//!   (a type reference, a constant use), and kept once per (caller, target).

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::php::{PCallKind, PhpFacts, IMPORT, SUPER};
use crate::extract::{origin, slug};
use crate::facts::{Confidence, Edge, LangFacts, Loc, Node, NodeKind};
use crate::lang::Lang;

/// Redirect target that removes a node (nothing links to it).
const DROP: &str = "@drop";

fn php_facts<'v>(v: &'v View) -> Option<&'v PhpFacts> {
    match &v.facts.lang {
        LangFacts::Php(p) => Some(p),
        _ => None,
    }
}

pub(crate) fn stub_node(id: String, label: String) -> Node {
    Node {
        id,
        label,
        kind: NodeKind::Stub,
        file_type: "code".into(),
        source_file: String::new(),
        loc: Loc::Empty,
        attrs: vec![origin()],
    }
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// A test or spec file: a `test`, `tests`, `spec`, `specs` or `__tests__` directory, or a
/// `*_test`, `*_spec`, `test_*`, `*.test.*` or `*.spec.*` file name.
fn is_test_path(path: &str) -> bool {
    let mut parts = path.split('/').collect::<Vec<_>>();
    let file = parts.pop().unwrap_or("");
    if parts
        .iter()
        .any(|d| matches!(*d, "test" | "tests" | "spec" | "specs" | "__tests__"))
    {
        return true;
    }
    let stem = file.split('.').next().unwrap_or(file);
    stem.ends_with("_test")
        || stem.ends_with("_spec")
        || stem.starts_with("test_")
        || file.contains(".test.")
        || file.contains(".spec.")
}

/// (the candidate is a test file exactly when the caller is, shared leading directory
/// components, same directory, same file).
type Proximity = (bool, usize, bool, bool);

fn proximity(from: &str, to: &str) -> Proximity {
    let (a, b) = (dir_of(from), dir_of(to));
    let shared = a
        .split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y && !x.is_empty())
        .count();
    (
        is_test_path(from) == is_test_path(to),
        shared,
        a == b,
        from == to,
    )
}

/// The candidate `(id, path)` nearest to `from`, if no other is as near (see [`proximity`]):
/// from a spec file, `test/z.rb` beats `rack-protection/lib/x.rb` although the latter shares a
/// directory with the caller; two candidates equally near bind nothing.
pub(crate) fn nearest(from: &str, cands: &[(String, String)]) -> Option<String> {
    let mut best: Option<(Proximity, &str)> = None;
    let mut tied = false;
    let mut seen: HashSet<&str> = HashSet::new();
    for (id, rel) in cands {
        if !seen.insert(id.as_str()) {
            continue;
        }
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
    let (_, id) = best?;
    (!tied).then(|| id.to_string())
}

/// A cross-file edge.
pub(crate) fn cross_edge(
    source: String,
    target: String,
    relation: &str,
    (rel, line): (&str, u32),
    context: Option<&str>,
    confidence: Confidence,
    score: f64,
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
    let php: Vec<(&View, &PhpFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Php)
        .filter_map(|v| php_facts(v).map(|p| (v, p)))
        .collect();
    if php.is_empty() {
        return res;
    }

    let mut by_fqn: HashMap<&str, String> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    let mut callables: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    for (v, p) in &php {
        let rel = v.file.rel.clone();
        for c in &p.classes {
            let id = v.id(&c.id).to_string();
            by_fqn.entry(c.fqn.as_str()).or_insert_with(|| id.clone());
            by_name
                .entry(c.name.as_str())
                .or_default()
                .push((id, rel.clone()));
        }
        for (name, local) in &p.callables {
            callables
                .entry(name.as_str())
                .or_default()
                .push((v.id(local).to_string(), rel.clone()));
        }
    }
    let unique = |name: &str| -> Option<String> {
        let v = by_name.get(name)?;
        let first = &v.first()?.0;
        v.iter().all(|(id, _)| id == first).then(|| first.clone())
    };

    // Imported classes and supertypes, then per-file stubs.
    let mut redirect: HashMap<String, String> = HashMap::new();
    let mut import_stubs: HashSet<String> = HashSet::new();
    for (v, _) in &php {
        for e in &v.facts.edges {
            let t = e.target.as_str();
            if redirect.contains_key(t) {
                continue;
            }
            if let Some(name) = t.strip_prefix(SUPER) {
                let target = unique(name).unwrap_or_else(|| {
                    let id = slug(name);
                    if import_stubs.insert(id.clone()) {
                        res.nodes.push(stub_node(id.clone(), name.to_string()));
                    }
                    id
                });
                redirect.insert(t.to_string(), target);
                continue;
            }
            let Some(fqn) = t.strip_prefix(IMPORT) else {
                continue;
            };
            let target = match by_fqn.get(fqn) {
                Some(id) => id.clone(),
                None => {
                    let id = slug(fqn);
                    if import_stubs.insert(id.clone()) {
                        res.nodes.push(stub_node(id.clone(), fqn.to_string()));
                    }
                    id
                }
            };
            redirect.insert(t.to_string(), target);
        }
    }
    for (v, p) in &php {
        for (local, label, linked) in &p.stubs {
            let stub = v.id(local);
            if !index
                .get(files, stub)
                .is_some_and(|n| n.kind == NodeKind::Stub)
            {
                continue;
            }
            if !linked {
                // Reserved only for collision renaming (see `extract::php`): never emitted,
                // unless a shared stub of the same id exists.
                if !import_stubs.contains(stub) {
                    redirect.insert(stub.to_string(), DROP.to_string());
                }
            } else if let Some(c) = unique(label) {
                redirect.insert(stub.to_string(), c);
            }
        }
    }

    // Calls the files could not bind.
    for (v, p) in &php {
        let rel = v.file.rel.as_str();
        let final_id = |t: &str| -> String { redirect.get(t).cloned().unwrap_or_else(|| t.into()) };
        let mut linked: HashSet<(String, String)> = v
            .facts
            .edges
            .iter()
            .map(|e| (e.source.clone(), final_id(&e.target)))
            .collect();
        let known: HashSet<&str> = p.known.iter().map(String::as_str).collect();
        let imported: HashMap<&str, &str> = p
            .imports
            .iter()
            .filter(|(_, alias)| alias.is_none())
            .map(|(fqn, _)| (fqn.rsplit('\\').next().unwrap_or(fqn), fqn.as_str()))
            .collect();
        for c in &p.calls {
            let caller = v.id(&c.caller).to_string();
            let found = match &c.kind {
                PCallKind::Bare(name) => callables
                    .get(name.as_str())
                    .and_then(|cands| nearest(rel, cands))
                    .map(|t| (t, Confidence::Inferred, 0.85)),
                PCallKind::Class(written) => {
                    let short = written.rsplit('\\').next().unwrap_or(written);
                    match imported.get(short).and_then(|fqn| by_fqn.get(fqn)) {
                        Some(t) => Some((t.clone(), Confidence::Extracted, 1.0)),
                        // A name the file uses as a type binds to that type's stub in the
                        // file, never by name across files.
                        None if known.contains(short) => None,
                        None => by_name
                            .get(short)
                            .and_then(|cands| nearest(rel, cands))
                            .map(|t| (t, Confidence::Inferred, 0.85)),
                    }
                }
            };
            let Some((target, confidence, score)) = found else {
                continue;
            };
            if !linked.insert((caller.clone(), target.clone())) {
                continue;
            }
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
    res.redirects.extend(redirect);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands(paths: &[&str]) -> Vec<(String, String)> {
        paths
            .iter()
            .map(|p| (format!("id:{p}"), p.to_string()))
            .collect()
    }

    #[test]
    fn nearest_prefers_test_kind_then_prefix() {
        let from = "rack-protection/spec/support/x.rb";
        // A test file beats a non-test file sharing a directory prefix.
        assert_eq!(
            nearest(
                from,
                &cands(&["rack-protection/lib/rack/a.rb", "test/b.rb"])
            ),
            Some("id:test/b.rb".into())
        );
        // Among test files, the longer shared prefix wins; equal ones tie.
        assert_eq!(
            nearest(from, &cands(&["test/b.rb", "rack-protection/spec/c.rb"])),
            Some("id:rack-protection/spec/c.rb".into())
        );
        assert_eq!(nearest(from, &cands(&["test/b.rb", "spec/c.rb"])), None);
        // A single candidate is always taken; the same file beats the same directory.
        assert_eq!(
            nearest("lib/a.rb", &cands(&["other/z.rb"])),
            Some("id:other/z.rb".into())
        );
        assert_eq!(
            nearest("a.rb", &cands(&["b.rb", "a.rb"])),
            Some("id:a.rb".into())
        );
    }

    #[test]
    fn test_paths() {
        for p in [
            "test/a.rb",
            "x/spec/b.rb",
            "a_test.go",
            "src/__tests__/x.ts",
            "foo_spec.rb",
        ] {
            assert!(is_test_path(p), "{p}");
        }
        for p in ["lib/a.rb", "src/testing/x.php", "contest.rb"] {
            assert!(!is_test_path(p), "{p}");
        }
    }
}

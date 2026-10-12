//! Cross-file pass for JSON, TOML and shell (`docs/graphify-format.md` §14, §11.6):
//!
//! - `depends_on` between `Cargo.toml`/`pyproject.toml` package nodes, matched on `slug(name)`.
//! - Shell `source`/`.`: `imports_from` to the sourced file (EXTRACTED for `./`, `../` and
//!   absolute paths, INFERRED for bare paths found next to the script); a bare path that does not
//!   exist becomes an `imports` edge to an external named after it. Existence is checked on disk,
//!   so a sourced file that is not indexed still gets an external node with its file id. Paths
//!   expanded from a leading variable are INFERRED and dropped when missing. A sourcing script
//!   never imports itself (the build would drop the self-loop).
//! - Shell calls into directly sourced scripts, when exactly one of them defines the name.
//! - Shell scripts run as commands: `calls` from the caller to the script's entry node, context
//!   `script_invocation`, when the path names an indexed shell file.
//! - JSON `ref_*` nodes with the same label (case-insensitive) in several files merge into the one
//!   with the shortest id. Labels with Shannon entropy under 2.5 bits per character (`react`,
//!   `effect`, `express`) stay per file. Graphify also merges some near-identical long labels
//!   (`@testing-library/jest-dom` into `@testing-library/dom`); no similarity threshold fits the
//!   probes (it looks like locality-sensitive hashing), so that is not reproduced.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use super::{Index, Resolution, View};
use crate::extract::data::{DataFacts, Sourced};
use crate::extract::{external_node, slug};
use crate::facts::{Confidence, Edge, LangFacts, Loc, NodeKind};
use crate::graph::norm_label;
use crate::ids::file_id;
use crate::lang::Lang;

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut out = Resolution::default();
    depends_on(files, &mut out);
    shell(files, index, &mut out);
    merge_refs(files, &mut out);
    out
}

fn facts<'a>(v: &'a View) -> Option<&'a DataFacts> {
    match &v.facts.lang {
        LangFacts::Data(d) => Some(d),
        _ => None,
    }
}

fn edge(
    v: &View,
    source: &str,
    target: &str,
    relation: &str,
    confidence: Confidence,
    line: u32,
    context: &str,
) -> Edge {
    Edge {
        source: source.into(),
        target: target.into(),
        relation: relation.into(),
        confidence,
        score: (confidence == Confidence::Extracted).then_some(1.0),
        context: Some(context.into()),
        source_file: v.file.rel.clone(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![crate::extract::origin()],
    }
}

fn depends_on(files: &[View], out: &mut Resolution) {
    let mut by_key: HashMap<&str, &str> = HashMap::new();
    for v in files {
        if let Some(p) = facts(v).and_then(|d| d.package.as_ref()) {
            by_key.entry(p.key.as_str()).or_insert(v.id(&p.id));
        }
    }
    for v in files {
        let Some(p) = facts(v).and_then(|d| d.package.as_ref()) else {
            continue;
        };
        let me = v.id(&p.id);
        for dep in &p.deps {
            if *dep == p.key {
                continue;
            }
            if let Some(target) = by_key.get(dep.as_str()) {
                out.edges.push(edge(
                    v,
                    me,
                    target,
                    "depends_on",
                    Confidence::Extracted,
                    1,
                    "dependency",
                ));
            }
        }
    }
}

/// Where a `source` path points.
enum Target {
    /// A path that exists on disk: target id, its repo-relative path when inside the root, and
    /// the edge confidence.
    File(String, Option<String>, Confidence),
    /// A bare path found nowhere: external `slug(path)`.
    Missing(String),
}

fn shell(files: &[View], index: &Index, out: &mut Resolution) {
    let shells: HashMap<&str, &View> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Shell)
        .map(|v| (v.file.rel.as_str(), v))
        .collect();
    let mut externals: HashSet<String> = HashSet::new();
    for v in files.iter().filter(|v| v.file.lang == Lang::Shell) {
        let Some(d) = facts(v) else { continue };
        let Some(from) = index.file_ids.get(&v.file.rel) else {
            continue;
        };
        let mut sourced: Vec<&View> = Vec::new();
        for s in &d.sources {
            let (relation, target, confidence) = match locate(v, s, index) {
                Some(Target::File(id, rel, conf)) => {
                    if let Some(rel) = rel {
                        if let Some(sv) = shells.get(rel.as_str()) {
                            if !sourced.iter().any(|x| std::ptr::eq(*x, *sv)) {
                                sourced.push(sv);
                            }
                        }
                    }
                    ("imports_from", id, conf)
                }
                Some(Target::Missing(id)) => ("imports", id, Confidence::Extracted),
                None => continue,
            };
            // A script sourcing itself: the build drops the self-loop (§8.3).
            if target == *from {
                continue;
            }
            if !index.contains(&target) && externals.insert(target.clone()) {
                out.nodes.push(external_node(target.clone()));
            }
            let mut e = edge(v, from, &target, relation, confidence, s.line, "import");
            e.score = None;
            out.edges.push(e);
        }
        if let Some(dir) = v.file.abs.parent() {
            let root = root_of(v);
            for c in &d.scripts {
                let candidate = normalize(&dir.join(&c.name));
                let rel = root
                    .as_deref()
                    .and_then(|r| candidate.strip_prefix(r).ok())
                    .map(|r| r.to_string_lossy().replace('\\', "/"));
                let Some(sv) = rel.and_then(|r| shells.get(r.as_str()).copied()) else {
                    continue;
                };
                let entry = format!("{}__entry", slug(&sv.file.rel));
                let (caller, target) = (v.id(&c.caller), sv.id(&entry));
                if caller != target {
                    out.edges.push(edge(
                        v,
                        caller,
                        target,
                        "calls",
                        Confidence::Extracted,
                        c.line,
                        "script_invocation",
                    ));
                }
            }
        }
        if d.calls.is_empty() || sourced.is_empty() {
            continue;
        }
        let mut defs: HashMap<&str, Vec<&str>> = HashMap::new();
        for sv in &sourced {
            if let Some(sd) = facts(sv) {
                for (name, id) in &sd.functions {
                    let id = sv.id(id);
                    let slot = defs.entry(name.as_str()).or_default();
                    if !slot.contains(&id) {
                        slot.push(id);
                    }
                }
            }
        }
        for c in &d.calls {
            if let Some([target]) = defs.get(c.name.as_str()).map(Vec::as_slice) {
                out.edges.push(edge(
                    v,
                    v.id(&c.caller),
                    target,
                    "calls",
                    Confidence::Extracted,
                    c.line,
                    "call",
                ));
            }
        }
    }
}

/// Resolve a `source` path the way Graphify does: relative to the script's directory, with
/// existence checked on disk. Variable-expanded paths are always INFERRED and never external.
fn locate(v: &View, s: &Sourced, index: &Index) -> Option<Target> {
    let path = s.path.as_str();
    let explicit = path.starts_with("./") || path.starts_with("../");
    let absolute = path.starts_with('/');
    let dir = v.file.abs.parent()?;
    let candidate = if absolute {
        PathBuf::from(path)
    } else {
        dir.join(path)
    };
    let candidate = normalize(&candidate);
    if !candidate.is_file() {
        return (!explicit && !absolute && !s.variable).then(|| Target::Missing(slug(path)));
    }
    let confidence = if (explicit || absolute) && !s.variable {
        Confidence::Extracted
    } else {
        Confidence::Inferred
    };
    let root = root_of(v)?;
    Some(match candidate.strip_prefix(&root) {
        Ok(rel) => {
            let rel = rel.to_string_lossy().replace('\\', "/");
            let id = index
                .file_ids
                .get(&rel)
                .cloned()
                .unwrap_or_else(|| file_id(&rel));
            Target::File(id, Some(rel), confidence)
        }
        Err(_) => {
            let stem = candidate.file_stem()?.to_string_lossy().into_owned();
            Target::File(format!("ext_{}", slug(&stem)), None, confidence)
        }
    })
}

/// The scan root: the file's absolute path minus its relative path.
fn root_of(v: &View) -> Option<PathBuf> {
    let depth = v.file.rel.split('/').count();
    v.file.abs.ancestors().nth(depth).map(Path::to_path_buf)
}

/// Lexical `.`/`..` normalization (like `os.path.normpath`).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Shannon entropy in bits per character.
fn entropy(s: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();
    let mut n = 0usize;
    for c in s.chars() {
        *counts.entry(c).or_default() += 1;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    counts
        .values()
        .map(|&k| {
            let p = k as f64 / n as f64;
            -p * p.log2()
        })
        .sum()
}

/// Same-label JSON reference nodes in different files merge into the shortest id.
fn merge_refs(files: &[View], out: &mut Resolution) {
    let mut groups: HashMap<String, Vec<&str>> = HashMap::new();
    for v in files.iter().filter(|v| v.file.lang == Lang::Json) {
        for n in &v.facts.nodes {
            if n.kind == NodeKind::Def && n.file_type == "concept" {
                let slot = groups.entry(norm_label(&n.label)).or_default();
                if !slot.contains(&n.id.as_str()) {
                    slot.push(&n.id);
                }
            }
        }
    }
    let mut merged: Vec<(String, String)> = Vec::new();
    for (label, ids) in groups {
        if ids.len() < 2 || entropy(&label) < 2.5 {
            continue;
        }
        let keep = ids
            .iter()
            .min_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)))
            .copied()
            .unwrap_or_default();
        for id in ids {
            if id != keep {
                merged.push((id.to_string(), keep.to_string()));
            }
        }
    }
    merged.sort();
    out.redirects.extend(merged);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_threshold_matches_observed_merges() {
        for merged in ["lodash", "requests", "typescript", "effect-ts", "reacts"] {
            assert!(entropy(merged) >= 2.5, "{merged}");
        }
        for kept in [
            "react",
            "effect",
            "express",
            "expressjs",
            "a-b-c-d",
            "server",
        ] {
            assert!(entropy(kept) < 2.5, "{kept}");
        }
    }

    #[test]
    fn normalize_is_lexical() {
        assert_eq!(
            normalize(Path::new("/r/a/./sub/../l.sh")),
            Path::new("/r/a/l.sh")
        );
    }
}

//! Cross-file pass for Rust (§11). See `docs/graphify-format.md`.
//!
//! 1. External nodes for `use` leaves (appended after every per-file node, so a type-reference
//!    stub with the same id wins, §11.2 rule 1).
//! 2. Type-stub resolution (§11.2 rule 3): a stub whose label names exactly one Rust struct, enum
//!    or trait (or, when no type has that name, exactly one other Rust definition: fn, const,
//!    variant, macro, impl owner) is deleted and redirected to it. Import edges that target the
//!    same id follow (rule 4).
//! 3. Bare calls left unresolved in-file (§11.5): among the Rust definitions labelled `name()`,
//!    `.name()` or `Name`, the one closest to the caller (longest shared directory prefix, then
//!    same directory, then same file) gets an INFERRED `calls` or `references` (`constructor`)
//!    edge with score 0.85. A tie for closest resolves nothing.
//! 4. `self.name()` inside an impl whose file has no method `name`: when the impl's type (its
//!    owner's `_rust_impl_key` name, else its label) names exactly one declared struct, enum or
//!    trait, the unique method `name` among all impls of that type name gets an EXTRACTED `calls`
//!    edge with score 1.0 (`impl Atlas` split across `lib.rs` and `rank.rs`).

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::{external_node, origin};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc, NodeKind};
use crate::lang::Family;

const INFERRED_SCORE: f64 = 0.85;

/// One definition a name can resolve to.
#[derive(Clone, Copy)]
struct Target<'a> {
    id: &'a str,
    /// A function or method (`calls`) rather than a type, variant or const (`constructor`).
    is_call: bool,
    /// Path of the defining file.
    path: &'a str,
}

/// Distinct ids per name, in first-seen order.
#[derive(Default)]
struct Candidates<'a>(HashMap<&'a str, Vec<Target<'a>>>);

impl<'a> Candidates<'a> {
    fn add(&mut self, key: &'a str, t: Target<'a>) {
        let v = self.0.entry(key).or_default();
        if !v.iter().any(|x| x.id == t.id) {
            v.push(t);
        }
    }

    fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    fn unique(&self, key: &str) -> Option<Target<'a>> {
        match self.0.get(key) {
            Some(v) if v.len() == 1 => Some(v[0]),
            _ => None,
        }
    }

    /// The single candidate nearest to `from`, if one is strictly nearer than the rest.
    fn closest(&self, key: &str, from: &str) -> Option<Target<'a>> {
        let v = self.0.get(key)?;
        if v.len() == 1 {
            return Some(v[0]);
        }
        let mut best: Option<((usize, bool, bool), Target<'a>)> = None;
        let mut tied = false;
        for t in v {
            let score = proximity(from, t.path);
            match &best {
                Some((s, _)) if score < *s => {}
                Some((s, _)) if score == *s => tied = true,
                _ => {
                    best = Some((score, *t));
                    tied = false;
                }
            }
        }
        if tied {
            None
        } else {
            best.map(|(_, t)| t)
        }
    }
}

/// `Pair/1` (an owner's `_rust_impl_key`) → `Pair`; a plain owner label is already the name.
fn type_name(key: &str) -> &str {
    key.split_once('/').map_or(key, |(name, _)| name)
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// (shared leading directory components, same directory, same file).
fn proximity(from: &str, to: &str) -> (usize, bool, bool) {
    let (a, b) = (dir_of(from), dir_of(to));
    let shared = a
        .split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y && !x.is_empty())
        .count();
    (shared, a == b, from == to)
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut out = Resolution::default();
    let rust: Vec<&View> = files
        .iter()
        .filter(|v| v.family() == Family::Rust)
        .collect();
    if rust.is_empty() {
        return out;
    }

    // Definitions by name, first occurrence of each id only (the build keeps the first node).
    // Stubs prefer declared types (struct/enum/trait nodes) over any other definition.
    let mut type_targets = Candidates::default();
    let mut stub_targets = Candidates::default();
    let mut call_targets = Candidates::default();
    let mut seen: HashSet<&str> = HashSet::new();
    for v in &rust {
        let path = v.file.rel.as_str();
        for n in &v.facts.nodes {
            if n.kind != NodeKind::Def || !seen.insert(n.id.as_str()) {
                continue;
            }
            let label = n.label.as_str();
            let t = |is_call| Target {
                id: n.id.as_str(),
                is_call,
                path,
            };
            if n.attr("_rust_declaration_count").is_some() {
                type_targets.add(label.strip_suffix("()").unwrap_or(label), t(false));
            }
            if let Some(name) = label.strip_prefix('.') {
                if let Some(name) = name.strip_suffix("()") {
                    call_targets.add(name, t(true));
                }
            } else if let Some(name) = label.strip_suffix("()") {
                stub_targets.add(name, t(false));
                call_targets.add(name, t(true));
            } else if let Some(name) = label.strip_suffix('!') {
                stub_targets.add(name, t(false));
            } else {
                stub_targets.add(label, t(false));
                call_targets.add(label, t(false));
            }
        }
    }

    let mut externals: HashSet<&str> = HashSet::new();
    let mut redirected: HashSet<&str> = HashSet::new();
    for v in &rust {
        let LangFacts::Rust(facts) = &v.facts.lang else {
            continue;
        };
        for id in &facts.externals {
            if externals.insert(id) {
                out.nodes.push(external_node(id.clone()));
            }
        }
        for n in &v.facts.nodes {
            if n.kind != NodeKind::Stub || redirected.contains(n.id.as_str()) {
                continue;
            }
            // Only a stub that is the node the build keeps for this id may be replaced.
            if index.get(files, &n.id).map(|k| k.kind) != Some(NodeKind::Stub) {
                continue;
            }
            let target = if type_targets.has(&n.label) {
                type_targets.unique(&n.label)
            } else {
                stub_targets.unique(&n.label)
            };
            if let Some(t) = target {
                redirected.insert(&n.id);
                out.redirects.push((n.id.clone(), t.id.to_string()));
            }
        }
    }

    // Methods by (owner type key, name): the key is the owner's `_rust_impl_key`, else its label.
    let mut type_methods: HashMap<(&str, &str), Vec<&str>> = HashMap::new();
    for v in &rust {
        for e in v.facts.edges.iter().filter(|e| e.relation == "method") {
            let (Some(owner), Some(method)) =
                (index.get(files, &e.source), index.get(files, &e.target))
            else {
                continue;
            };
            let Some(name) = method
                .label
                .strip_prefix('.')
                .and_then(|l| l.strip_suffix("()"))
            else {
                continue;
            };
            let key = match owner.attr("_rust_impl_key") {
                Some(Attr::Str(k)) => k.as_str(),
                _ => owner.label.as_str(),
            };
            let ids = type_methods.entry((type_name(key), name)).or_default();
            if !ids.contains(&method.id.as_str()) {
                ids.push(&method.id);
            }
        }
    }

    for v in &rust {
        let LangFacts::Rust(facts) = &v.facts.lang else {
            continue;
        };
        let mut done: HashSet<(&str, &str)> = HashSet::new();
        for call in &facts.self_calls {
            // The impl's type must name exactly one declared struct/enum/trait in the repo.
            let ty = type_name(&call.owner_key);
            if !matches!(type_targets.0.get(ty).map(Vec::len), Some(1)) {
                continue;
            }
            let Some([target]) = type_methods
                .get(&(ty, call.name.as_str()))
                .map(Vec::as_slice)
            else {
                continue;
            };
            let caller = v.id(&call.caller);
            if caller == *target || !done.insert((caller, target)) {
                continue;
            }
            out.edges.push(Edge {
                source: caller.to_string(),
                target: target.to_string(),
                relation: "calls".into(),
                confidence: Confidence::Extracted,
                score: Some(1.0),
                context: Some("call".into()),
                source_file: v.file.rel.clone(),
                loc: Loc::Line(call.line),
                weight: Some(1.0),
                attrs: vec![origin()],
            });
        }
        for call in &facts.calls {
            let Some(t) = call_targets.closest(&call.name, &v.file.rel) else {
                continue;
            };
            let caller = v.id(&call.caller);
            if caller == t.id || !done.insert((caller, t.id)) {
                continue;
            }
            let (relation, context) = if t.is_call {
                ("calls", "call")
            } else {
                ("references", "constructor")
            };
            out.edges.push(Edge {
                source: caller.to_string(),
                target: t.id.to_string(),
                relation: relation.into(),
                confidence: Confidence::Inferred,
                score: Some(INFERRED_SCORE),
                context: Some(context.into()),
                source_file: v.file.rel.clone(),
                loc: Loc::Line(call.line),
                weight: Some(1.0),
                attrs: vec![origin()],
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t<'a>(id: &'a str, path: &'a str) -> Target<'a> {
        Target {
            id,
            is_call: true,
            path,
        }
    }

    #[test]
    fn closest_candidate_wins_and_ties_resolve_nothing() {
        let mut c = Candidates::default();
        // Same directory beats a deeper directory with the same shared prefix.
        c.add("p2", t("deep", "src/sub/deep/g.rs"));
        c.add("p2", t("near", "src/h.rs"));
        assert_eq!(c.closest("p2", "src/a.rs").map(|t| t.id), Some("near"));
        // A longer shared prefix beats a shorter one.
        c.add("q1", t("x", "src/a/x.rs"));
        c.add("q1", t("e", "src/a/b/d/e.rs"));
        assert_eq!(c.closest("q1", "src/a/b/c.rs").map(|t| t.id), Some("e"));
        // Two sibling directories tie; nothing shared ties too.
        c.add("m", t("s1", "src/sub/e.rs"));
        c.add("m", t("s2", "src/sub2/f.rs"));
        assert!(c.closest("m", "src/a.rs").is_none());
        c.add("f", t("f1", "x/y/c.rs"));
        c.add("f", t("f2", "x/d.rs"));
        assert!(c.closest("f", "src/a.rs").is_none());
        // The same file beats the same directory.
        c.add("s", t("same", "src/a.rs"));
        c.add("s", t("dir", "src/j.rs"));
        assert_eq!(c.closest("s", "src/a.rs").map(|t| t.id), Some("same"));
    }

    /// Extract and resolve a small Rust repository; returns `source -relation-> target [conf]`.
    fn build(files: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
        use crate::discover::SourceFile;
        use crate::extract::extract_file;
        use crate::lang::Lang;
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|(rel, _)| SourceFile {
                rel: rel.to_string(),
                abs: rel.into(),
                lang: Lang::Rust,
            })
            .collect();
        let input = sources
            .iter()
            .zip(files)
            .map(|(sf, (rel, src))| (sf, extract_file(Lang::Rust, rel, src.as_bytes())))
            .collect();
        let g = crate::resolve::resolve(input);
        let nodes = g.nodes.iter().map(|n| n.id.clone()).collect();
        let edges = g
            .edges
            .iter()
            .map(|e| {
                format!(
                    "{} -{}-> {} [{}]",
                    e.source,
                    e.relation,
                    e.target,
                    e.confidence.as_str()
                )
            })
            .collect();
        (nodes, edges)
    }

    #[test]
    fn stubs_imports_and_calls_resolve_across_files() {
        let (nodes, edges) = build(&[
            (
                "src/a.rs",
                "use crate::b::Thing;\nfn f(t: Thing, r: Round) -> Shared { helper(); Meters(1); }\n\
                 impl Atlas { fn rank(&self) { self.files(); } }\n",
            ),
            (
                "src/b.rs",
                "pub struct Thing;\npub enum E { Round }\npub struct Meters(u8);\n\
                 pub fn helper() {}\npub struct Atlas;\nimpl Atlas { fn files(&self) {} }\n",
            ),
            // `Shared` is declared twice: its stub stays. `Round` the struct beats `E::Round`.
            ("src/c.rs", "pub struct Shared;\npub struct Round;\n"),
            ("src/d/e.rs", "pub struct Shared;\npub fn helper() {}\n"),
        ]);
        let has = |e: &str| edges.iter().any(|x| x == e);
        // The import of a name used as a type in that file follows the resolved stub.
        assert!(has("src_a -imports_from-> src_b_thing [EXTRACTED]"));
        assert!(has("src_a_f -references-> src_b_thing [EXTRACTED]"));
        assert!(has("src_a_f -references-> src_c_round [EXTRACTED]"));
        assert!(has("src_a_f -references-> shared [EXTRACTED]"));
        assert!(nodes.iter().any(|n| n == "shared"));
        assert!(!nodes.iter().any(|n| n == "thing"));
        // `helper` exists in src/b.rs and src/d/e.rs: the same directory wins.
        assert!(has("src_a_f -calls-> src_b_helper [INFERRED]"));
        assert!(has("src_a_f -references-> src_b_meters [INFERRED]"));
        assert!(has(
            "src_a_atlas_rank -calls-> src_b_atlas_files [EXTRACTED]"
        ));
    }
}

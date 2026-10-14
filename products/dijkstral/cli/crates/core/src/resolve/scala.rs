//! Cross-file pass for Scala (§22). See `docs/graphify-format.md`.
//!
//! - Imports target the external node of the path's first segment (`import shop.model.User`
//!   → `shop`), or the only file with that stem (§8.1).
//! - Per-file type stubs bind like Kotlin's: the only definition labelled `Name`, else the only
//!   function `Name()`.
//! - Bare calls the file could not bind bind INFERRED 0.85 to the nearest definition of that
//!   name (path proximity; a tie binds nothing), unless a type of that name is used in the file
//!   or the caller already has an edge to it. Imports and member calls never bind calls.

use std::collections::{HashMap, HashSet};

use super::kotlin::{edge, pick, shared, DROP};
use super::{Index, Resolution, View};
use crate::extract::external_node;
use crate::extract::scala::{ScalaFacts, VIRTUAL};
use crate::facts::{Confidence, LangFacts, NodeKind};
use crate::lang::Lang;

fn scala_facts<'v>(v: &'v View) -> Option<&'v ScalaFacts> {
    match &v.facts.lang {
        LangFacts::Scala(s) => Some(s),
        _ => None,
    }
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let scala: Vec<(&View, &ScalaFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Scala)
        .filter_map(|v| scala_facts(v).map(|s| (v, s)))
        .collect();
    if scala.is_empty() {
        return res;
    }
    let sh = shared(files);

    let mut redirect: HashMap<String, String> = HashMap::new();
    let mut externals: HashSet<String> = HashSet::new();
    for (v, s) in &scala {
        let file_id = v.id(&s.file_id).to_string();
        for e in &v.facts.edges {
            let Some(body) = e.target.strip_prefix(VIRTUAL) else {
                continue;
            };
            let Some((_, ext)) = body.split_once('|') else {
                continue;
            };
            // A node with that id (another file's stub `diff`), else the only file with that
            // stem, else the external node; an import of the file itself is dropped.
            let target = if index.contains(ext) {
                ext.to_string()
            } else {
                match sh.stem_target(ext, Lang::Scala) {
                    Some(t) => t,
                    None => {
                        if externals.insert(ext.to_string()) {
                            res.nodes.push(external_node(ext.to_string()));
                        }
                        ext.to_string()
                    }
                }
            };
            let target = if target == file_id {
                DROP.to_string()
            } else {
                target
            };
            redirect.insert(e.target.clone(), target);
        }
        for (local, label) in &s.stubs {
            let stub = v.id(local);
            if !index
                .get(files, stub)
                .is_some_and(|n| n.kind == NodeKind::Stub)
            {
                continue;
            }
            if let Some(t) = sh.stub_target(label) {
                redirect.insert(stub.to_string(), t);
            }
        }
    }
    let follow = |id: &str| -> String {
        let mut cur = id;
        for _ in 0..4 {
            match redirect.get(cur) {
                Some(next) => cur = next,
                None => break,
            }
        }
        cur.to_string()
    };

    let mut pairs: HashSet<(String, String)> = HashSet::new();
    for (v, _) in &scala {
        for e in &v.facts.edges {
            pairs.insert((e.source.clone(), follow(&e.target)));
        }
    }
    for (v, s) in &scala {
        let rel = v.file.rel.as_str();
        for c in &s.calls {
            if c.via_stub {
                continue;
            }
            let caller = v.id(&c.caller).to_string();
            let Some(target) = sh
                .by_name
                .get(c.name.as_str())
                .and_then(|cands| pick(rel, &caller, cands))
            else {
                continue;
            };
            if !pairs.insert((caller.clone(), target.clone())) {
                continue;
            }
            res.edges.push(edge(
                caller,
                target,
                (rel, c.line),
                Confidence::Inferred,
                0.85,
            ));
        }
    }
    res.redirects.extend(redirect);
    res
}

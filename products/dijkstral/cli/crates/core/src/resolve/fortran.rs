//! Cross-file pass for Fortran (§35). See `docs/graphify-format.md`.
//!
//! Graphify resolves nothing else across Fortran files (calls, `use` and type-bound targets stay
//! in their file): a per-file type stub (`type(T)` with no `T` emitted earlier in the file, or a
//! type-bound or generic target missing from the file) binds to the only definition of any
//! language labelled `t`, else the only function `t()` — a module, a `use` node, a type, a
//! procedure, a JSON key. Labels compare exactly, so a Python class `Widget` never takes the
//! lower-cased stub `widget`; an ambiguous or unknown name stays a stub.

use super::kotlin::shared;
use super::{Index, Resolution, View};
use crate::facts::{LangFacts, NodeKind};

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let stubs: Vec<(&View, &(String, String))> = files
        .iter()
        .filter_map(|v| match &v.facts.lang {
            LangFacts::Fortran(f) => Some((v, f)),
            _ => None,
        })
        .flat_map(|(v, f)| f.stubs.iter().map(move |s| (v, s)))
        .collect();
    if stubs.is_empty() {
        return res;
    }
    let sh = shared(files);
    for (v, (local, label)) in stubs {
        let stub = v.id(local);
        if !index
            .get(files, stub)
            .is_some_and(|n| n.kind == NodeKind::Stub)
        {
            continue;
        }
        if let Some(target) = sh.stub_target(label) {
            if target != stub {
                res.redirects.push((stub.to_string(), target));
            }
        }
    }
    res
}

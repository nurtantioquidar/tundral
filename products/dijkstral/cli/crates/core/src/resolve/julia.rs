//! Cross-file pass for Julia (§32). See `docs/graphify-format.md`.
//!
//! The only cross-file step: a type stub (`x::T`, `<: T`) binds to the only Julia definition
//! labelled `T` (a type, module or `using` node), else to the only function labelled `T()`;
//! otherwise it stays a stub. Calls never cross files.

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::facts::NodeKind;
use crate::lang::Lang;

pub fn resolve(files: &[View], _index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let julia: Vec<&View> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Julia)
        .collect();
    if julia.is_empty() {
        return res;
    }
    let mut by_label: HashMap<&str, Vec<&str>> = HashMap::new();
    for v in &julia {
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
            let ids = by_label.entry(n.label.as_str()).or_default();
            if !ids.contains(&n.id.as_str()) {
                ids.push(n.id.as_str());
            }
        }
    }
    let unique = |label: &str| match by_label.get(label).map(Vec::as_slice) {
        Some([only]) => Some(only.to_string()),
        _ => None,
    };
    let mut done: HashSet<&str> = HashSet::new();
    for v in &julia {
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Stub) {
            if !done.insert(n.id.as_str()) {
                continue;
            }
            if let Some(t) = unique(&n.label).or_else(|| unique(&format!("{}()", n.label))) {
                res.redirects.push((n.id.clone(), t));
            }
        }
    }
    res
}

//! Cross-file pass for Verilog (§36). See `docs/graphify-format.md`.
//!
//! An instantiation stub binds to the only definition, in any language, whose label without a
//! trailing `()` equals the stub's label exactly (case-sensitive): a module, but also a function,
//! task, class or a Python class. Otherwise it stays a stub. Graphify does not count these stubs
//! as file emitters in the §4.5 collision rename: two files instantiating `ghost` share one node
//! `ghost`, and a stub keeps its plain id even when a file node has the same id (only the file
//! node is renamed). The extraction emits the stubs per file, so the general rename prefixes
//! them; this pass folds them back onto the plain id.

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::origin;
use crate::extract::verilog::score;
use crate::facts::{LangFacts, Loc, Node, NodeKind};

pub fn resolve(files: &[View], _index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let verilog: Vec<_> = files
        .iter()
        .filter_map(|v| match &v.facts.lang {
            LangFacts::Verilog(f) if !f.stubs.is_empty() => Some((v, f)),
            _ => None,
        })
        .collect();
    if verilog.is_empty() {
        return res;
    }

    // Definitions by label without `()`, any language.
    let mut by_label: HashMap<&str, Vec<&str>> = HashMap::new();
    for v in files {
        for n in v.facts.nodes.iter().filter(|n| n.kind == NodeKind::Def) {
            let name = n.label.strip_suffix("()").unwrap_or(&n.label);
            let ids = by_label.entry(name).or_default();
            if !ids.contains(&n.id.as_str()) {
                ids.push(n.id.as_str());
            }
        }
    }

    let mut kept: HashSet<String> = HashSet::new();
    for (v, f) in verilog {
        for (local, label) in &f.stubs {
            let stub = v.id(local);
            match by_label.get(label.as_str()).map(Vec::as_slice) {
                Some([only]) => {
                    if *only != stub {
                        res.redirects.push((stub.to_string(), only.to_string()));
                    }
                }
                _ if stub != local => {
                    res.redirects.push((stub.to_string(), local.clone()));
                    if kept.insert(local.clone()) {
                        res.nodes.push(Node {
                            id: local.clone(),
                            label: label.clone(),
                            kind: NodeKind::Stub,
                            file_type: "code".into(),
                            source_file: String::new(),
                            loc: Loc::Empty,
                            attrs: vec![origin(), score()],
                        });
                    }
                }
                _ => {}
            }
        }
    }
    res
}

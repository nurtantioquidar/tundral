//! Cross-file pass for SQL (§45.4). See `docs/graphify-format.md`.
//!
//! Each file emits a stub `slug(name)` for every name it does not define, so the §4.5 rename
//! prefixes a stub that more than one file emits, or that collides with another file's id (and
//! renames that file's node too, even when the stub then resolves: a file `t.sql` becomes
//! `t_sql_t` once another file reads `t`). A stub whose name has exactly one SQL definition
//! anywhere (by key, labels without `()`) is replaced by it. Any other stub stays, and the files
//! that share its name share the first file's stub: two files reading an undefined `zz` both point
//! at `a_sql_zz`.

use std::collections::HashMap;

use super::{Index, Resolution, View};
use crate::facts::LangFacts;

pub fn resolve(files: &[View], _index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let sql: Vec<_> = files
        .iter()
        .filter_map(|v| match &v.facts.lang {
            LangFacts::Sql(f) => Some((v, f)),
            _ => None,
        })
        .collect();
    let mut by_key: HashMap<&str, Vec<&str>> = HashMap::new();
    for (v, f) in &sql {
        for (key, local) in &f.defs {
            let ids = by_key.entry(key.as_str()).or_default();
            let id = v.id(local);
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let mut shared: HashMap<&str, &str> = HashMap::new();
    for (v, f) in &sql {
        for (local, key) in &f.stubs {
            let stub = v.id(local);
            match by_key.get(key.as_str()).map(Vec::as_slice) {
                Some([only]) => res.redirects.push((stub.to_string(), only.to_string())),
                _ => match shared.get(key.as_str()) {
                    Some(&kept) if kept != stub => {
                        res.redirects.push((stub.to_string(), kept.to_string()))
                    }
                    Some(_) => {}
                    None => {
                        shared.insert(key, stub);
                    }
                },
            }
        }
    }
    res
}

//! Cross-file pass for Ruby (§20). See `docs/graphify-format.md`.
//!
//! - A constant path in a call (`Foo`, `Foo::Bar`) names the only module or class labelled
//!   `path` or `…::path`; a label several nodes carry (a module reopened in several files)
//!   names nothing. `::Foo` must match exactly.
//! - `Foo.new` calls class `Foo`; `Foo.m` its method `m` of any kind, or the class itself when
//!   it defines no `m` (EXTRACTED 1.0).
//! - `include`/`extend`/`prepend Foo` give `mixes_in` (EXTRACTED 1.0, context `mixin`): `Foo`
//!   is looked up in the lexical scopes first (`Sinatra::Helpers` from `Sinatra::Base`), then
//!   like a call.
//! - A bare call that did not bind in its file goes to the method or function of that name
//!   nearest to the caller's file (`resolve::php::nearest`; INFERRED 0.85).
//!
//! Each (caller, target) pair is linked once, in-file edges included.

use std::collections::{HashMap, HashSet};

use super::php::{cross_edge, nearest};
use super::{Index, Resolution, View};
use crate::extract::ruby::{RCallKind, RubyFacts};
use crate::facts::{Confidence, LangFacts};
use crate::lang::Lang;

fn ruby_facts<'v>(v: &'v View) -> Option<&'v RubyFacts> {
    match &v.facts.lang {
        LangFacts::Ruby(r) => Some(r),
        _ => None,
    }
}

struct Consts<'a> {
    by_label: HashMap<&'a str, Vec<String>>,
}

impl Consts<'_> {
    fn exact(&self, label: &str) -> Option<Option<String>> {
        let ids = self.by_label.get(label)?;
        Some((ids.len() == 1).then(|| ids[0].clone()))
    }

    /// The module or class a mixin names: through the lexical scopes (innermost first), else
    /// as a call does.
    fn resolve_lexical(&self, path: &str, scopes: &[String]) -> Option<String> {
        if !path.starts_with("::") {
            for s in scopes {
                if let Some(found) = self.exact(&format!("{s}::{path}")) {
                    return found;
                }
            }
        }
        self.resolve(path)
    }

    /// The module or class `path` names in a call: the only node labelled `path` or `…::path`.
    fn resolve(&self, path: &str) -> Option<String> {
        if let Some(rooted) = path.strip_prefix("::") {
            return self.exact(rooted).flatten();
        }
        let suffix = format!("::{path}");
        let mut hit: Option<&String> = None;
        for (label, ids) in &self.by_label {
            if *label == path || label.ends_with(&suffix) {
                if hit.is_some() || ids.len() != 1 {
                    return None;
                }
                hit = ids.first();
            }
        }
        hit.cloned()
    }
}

pub fn resolve(files: &[View], _index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let ruby: Vec<(&View, &RubyFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Ruby)
        .filter_map(|v| ruby_facts(v).map(|r| (v, r)))
        .collect();
    if ruby.is_empty() {
        return res;
    }

    let mut consts = Consts {
        by_label: HashMap::new(),
    };
    let mut methods: HashMap<(String, &str), String> = HashMap::new();
    let mut callables: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    for (v, r) in &ruby {
        for (local, label) in &r.classes {
            let id = v.id(local).to_string();
            let ids = consts.by_label.entry(label.as_str()).or_default();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        for (owner, name, local) in &r.methods {
            methods
                .entry((v.id(owner).to_string(), name.as_str()))
                .or_insert_with(|| v.id(local).to_string());
        }
        for (name, local) in &r.callables {
            callables
                .entry(name.as_str())
                .or_default()
                .push((v.id(local).to_string(), v.file.rel.clone()));
        }
    }

    let mut linked: Vec<HashSet<(String, String)>> = ruby
        .iter()
        .map(|(v, _)| {
            v.facts
                .edges
                .iter()
                .map(|e| (e.source.clone(), e.target.clone()))
                .collect()
        })
        .collect();

    // Graphify's order: inferred bare calls, then mixins, then calls on constants.
    for (fi, (v, r)) in ruby.iter().enumerate() {
        let rel = v.file.rel.as_str();
        for c in &r.calls {
            let RCallKind::Bare(name) = &c.kind else {
                continue;
            };
            let caller = v.id(&c.caller).to_string();
            let Some(target) = callables
                .get(name.as_str())
                .and_then(|cands| nearest(rel, cands))
            else {
                continue;
            };
            if linked[fi].insert((caller.clone(), target.clone())) {
                res.edges.push(cross_edge(
                    caller,
                    target,
                    "calls",
                    (rel, c.line),
                    Some("call"),
                    Confidence::Inferred,
                    0.85,
                ));
            }
        }
    }
    for (fi, (v, r)) in ruby.iter().enumerate() {
        let rel = v.file.rel.as_str();
        for m in &r.mixins {
            let source = v.id(&m.source).to_string();
            let Some(target) = consts.resolve_lexical(&m.path, &m.scopes) else {
                continue;
            };
            if linked[fi].insert((source.clone(), target.clone())) {
                res.edges.push(cross_edge(
                    source,
                    target,
                    "mixes_in",
                    (rel, m.line),
                    Some("mixin"),
                    Confidence::Extracted,
                    1.0,
                ));
            }
        }
    }
    for (fi, (v, r)) in ruby.iter().enumerate() {
        let rel = v.file.rel.as_str();
        for c in &r.calls {
            let RCallKind::Const { path, method } = &c.kind else {
                continue;
            };
            let caller = v.id(&c.caller).to_string();
            let Some(class) = consts.resolve(path) else {
                continue;
            };
            // `Foo.new` and a method `Foo` does not define link the class itself.
            let target = match methods.get(&(class.clone(), method.as_str())) {
                Some(m) if method != "new" => m.clone(),
                _ => class,
            };
            if linked[fi].insert((caller.clone(), target.clone())) {
                res.edges.push(cross_edge(
                    caller,
                    target,
                    "calls",
                    (rel, c.line),
                    Some("call"),
                    Confidence::Extracted,
                    1.0,
                ));
            }
        }
    }
    res
}

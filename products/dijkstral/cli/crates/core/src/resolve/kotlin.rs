//! Cross-file pass for Kotlin (§18). See `docs/graphify-format.md`.
//!
//! - Imports: the virtual target `@ki:<path>|<fqn>` binds to the class, object or top-level
//!   function with that FQN (Kotlin or Java), else to the file's own stub of the same name
//!   (a type the file uses), else to the external node `slug(last segment)`. Aliases do not
//!   matter; wildcard imports produce nothing.
//! - Per-file type stubs bind to the only definition anywhere with exactly that label (an
//!   anonymous object's label counts: `object : Runnable` implements itself), else stay.
//! - Bare calls the file could not bind: an import whose last segment is the callee and whose
//!   FQN names a definition binds EXTRACTED 1.0; otherwise, unless a type of that name is used
//!   in the file, the nearest callable of that name binds INFERRED 0.85 (path proximity; a tie
//!   binds nothing). Then `Type.m()` binds EXTRACTED 1.0 when `Type` names one Kotlin class
//!   with a method `m` (companion members included). Member calls on other receivers never
//!   bind across files. No call edge is added where the caller already has an edge to the
//!   target (`f(x: Foo) { Foo() }` keeps only the reference).

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::kotlin::{KCallKind, KotlinFacts, VIRTUAL};
use crate::extract::{external_node, origin, slug};
use crate::facts::{Confidence, Edge, LangFacts, Loc, NodeKind};
use crate::lang::Lang;

/// Redirect target for an import edge the build drops (it then dangles).
pub(super) const DROP: &str = "@drop";

fn kotlin_facts<'v>(v: &'v View) -> Option<&'v KotlinFacts> {
    match &v.facts.lang {
        LangFacts::Kotlin(k) => Some(k),
        _ => None,
    }
}

/// The name a definition answers to in a call: `.m()` → `m`, `f()` → `f`, `T` → `T`.
pub(super) fn call_name(label: &str) -> &str {
    let l = label.strip_prefix('.').unwrap_or(label);
    l.strip_suffix("()").unwrap_or(l)
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Shared leading directories, then whether the candidate is in the caller's directory.
fn proximity(from: &str, to: &str) -> (usize, bool) {
    let (a, b) = (parent(from), parent(to));
    let common = a
        .split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y && !x.is_empty())
        .count();
    (common, a == b)
}

/// The candidate nearest to `from` by shared leading directories, if no other is as near; a
/// sole candidate binds even when it shares none.
fn nearest<'c>(from: &str, caller: &str, cands: &'c [(String, String)]) -> Option<&'c str> {
    let mut best: Option<((usize, bool), &str)> = None;
    let (mut tied, mut count) = (false, 0);
    for (id, rel) in cands {
        if id == caller {
            continue;
        }
        count += 1;
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
    let (p, id) = best?;
    (!tied && (p.0 > 0 || count == 1)).then_some(id)
}

/// A definition a bare call may bind to.
pub(super) struct Candidate {
    id: String,
    rel: String,
}

/// The binding of a bare call among same-named definitions: the nearest by path.
pub(super) fn pick(rel: &str, caller: &str, cands: &[Candidate]) -> Option<String> {
    let pool: Vec<(String, String)> = cands
        .iter()
        .map(|c| (c.id.clone(), c.rel.clone()))
        .collect();
    nearest(rel, caller, &pool).map(str::to_string)
}

fn push_unique(v: &mut Vec<String>, id: &str) {
    if !v.iter().any(|x| x == id) {
        v.push(id.to_string());
    }
}

pub(super) fn edge(
    source: String,
    target: String,
    (rel, line): (&str, u32),
    confidence: Confidence,
    score: f64,
) -> Edge {
    Edge {
        source,
        target,
        relation: "calls".into(),
        confidence,
        score: Some(score),
        context: Some("call".into()),
        source_file: rel.into(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

/// Indexes over every file that the Kotlin and Scala passes share.
pub(super) struct Shared<'a> {
    /// Label → definitions (stub binding).
    pub by_label: HashMap<&'a str, Vec<String>>,
    /// Call name → definitions (bare calls bind by name to any definition, enum entries and
    /// anonymous objects included).
    pub by_name: HashMap<&'a str, Vec<Candidate>>,
    /// `slug(file stem)` → (file node id, language).
    pub stems: HashMap<String, Vec<(String, Lang)>>,
}

pub(super) fn shared<'a>(files: &'a [View]) -> Shared<'a> {
    let mut by_label: HashMap<&str, Vec<String>> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<Candidate>> = HashMap::new();
    let mut stems: HashMap<String, Vec<(String, Lang)>> = HashMap::new();
    for v in files {
        for n in &v.facts.nodes {
            if n.kind == NodeKind::File {
                let name = v.file.rel.rsplit('/').next().unwrap_or(&v.file.rel);
                let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
                stems
                    .entry(slug(stem))
                    .or_default()
                    .push((n.id.clone(), v.file.lang));
            }
            if n.kind != NodeKind::Def {
                continue;
            }
            push_unique(by_label.entry(n.label.as_str()).or_default(), &n.id);
            let c = by_name.entry(call_name(&n.label)).or_default();
            if !c.iter().any(|x| x.id == n.id) {
                c.push(Candidate {
                    id: n.id.clone(),
                    rel: n.source_file.clone(),
                });
            }
        }
    }
    Shared {
        by_label,
        by_name,
        stems,
    }
}

impl Shared<'_> {
    /// A stub binds to the only definition labelled `Name`, else the only function `Name()` (a
    /// factory function stands in for two same-named classes).
    pub fn stub_target(&self, label: &str) -> Option<String> {
        let unique = |key: &str| match self.by_label.get(key).map(Vec::as_slice) {
            Some([only]) => Some(only.clone()),
            _ => None,
        };
        unique(label).or_else(|| unique(&format!("{label}()")))
    }

    /// An unresolved import's external node is replaced by the only file with that stem
    /// (§8.1): the file when it is in the importer's language, nothing (`DROP`) otherwise.
    pub fn stem_target(&self, ext: &str, lang: Lang) -> Option<String> {
        match self.stems.get(ext).map(Vec::as_slice) {
            Some([(file, l)]) if *l == lang => Some(file.clone()),
            Some([_]) => Some(DROP.to_string()),
            _ => None,
        }
    }
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let kotlin: Vec<(&View, &KotlinFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::Kotlin)
        .filter_map(|v| kotlin_facts(v).map(|k| (v, k)))
        .collect();
    if kotlin.is_empty() {
        return res;
    }

    // `package.Name` → definitions: every class-like node (anonymous objects and nested types
    // included, by their flat label) and top-level function of a Kotlin or Java file. Only an
    // unambiguous key binds.
    let mut by_fqn_all: HashMap<String, Vec<String>> = HashMap::new();
    for v in files {
        let (package, kotlin_file) = match &v.facts.lang {
            LangFacts::Kotlin(k) => (k.package.as_str(), true),
            LangFacts::Java(j) => (j.package.as_str(), false),
            _ => continue,
        };
        for n in &v.facts.nodes {
            if n.kind != NodeKind::Def {
                continue;
            }
            // Methods (`.m()`) are not importable; enum entries are and make a name ambiguous
            // (`okhttp3.Dns` the interface and an entry `Dns` of a test enum).
            if n.label.starts_with('.') || (!kotlin_file && n.label.ends_with("()")) {
                continue;
            }
            let name = call_name(&n.label);
            let key = if package.is_empty() {
                name.to_string()
            } else {
                format!("{package}.{name}")
            };
            push_unique(by_fqn_all.entry(key).or_default(), &n.id);
        }
    }
    let by_fqn: HashMap<String, String> = by_fqn_all
        .into_iter()
        .filter_map(|(k, v)| (v.len() == 1).then(|| (k, v[0].clone())))
        .collect();
    let sh = shared(files);
    // Kotlin classes by simple name, and their methods.
    let mut classes: HashMap<&str, Vec<String>> = HashMap::new();
    // (class, name) → its method; `None` when overloaded (`Cookie.parse(…)` binds nothing).
    let mut methods: HashMap<(String, &str), Option<String>> = HashMap::new();
    for (v, k) in &kotlin {
        for c in &k.classes {
            push_unique(classes.entry(c.name.as_str()).or_default(), v.id(&c.id));
        }
        for m in &k.methods {
            methods
                .entry((v.id(&m.owner).to_string(), m.name.as_str()))
                .and_modify(|e| *e = None)
                .or_insert_with(|| Some(v.id(&m.id).to_string()));
        }
    }

    // Imports and stubs.
    let mut redirect: HashMap<String, String> = HashMap::new();
    let mut externals: HashSet<String> = HashSet::new();
    for (v, k) in &kotlin {
        let stubs: HashMap<&str, &str> = k
            .stubs
            .iter()
            .map(|(id, _)| (id.as_str(), v.id(id)))
            .collect();
        for e in &v.facts.edges {
            let Some(body) = e.target.strip_prefix(VIRTUAL) else {
                continue;
            };
            let Some((_, fqn)) = body.split_once('|') else {
                continue;
            };
            let last = slug(fqn.rsplit('.').next().unwrap_or(fqn));
            let target = match by_fqn.get(fqn) {
                Some(t) => t.clone(),
                None => match stubs.get(last.as_str()) {
                    Some(s) => s.to_string(),
                    // Another file's stub with that id takes the edge, as any node would.
                    None if index.contains(&last) => last,
                    None => match sh.stem_target(&last, Lang::Kotlin) {
                        Some(t) => t,
                        None => {
                            if externals.insert(last.clone()) {
                                res.nodes.push(external_node(last.clone()));
                            }
                            last
                        }
                    },
                },
            };
            redirect.insert(e.target.clone(), target);
        }
        for (local, label) in &k.stubs {
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

    // Calls: bare calls in every file, then `Type.m()`.
    let mut pairs: HashSet<(String, String)> = HashSet::new();
    for (v, _) in &kotlin {
        for e in &v.facts.edges {
            pairs.insert((e.source.clone(), follow(&e.target)));
        }
    }
    for phase in 0..2 {
        for (v, k) in &kotlin {
            let rel = v.file.rel.as_str();
            let imports: HashMap<&str, &str> = {
                let mut m = HashMap::new();
                for (last, fqn) in &k.imports {
                    m.entry(last.as_str()).or_insert(fqn.as_str());
                }
                m
            };
            for c in &k.calls {
                let caller = v.id(&c.caller).to_string();
                let found = match (&c.kind, phase) {
                    (KCallKind::Bare { via_stub }, 0) => {
                        let fqn = imports.get(c.name.as_str()).copied();
                        let member = fqn
                            .and_then(|f| f.rsplit_once('.'))
                            .and_then(|(owner, _)| by_fqn.get(owner))
                            .and_then(|owner| methods.get(&(owner.clone(), c.name.as_str())))
                            .and_then(|m| m.clone());
                        // The nearest definition of that name; when none is nearest, the member
                        // an import names (`import okhttp3.TestUtil.assumeNetwork`).
                        match fqn.and_then(|f| by_fqn.get(f)) {
                            Some(t) => Some((t.clone(), Confidence::Extracted, 1.0)),
                            None if *via_stub => None,
                            None => sh
                                .by_name
                                .get(c.name.as_str())
                                .and_then(|cands| pick(rel, &caller, cands))
                                .or(member)
                                .map(|t| (t, Confidence::Inferred, 0.85)),
                        }
                    }
                    (KCallKind::Object(recv), 1) => match classes.get(recv.as_str()) {
                        Some(cs) if cs.len() == 1 => methods
                            .get(&(cs[0].clone(), c.name.as_str()))
                            .and_then(|m| m.clone())
                            .map(|t| (t, Confidence::Extracted, 1.0)),
                        _ => None,
                    },
                    _ => None,
                };
                let Some((target, confidence, score)) = found else {
                    continue;
                };
                if !pairs.insert((caller.clone(), target.clone())) {
                    continue;
                }
                res.edges
                    .push(edge(caller, target, (rel, c.line), confidence, score));
            }
        }
    }
    res.redirects.extend(redirect);
    res
}

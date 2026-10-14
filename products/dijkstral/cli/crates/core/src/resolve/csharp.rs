//! Cross-file pass for C# (§18). See `docs/graphify-format.md`.
//!
//! - Partial types: every `partial` declaration of the same namespace, name and arity merges into
//!   the first file's node (files in path order); the other parts' nodes are redirected to it,
//!   while their members keep their own ids.
//! - Type names resolve as C# looks them up from the referring namespace declaration: its
//!   namespace and every enclosing one, then the `using` namespaces and aliases of each scope out
//!   to the file. Nested types are found only through their outer type. A name that resolves to no
//!   local type becomes the shared stub `slug(Name)`; global `using`s in other files do not count.
//! - Base-list entries are `implements` when they name a local interface or look like one
//!   (`I` + capital), `inherits` otherwise; interfaces always `inherit`.
//! - `using N;` targets the namespace node (written by every declaring file; the build keeps the
//!   first), or an external `slug(N)` when `N` is not a local namespace. `using A = T;` targets
//!   the local type `T`, else an external.
//! - Calls not bound in the file: `this.m()`, `base.m()` and `X.m()` with a capitalized receiver
//!   (a type, else a field or property whose declared type is a local type) are EXTRACTED 1.0; a
//!   typed lower-case receiver and a bare call (through the enclosing type and its bases, else
//!   the only method of that name) are INFERRED 0.85. Receiver types resolve by namespace first,
//!   then by unique simple name. A capitalized or typed receiver whose type names no local type
//!   is recorded in the caller's `metadata.unresolved_calls`.
//! - `new T()` (INFERRED 0.85): by C# name lookup; when the file never uses `T` as a type, else
//!   the method or type named `T` nearest the caller's directory; when it does, never to a type
//!   the caller already references.
//! - `dispatches_to` (INFERRED 0.85, context `call`): an interface method to the only
//!   implementing method of a local class that lists the interface in its base list, unless a
//!   call already joins the two.

use std::collections::{HashMap, HashSet};

use super::{Index, Resolution, View};
use crate::extract::csharp::{json_str, namespace_id, CsCallKind, CsFacts, VIRTUAL};
use crate::extract::{external_node, origin, slug};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc, Node, NodeKind};
use crate::lang::Lang;

/// Redirect target for references that resolve to nothing: the edge dangles and is dropped.
const DROP: &str = "@drop";

fn cs_facts<'v>(v: &'v View) -> Option<&'v CsFacts> {
    match &v.facts.lang {
        LangFacts::CSharp(f) => Some(f),
        _ => None,
    }
}

fn stub_node(id: String, label: String) -> Node {
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

fn edge(
    source: String,
    target: String,
    relation: &str,
    (rel, line): (&str, u32),
    confidence: Confidence,
    score: f64,
) -> Edge {
    Edge {
        source,
        target,
        relation: relation.into(),
        confidence,
        score: Some(score),
        context: Some("call".into()),
        source_file: rel.into(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: vec![origin()],
    }
}

/// `I` followed by a capital letter: Graphify's interface naming test.
fn looks_like_interface(name: &str) -> bool {
    let mut c = name.chars();
    c.next() == Some('I') && c.next().is_some_and(char::is_uppercase)
}

/// Among distinct candidate ids, the one whose file shares the most leading directories with
/// `rel`; none on a tie.
fn nearest<'i>(
    rel: &str,
    cands: impl Iterator<Item = &'i String>,
    file_of: &HashMap<String, &str>,
) -> Option<String> {
    let dirs = |p: &str| -> Vec<String> {
        let mut parts: Vec<String> = p.split('/').map(str::to_string).collect();
        parts.pop();
        parts
    };
    let mine = dirs(rel);
    let mut best: Option<(usize, String)> = None;
    let mut tie = false;
    let mut seen = HashSet::new();
    for id in cands {
        if !seen.insert(id) {
            continue;
        }
        let theirs = dirs(file_of.get(id).copied().unwrap_or(""));
        let shared = mine.iter().zip(&theirs).take_while(|(a, b)| a == b).count();
        match &best {
            Some((s, _)) if shared < *s => {}
            Some((s, _)) if shared == *s => tie = true,
            _ => {
                best = Some((shared, id.clone()));
                tie = false;
            }
        }
    }
    if tie {
        None
    } else {
        best.map(|(_, id)| id)
    }
}

/// The last dotted segment.
fn last(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// Every local type, by canonical id.
struct Types {
    /// Lookup name (namespace, enclosing types, name) → (arity, id).
    by_fqn: HashMap<String, Vec<(u32, String)>>,
    /// Simple name → ids.
    by_name: HashMap<String, Vec<String>>,
    interfaces: HashSet<String>,
}

impl Types {
    fn fqn(&self, name: &str, arity: u32) -> Option<String> {
        let list = self.by_fqn.get(name)?;
        list.iter()
            .find(|(a, _)| *a == arity)
            .or_else(|| list.first())
            .map(|(_, id)| id.clone())
    }

    /// The type `name` denotes from scope `scope` of a file (§18 lookup order).
    fn resolve(&self, f: &CsFacts, scope: u32, name: &str, arity: u32) -> Option<String> {
        let (head, rest) = match name.split_once('.') {
            Some((h, r)) => (h, Some(r)),
            None => (name, None),
        };
        let mut cur = Some(scope as usize);
        while let Some(i) = cur {
            let sc = f.scopes.get(i)?;
            if let Some((_, target)) = sc.aliases.iter().find(|(a, _)| a == head) {
                let full = match rest {
                    Some(r) => format!("{target}.{r}"),
                    None => target.clone(),
                };
                return self.fqn(&full, arity);
            }
            let outer = sc.parent.and_then(|p| f.scopes.get(p as usize));
            let stop = outer.map(|o| o.ns.as_str());
            // The declaration's own namespace, its usings, then the namespaces it implies
            // (`namespace A.B` also opens `A`).
            let mut prefixes = Vec::new();
            let mut p = sc.ns.as_str();
            loop {
                prefixes.push(p);
                match p.rsplit_once('.') {
                    Some((q, _)) if Some(q) != stop => p = q,
                    _ => break,
                }
            }
            for (k, p) in prefixes.iter().enumerate() {
                let cand = if p.is_empty() {
                    name.to_string()
                } else {
                    format!("{p}.{name}")
                };
                if let Some(id) = self.fqn(&cand, arity) {
                    return Some(id);
                }
                if k == 0 {
                    for u in &sc.usings {
                        if let Some(id) = self.fqn(&format!("{u}.{name}"), arity) {
                            return Some(id);
                        }
                    }
                }
            }
            cur = sc.parent.map(|p| p as usize);
        }
        None
    }

    /// `resolve`, else the only local type with that simple name.
    fn resolve_or_unique(&self, f: &CsFacts, scope: u32, name: &str) -> Option<String> {
        self.resolve(f, scope, name, 0)
            .or_else(|| match self.by_name.get(last(name)) {
                Some(ids) if ids.len() == 1 => Some(ids[0].clone()),
                _ => None,
            })
    }
}

pub fn resolve(files: &[View], _index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let cs: Vec<(&View, &CsFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::CSharp)
        .filter_map(|v| cs_facts(v).map(|f| (v, f)))
        .collect();
    if cs.is_empty() {
        return res;
    }
    let by_rel: HashMap<&str, usize> = cs
        .iter()
        .enumerate()
        .map(|(i, (v, _))| (v.file.rel.as_str(), i))
        .collect();

    // Partial types merge into the first declaration.
    let mut canon: HashMap<String, String> = HashMap::new();
    {
        let mut first: HashMap<(String, u32), String> = HashMap::new();
        for (v, f) in &cs {
            for t in f.types.iter().filter(|t| t.partial) {
                let id = v.id(&t.id).to_string();
                match first.get(&(t.fqn.clone(), t.arity)) {
                    Some(c) if *c != id => {
                        canon.insert(id, c.clone());
                    }
                    Some(_) => {}
                    None => {
                        first.insert((t.fqn.clone(), t.arity), id);
                    }
                }
            }
        }
    }
    let canon_of =
        |id: &str| -> String { canon.get(id).cloned().unwrap_or_else(|| id.to_string()) };
    for (from, to) in &canon {
        res.redirects.push((from.clone(), to.clone()));
    }

    // Type, member and namespace tables (canonical ids).
    let mut types = Types {
        by_fqn: HashMap::new(),
        by_name: HashMap::new(),
        interfaces: HashSet::new(),
    };
    // Canonical type id → its declarations (file index, type index).
    let mut decls: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    let mut methods: HashMap<(String, String), String> = HashMap::new();
    let mut by_method_name: HashMap<&str, Vec<String>> = HashMap::new();
    let mut iface_methods: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut metas: HashMap<String, String> = HashMap::new();
    let mut ns_seen: HashSet<&str> = HashSet::new();
    let mut file_of: HashMap<String, &str> = HashMap::new();
    for (fi, (v, f)) in cs.iter().enumerate() {
        for (ti, t) in f.types.iter().enumerate() {
            let id = canon_of(v.id(&t.id));
            let list = types.by_fqn.entry(t.fqn.clone()).or_default();
            if !list.iter().any(|(_, x)| *x == id) {
                list.push((t.arity, id.clone()));
            }
            let names = types.by_name.entry(t.name.clone()).or_default();
            if !names.contains(&id) {
                names.push(id.clone());
            }
            if t.interface {
                types.interfaces.insert(id.clone());
            }
            decls.entry(id.clone()).or_default().push((fi, ti));
            file_of.entry(id.clone()).or_insert(v.file.rel.as_str());
            metas.entry(id).or_insert_with(|| t.meta.clone());
        }
        for m in &f.members {
            let id = v.id(&m.id).to_string();
            file_of.entry(id.clone()).or_insert(v.file.rel.as_str());
            metas.entry(id.clone()).or_insert_with(|| m.meta.clone());
            if !m.method {
                continue;
            }
            let owner = if m.owner.is_empty() {
                String::new()
            } else {
                canon_of(v.id(&m.owner))
            };
            methods
                .entry((owner.clone(), m.name.clone()))
                .or_insert_with(|| id.clone());
            let named = by_method_name.entry(m.name.as_str()).or_default();
            if !named.contains(&id) {
                named.push(id.clone());
            }
            if !owner.is_empty() {
                let list = iface_methods.entry(owner).or_default();
                if !list.iter().any(|(_, x)| *x == id) {
                    list.push((m.name.clone(), id));
                }
            }
        }
        for (ns, _) in &f.namespaces {
            ns_seen.insert(ns.as_str());
        }
    }

    // Base types of a canonical type (all its declarations), resolved.
    let bases_of = |id: &str| -> Vec<String> {
        let mut out = Vec::new();
        for &(fi, ti) in decls.get(id).into_iter().flatten() {
            let f = cs[fi].1;
            let t = &f.types[ti];
            for (b, arity) in &t.bases {
                if let Some(bid) = types.resolve(f, t.scope, b, *arity) {
                    if bid != id && !out.contains(&bid) {
                        out.push(bid);
                    }
                }
            }
        }
        out
    };
    // A method of a type or of its bases, nearest first.
    let find_member = |class: &str, name: &str, skip_self: bool| -> Option<String> {
        let mut queue: Vec<String> = if skip_self {
            bases_of(class)
        } else {
            vec![class.to_string()]
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut i = 0;
        while i < queue.len() && i < 64 {
            let c = queue[i].clone();
            i += 1;
            if !seen.insert(c.clone()) {
                continue;
            }
            if let Some(m) = methods.get(&(c.clone(), name.to_string())) {
                return Some(m.clone());
            }
            queue.extend(bases_of(&c));
        }
        None
    };

    // Virtual targets.
    let mut redirected: HashSet<&str> = HashSet::new();
    let mut stubs: HashSet<String> = HashSet::new();
    let mut externals: HashSet<String> = HashSet::new();
    let mut stub = |name: &str, res: &mut Resolution| -> String {
        let label = last(name);
        let id = slug(label);
        if id.is_empty() {
            return DROP.to_string();
        }
        if stubs.insert(id.clone()) {
            res.nodes.push(stub_node(id.clone(), label.to_string()));
        }
        id
    };
    let mut external = |id: String, res: &mut Resolution| -> String {
        if id.is_empty() {
            return DROP.to_string();
        }
        if externals.insert(id.clone()) {
            res.nodes.push(external_node(id.clone()));
        }
        id
    };
    // Razor and XAML nodes act as types of the global namespace (§43.5): a type C# cannot find
    // binds to the only such node with its simple name.
    let markup_types = super::dotnet::markup_types(files);
    let markup = |name: &str| markup_types.get(last(name)).cloned().flatten();
    for (v, _) in &cs {
        for e in &v.facts.edges {
            let t = e.target.as_str();
            if !t.starts_with(VIRTUAL) || !redirected.insert(t) {
                continue;
            }
            let body = &t[VIRTUAL.len() + 2..];
            let parts: Vec<&str> = body.split('|').collect();
            let file = |rel: &str| by_rel.get(rel).map(|&i| cs[i].1);
            let target = match &t[VIRTUAL.len()..VIRTUAL.len() + 1] {
                // `t:<path>|<scope>|<arity>|<name>`
                "t" if parts.len() == 4 => {
                    let (scope, arity) =
                        (parts[1].parse().unwrap_or(0), parts[2].parse().unwrap_or(0));
                    match file(parts[0])
                        .and_then(|f| types.resolve(f, scope, parts[3], arity))
                        .or_else(|| markup(parts[3]))
                    {
                        Some(id) => id,
                        None => stub(parts[3], &mut res),
                    }
                }
                // `b:<path>|<scope>|<arity>|<name>|<i|m>`
                "b" if parts.len() == 5 => {
                    let (scope, arity) =
                        (parts[1].parse().unwrap_or(0), parts[2].parse().unwrap_or(0));
                    let found = file(parts[0])
                        .and_then(|f| types.resolve(f, scope, parts[3], arity))
                        .or_else(|| markup(parts[3]));
                    let implements = found
                        .as_ref()
                        .is_some_and(|id| types.interfaces.contains(id))
                        || looks_like_interface(last(parts[3]));
                    if implements != (parts[4] == "m") {
                        DROP.to_string()
                    } else {
                        match found {
                            Some(id) => id,
                            None => stub(parts[3], &mut res),
                        }
                    }
                }
                // `n:<namespace>`
                "n" => {
                    if ns_seen.contains(body) {
                        namespace_id(body)
                    } else {
                        external(slug(body), &mut res)
                    }
                }
                // `a:<path>|<scope>|<target>|<written>`
                "a" if parts.len() == 4 => match types.fqn(parts[2], 0) {
                    Some(id) => id,
                    None => external(slug(parts[3]), &mut res),
                },
                _ => DROP.to_string(),
            };
            res.redirects.push((t.to_string(), target));
        }
    }

    let unique_method = |name: &str| -> Option<String> {
        match by_method_name.get(name) {
            Some(ids) if ids.len() == 1 => Some(ids[0].clone()),
            _ => None,
        }
    };
    // Calls, per file: unbound bare calls and `new` first, then the rest, in source order.
    let mut called: HashSet<(String, String)> = HashSet::new();
    // (source, resolved target) of every type reference.
    let mut referenced: HashSet<(String, String)> = HashSet::new();
    {
        let resolved: HashMap<&str, &str> = res
            .redirects
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        for (v, _) in &cs {
            for e in &v.facts.edges {
                match e.relation.as_str() {
                    "calls" => {
                        called.insert((canon_of(&e.source), canon_of(&e.target)));
                    }
                    "references" => {
                        let t = resolved
                            .get(e.target.as_str())
                            .copied()
                            .unwrap_or(&e.target);
                        referenced.insert((canon_of(&e.source), t.to_string()));
                    }
                    _ => {}
                }
            }
        }
    }
    let mut unresolved: Vec<(String, Vec<String>)> = Vec::new();
    for (v, f) in &cs {
        let rel = v.file.rel.as_str();
        let mut recorded: HashSet<(String, String, String)> = HashSet::new();
        let mut record = |caller: &str,
                          callee: &str,
                          ty: &str,
                          line: u32,
                          unresolved: &mut Vec<(String, Vec<String>)>| {
            if !recorded.insert((caller.to_string(), callee.to_string(), ty.to_string())) {
                return;
            }
            let entry = format!(
                "{{\"callee\":{},\"receiver_type\":{},\"lang\":\"csharp\",\"line\":\"L{line}\"}}",
                json_str(callee),
                json_str(ty)
            );
            match unresolved.iter_mut().find(|(id, _)| id == caller) {
                Some((_, list)) => list.push(entry),
                None => unresolved.push((caller.to_string(), vec![entry])),
            }
        };
        for phase in 0..2 {
            for c in &f.calls {
                let p = match c.kind {
                    CsCallKind::Bare | CsCallKind::New | CsCallKind::NewTyped => 0,
                    _ => 1,
                };
                if p != phase {
                    continue;
                }
                let caller = canon_of(v.id(&c.caller));
                let owner = if c.owner.is_empty() {
                    String::new()
                } else {
                    canon_of(v.id(&c.owner))
                };
                let found: Option<(String, Confidence, f64)> = match c.kind {
                    CsCallKind::Bare => {
                        let own = if owner.is_empty() {
                            None
                        } else {
                            find_member(&owner, &c.name, false)
                        };
                        own.or_else(|| unique_method(&c.name))
                            .map(|t| (t, Confidence::Inferred, 0.85))
                    }
                    // `new T()` with `T` not used as a type in the file: C# name lookup, else
                    // the only method named `T`, else the only type named `T`.
                    CsCallKind::New => types
                        .resolve(f, c.scope, &c.name, 0)
                        .or_else(|| {
                            // Methods and types named `T`: the one nearest the caller's file.
                            let n = last(&c.name);
                            let cands = by_method_name
                                .get(n)
                                .into_iter()
                                .flatten()
                                .chain(types.by_name.get(n).into_iter().flatten());
                            nearest(rel, cands, &file_of)
                        })
                        .map(|t| (t, Confidence::Inferred, 0.85)),
                    // `new T()` with `T` also used as a type: through C# name lookup only, and
                    // not when the caller already references `T`.
                    CsCallKind::NewTyped => types
                        .resolve(f, c.scope, &c.name, 0)
                        .filter(|t| !referenced.contains(&(caller.clone(), t.clone())))
                        .map(|t| (t, Confidence::Inferred, 0.85)),
                    CsCallKind::This => {
                        find_member(&owner, &c.name, false).map(|t| (t, Confidence::Extracted, 1.0))
                    }
                    CsCallKind::Base => {
                        find_member(&owner, &c.name, true).map(|t| (t, Confidence::Extracted, 1.0))
                    }
                    // `X.m()`: `X` as a type, else its declared type; failing both, the only
                    // method of that name (INFERRED).
                    CsCallKind::Static => {
                        let class =
                            types
                                .resolve_or_unique(f, c.scope, &c.receiver)
                                .or_else(|| {
                                    (!c.receiver_type.is_empty())
                                        .then(|| {
                                            types.resolve_or_unique(f, c.scope, &c.receiver_type)
                                        })
                                        .flatten()
                                });
                        match class {
                            Some(class) => find_member(&class, &c.name, false)
                                .map(|t| (t, Confidence::Extracted, 1.0)),
                            None => {
                                let ty = if c.receiver_type.is_empty() {
                                    c.receiver.as_str()
                                } else {
                                    c.receiver_type.as_str()
                                };
                                if !types.by_name.contains_key(last(ty))
                                    && !types.by_name.contains_key(last(&c.receiver))
                                {
                                    record(&caller, &c.name, ty, c.line, &mut unresolved);
                                }
                                None
                            }
                        }
                    }
                    CsCallKind::Member => {
                        let ty = c.receiver_type.as_str();
                        match types.resolve_or_unique(f, c.scope, ty) {
                            Some(class) => find_member(&class, &c.name, false)
                                .map(|t| (t, Confidence::Inferred, 0.85)),
                            None => {
                                if !types.by_name.contains_key(last(ty)) {
                                    record(&caller, &c.name, ty, c.line, &mut unresolved);
                                }
                                None
                            }
                        }
                    }
                };
                let Some((target, confidence, score)) = found else {
                    continue;
                };
                if !called.insert((caller.clone(), target.clone())) {
                    continue;
                }
                res.edges.push(edge(
                    caller,
                    target,
                    "calls",
                    (rel, c.line),
                    confidence,
                    score,
                ));
            }
        }
    }
    for (id, entries) in unresolved {
        let meta = metas.get(&id).map_or("", String::as_str);
        let sep = if meta.is_empty() { "" } else { "," };
        let json = format!(
            "{{{meta}{sep}\"unresolved_calls\":[{}]}}",
            entries.join(",")
        );
        res.attrs.push((id, "metadata".into(), Attr::Json(json)));
    }

    // dispatches_to: an interface method with exactly one implementing method.
    // Interface method → its implementing methods: (method, file index, class line).
    type Impls = Vec<(String, Vec<(String, usize, u32)>)>;
    let mut impls: Impls = Vec::new();
    for (fi, (v, f)) in cs.iter().enumerate() {
        for t in f.types.iter().filter(|t| !t.interface) {
            let class = canon_of(v.id(&t.id));
            for (b, arity) in &t.bases {
                let Some(iface) = types.resolve(f, t.scope, b, *arity) else {
                    continue;
                };
                if !types.interfaces.contains(&iface) {
                    continue;
                }
                for (name, im) in iface_methods.get(&iface).into_iter().flatten() {
                    let Some(cm) = methods.get(&(class.clone(), name.clone())) else {
                        continue;
                    };
                    let slot = match impls.iter_mut().position(|(k, _)| k == im) {
                        Some(i) => i,
                        None => {
                            impls.push((im.clone(), Vec::new()));
                            impls.len() - 1
                        }
                    };
                    if !impls[slot].1.iter().any(|(x, _, _)| x == cm) {
                        impls[slot].1.push((cm.clone(), fi, t.line));
                    }
                }
            }
        }
    }
    for (im, list) in impls {
        let [(cm, fi, line)] = list.as_slice() else {
            continue;
        };
        if called.contains(&(im.clone(), cm.clone())) || called.contains(&(cm.clone(), im.clone()))
        {
            continue;
        }
        let mut e = edge(
            im,
            cm.clone(),
            "dispatches_to",
            (cs[*fi].0.file.rel.as_str(), *line),
            Confidence::Inferred,
            0.85,
        );
        e.context = Some("call".into());
        res.edges.push(e);
    }
    res
}

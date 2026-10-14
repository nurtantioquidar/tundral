//! Cross-file pass for .NET project and markup files (§43). See `docs/graphify-format.md`.
//!
//! - A project named by several files (its own file node, solution entries, `ProjectReference`s)
//!   keeps the shortest of its names, ties broken by the smaller string.
//! - Same-label concept nodes of different files (`framework_net8_0`, `sdk_microsoft_net_sdk`,
//!   routes, XAML bindings) merge into the shortest id when the label's entropy is at least 2.5
//!   bits per character, as JSON references do (§16).
//! - Razor (§43.3): `@using N` of a local namespace imports the namespace node, `@using A = T`
//!   of a local type the type; `@inject T x` resolves `T` as C# would from the file's usings and
//!   aliases (else the shared stub `slug(T)`); `@inherits T` and component tags bind to the only
//!   local type of that name when its namespace is global, imported by the file, or imported by an
//!   `_Imports.razor` in the same directory. A node of the file whose every edge was bound
//!   elsewhere is dropped.
//! - XAML (§43.4): `x:Class` binds to that class in the `.xaml.cs` beside the file (the file's
//!   own class node is then dropped); an attribute naming a method of that class with an `…Args`
//!   parameter is an `event` reference. Inside the directory of the nearest `.csproj`, an
//!   explicit `DataContext` type or the `<View>ViewModel` the naming convention gives binds to the
//!   first C# class of that name; each such view model gets the members CommunityToolkit
//!   generates (`defines`, INFERRED), and a binding path naming one of them an INFERRED edge.
//! - Razor and XAML code nodes double as types of C#'s global namespace (`markup_types`, used by
//!   `crate::resolve::csharp`).

use std::collections::{BTreeMap, HashMap, HashSet};

use super::data::entropy;
use super::{Index, Resolution, View};
use crate::extract::csharp::{namespace_id, CsFacts};
use crate::extract::dotnet::razor::{RazorFacts, RefKind};
use crate::extract::dotnet::xaml::{XamlFacts, XamlKind, VIEW_SUFFIXES};
use crate::extract::dotnet::DotnetFacts;
use crate::extract::{origin, slug, Ctx};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc, Node, NodeKind};
use crate::graph::norm_label;
use crate::lang::{Family, Lang};

/// Redirect target that deletes a node (edges to it dangle and are dropped).
const DROP: &str = "@drop";

fn dotnet_facts<'v>(v: &'v View) -> Option<&'v DotnetFacts> {
    match &v.facts.lang {
        LangFacts::Dotnet(f) => Some(f),
        _ => None,
    }
}

pub fn resolve(files: &[View], _index: &Index) -> Resolution {
    let mut res = Resolution::default();
    let views: Vec<&View> = files
        .iter()
        .filter(|v| v.family() == Family::Dotnet)
        .collect();
    if views.is_empty() {
        return res;
    }
    project_labels(&views, &mut res);
    merge_concepts(&views, &mut res);
    let cs = CsIndex::new(files);
    razor(&views, &cs, &mut res);
    xaml(&views, files, &mut res);
    res
}

/// Razor and XAML code nodes by label: the id when exactly one node carries that label.
pub(crate) fn markup_types(files: &[View]) -> HashMap<String, Option<String>> {
    let mut map: HashMap<String, Option<String>> = HashMap::new();
    for v in files
        .iter()
        .filter(|v| matches!(v.file.lang, Lang::Razor | Lang::Xaml))
    {
        for n in &v.facts.nodes {
            if n.kind == NodeKind::Def && n.file_type == "code" {
                map.entry(n.label.clone())
                    .and_modify(|e| {
                        if e.as_deref() != Some(n.id.as_str()) {
                            *e = None;
                        }
                    })
                    .or_insert_with(|| Some(n.id.clone()));
            }
        }
    }
    map
}

fn xaml_facts<'v>(v: &'v View) -> Option<&'v XamlFacts> {
    dotnet_facts(v)?.xaml.as_ref()
}

/// `@cs:t:<path>|<scope>|<arity>|<name>` → `<name>`.
fn cs_type_name(target: &str) -> Option<&str> {
    target
        .strip_prefix(crate::extract::csharp::VIRTUAL)?
        .strip_prefix("t:")?
        .rsplit('|')
        .next()
}

/// `\w+Args`: the parameter type of an event handler.
fn is_args_type(name: &str) -> bool {
    name.len() > 4
        && name.ends_with("Args")
        && name.chars().all(|c| c == '_' || c.is_alphanumeric())
}

fn xaml(views: &[&View], files: &[View], res: &mut Resolution) {
    let cs: Vec<(&View, &CsFacts)> = files
        .iter()
        .filter(|v| v.file.lang == Lang::CSharp)
        .filter_map(|v| match &v.facts.lang {
            LangFacts::CSharp(f) => Some((v, f)),
            _ => None,
        })
        .collect();
    let by_rel: HashMap<&str, (&View, &CsFacts)> = cs
        .iter()
        .map(|(v, f)| (v.file.rel.as_str(), (*v, *f)))
        .collect();
    // Directories of the parsed project files, for the view-model scope.
    let projects: Vec<&str> = views
        .iter()
        .filter(|v| {
            v.file.lang == Lang::MsBuild && v.facts.nodes.iter().any(|n| n.kind == NodeKind::File)
        })
        .map(|v| v.file.rel.rsplit_once('/').map_or("", |(d, _)| d))
        .collect();
    let in_dir = |rel: &str, dir: &str| dir.is_empty() || rel.starts_with(&format!("{dir}/"));
    // The first C# class named `name` under `dir`: (final id, file, local id).
    let class_in = |name: &str, dir: &str| -> Option<(String, &View, &CsFacts, String)> {
        cs.iter()
            .filter(|(v, _)| in_dir(&v.file.rel, dir))
            .find_map(|(v, f)| {
                f.types
                    .iter()
                    .find(|t| t.name == name && t.arity == 0)
                    .map(|t| (v.id(&t.id).to_string(), *v, *f, t.id.clone()))
            })
    };
    let mut generated_for: Vec<String> = Vec::new();
    for v in views.iter().filter(|v| v.file.lang == Lang::Xaml) {
        let Some(f) = xaml_facts(v) else {
            continue;
        };
        // x:Class and its handlers.
        let mut handlers: Vec<(String, String)> = Vec::new();
        if let Some((fqn, own, target)) = &f.class {
            let code_behind = format!("{}.cs", v.file.rel);
            let class = by_rel.get(code_behind.as_str()).and_then(|(cv, cf)| {
                let t = cf.types.iter().find(|t| t.fqn == *fqn && t.arity == 0)?;
                Some((*cv, *cf, t))
            });
            match class {
                Some((cv, cf, t)) => {
                    res.redirects
                        .push((target.clone(), cv.id(&t.id).to_string()));
                    res.redirects
                        .push((v.id(own).to_string(), DROP.to_string()));
                    for m in cf.members.iter().filter(|m| m.method && m.owner == t.id) {
                        let id = cv.id(&m.id).to_string();
                        let handler = cv.facts.edges.iter().any(|e| {
                            e.source == id
                                && e.relation == "references"
                                && e.context.as_deref() == Some("parameter_type")
                                && cs_type_name(&e.target).is_some_and(is_args_type)
                        });
                        if handler && !handlers.iter().any(|(n, _)| *n == m.name) {
                            handlers.push((m.name.clone(), id));
                        }
                    }
                }
                None => res.redirects.push((target.clone(), v.id(own).to_string())),
            }
        }
        // View models, inside the nearest project's directory.
        let scope = projects
            .iter()
            .filter(|d| in_dir(&v.file.rel, d))
            .max_by_key(|d| d.len())
            .copied();
        let mut models: Vec<(String, &View, &CsFacts, String)> = Vec::new();
        for r in &f.refs {
            let target = match r.kind {
                XamlKind::Event => handlers
                    .iter()
                    .find(|(n, _)| *n == r.name)
                    .map(|(_, id)| id.clone()),
                XamlKind::ViewModel | XamlKind::Inferred => {
                    let name = if r.kind == XamlKind::Inferred {
                        VIEW_SUFFIXES
                            .iter()
                            .find_map(|s| r.name.strip_suffix(s).filter(|b| !b.is_empty()))
                            .map(|b| format!("{b}ViewModel"))
                    } else {
                        Some(r.name.clone())
                    };
                    let found = scope.zip(name).and_then(|(d, n)| class_in(&n, d));
                    found.map(|m| {
                        let id = m.0.clone();
                        if !generated_for.contains(&id) {
                            generated_for.push(id.clone());
                            generated_members(&m, res);
                        }
                        models.push(m);
                        id
                    })
                }
                XamlKind::Member => models.iter().find_map(|(id, _, mf, local)| {
                    mf.generated
                        .iter()
                        .find(|g| g.owner == *local && g.name == r.name)
                        .map(|g| Ctx::child_id(id, &g.name))
                }),
            };
            res.redirects
                .push((r.target.clone(), target.unwrap_or_else(|| DROP.to_string())));
        }
    }
}

/// The members CommunityToolkit generates on a view model: nodes and `defines` edges.
fn generated_members(m: &(String, &View, &CsFacts, String), res: &mut Resolution) {
    let (vm, v, f, local) = m;
    for g in f.generated.iter().filter(|g| g.owner == *local) {
        let id = Ctx::child_id(vm, &g.name);
        res.nodes.push(Node {
            id: id.clone(),
            label: g.name.clone(),
            kind: NodeKind::Def,
            file_type: "code".into(),
            source_file: v.file.rel.clone(),
            loc: Loc::Line(g.line),
            attrs: vec![origin()],
        });
        let context = if g.command {
            "communitytoolkit_relay_command"
        } else {
            "communitytoolkit_observable_property"
        };
        res.edges.push(Edge {
            source: vm.clone(),
            target: id,
            relation: "defines".into(),
            confidence: Confidence::Inferred,
            score: None,
            context: Some(context.into()),
            source_file: v.file.rel.clone(),
            loc: Loc::Line(g.line),
            weight: Some(1.0),
            attrs: vec![("_origin".into(), Attr::Str("ast".into()))],
        });
    }
}

/// The local C# types and namespaces Razor and XAML bind to.
struct CsIndex {
    /// Lookup name (namespace, enclosing types, name) → (arity, id).
    by_fqn: HashMap<String, Vec<(u32, String)>>,
    /// Simple name → (namespace, id), one entry per type (partial parts merged).
    by_name: HashMap<String, Vec<(String, String)>>,
    namespaces: HashSet<String>,
}

impl CsIndex {
    fn new(files: &[View]) -> CsIndex {
        let mut ix = CsIndex {
            by_fqn: HashMap::new(),
            by_name: HashMap::new(),
            namespaces: HashSet::new(),
        };
        // Partial declarations merge into the first file's (files are in path order).
        let mut partial: HashMap<(String, u32), String> = HashMap::new();
        for v in files.iter().filter(|v| v.file.lang == Lang::CSharp) {
            let LangFacts::CSharp(f) = &v.facts.lang else {
                continue;
            };
            ix.add_file(v, f, &mut partial);
        }
        ix
    }

    fn add_file(&mut self, v: &View, f: &CsFacts, partial: &mut HashMap<(String, u32), String>) {
        for (ns, _) in &f.namespaces {
            self.namespaces.insert(ns.clone());
        }
        for t in &f.types {
            let mut id = v.id(&t.id).to_string();
            if t.partial {
                id = partial
                    .entry((t.fqn.clone(), t.arity))
                    .or_insert(id)
                    .clone();
            }
            let list = self.by_fqn.entry(t.fqn.clone()).or_default();
            if !list.iter().any(|(_, x)| *x == id) {
                list.push((t.arity, id.clone()));
            }
            let ns = f
                .scopes
                .get(t.scope as usize)
                .map_or(String::new(), |s| s.ns.clone());
            let named = self.by_name.entry(t.name.clone()).or_default();
            if !named.iter().any(|(_, x)| *x == id) {
                named.push((ns, id));
            }
        }
    }

    fn fqn(&self, name: &str, arity: u32) -> Option<String> {
        let list = self.by_fqn.get(name)?;
        list.iter()
            .find(|(a, _)| *a == arity)
            .or_else(|| list.first())
            .map(|(_, id)| id.clone())
    }

    /// C# name lookup from a file-level scope: aliases, the global namespace, then the usings.
    fn lookup(&self, f: &RazorFacts, name: &str, arity: u32) -> Option<String> {
        let (head, rest) = match name.split_once('.') {
            Some((h, r)) => (h, Some(r)),
            None => (name, None),
        };
        if let Some((_, target)) = f.aliases.iter().find(|(a, _)| a == head) {
            return match rest {
                Some(r) => self.fqn(&format!("{target}.{r}"), arity),
                None => self.fqn(target, arity),
            };
        }
        if let Some(id) = self.fqn(name, arity) {
            return Some(id);
        }
        // Two usings that both supply the name make it ambiguous: nothing binds.
        let mut found: Vec<String> = Vec::new();
        for u in &f.usings {
            if let Some(id) = self.fqn(&format!("{u}.{name}"), arity) {
                if !found.contains(&id) {
                    found.push(id);
                }
            }
        }
        match found.as_slice() {
            [id] => Some(id.clone()),
            _ => None,
        }
    }

    /// The only local type named `name`, if its namespace is visible.
    fn bind(&self, name: &str, visible: &[&str]) -> Option<String> {
        match self.by_name.get(name).map(Vec::as_slice) {
            Some([(ns, id)]) if ns.is_empty() || visible.contains(&ns.as_str()) => Some(id.clone()),
            _ => None,
        }
    }
}

fn razor(views: &[&View], cs: &CsIndex, res: &mut Resolution) {
    let imports: HashMap<&str, &RazorFacts> = views
        .iter()
        .filter(|v| {
            v.file.lang == Lang::Razor && v.file.rel.rsplit('/').next() == Some("_Imports.razor")
        })
        .filter_map(|v| {
            Some((
                v.file.rel.rsplit_once('/').map_or("", |(d, _)| d),
                razor_facts(v)?,
            ))
        })
        .collect();
    let mut stubs: HashSet<String> = HashSet::new();
    for v in views.iter().filter(|v| v.file.lang == Lang::Razor) {
        let Some(f) = razor_facts(v) else {
            continue;
        };
        let dir = v.file.rel.rsplit_once('/').map_or("", |(d, _)| d);
        let mut visible: Vec<&str> = f.usings.iter().map(String::as_str).collect();
        if let Some(imp) = imports.get(dir) {
            visible.extend(imp.usings.iter().map(String::as_str));
        }
        // Final target of every bound edge, then the file's nodes left without an edge.
        let mut targets: Vec<(String, String)> = Vec::new();
        for r in &f.refs {
            let node = v.id(&r.node).to_string();
            let target = match r.kind {
                RefKind::Namespace if cs.namespaces.contains(&r.name) => namespace_id(&r.name),
                RefKind::Alias => cs.fqn(&r.name, 0).unwrap_or(node.clone()),
                RefKind::Inject => cs.lookup(f, &r.name, r.arity).unwrap_or_else(|| {
                    let label = r.name.rsplit('.').next().unwrap_or(&r.name);
                    let id = slug(label);
                    if stubs.insert(id.clone()) {
                        res.nodes.push(stub_node(id.clone(), label.to_string()));
                    }
                    id
                }),
                RefKind::Bind if r.stub => cs.bind(&r.name, &visible).unwrap_or(node.clone()),
                _ => node.clone(),
            };
            targets.push((node, target.clone()));
        }
        // Graphify keeps a call or base-type edge whole over an `@inject` reference to the same
        // node (the reference's context and metadata do not carry over).
        let strong: HashSet<&str> = f
            .refs
            .iter()
            .zip(&targets)
            .filter(|(r, _)| r.kind == RefKind::Bind)
            .map(|(_, (_, t))| t.as_str())
            .collect();
        for (r, (_, target)) in f.refs.iter().zip(&targets) {
            let to = if r.kind == RefKind::Inject && strong.contains(target.as_str()) {
                DROP.to_string()
            } else {
                target.clone()
            };
            res.redirects.push((r.target.clone(), to));
        }
        let mut kept: HashSet<&str> = targets.iter().map(|(_, t)| t.as_str()).collect();
        kept.extend(v.facts.edges.iter().map(|e| e.target.as_str()));
        let mut dropped: HashSet<&str> = HashSet::new();
        for (node, _) in &targets {
            if !kept.contains(node.as_str()) && dropped.insert(node.as_str()) {
                res.redirects.push((node.clone(), DROP.to_string()));
            }
        }
    }
}

fn razor_facts<'v>(v: &'v View) -> Option<&'v RazorFacts> {
    dotnet_facts(v)?.razor.as_ref()
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

/// Every naming of a project, by id: the shortest label wins (§43.2).
fn project_labels(views: &[&View], res: &mut Resolution) {
    let mut names: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for v in views {
        let projects = dotnet_facts(v).map_or(&[][..], |f| f.projects.as_slice());
        for n in &v.facts.nodes {
            let own_file = n.kind == NodeKind::File && v.file.lang == Lang::MsBuild;
            if own_file || projects.contains(&n.id) {
                names
                    .entry(n.id.as_str())
                    .or_default()
                    .push(n.label.as_str());
            }
        }
    }
    for (id, labels) in names {
        if let Some(best) = labels
            .iter()
            .min_by(|a, b| a.chars().count().cmp(&b.chars().count()).then(a.cmp(b)))
        {
            res.labels.push((id.to_string(), best.to_string()));
        }
    }
}

/// Same-label concept nodes in different files merge into the shortest id.
fn merge_concepts(views: &[&View], res: &mut Resolution) {
    let mut groups: HashMap<String, Vec<&str>> = HashMap::new();
    for v in views {
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
    res.redirects.extend(merged);
}

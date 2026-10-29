//! Fortran extraction (§35). See `docs/graphify-format.md`.
//!
//! Free-form Fortran through tree-sitter-fortran 0.6.0 (Graphify's version). Every name is
//! lower-cased (Fortran is case-insensitive); ids are flat `file_slug(name)` whatever the nesting.
//! There are no `_callable` flags and no rationale nodes.
//!
//! - `module` and `program` (label without parentheses) are defined by the file; their children
//!   are walked in source order with them as the owner. `submodule` gives nothing at all.
//! - `subroutine`/`function` (label `name()`) are defined by their owner: the file, a module, a
//!   program, or the host procedure of a `contains` section or of an interface block in its body.
//!   A procedure first emits its `references` (below), then walks its other children (`use`,
//!   interface blocks, internal procedures) in source order. Preprocessor branches and error
//!   nodes are walked like their parent.
//! - `use m` gives a node `slug(m)` (label `m`, located at the statement, not a stub) and an
//!   `imports` edge (context `use`) from the enclosing module, program or procedure.
//! - Derived types are defined by their owner; extended bases, components and `generic`/`final`
//!   bindings give nothing. A type-bound `procedure` statement gives `method` (context
//!   `type_bound_procedure`) to its first declarator's implementation.
//! - A named interface is a node `name()` defined by its owner; its `procedure` statements give
//!   `dispatches_to` (context `generic_interface`) to every name they list, and its bodies give
//!   nothing. Unnamed and abstract interface bodies are ordinary procedures. Operator and
//!   assignment interfaces give nothing.
//! - `references` come from a procedure's own declarations `type(T)`/`class(T)` of a plain
//!   identifier: `parameter_type` for a dummy argument, `return_type` for the result variable
//!   (the `result(...)` name, else the function name), one edge per declarator, at the
//!   declaration line. `T` binds to a node of the file emitted so far with id `file_slug(T)` (the
//!   procedure itself: no edge), else to a per-file stub `slug(T)`, bound across files by
//!   `crate::resolve::fortran`. Type-bound and generic targets bind to any node of the file, else
//!   a stub.
//! - Calls come after the walk, in source order, from the innermost procedure or program: `call
//!   s(...)` targets `file_slug(s)` when the file defines it, else a per-file stub `slug(s)`, a
//!   function reference or array element `f(...)` only when the file has a node `file_slug(f)`
//!   (procedures, interfaces, types as constructors, modules) other than the caller itself.
//!   Recursion through `call` keeps its self-loop.
//! - The build keeps one link per pair (§8.2) but folds `defines` against a reversed `calls`
//!   unlike `contains`; this file folds those pairs itself ([`fold_defines`]).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{slug, Ctx};
use crate::facts::{Edge, LangFacts};

/// Facts for the cross-file pass (`crate::resolve::fortran`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FortranFacts {
    /// Per-file type stubs: (local id, label).
    pub stubs: Vec<(String, String)>,
}

struct F<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    /// Ids of nodes emitted so far (the build keeps the first node of an id).
    emitted: HashSet<String>,
    stubs: Vec<(String, String)>,
    /// (type id, implementation name, line): the first declarator of each type-bound procedure.
    bound: Vec<(String, String, u32)>,
    /// (interface id, procedure name, line) for every name a named interface lists.
    dispatch: Vec<(String, String, u32)>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let Some(tree) = tree else {
        return;
    };
    let file_id = cx.file_id.clone();
    let mut f = F {
        cx,
        emitted: HashSet::from([file_id.clone()]),
        stubs: Vec::new(),
        bound: Vec::new(),
        dispatch: Vec::new(),
    };
    let root = tree.root_node();
    f.scope(root, &file_id);
    for (owner, name, line) in std::mem::take(&mut f.bound) {
        let target = f.target(&name);
        let ctx = Some("type_bound_procedure");
        f.cx.add_edge(&owner, &target, "method", line, ctx);
    }
    for (owner, name, line) in std::mem::take(&mut f.dispatch) {
        let target = f.target(&name);
        let ctx = Some("generic_interface");
        f.cx.add_edge(&owner, &target, "dispatches_to", line, ctx);
    }
    f.calls(root, None);
    let stubs = std::mem::take(&mut f.stubs);
    fold_defines(&mut cx.facts.edges);
    cx.facts.lang = LangFacts::Fortran(FortranFacts { stubs });
}

impl F<'_, '_> {
    /// Source text, lower-cased (Python `str.lower`).
    fn text(&self, n: TsNode) -> String {
        self.cx.text(n).trim().to_lowercase()
    }

    fn local_id(&self, name: &str) -> String {
        Ctx::child_id(&self.cx.file_id, name)
    }

    fn def(&mut self, id: &str, label: String, line: u32) {
        if self.emitted.insert(id.to_string()) {
            self.cx.add_def(id.to_string(), label, line, vec![]);
        }
    }

    /// The per-file stub for `name`, created on first use.
    fn stub(&mut self, name: &str) -> String {
        let id = slug(name);
        if self.emitted.insert(id.clone()) {
            self.cx.add_stub(id.clone(), name.to_string());
            self.stubs.push((id.clone(), name.to_string()));
        }
        id
    }

    /// A name bound against every node of the file, else a stub.
    fn target(&mut self, name: &str) -> String {
        let id = self.local_id(name);
        if self.emitted.contains(&id) {
            id
        } else {
            self.stub(name)
        }
    }

    fn scope(&mut self, node: TsNode, owner: &str) {
        for ch in named_children(node) {
            self.item(ch, owner);
        }
    }

    fn item(&mut self, ch: TsNode, owner: &str) {
        match ch.kind() {
            "module" => self.container(ch, owner, "module_statement"),
            "program" => self.container(ch, owner, "program_statement"),
            "subroutine" | "function" => self.procedure(ch, owner),
            "use_statement" => self.use_stmt(ch, owner),
            "derived_type_definition" => self.derived_type(ch, owner),
            "interface" => self.interface(ch, owner),
            "internal_procedures" | "ERROR" => self.scope(ch, owner),
            k if k.starts_with("preproc_") => self.scope(ch, owner),
            _ => {}
        }
    }

    /// `module` / `program`: a node defined by `owner`, then its children in source order.
    fn container(&mut self, node: TsNode, owner: &str, stmt_kind: &str) {
        let name = child_of_kind(node, stmt_kind).and_then(|s| child_of_kind(s, "name"));
        let Some(name) = name else {
            self.scope(node, owner);
            return;
        };
        let name = self.text(name);
        let id = self.local_id(&name);
        let line = Ctx::line(node);
        self.def(&id, name, line);
        self.cx.add_edge(owner, &id, "defines", line, None);
        self.scope(node, &id);
    }

    fn procedure(&mut self, node: TsNode, owner: &str) {
        let Some(stmt) = child_of_kind(node, "subroutine_statement")
            .or_else(|| child_of_kind(node, "function_statement"))
        else {
            return;
        };
        let Some(name) = stmt.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name);
        let id = self.local_id(&name);
        let line = Ctx::line(node);
        self.def(&id, format!("{name}()"), line);
        self.cx.add_edge(owner, &id, "defines", line, None);

        let params: Vec<String> = stmt
            .child_by_field_name("parameters")
            .map(|ps| {
                named_children(ps)
                    .into_iter()
                    .filter(|p| p.kind() == "identifier")
                    .map(|p| self.text(p))
                    .collect()
            })
            .unwrap_or_default();
        let result = (stmt.kind() == "function_statement").then(|| {
            child_of_kind(stmt, "function_result")
                .and_then(|r| child_of_kind(r, "identifier"))
                .map_or_else(|| name.clone(), |r| self.text(r))
        });

        let children = named_children(node);
        for decl in children
            .iter()
            .filter(|c| c.kind() == "variable_declaration")
        {
            let Some(tname) = decl
                .child_by_field_name("type")
                .filter(|t| t.kind() == "derived_type")
                .and_then(|t| t.child_by_field_name("name"))
            else {
                continue;
            };
            let tname = self.text(tname);
            let dline = Ctx::line(*decl);
            let mut dc = decl.walk();
            let vars: Vec<String> = decl
                .children_by_field_name("declarator", &mut dc)
                .filter(|d| d.kind() == "identifier")
                .map(|d| self.text(d))
                .collect();
            for var in vars {
                let context = if params.contains(&var) {
                    "parameter_type"
                } else if result.as_deref() == Some(var.as_str()) {
                    "return_type"
                } else {
                    continue;
                };
                let local = self.local_id(&tname);
                if local == id {
                    continue;
                }
                let target = if self.emitted.contains(&local) {
                    local
                } else {
                    self.stub(&tname)
                };
                self.cx
                    .add_edge(&id, &target, "references", dline, Some(context));
            }
        }
        for ch in children {
            if ch.kind() != "variable_declaration" {
                self.item(ch, &id);
            }
        }
    }

    fn use_stmt(&mut self, node: TsNode, owner: &str) {
        let Some(m) = child_of_kind(node, "module_name") else {
            return;
        };
        let name = self.text(m);
        let id = slug(&name);
        if id.is_empty() {
            return;
        }
        let line = Ctx::line(node);
        self.def(&id, name, line);
        self.cx.add_edge(owner, &id, "imports", line, Some("use"));
    }

    fn derived_type(&mut self, node: TsNode, owner: &str) {
        let Some(name) = child_of_kind(node, "derived_type_statement")
            .and_then(|s| child_of_kind(s, "type_name"))
        else {
            return;
        };
        let name = self.text(name);
        let id = self.local_id(&name);
        let line = Ctx::line(node);
        self.def(&id, name, line);
        self.cx.add_edge(owner, &id, "defines", line, None);
        let Some(procs) = child_of_kind(node, "derived_type_procedures") else {
            return;
        };
        for st in named_children(procs) {
            if st.kind() != "procedure_statement" {
                continue;
            }
            let mut dc = st.walk();
            let first = st.children_by_field_name("declarator", &mut dc).next();
            let target = first.and_then(|d| match d.kind() {
                "method_name" => Some(d),
                "binding" => child_of_kind(d, "method_name"),
                _ => None,
            });
            if let Some(t) = target {
                let t = self.text(t);
                self.bound.push((id.clone(), t, Ctx::line(st)));
            }
        }
    }

    fn interface(&mut self, node: TsNode, owner: &str) {
        let Some(stmt) = child_of_kind(node, "interface_statement") else {
            return;
        };
        if child_of_kind(stmt, "operator").is_some() || child_of_kind(stmt, "assignment").is_some()
        {
            return;
        }
        let Some(name) = named_interface(stmt) else {
            for body in named_children(node) {
                if matches!(body.kind(), "subroutine" | "function") {
                    self.procedure(body, owner);
                }
            }
            return;
        };
        let name = self.text(name);
        let id = self.local_id(&name);
        let line = Ctx::line(node);
        self.def(&id, format!("{name}()"), line);
        self.cx.add_edge(owner, &id, "defines", line, None);
        for st in named_children(node) {
            if st.kind() != "procedure_statement" {
                continue;
            }
            let mut dc = st.walk();
            let names: Vec<String> = st
                .children_by_field_name("declarator", &mut dc)
                .filter(|d| d.kind() == "method_name")
                .map(|d| self.text(d))
                .collect();
            for t in names {
                self.dispatch.push((id.clone(), t, Ctx::line(st)));
            }
        }
    }

    /// Calls in source order; `caller` is the innermost procedure or program.
    fn calls(&mut self, node: TsNode, caller: Option<&str>) {
        let mut own: Option<String> = None;
        match node.kind() {
            "submodule" | "derived_type_definition" => return,
            "interface" => {
                if child_of_kind(node, "interface_statement")
                    .and_then(named_interface)
                    .is_some()
                {
                    return;
                }
            }
            "module" => {
                for ch in named_children(node) {
                    self.calls(ch, None);
                }
                return;
            }
            "program" => {
                own = child_of_kind(node, "program_statement")
                    .and_then(|s| child_of_kind(s, "name"))
                    .map(|n| self.local_id(&self.text(n)));
            }
            "subroutine" | "function" => {
                own = child_of_kind(node, "subroutine_statement")
                    .or_else(|| child_of_kind(node, "function_statement"))
                    .and_then(|s| s.child_by_field_name("name"))
                    .map(|n| self.local_id(&self.text(n)));
            }
            "subroutine_call" => {
                let callee = node
                    .child_by_field_name("subroutine")
                    .filter(|s| s.kind() == "identifier");
                if let (Some(c), Some(s)) = (caller, callee) {
                    // A name the file does not define goes through a per-file stub, bound
                    // across files like a type stub.
                    let name = self.text(s);
                    let target = self.target(&name);
                    let line = Ctx::line(node);
                    self.cx.add_edge(c, &target, "calls", line, Some("call"));
                }
            }
            "call_expression" => {
                let callee = node.named_child(0).filter(|f| f.kind() == "identifier");
                if let (Some(c), Some(f)) = (caller, callee) {
                    let target = self.local_id(&self.text(f));
                    // A function naming itself in an expression is not a call (only `call s`
                    // keeps a self-loop).
                    if target != c && self.emitted.contains(&target) {
                        let line = Ctx::line(node);
                        self.cx.add_edge(c, &target, "calls", line, Some("call"));
                    }
                }
            }
            _ => {}
        }
        let caller = own.as_deref().or(caller);
        for ch in named_children(node) {
            self.calls(ch, caller);
        }
    }
}

/// The name of a named, non-abstract interface block.
fn named_interface(stmt: TsNode) -> Option<TsNode> {
    if child_of_kind(stmt, "abstract_specifier").is_some() {
        return None;
    }
    child_of_kind(stmt, "name")
}

fn named_children(node: TsNode) -> Vec<TsNode> {
    let mut cur = node.walk();
    node.named_children(&mut cur).collect()
}

fn child_of_kind<'t>(node: TsNode<'t>, kind: &str) -> Option<TsNode<'t>> {
    let mut cur = node.walk();
    let found = node.named_children(&mut cur).find(|c| c.kind() == kind);
    found
}

/// Graphify folds a `defines` link against `calls` and `references` on the same pair unlike the
/// generic rule of §8.2 (`crate::graph`). Observed on probes, it behaves as if each direction
/// were folded first (a `defines` keeps itself and takes a same-direction call's `context`;
/// `calls` beats `references`; the later edge wins otherwise), then the edge whose source id is
/// larger were folded over the other: a `defines` and a reversed `calls` give whichever has the
/// larger source, a `defines` with the smaller source ignores a reversed `references`, and a
/// surviving later edge keeps the earlier one's `context` when it has none (`defines` with
/// context `call` or `parameter_type`). Each pair holding exactly one `defines` plus `calls` or
/// `references` is folded here into one edge at the position of the `defines`; the build then
/// sees a single edge.
pub fn fold_defines(edges: &mut Vec<Edge>) {
    use std::collections::HashMap;
    let key = |e: &Edge| {
        if e.source <= e.target {
            (e.source.clone(), e.target.clone())
        } else {
            (e.target.clone(), e.source.clone())
        }
    };
    if !edges
        .iter()
        .any(|e| e.relation == "defines" && e.source != e.target)
    {
        return;
    }
    let mut by_pair: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        by_pair.entry(key(e)).or_default().push(i);
    }
    let mut drop: HashSet<usize> = HashSet::new();
    let mut replace: Vec<(usize, Edge)> = Vec::new();
    for members in by_pair.values() {
        let defines: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&i| edges[i].relation == "defines")
            .collect();
        let [di] = defines.as_slice() else {
            continue;
        };
        let def = &edges[*di];
        if members.len() < 2
            || def.source == def.target
            || members
                .iter()
                .any(|&i| i != *di && !matches!(edges[i].relation.as_str(), "calls" | "references"))
        {
            continue;
        }
        // Phase 1: each direction on its own, in emission order.
        let fold_dir = |src: &str| -> Option<Edge> {
            let mut cur: Option<Edge> = None;
            for &i in members.iter().filter(|&&i| edges[i].source == src) {
                let n = &edges[i];
                cur = Some(match cur {
                    None => n.clone(),
                    Some(e) if e.relation == "defines" => {
                        let mut kept = e;
                        if kept.context.is_none() && n.relation == "calls" {
                            kept.context = n.context.clone();
                        }
                        kept
                    }
                    Some(e) if e.relation == "calls" && n.relation == "references" => e,
                    Some(e) => merged(&e, n),
                });
            }
            cur
        };
        let (lo, hi) = key(def);
        let (Some(first), Some(second)) = (fold_dir(&lo), fold_dir(&hi)) else {
            // One direction only: the generic fold already agrees.
            continue;
        };
        // Phase 2: the larger source's edge over the smaller source's.
        let survivor = if first.relation == "defines" && second.relation == "references" {
            first
        } else {
            merged(&first, &second)
        };
        for &i in members {
            drop.insert(i);
        }
        drop.remove(di);
        replace.push((*di, survivor));
    }
    for (i, e) in replace {
        edges[i] = e;
    }
    if !drop.is_empty() {
        let mut i = 0;
        edges.retain(|_| {
            let keep = !drop.contains(&i);
            i += 1;
            keep
        });
    }
}

/// `{**old, **new}` for the fields an in-file edge carries: `new` whole, `old`'s context when
/// `new` has none.
fn merged(old: &Edge, new: &Edge) -> Edge {
    let mut e = new.clone();
    if e.context.is_none() {
        e.context = old.context.clone();
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Confidence, Loc};
    use crate::lang::Lang;

    fn run(rel: &str, src: &str) -> crate::facts::FileFacts {
        crate::extract::extract_file(Lang::Fortran, rel, src.as_bytes())
    }

    fn edge_lines(edges: &[Edge]) -> Vec<String> {
        edges
            .iter()
            .map(|e| {
                format!(
                    "{} -{}-> {} {}",
                    e.source,
                    e.relation,
                    e.target,
                    e.context.as_deref().unwrap_or("-")
                )
            })
            .collect()
    }

    #[test]
    fn modules_types_and_procedures() {
        let facts = run(
            "geo.f90",
            "module Geometry\n  use Constants\n  type :: Point\n    real :: x\n  contains\n    procedure :: norm => p_norm, other\n  end type\ncontains\n  function p_norm(p) result(r)\n    class(Point), intent(in) :: p\n    type(Point) :: r\n    r = Point(1.0)\n  end function\nend module\n",
        );
        let labels: Vec<_> = facts.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            ["geo.f90", "geometry", "constants", "point", "p_norm()"]
        );
        assert_eq!(
            edge_lines(&facts.edges),
            [
                "geo -defines-> geo_geometry -",
                "geo_geometry -imports-> constants use",
                "geo_geometry -defines-> geo_point -",
                "geo_geometry -defines-> geo_p_norm -",
                "geo_p_norm -references-> geo_point parameter_type",
                "geo_p_norm -references-> geo_point return_type",
                "geo_point -method-> geo_p_norm type_bound_procedure",
                "geo_p_norm -calls-> geo_point call",
            ]
        );
    }

    #[test]
    fn calls_and_stubs() {
        let facts = run(
            "a.f90",
            "subroutine s(t)\n  type(Later) :: t\n  real :: v(3)\n  call nowhere(1)\n  x = v(1) + f(2)\n  call s(t)\nend subroutine\nfunction f(n)\n  f = n\nend function\n",
        );
        assert!(facts
            .nodes
            .iter()
            .any(|n| n.id == "later" && n.loc == Loc::Empty));
        assert_eq!(
            edge_lines(&facts.edges),
            [
                "a -defines-> a_s -",
                "a_s -references-> later parameter_type",
                "a -defines-> a_f -",
                "a_s -calls-> nowhere call",
                "a_s -calls-> a_f call",
                "a_s -calls-> a_s call",
            ]
        );
    }

    fn e(source: &str, target: &str, relation: &str, ctx: Option<&str>) -> Edge {
        Edge {
            source: source.into(),
            target: target.into(),
            relation: relation.into(),
            confidence: Confidence::Extracted,
            score: None,
            context: ctx.map(str::to_string),
            source_file: "x.f90".into(),
            loc: Loc::Line(1),
            weight: Some(1.0),
            attrs: vec![],
        }
    }

    #[test]
    fn defines_folds_against_reversed_calls_by_source() {
        let fold = |mut v: Vec<Edge>| {
            fold_defines(&mut v);
            edge_lines(&v)
        };
        let call = Some("call");
        assert_eq!(
            fold(vec![
                e("aa", "bb", "defines", None),
                e("bb", "aa", "calls", call)
            ]),
            ["bb -calls-> aa call"]
        );
        assert_eq!(
            fold(vec![
                e("zz", "bb", "defines", None),
                e("bb", "zz", "calls", call)
            ]),
            ["zz -defines-> bb call"]
        );
        assert_eq!(
            // One direction only: left to the build, which gives the same result.
            fold(vec![
                e("aa", "bb", "defines", None),
                e("aa", "bb", "calls", call)
            ]),
            ["aa -defines-> bb -", "aa -calls-> bb call"]
        );
        let param = Some("parameter_type");
        assert_eq!(
            fold(vec![
                e("aa", "zz", "defines", None),
                e("zz", "aa", "references", param)
            ]),
            ["aa -defines-> zz -"]
        );
        assert_eq!(
            fold(vec![
                e("zz", "bb", "defines", None),
                e("bb", "zz", "references", param)
            ]),
            ["zz -defines-> bb parameter_type"]
        );
        assert_eq!(
            fold(vec![
                e("zz", "bb", "defines", None),
                e("bb", "zz", "references", param),
                e("bb", "zz", "calls", call)
            ]),
            ["zz -defines-> bb call"]
        );
    }
}

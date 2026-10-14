//! Elixir (§29) extraction. See `docs/graphify-format.md`.
//!
//! One file gives its file node; every `defmodule` and `defprotocol` outside a function body
//! (id `file_<slug(name as written)>`, label the name as written, `_elixir_module` when not
//! nested in another module); every `defimpl` (id `file_defimpl_<proto>[_<for>]`, label
//! `Proto (for X)` or `Proto`); and every `def`, `defp`, `defmacro`, `defmacrop`, `defguard`,
//! `defguardp` and `defdelegate` outside a function body (id `owner_<slug(name)>`, label
//! `name()`; at file level the owner is `slug(path with extension)`). Definitions are found at
//! any depth of a module body: inside `if`, `for`, `quote`, `describe` or an anonymous function.
//!
//! In-file edges: `contains` (file → module, also for a nested `defmodule`; enclosing module →
//! nested `defprotocol` or `defimpl`; file → file-level function), `implements` (defimpl → a
//! protocol of the file defined before it), `method` (owner → function, one per clause) and
//! `calls` that bind in the file: a bare `f(...)` to the file's last function labelled `f()`, and
//! `Mod.f(...)` to `Mod`'s function when `Mod` (after alias expansion) names a module of the file.
//!
//! Module names follow Elixir: a nested module's full name is its parent's plus its own, and it
//! aliases its first segment in the parent (`defmodule Shop.Other` inside `Shop.A` turns `Shop.B`
//! into `Shop.A.Shop.B`); `alias` applies to the whole module body, or to one function when
//! written in it. Everything else goes to [`ElixirFacts`] for `crate::resolve::elixir`: imports
//! (`alias`, `import`, `use`, `require` outside function bodies), calls that did not bind
//! (`Mod.f` with its expanded module, bare `f` with the `import`/`use` modules in scope) and the
//! module and function tables.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{flag, slug, Ctx};
use crate::facts::LangFacts;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExCallKind {
    /// `f(...)` that did not bind in the file, with the `import`/`use` modules in scope
    /// (innermost module first, then the file level), as written.
    Bare { name: String, scope: Vec<String> },
    /// `Mod.f(...)` with `Mod` alias-expanded.
    Member { module: String, name: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExCall {
    /// Local id of the calling function.
    pub caller: String,
    pub kind: ExCallKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExModule {
    /// Local id.
    pub id: String,
    pub label: String,
    /// Not nested in another module (carries `_elixir_module`).
    pub top: bool,
}

/// Facts the cross-file pass (`crate::resolve::elixir`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ElixirFacts {
    /// `defmodule` and `defprotocol` nodes.
    pub modules: Vec<ExModule>,
    /// Functions: (owner local id, `""` at file level; name from the node's label; local id).
    pub functions: Vec<(String, String, String)>,
    pub calls: Vec<ExCall>,
    /// `alias`/`import`/`use`/`require` targets: (`slug(module as written)`, line).
    pub imports: Vec<(String, u32)>,
}

const DEF_KINDS: &[&str] = &[
    "def",
    "defp",
    "defmacro",
    "defmacrop",
    "defguard",
    "defguardp",
    "defdelegate",
];

/// Bare call names that are never calls (sorted): Elixir special forms and definition macros,
/// plus Graphify's builtin denylist as observed on Elixir (the C list without `object`, plus
/// `require`).
const NOT_CALLS: &[&str] = &[
    "abs",
    "alias",
    "all",
    "any",
    "bool",
    "bytes",
    "callable",
    "case",
    "cond",
    "decodeURI",
    "decodeURIComponent",
    "def",
    "defdelegate",
    "defguard",
    "defimpl",
    "defmacro",
    "defmacrop",
    "defmodule",
    "defp",
    "defprotocol",
    "defstruct",
    "delattr",
    "dict",
    "dir",
    "encodeURI",
    "encodeURIComponent",
    "enumerate",
    "filter",
    "float",
    "for",
    "getattr",
    "hasattr",
    "hash",
    "id",
    "if",
    "import",
    "int",
    "isFinite",
    "isNaN",
    "isinstance",
    "iter",
    "len",
    "list",
    "map",
    "max",
    "min",
    "next",
    "open",
    "parseFloat",
    "parseInt",
    "print",
    "range",
    "repr",
    "require",
    "reversed",
    "round",
    "set",
    "setattr",
    "sorted",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "unless",
    "use",
    "vars",
    "with",
    "zip",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModKind {
    Module,
    Protocol,
    Impl,
}

/// A module-like scope found by the prescan.
struct ModInfo {
    kind: ModKind,
    parent: Option<usize>,
    /// Full name (`Parent.Child`); a defimpl has none of its own.
    full: String,
    /// The name as written after `defmodule`/`defprotocol`/`defimpl`.
    written: String,
    /// Alias → module, from `alias` and nested `defmodule`s anywhere in the body.
    aliases: HashMap<String, String>,
    /// `import`/`use` targets as written, in source order.
    scope: Vec<String>,
    /// Local id once emitted.
    id: Option<String>,
}

struct Entry {
    id: String,
    label: String,
}

enum SiteKind {
    Bare(String),
    Member(String, String),
}

struct Site {
    caller: String,
    module: Option<usize>,
    kind: SiteKind,
    line: u32,
}

struct Walker<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    mods: Vec<ModInfo>,
    by_node: HashMap<usize, usize>,
    /// File-level aliases and `import`/`use` targets.
    file_aliases: HashMap<String, String>,
    file_scope: Vec<String>,
    /// Function nodes in creation order, and every node id emitted.
    funcs: Vec<Entry>,
    ids: HashSet<String>,
    /// Functions per module index (entry indices), for `Mod.f`.
    mod_funcs: HashMap<usize, Vec<usize>>,
    sites: Vec<Site>,
    facts: ElixirFacts,
}

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn line(node: TsNode) -> u32 {
    Ctx::line(node)
}

/// The macro a call invokes when its target is a plain identifier (`defmodule`, `def`, `foo`).
fn call_name<'a>(node: TsNode, src: &'a [u8]) -> Option<&'a str> {
    if node.kind() != "call" {
        return None;
    }
    let target = node.child_by_field_name("target")?;
    (target.kind() == "identifier").then(|| target.utf8_text(src).unwrap_or(""))
}

fn arguments(node: TsNode) -> Vec<TsNode> {
    let mut c = node.walk();
    let found = node
        .named_children(&mut c)
        .find(|n| n.kind() == "arguments");
    found.map(named_children).unwrap_or_default()
}

fn do_block(node: TsNode) -> Option<TsNode> {
    let mut c = node.walk();
    let found = node.named_children(&mut c).find(|n| n.kind() == "do_block");
    found
}

/// `for: X` in a defimpl's keywords, when `X` is a single alias.
fn keyword_value<'t>(args: &[TsNode<'t>], key: &str, src: &[u8]) -> Option<TsNode<'t>> {
    for a in args.iter().filter(|a| a.kind() == "keywords") {
        for pair in named_children(*a) {
            let k = pair.child_by_field_name("key")?;
            let text = k.utf8_text(src).unwrap_or("");
            if text.trim_end().trim_end_matches(':') == key {
                return pair.child_by_field_name("value");
            }
        }
    }
    None
}

/// The value of a `do:` keyword written on the keyword's own line (`do: f(x)`).
fn do_keyword<'t>(args: &[TsNode<'t>], src: &[u8]) -> Option<TsNode<'t>> {
    for a in args.iter().filter(|a| a.kind() == "keywords") {
        for pair in named_children(*a) {
            let key = pair.child_by_field_name("key")?;
            let text = key.utf8_text(src).unwrap_or("");
            if text.trim_end_matches([' ', '\t']) == "do:" {
                return pair.child_by_field_name("value");
            }
        }
    }
    None
}

/// The defined name of a `def` head: `f(x)`, `f`, `f(x) when g`, `a + b` (the left operand).
fn def_name<'a>(head: TsNode, src: &'a [u8]) -> Option<&'a str> {
    match head.kind() {
        "identifier" => head.utf8_text(src).ok(),
        "call" => {
            let t = head.child_by_field_name("target")?;
            (t.kind() == "identifier").then(|| t.utf8_text(src).unwrap_or(""))
        }
        "binary_operator" => def_name(head.child_by_field_name("left")?, src),
        _ => None,
    }
}

/// Module names an `alias`/`import`/`use`/`require` argument names, as written:
/// `A.B`, `A.{B, C.D}` (→ `A.B`, `A.C.D`), `__MODULE__.X`. Atoms name nothing.
fn module_targets(arg: TsNode, src: &[u8]) -> Vec<String> {
    let text = |n: TsNode| n.utf8_text(src).unwrap_or("").to_string();
    match arg.kind() {
        "alias" => vec![text(arg)],
        "dot" => {
            let (Some(l), Some(r)) = (
                arg.child_by_field_name("left"),
                arg.child_by_field_name("right"),
            ) else {
                return Vec::new();
            };
            if r.kind() == "tuple" {
                named_children(r)
                    .into_iter()
                    .filter(|x| x.kind() == "alias")
                    .map(|x| format!("{}.{}", text(l), text(x)))
                    .collect()
            } else {
                vec![text(arg)]
            }
        }
        _ => Vec::new(),
    }
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(Vec::new());
    let mut w = Walker {
        cx,
        mods: Vec::new(),
        by_node: HashMap::new(),
        file_aliases: HashMap::new(),
        file_scope: Vec::new(),
        funcs: Vec::new(),
        ids: HashSet::new(),
        mod_funcs: HashMap::new(),
        sites: Vec::new(),
        facts: ElixirFacts::default(),
    };
    if let Some(tree) = tree {
        let root = tree.root_node();
        w.prescan(root, None);
        w.walk(root, None);
    }
    w.finish();
}

impl Walker<'_, '_> {
    fn text(&self, n: TsNode) -> String {
        n.utf8_text(self.cx.src).unwrap_or("").to_string()
    }

    // ---- pass 1: module names, aliases and import scopes -------------------------------------

    fn prescan(&mut self, node: TsNode, module: Option<usize>) {
        if let Some(name) = call_name(node, self.cx.src) {
            match name {
                "defmodule" | "defprotocol" | "defimpl" => {
                    let args = arguments(node);
                    let written = args
                        .first()
                        .filter(|a| a.kind() == "alias")
                        .map(|a| self.text(*a));
                    let kind = match name {
                        "defmodule" => ModKind::Module,
                        "defprotocol" => ModKind::Protocol,
                        _ => ModKind::Impl,
                    };
                    // A module named by anything but an alias (`defimpl encoder, ...`) yields
                    // nothing at all.
                    let Some(written) = written else {
                        return;
                    };
                    let full = match (kind, module) {
                        // `defimpl P, for: X` is the module `P.X`.
                        (ModKind::Impl, _) => keyword_value(&args, "for", self.cx.src)
                            .filter(|v| v.kind() == "alias")
                            .map(|v| format!("{written}.{}", self.text(v)))
                            .unwrap_or_default(),
                        (_, Some(p)) => format!("{}.{written}", self.mods[p].full),
                        (_, None) => written.clone(),
                    };
                    if let (ModKind::Module | ModKind::Protocol, Some(p)) = (kind, module) {
                        let first = written.split('.').next().unwrap_or(&written).to_string();
                        let target = format!("{}.{first}", self.mods[p].full);
                        self.mods[p].aliases.insert(first, target);
                    }
                    self.mods.push(ModInfo {
                        kind,
                        parent: module,
                        full,
                        written: written.clone(),
                        aliases: HashMap::new(),
                        scope: Vec::new(),
                        id: None,
                    });
                    let idx = self.mods.len() - 1;
                    self.by_node.insert(node.id(), idx);
                    if let Some(body) = do_block(node) {
                        self.prescan_children(body, Some(idx));
                    }
                    return;
                }
                n if DEF_KINDS.contains(&n) => return,
                "alias" => {
                    let aliases = self.alias_pairs(node, module);
                    let map = match module {
                        Some(m) => &mut self.mods[m].aliases,
                        None => &mut self.file_aliases,
                    };
                    for (k, v) in aliases {
                        map.insert(k, v);
                    }
                    return;
                }
                "import" | "use" => {
                    let targets: Vec<String> = arguments(node)
                        .first()
                        .map(|a| module_targets(*a, self.cx.src))
                        .unwrap_or_default();
                    match module {
                        Some(m) => self.mods[m].scope.extend(targets),
                        None => self.file_scope.extend(targets),
                    }
                    return;
                }
                _ => {}
            }
        }
        self.prescan_children(node, module);
    }

    fn prescan_children(&mut self, node: TsNode, module: Option<usize>) {
        for c in named_children(node) {
            self.prescan(c, module);
        }
    }

    /// `alias A.B` → (`B`, `A.B`); `alias A.B, as: X` → (`X`, `A.B`); `alias A.{B, C.D}` →
    /// (`B`, `A.B`), (`D`, `A.C.D`). `__MODULE__` in the target is the current module.
    fn alias_pairs(&self, node: TsNode, module: Option<usize>) -> Vec<(String, String)> {
        let args = arguments(node);
        let Some(first) = args.first() else {
            return Vec::new();
        };
        let current = module.map(|m| self.current_full(m));
        let fix = |t: String| match (&current, t.strip_prefix("__MODULE__")) {
            (Some(c), Some(rest)) => format!("{c}{rest}"),
            _ => t,
        };
        let targets: Vec<String> = module_targets(*first, self.cx.src)
            .into_iter()
            .map(fix)
            .collect();
        if let Some(as_) = keyword_value(&args, "as", self.cx.src) {
            if let Some(t) = targets.first() {
                return vec![(self.text(as_), t.clone())];
            }
        }
        targets
            .into_iter()
            .map(|t| (t.rsplit('.').next().unwrap_or(&t).to_string(), t))
            .collect()
    }

    /// The full name `__MODULE__` stands for in module `m` (a defimpl uses its enclosing one).
    fn current_full(&self, m: usize) -> String {
        let mut cur = Some(m);
        while let Some(i) = cur {
            if self.mods[i].kind != ModKind::Impl {
                return self.mods[i].full.clone();
            }
            cur = self.mods[i].parent;
        }
        String::new()
    }

    /// Expand a module reference as written: function-local aliases, then each enclosing
    /// module's, innermost first, then the file's; `__MODULE__` is the current module.
    fn expand(
        &self,
        written: &str,
        module: Option<usize>,
        local: &HashMap<String, String>,
    ) -> String {
        if let Some(rest) = written.strip_prefix("__MODULE__") {
            if let Some(m) = module {
                return format!("{}{rest}", self.current_full(m));
            }
        }
        let (first, rest) = match written.split_once('.') {
            Some((f, r)) => (f, Some(r)),
            None => (written, None),
        };
        let join = |base: &String| match rest {
            Some(r) => format!("{base}.{r}"),
            None => base.clone(),
        };
        if let Some(t) = local.get(first) {
            return join(t);
        }
        let mut cur = module;
        while let Some(i) = cur {
            if let Some(t) = self.mods[i].aliases.get(first) {
                return join(t);
            }
            cur = self.mods[i].parent;
        }
        if let Some(t) = self.file_aliases.get(first) {
            return join(t);
        }
        written.to_string()
    }

    // ---- pass 2: nodes, structural edges and call sites ---------------------------------------

    fn walk(&mut self, node: TsNode, module: Option<usize>) {
        if let Some(name) = call_name(node, self.cx.src) {
            match name {
                "defmodule" | "defprotocol" | "defimpl" => {
                    if let Some(&idx) = self.by_node.get(&node.id()) {
                        self.module(node, idx, module);
                    }
                    return;
                }
                n if DEF_KINDS.contains(&n) => {
                    self.function(node, module);
                    return;
                }
                "alias" | "import" | "use" | "require" => {
                    let l = line(node);
                    if let Some(a) = arguments(node).first() {
                        for t in module_targets(*a, self.cx.src) {
                            self.facts.imports.push((slug(&t), l));
                        }
                    }
                    return;
                }
                _ => {}
            }
        }
        for c in named_children(node) {
            self.walk(c, module);
        }
    }

    fn add_node(&mut self, id: &str, label: String, l: u32, top_module: bool) -> bool {
        if !self.ids.insert(id.to_string()) {
            return false;
        }
        let extra = if top_module {
            vec![flag("_elixir_module")]
        } else {
            Vec::new()
        };
        self.cx.add_def(id.to_string(), label, l, extra);
        true
    }

    fn module(&mut self, node: TsNode, idx: usize, parent: Option<usize>) {
        let args = arguments(node);
        let written = self.text(args[0]);
        let l = line(node);
        let file_id = self.cx.file_id.clone();
        let parent_id = parent.and_then(|p| self.mods[p].id.clone());
        let kind = self.mods[idx].kind;
        let (id, label) = if kind == ModKind::Impl {
            let target = keyword_value(&args, "for", self.cx.src)
                .filter(|v| v.kind() == "alias")
                .map(|v| self.text(v));
            match target {
                Some(t) => (
                    Ctx::child_id(&file_id, &format!("defimpl {written} {t}")),
                    format!("{written} (for {t})"),
                ),
                None => (
                    Ctx::child_id(&file_id, &format!("defimpl {written}")),
                    written.clone(),
                ),
            }
        } else {
            (Ctx::child_id(&file_id, &written), written.clone())
        };
        let top = parent.is_none() && kind != ModKind::Impl;
        let new = self.add_node(&id, label.clone(), l, top);
        // A nested defmodule hangs off the file; a nested protocol or impl off its module.
        let from = match (kind, &parent_id) {
            (ModKind::Module, _) | (_, None) => file_id.clone(),
            (_, Some(p)) => p.clone(),
        };
        self.cx.add_edge(&from, &id, "contains", l, None);
        if kind == ModKind::Impl {
            let expanded = self.expand(&written, parent, &HashMap::new());
            let proto = self.mods.iter().find(|m| {
                m.kind == ModKind::Protocol
                    && m.id.is_some()
                    && (m.full == expanded || m.written == written)
            });
            if let Some(p) = proto.and_then(|p| p.id.clone()) {
                self.cx.add_edge(&id, &p, "implements", l, None);
            }
        } else if new {
            self.facts.modules.push(ExModule {
                id: id.clone(),
                label,
                top,
            });
        }
        self.mods[idx].id = Some(id);
        if let Some(body) = do_block(node) {
            for c in named_children(body) {
                self.walk(c, Some(idx));
            }
        }
    }

    fn function(&mut self, node: TsNode, module: Option<usize>) {
        let args = arguments(node);
        let Some(head) = args.first().copied() else {
            return;
        };
        let Some(name) = def_name(head, self.cx.src).map(str::to_string) else {
            return;
        };
        let l = line(node);
        let owner = module.and_then(|m| self.mods[m].id.clone());
        let (base, edge_from, relation) = match &owner {
            Some(o) => (o.clone(), o.clone(), "method"),
            None => (slug(self.cx.rel), self.cx.file_id.clone(), "contains"),
        };
        let id = Ctx::child_id(&base, &name);
        if self.add_node(&id, format!("{name}()"), l, false) {
            self.funcs.push(Entry {
                id: id.clone(),
                label: format!("{name}()"),
            });
            if let Some(m) = module {
                self.mod_funcs
                    .entry(m)
                    .or_default()
                    .push(self.funcs.len() - 1);
            }
            self.facts.functions.push((
                owner.clone().unwrap_or_default(),
                name.clone(),
                id.clone(),
            ));
        }
        self.cx.add_edge(&edge_from, &id, relation, l, None);

        // Calls in the body: the `do` block, or a `do:` keyword whose value starts on the same
        // line (Graphify reads no body after `do:` and a line break, EX7). Aliases written in
        // the body apply to it.
        let mut local = HashMap::new();
        let body: Vec<TsNode> = do_keyword(&args, self.cx.src)
            .into_iter()
            .chain(do_block(node))
            .collect();
        for b in &body {
            self.local_aliases(*b, module, &mut local);
        }
        for b in body {
            self.calls(b, &id, module, &local);
        }
    }

    fn local_aliases(
        &self,
        node: TsNode,
        module: Option<usize>,
        out: &mut HashMap<String, String>,
    ) {
        if call_name(node, self.cx.src) == Some("alias") {
            out.extend(self.alias_pairs(node, module));
            return;
        }
        for c in named_children(node) {
            self.local_aliases(c, module, out);
        }
    }

    fn calls(
        &mut self,
        node: TsNode,
        caller: &str,
        module: Option<usize>,
        local: &HashMap<String, String>,
    ) {
        if node.kind() == "call" {
            if let Some(target) = node.child_by_field_name("target") {
                let l = line(node);
                match target.kind() {
                    "identifier" => {
                        let name = self.text(target);
                        if NOT_CALLS.binary_search(&name.as_str()).is_err() {
                            self.sites.push(Site {
                                caller: caller.to_string(),
                                module,
                                kind: SiteKind::Bare(name),
                                line: l,
                            });
                        }
                    }
                    "dot" => {
                        let right = target.child_by_field_name("right");
                        let left = target.child_by_field_name("left");
                        if let (Some(r), Some(lft)) = (right, left) {
                            if r.kind() == "identifier" {
                                if let Some(m) = self.module_ref(lft, module, local) {
                                    let name = self.text(r);
                                    self.sites.push(Site {
                                        caller: caller.to_string(),
                                        module,
                                        kind: SiteKind::Member(m, name),
                                        line: l,
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        for c in named_children(node) {
            self.calls(c, caller, module, local);
        }
    }

    /// The module a call's receiver names: an alias (expanded), `__MODULE__` or
    /// `__MODULE__.X`. Variables, calls and atoms name none.
    fn module_ref(
        &self,
        left: TsNode,
        module: Option<usize>,
        local: &HashMap<String, String>,
    ) -> Option<String> {
        let text = self.text(left);
        match left.kind() {
            "alias" => Some(self.expand(&text, module, local)),
            "identifier" if text == "__MODULE__" => module.map(|m| self.current_full(m)),
            "dot" if text.starts_with("__MODULE__.") => {
                let rest = &text["__MODULE__".len()..];
                module.map(|m| format!("{}{rest}", self.current_full(m)))
            }
            _ => None,
        }
    }

    /// Scope of a bare call: `import`/`use` of the enclosing modules, innermost first, then
    /// of the file.
    fn scope(&self, module: Option<usize>) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = module;
        while let Some(i) = cur {
            out.extend(self.mods[i].scope.iter().cloned());
            cur = self.mods[i].parent;
        }
        out.extend(self.file_scope.iter().cloned());
        out
    }

    fn finish(mut self) {
        // Bare names bind to the file's last function with that label.
        let mut by_label: HashMap<&str, &str> = HashMap::new();
        for f in &self.funcs {
            by_label.insert(f.label.as_str(), f.id.as_str());
        }
        let module_by_full: HashMap<&str, usize> = self
            .mods
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.full.is_empty() && m.id.is_some())
            .rev()
            .map(|(i, m)| (m.full.as_str(), i))
            .collect();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut local_edges: Vec<(String, String, u32)> = Vec::new();
        let mut raw: Vec<ExCall> = Vec::new();

        for s in &self.sites {
            match &s.kind {
                SiteKind::Bare(name) => {
                    let label = format!("{name}()");
                    match by_label.get(label.as_str()) {
                        Some(&t) if t != s.caller => {
                            if seen.insert((s.caller.clone(), t.to_string())) {
                                local_edges.push((s.caller.clone(), t.to_string(), s.line));
                            }
                        }
                        _ => raw.push(ExCall {
                            caller: s.caller.clone(),
                            kind: ExCallKind::Bare {
                                name: name.clone(),
                                scope: self.scope(s.module),
                            },
                            line: s.line,
                        }),
                    }
                }
                SiteKind::Member(module, name) => {
                    // `Mod.f` matches by id, so `__field__` finds `field` (EX8).
                    if let Some(&m) = module_by_full.get(module.as_str()) {
                        let want = self.mods[m]
                            .id
                            .as_deref()
                            .map(|mid| Ctx::child_id(mid, name));
                        let target = self.mod_funcs.get(&m).and_then(|fs| {
                            fs.iter()
                                .map(|&i| &self.funcs[i])
                                .find(|f| Some(&f.id) == want.as_ref())
                        });
                        if let Some(t) = target {
                            if t.id != s.caller && seen.insert((s.caller.clone(), t.id.clone())) {
                                local_edges.push((s.caller.clone(), t.id.clone(), s.line));
                            }
                            continue;
                        }
                    }
                    raw.push(ExCall {
                        caller: s.caller.clone(),
                        kind: ExCallKind::Member {
                            module: module.clone(),
                            name: name.clone(),
                        },
                        line: s.line,
                    });
                }
            }
        }
        for (a, b, l) in local_edges {
            self.cx.add_edge(&a, &b, "calls", l, Some("call"));
        }
        self.facts.calls = raw;
        self.cx.facts.lang = LangFacts::Elixir(self.facts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_calls_are_sorted() {
        assert!(NOT_CALLS.windows(2).all(|w| w[0] < w[1]));
    }
}

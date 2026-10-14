//! PowerShell (`.ps1`, `.psm1`, `.psd1`; §41). See `docs/graphify-format.md`.
//!
//! One tree-sitter-powershell parse (the grammar Graphify uses, same version), walked in source
//! order, error nodes included:
//!
//! - `function`/`filter`/`workflow` at any depth outside class methods → `file_slug(name)`,
//!   label `name()` (`global:Scoped()`), `contains` from the file. Nothing is `_callable`.
//! - `class` → `file_slug(Name)`; its base list gives `inherits` (first entry) and `implements`
//!   (the rest); typed properties give `references` (`field`, never to the class itself);
//!   methods (constructors too, overloads merged) → `class_slug(name)`, label `.Name()`, with
//!   `references` to the return type (`return_type`) and typed parameters (`parameter_type`), each
//!   on the type's own line. `enum` → node plus `case_of` members.
//! - A type is the last segment of a plain type name (`[System.IO.FileInfo]` → `FileInfo`); array
//!   and generic types give nothing. It binds to a class or enum of the file, else to a per-file
//!   stub `slug(Name)`, which the cross-file pass may bind to the only PowerShell type so named.
//! - `Import-Module <bare word>`, `. <bare path>` and `using <kind> <name>` give `imports_from`
//!   (no context) from the file to an external: the stem of the path (`Az.Accounts` → `az`,
//!   `.\Utils.ps1` → `utils`) or, for `using`, the last dotted segment (`System.IO` → `io`,
//!   `using module ./Shop.psm1` → `psm1`). Quoted or variable paths give nothing. In a `.psd1`
//!   manifest, `RootModule`, `NestedModules` (every string inside) and `RequiredModules` (strings
//!   and `ModuleName` entries, sorted) give `imports_from` with context `import` on the key's line.
//! - Calls belong to the innermost function or method (top-level code has no caller). A command
//!   naming a function of the file (case-insensitive), `$this.M()` and `[Class]::M()` on a class
//!   of the file are EXTRACTED calls without context, once per pair in caller order; any other
//!   command name is left to the cross-file pass. Class method bodies contribute calls only: no
//!   functions or imports inside them.
//! - No rationale: comments are ignored.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::{slug, Ctx};
use crate::facts::LangFacts;

/// Recursion bound for generated inputs.
const MAX_DEPTH: u32 = 512;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PsFacts {
    /// External ids of the file's imports, for the same-stem file remap.
    pub imports: Vec<String>,
    /// Unresolved type stubs: (local id, name).
    pub stubs: Vec<(String, String)>,
    /// Classes and enums: (name, local id).
    pub types: Vec<(String, String)>,
    /// Functions and methods by name, for commands of other files: (name, local id).
    pub callables: Vec<(String, String)>,
    /// Commands the file could not bind: (caller local id, name, line).
    pub calls: Vec<(String, String, u32)>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let mut w = Walker {
        facts: PsFacts::default(),
        types: HashMap::new(),
        functions: HashMap::new(),
        methods: HashMap::new(),
        seen: HashSet::new(),
        callers: Vec::new(),
        events: Vec::new(),
        cx,
    };
    if let Some(tree) = tree {
        let root = tree.root_node();
        w.collect(root, 0);
        w.walk(root, Scope::default(), 0);
        if w.cx.rel.to_ascii_lowercase().ends_with(".psd1") {
            w.manifest(root);
        }
        w.calls();
    }
    let facts = w.facts;
    cx.facts.lang = LangFacts::PowerShell(facts);
}

#[derive(Clone, Copy, Default)]
struct Scope {
    /// Innermost function or method (index into `callers`).
    caller: Option<usize>,
    /// Inside a class method body.
    in_method: bool,
}

enum Event {
    /// A command name.
    Command {
        caller: usize,
        name: String,
        line: u32,
    },
    /// `$this.M()` (`class: None`) or `[Class]::M()`.
    Member {
        caller: usize,
        class: Option<String>,
        method: String,
        line: u32,
    },
}

struct Walker<'a, 'b> {
    cx: &'b mut Ctx<'a>,
    facts: PsFacts,
    /// Class and enum names of the file → id (first wins).
    types: HashMap<String, String>,
    /// Lower-cased function names of the file → id (first wins).
    functions: HashMap<String, String>,
    /// Class id → (lower-cased method name → id).
    methods: HashMap<String, HashMap<String, String>>,
    /// Node ids already emitted.
    seen: HashSet<String>,
    /// Callers in definition order: (id, enclosing class id).
    callers: Vec<(String, Option<String>)>,
    events: Vec<Event>,
}

fn kids(node: TsNode) -> Vec<TsNode> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

fn first_kind<'t>(node: TsNode<'t>, kind: &str) -> Option<TsNode<'t>> {
    kids(node).into_iter().find(|k| k.kind() == kind)
}

impl Walker<'_, '_> {
    fn text(&self, node: TsNode) -> &str {
        self.cx.text(node)
    }

    fn node_once(&mut self, id: &str, label: String, line: u32) {
        if self.seen.insert(id.to_string()) {
            self.cx.add_def(id.to_string(), label, line, vec![]);
        }
    }

    /// Pre-pass: names of the file's functions, classes, enums and methods, so references and
    /// calls bind to definitions that come later in the file.
    fn collect(&mut self, node: TsNode, depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        match node.kind() {
            "function_statement" => {
                if let Some(name) = first_kind(node, "function_name") {
                    let name = self.text(name).to_string();
                    let id = Ctx::child_id(&self.cx.file_id, &name);
                    self.functions.entry(name.to_lowercase()).or_insert(id);
                }
            }
            "class_statement" | "enum_statement" => {
                if let Some(name) = first_kind(node, "simple_name") {
                    let name = self.text(name).to_string();
                    let id = Ctx::child_id(&self.cx.file_id, &name);
                    self.types.entry(name).or_insert_with(|| id.clone());
                    if node.kind() == "class_statement" {
                        for m in kids(node) {
                            if m.kind() != "class_method_definition" {
                                continue;
                            }
                            if let Some(mn) = first_kind(m, "simple_name") {
                                let mn = self.text(mn).to_string();
                                let mid = Ctx::child_id(&id, &mn);
                                self.methods
                                    .entry(id.clone())
                                    .or_default()
                                    .entry(mn.to_lowercase())
                                    .or_insert(mid);
                            }
                        }
                    }
                }
                return;
            }
            _ => {}
        }
        for k in kids(node) {
            self.collect(k, depth + 1);
        }
    }

    fn walk(&mut self, node: TsNode, sc: Scope, depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        match node.kind() {
            "function_statement" => {
                if sc.in_method {
                    return;
                }
                let Some(name) = first_kind(node, "function_name") else {
                    return;
                };
                let name = self.text(name).to_string();
                let id = Ctx::child_id(&self.cx.file_id, &name);
                let line = Ctx::line(node);
                self.node_once(&id, format!("{name}()"), line);
                let file = self.cx.file_id.clone();
                self.cx.add_edge(&file, &id, "contains", line, None);
                self.facts.callables.push((name, id.clone()));
                self.callers.push((id, None));
                let inner = Scope {
                    caller: Some(self.callers.len() - 1),
                    in_method: false,
                };
                for k in kids(node) {
                    self.walk(k, inner, depth + 1);
                }
                return;
            }
            "class_statement" if !sc.in_method => {
                self.class(node, depth);
                return;
            }
            "enum_statement" if !sc.in_method => {
                self.enumeration(node);
                return;
            }
            "command" => self.command(node, sc),
            "invokation_expression" => self.invocation(node, sc),
            _ => {}
        }
        for k in kids(node) {
            self.walk(k, sc, depth + 1);
        }
    }

    fn class(&mut self, node: TsNode, depth: u32) {
        let names: Vec<TsNode> = kids(node)
            .into_iter()
            .filter(|k| k.kind() == "simple_name")
            .collect();
        let Some((&name_node, bases)) = names.split_first() else {
            return;
        };
        let name = self.text(name_node).to_string();
        let id = Ctx::child_id(&self.cx.file_id, &name);
        let line = Ctx::line(node);
        self.node_once(&id, name.clone(), line);
        let file = self.cx.file_id.clone();
        self.cx.add_edge(&file, &id, "contains", line, None);
        self.facts.types.push((name.clone(), id.clone()));
        for (i, b) in bases.iter().enumerate() {
            let base = self.text(*b).to_string();
            let target = self.type_target(&base);
            let rel = if i == 0 { "inherits" } else { "implements" };
            self.cx.add_edge(&id, &target, rel, Ctx::line(*b), None);
        }
        for m in kids(node) {
            match m.kind() {
                "class_property_definition" => {
                    let Some(t) = first_kind(m, "type_literal") else {
                        continue;
                    };
                    let Some(tname) = self.type_name(t) else {
                        continue;
                    };
                    if tname == name {
                        continue;
                    }
                    let target = self.type_target(&tname);
                    self.cx
                        .add_edge(&id, &target, "references", Ctx::line(t), Some("field"));
                }
                "class_method_definition" => self.method(m, &id, depth),
                _ => {}
            }
        }
    }

    fn method(&mut self, node: TsNode, class: &str, depth: u32) {
        let Some(name) = first_kind(node, "simple_name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(class, &name);
        let line = Ctx::line(node);
        self.node_once(&id, format!(".{name}()"), line);
        self.cx.add_edge(class, &id, "method", line, None);
        self.facts.callables.push((name, id.clone()));
        if let Some(ret) = first_kind(node, "type_literal") {
            if let Some(t) = self.type_name(ret) {
                let target = self.type_target(&t);
                self.cx.add_edge(
                    &id,
                    &target,
                    "references",
                    Ctx::line(ret),
                    Some("return_type"),
                );
            }
        }
        if let Some(params) = first_kind(node, "class_method_parameter_list") {
            for p in kids(params) {
                let Some(t) = first_kind(p, "type_literal") else {
                    continue;
                };
                if let Some(tname) = self.type_name(t) {
                    let target = self.type_target(&tname);
                    self.cx.add_edge(
                        &id,
                        &target,
                        "references",
                        Ctx::line(t),
                        Some("parameter_type"),
                    );
                }
            }
        }
        self.callers.push((id, Some(class.to_string())));
        let sc = Scope {
            caller: Some(self.callers.len() - 1),
            in_method: true,
        };
        if let Some(body) = first_kind(node, "script_block") {
            self.walk(body, sc, depth + 1);
        }
    }

    fn enumeration(&mut self, node: TsNode) {
        let Some(name) = first_kind(node, "simple_name") else {
            return;
        };
        let name = self.text(name).to_string();
        let id = Ctx::child_id(&self.cx.file_id, &name);
        let line = Ctx::line(node);
        self.node_once(&id, name.clone(), line);
        let file = self.cx.file_id.clone();
        self.cx.add_edge(&file, &id, "contains", line, None);
        self.facts.types.push((name, id.clone()));
        for m in kids(node) {
            if m.kind() != "enum_member" {
                continue;
            }
            let Some(mn) = first_kind(m, "simple_name") else {
                continue;
            };
            let mn = self.text(mn).to_string();
            let mid = Ctx::child_id(&id, &mn);
            let ml = Ctx::line(m);
            self.node_once(&mid, mn, ml);
            self.cx.add_edge(&id, &mid, "case_of", ml, None);
        }
    }

    /// The simple name a `[Type]` literal refers to: the last segment of a plain type name.
    fn type_name(&self, lit: TsNode) -> Option<String> {
        let spec = first_kind(lit, "type_spec")?;
        let name = first_kind(spec, "type_name")?;
        let last = kids(name)
            .into_iter()
            .rfind(|k| k.kind() == "type_identifier")?;
        Some(self.text(last).to_string())
    }

    /// A class or enum of the file, else the file's stub for `name` (emitted at first use).
    fn type_target(&mut self, name: &str) -> String {
        if let Some(id) = self.types.get(name) {
            return id.clone();
        }
        let id = slug(name);
        if self.seen.insert(id.clone()) {
            self.cx.add_stub(id.clone(), name.to_string());
            self.facts.stubs.push((id.clone(), name.to_string()));
        }
        id
    }

    fn command(&mut self, node: TsNode, sc: Scope) {
        let ks = kids(node);
        let Some(first) = ks.first() else {
            return;
        };
        match first.kind() {
            "command_name" => {
                let name = self.text(*first).to_string();
                if !sc.in_method {
                    self.import_command(&name, &ks, Ctx::line(node));
                }
                if let Some(caller) = sc.caller {
                    self.events.push(Event::Command {
                        caller,
                        name,
                        line: Ctx::line(node),
                    });
                }
            }
            "command_invokation_operator" if self.text(*first) == "." && !sc.in_method => {
                let Some(expr) = ks.get(1).filter(|k| k.kind() == "command_name_expr") else {
                    return;
                };
                let Some(path) = first_kind(*expr, "command_name") else {
                    return;
                };
                let ext = slug(&stem(self.text(path)));
                self.import(ext, Ctx::line(node), None);
            }
            _ => {}
        }
    }

    fn import_command(&mut self, name: &str, ks: &[TsNode], line: u32) {
        let tokens: Vec<String> = ks
            .iter()
            .filter(|k| k.kind() == "command_elements")
            .flat_map(|e| kids(*e))
            .filter(|k| k.kind() == "generic_token")
            .map(|k| self.text(k).to_string())
            .collect();
        let ext = if name.eq_ignore_ascii_case("Import-Module") {
            let Some(t) = tokens.first() else { return };
            slug(&stem(t))
        } else if name.eq_ignore_ascii_case("using") {
            let Some(t) = tokens.get(1) else { return };
            slug(t.rsplit('.').next().unwrap_or(t))
        } else {
            return;
        };
        self.import(ext, line, None);
    }

    fn import(&mut self, ext: String, line: u32, context: Option<&str>) {
        if ext.is_empty() {
            return;
        }
        let file = self.cx.file_id.clone();
        self.cx.add_edge(&file, &ext, "imports_from", line, context);
        self.facts.imports.push(ext);
    }

    fn invocation(&mut self, node: TsNode, sc: Scope) {
        let Some(caller) = sc.caller else { return };
        let ks = kids(node);
        let Some(member) = ks.iter().find(|k| k.kind() == "member_name") else {
            return;
        };
        let method = self.text(*member).to_string();
        let class = match ks.first() {
            Some(k) if k.kind() == "variable" && self.text(*k).eq_ignore_ascii_case("$this") => {
                None
            }
            Some(k) if k.kind() == "type_literal" => match self.type_name(*k) {
                Some(t) => Some(t),
                None => return,
            },
            _ => return,
        };
        self.events.push(Event::Member {
            caller,
            class,
            method,
            line: Ctx::line(node),
        });
    }

    /// In-file calls, by caller in definition order, once per pair; the rest go to `facts.calls`.
    fn calls(&mut self) {
        let mut by_caller: Vec<Vec<usize>> = vec![Vec::new(); self.callers.len()];
        for (i, e) in self.events.iter().enumerate() {
            let c = match e {
                Event::Command { caller, .. } | Event::Member { caller, .. } => *caller,
            };
            by_caller[c].push(i);
        }
        let mut done: HashSet<(String, String)> = HashSet::new();
        for (ci, list) in by_caller.iter().enumerate() {
            let (src, own_class) = self.callers[ci].clone();
            for &ei in list {
                let (target, line) = match &self.events[ei] {
                    Event::Command { name, line, .. } => {
                        match self.functions.get(&name.to_lowercase()) {
                            Some(t) => (t.clone(), *line),
                            None => {
                                self.facts.calls.push((src.clone(), name.clone(), *line));
                                continue;
                            }
                        }
                    }
                    Event::Member {
                        class,
                        method,
                        line,
                        ..
                    } => {
                        let class_id = match class {
                            None => own_class.clone(),
                            Some(c) => self.types.get(c).cloned(),
                        };
                        let Some(t) = class_id
                            .and_then(|c| self.methods.get(&c))
                            .and_then(|m| m.get(&method.to_lowercase()))
                        else {
                            continue;
                        };
                        (t.clone(), *line)
                    }
                };
                if done.insert((src.clone(), target.clone())) {
                    self.cx.add_edge(&src, &target, "calls", line, None);
                }
            }
        }
    }

    /// `.psd1` module manifest entries of the top-level hashtable.
    fn manifest(&mut self, root: TsNode) {
        let Some(body) = find_kind(root, "hash_literal_body", 0) else {
            return;
        };
        for entry in kids(body) {
            if entry.kind() != "hash_entry" {
                continue;
            }
            let Some(key) = first_kind(entry, "key_expression") else {
                continue;
            };
            if first_kind(key, "simple_name").is_none() {
                continue;
            }
            let Some(value) = first_kind(entry, "pipeline") else {
                continue;
            };
            let line = Ctx::line(entry);
            let names: Vec<String> = match self.text(key) {
                "RootModule" | "NestedModules" => {
                    let mut out = Vec::new();
                    self.strings(value, false, &mut out);
                    out
                }
                "RequiredModules" => {
                    let mut out = Vec::new();
                    self.strings(value, true, &mut out);
                    out.sort();
                    out
                }
                _ => continue,
            };
            for n in names {
                self.import(slug(&stem(&n)), line, Some("import"));
            }
        }
    }

    /// String literals under `node` in order; with `modules`, a nested hashtable contributes its
    /// `ModuleName` entry only.
    fn strings(&self, node: TsNode, modules: bool, out: &mut Vec<String>) {
        match node.kind() {
            "string_literal" => {
                let t = self.text(node);
                let inner = t
                    .strip_prefix(['\'', '"'])
                    .and_then(|s| s.strip_suffix(['\'', '"']))
                    .unwrap_or(t);
                out.push(inner.to_string());
                return;
            }
            "hash_literal_body" if modules => {
                for entry in kids(node) {
                    let key = first_kind(entry, "key_expression").map(|k| self.text(k));
                    if key == Some("ModuleName") {
                        if let Some(v) = first_kind(entry, "pipeline") {
                            self.strings(v, false, out);
                        }
                    }
                }
                return;
            }
            _ => {}
        }
        for k in kids(node) {
            self.strings(k, modules, out);
        }
    }
}

fn find_kind<'t>(node: TsNode<'t>, kind: &str, depth: u32) -> Option<TsNode<'t>> {
    if node.kind() == kind {
        return Some(node);
    }
    if depth > MAX_DEPTH {
        return None;
    }
    kids(node)
        .into_iter()
        .find_map(|k| find_kind(k, kind, depth + 1))
}

/// The file stem of a path written with `/` or `\`: the last component without its last
/// extension (`Az.Accounts` → `Az`, `.\lib\b.c.psm1` → `b.c`).
pub fn stem(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match base.rfind('.') {
        Some(i) if i > 0 => base[..i].to_string(),
        _ => base.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_drop_directories_and_the_last_extension() {
        assert_eq!(stem("Az.Accounts"), "Az");
        assert_eq!(stem("./mods/b.c.psm1"), "b.c");
        assert_eq!(stem(r"C:\abs\Win.psm1"), "Win");
        assert_eq!(stem(".hidden"), ".hidden");
    }
}

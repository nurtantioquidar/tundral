//! Julia (§32) extraction. See `docs/graphify-format.md`.
//!
//! Every definition id hangs off the file id, whatever module encloses it: modules
//! (`file_<slug(name)>`, label the name), abstract types and structs (label the bare name),
//! functions and short-form methods (`file_<slug(last name segment)>`, label the name as written
//! plus `()`: `area()`, `Base.show()`; an operator such as `Base.:+` slugs to nothing and lands on
//! the file id), functors `(c::Circle)(x)` (`file_circle_call`, `(Circle)()`) and macros
//! (`file_<name>`, `@name`). `using`/`import` give a node per module (id `slug(module path)`,
//! label the path without leading dots, located at the statement) and an `imports` edge from the
//! enclosing module or the file.
//!
//! Edges: `defines` (file → every module, nested ones included; module or file → its types and
//! functions, one per method), `inherits` (`<: Parent`) and `references` with context `field`
//! (`x::T` with a plain type name that is not one of the struct's parameters). Both target a
//! node of the file defined before the struct when one has the id `file_<slug(T)>`, else a
//! per-file stub `slug(T)`, which `crate::resolve::julia` binds by label. Calls inside function
//! bodies (nested function definitions excluded) target `file_<slug(callee)>` whether or not it
//! exists: only calls to the file's own definitions survive the build. Comments and docstrings
//! give nothing.

use std::collections::HashSet;

use tree_sitter::{Node as TsNode, Tree};

use super::{slug, Ctx};

fn named_children<'t>(node: TsNode<'t>) -> Vec<TsNode<'t>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}

struct Walker<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    ids: HashSet<String>,
    stubs: HashSet<String>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(Vec::new());
    let mut w = Walker {
        cx,
        ids: HashSet::new(),
        stubs: HashSet::new(),
    };
    if let Some(tree) = tree {
        let file = w.cx.file_id.clone();
        w.ids.insert(file.clone());
        for c in named_children(tree.root_node()) {
            w.walk(c, &file);
        }
    }
}

impl Walker<'_, '_> {
    fn text(&self, n: TsNode) -> String {
        n.utf8_text(self.cx.src).unwrap_or("").to_string()
    }

    fn add(&mut self, id: &str, label: String, line: u32) {
        if self.ids.insert(id.to_string()) {
            self.cx.add_def(id.to_string(), label, line, Vec::new());
        }
    }

    /// Definitions at module (or file) level, found at any depth outside function bodies.
    fn walk(&mut self, node: TsNode, owner: &str) {
        let line = Ctx::line(node);
        let file = self.cx.file_id.clone();
        match node.kind() {
            "module_definition" => {
                let Some(name) = node.child_by_field_name("name").map(|n| self.text(n)) else {
                    return;
                };
                let id = Ctx::child_id(&file, &name);
                self.add(&id, name, line);
                self.cx.add_edge(&file, &id, "defines", line, None);
                for c in named_children(node) {
                    if Some(c) != node.child_by_field_name("name") {
                        self.walk(c, &id);
                    }
                }
            }
            "using_statement" | "import_statement" => {
                for item in named_children(node) {
                    let Some(name) = self.import_name(item) else {
                        continue;
                    };
                    let id = slug(&name);
                    if id.is_empty() {
                        continue;
                    }
                    self.add(&id, name, line);
                    self.cx
                        .add_edge(owner, &id, "imports", line, Some("import"));
                }
            }
            "abstract_definition" | "struct_definition" => self.type_def(node, owner),
            "function_definition" | "macro_definition" => {
                let macro_ = node.kind() == "macro_definition";
                let sig = named_children(node)
                    .into_iter()
                    .find(|c| c.kind() == "signature");
                let Some(call) = sig.and_then(|s| signature_call(s)) else {
                    return;
                };
                let Some(id) = self.function_node(call, owner, line, macro_) else {
                    return;
                };
                for c in named_children(node) {
                    if c.kind() != "signature" {
                        self.calls(c, &id);
                    }
                }
            }
            "assignment" => {
                let kids = named_children(node);
                let Some(call) = kids.first().and_then(|l| signature_call(*l)) else {
                    return;
                };
                let Some(id) = self.function_node(call, owner, line, false) else {
                    return;
                };
                for c in kids.into_iter().skip(1) {
                    self.calls(c, &id);
                }
            }
            "const_statement" | "primitive_definition" => {}
            _ => {
                for c in named_children(node) {
                    self.walk(c, owner);
                }
            }
        }
    }

    /// `Base.Threads`, `..Parent` → `Parent`; `X: a, b` → `X`; `X as Y` gives nothing.
    fn import_name(&self, item: TsNode) -> Option<String> {
        match item.kind() {
            "identifier" => Some(self.text(item)),
            "scoped_identifier" => {
                let mut parts = Vec::new();
                collect_identifiers(item, self.cx.src, &mut parts);
                (!parts.is_empty()).then(|| parts.join("."))
            }
            "import_path" => Some(self.text(item).trim_start_matches('.').to_string()),
            "selected_import" => named_children(item)
                .first()
                .and_then(|f| self.import_name(*f)),
            _ => None,
        }
    }

    /// A function, method or macro node for the call in its signature; returns its id.
    fn function_node(
        &mut self,
        call: TsNode,
        owner: &str,
        line: u32,
        macro_: bool,
    ) -> Option<String> {
        let file = self.cx.file_id.clone();
        let callee = named_children(call).into_iter().next()?;
        let (id, label) = match callee.kind() {
            "identifier" | "operator" => {
                let name = self.text(callee);
                let label = if macro_ {
                    format!("@{name}")
                } else {
                    format!("{name}()")
                };
                (Ctx::child_id(&file, &name), label)
            }
            "field_expression" => {
                let last = last_name(callee, self.cx.src)?;
                (
                    Ctx::child_id(&file, &last),
                    format!("{}()", self.text(callee)),
                )
            }
            "parenthesized_expression" => {
                let typed = named_children(callee)
                    .into_iter()
                    .find(|c| c.kind() == "typed_expression")?;
                let ty = named_children(typed).into_iter().nth(1)?;
                let t = self.text(ty);
                (
                    Ctx::child_id(&file, &format!("{t}_call")),
                    format!("({t})()"),
                )
            }
            _ => return None,
        };
        self.add(&id, label, line);
        self.cx.add_edge(owner, &id, "defines", line, None);
        Some(id)
    }

    fn type_def(&mut self, node: TsNode, owner: &str) {
        let line = Ctx::line(node);
        let file = self.cx.file_id.clone();
        let Some(head) = named_children(node)
            .into_iter()
            .find(|c| c.kind() == "type_head")
        else {
            return;
        };
        let Some(inner) = named_children(head).into_iter().next() else {
            return;
        };
        let (decl, parent) = if inner.kind() == "binary_expression" {
            let parts = named_children(inner);
            let op = parts.iter().find(|p| p.kind() == "operator");
            if op.map(|o| self.text(*o)).as_deref() != Some("<:") {
                return;
            }
            let operands: Vec<TsNode> = parts
                .into_iter()
                .filter(|p| p.kind() != "operator")
                .collect();
            (operands.first().copied(), operands.get(1).copied())
        } else {
            (Some(inner), None)
        };
        let Some(decl) = decl else {
            return;
        };
        let (name, params) = match decl.kind() {
            "identifier" => (self.text(decl), Vec::new()),
            "parametrized_type_expression" => {
                let kids = named_children(decl);
                let Some(base) = kids.first().filter(|k| k.kind() == "identifier") else {
                    return;
                };
                let mut params = Vec::new();
                for c in kids.iter().filter(|k| k.kind() == "curly_expression") {
                    for p in named_children(*c) {
                        let n = if p.kind() == "binary_expression" {
                            named_children(p).into_iter().next()
                        } else {
                            Some(p)
                        };
                        if let Some(n) = n.filter(|n| n.kind() == "identifier") {
                            params.push(self.text(n));
                        }
                    }
                }
                (self.text(*base), params)
            }
            _ => return,
        };
        let id = Ctx::child_id(&file, &name);
        self.add(&id, name, line);
        self.cx.add_edge(owner, &id, "defines", line, None);
        if let Some(p) = parent.and_then(|p| base_type(p, self.cx.src)) {
            let target = self.type_target(&p);
            self.cx.add_edge(&id, &target, "inherits", line, None);
        }
        if node.kind() != "struct_definition" {
            return;
        }
        for field in named_children(node) {
            if field.kind() != "typed_expression" {
                continue;
            }
            let kids = named_children(field);
            let Some(ty) = kids.get(1).filter(|t| t.kind() == "identifier") else {
                continue;
            };
            let t = self.text(*ty);
            if params.contains(&t) {
                continue;
            }
            let target = self.type_target(&t);
            self.cx
                .add_edge(&id, &target, "references", Ctx::line(field), Some("field"));
        }
    }

    /// A type name binds to the file's node with id `file_<slug(name)>` defined so far, else
    /// to a per-file stub.
    fn type_target(&mut self, name: &str) -> String {
        let own = Ctx::child_id(&self.cx.file_id, name);
        if self.ids.contains(&own) {
            return own;
        }
        let id = slug(name);
        if self.stubs.insert(id.clone()) {
            self.cx.add_stub(id.clone(), name.to_string());
        }
        id
    }

    /// Calls in a function body; nested function definitions are skipped.
    fn calls(&mut self, node: TsNode, caller: &str) {
        match node.kind() {
            "function_definition" | "macro_definition" => return,
            "call_expression" | "broadcast_call_expression" => {
                if let Some(callee) = named_children(node).into_iter().next() {
                    let name = match callee.kind() {
                        "identifier" => Some(self.text(callee)),
                        "field_expression" => last_name(callee, self.cx.src),
                        _ => None,
                    };
                    if let Some(name) = name {
                        self.call_edge(caller, &name, Ctx::line(node));
                    }
                }
            }
            "binary_expression" => {
                let kids = named_children(node);
                let piped = kids
                    .iter()
                    .any(|k| k.kind() == "operator" && self.text(*k) == "|>");
                if let (true, Some(last)) = (piped, kids.last()) {
                    if last.kind() == "identifier" {
                        let name = self.text(*last);
                        self.call_edge(caller, &name, Ctx::line(node));
                    }
                }
            }
            _ => {}
        }
        for c in named_children(node) {
            self.calls(c, caller);
        }
    }

    fn call_edge(&mut self, caller: &str, name: &str, line: u32) {
        let target = Ctx::child_id(&self.cx.file_id, name);
        self.cx
            .add_edge(caller, &target, "calls", line, Some("call"));
    }
}

/// The call in a signature: `f(x)`, `f(x)::T`, `f(x) where T`.
fn signature_call(node: TsNode) -> Option<TsNode> {
    match node.kind() {
        "call_expression" => Some(node),
        "signature" | "typed_expression" | "where_expression" => named_children(node)
            .into_iter()
            .next()
            .and_then(signature_call),
        _ => None,
    }
}

/// The last name of `a.b.c` (`c`) or `Base.:+` (`+`).
fn last_name(field: TsNode, src: &[u8]) -> Option<String> {
    let last = named_children(field).into_iter().last()?;
    let text = last.utf8_text(src).ok()?;
    match last.kind() {
        "identifier" | "operator" => Some(text.to_string()),
        "quote_expression" => Some(text.trim_start_matches(':').to_string()),
        _ => None,
    }
}

/// `Parent` of `<: Parent` or `<: Parent{T}`; qualified parents give nothing.
fn base_type(node: TsNode, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => node.utf8_text(src).ok().map(str::to_string),
        "parametrized_type_expression" => named_children(node)
            .into_iter()
            .next()
            .filter(|n| n.kind() == "identifier")
            .and_then(|n| n.utf8_text(src).ok().map(str::to_string)),
        _ => None,
    }
}

fn collect_identifiers(node: TsNode, src: &[u8], out: &mut Vec<String>) {
    for c in named_children(node) {
        match c.kind() {
            "identifier" => out.push(c.utf8_text(src).unwrap_or("").to_string()),
            "scoped_identifier" => collect_identifiers(c, src, out),
            _ => {}
        }
    }
}

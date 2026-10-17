//! Local type inference for Go method calls (dijkstral-native, quirk G5 not reproduced for
//! typed receivers).
//!
//! The walk over a function body that records its calls also tracks, per block scope, the
//! variables whose type a declaration fixes: the receiver, parameters and named results, `var x
//! T`, `x := T{..}` / `&T{..}` / `new(T)` / `v.(T)`, and what a call returns (`x, err :=
//! NewT()`, `a.B().C()`), a field's declared type, and a type switch's single-type case. Each
//! `recv.M(..)` on such a receiver is recorded with a symbolic type ([`GoTy`]) that the cross-file
//! pass (`crate::resolve::go`) evaluates against the package's declarations.
//!
//! Soundness rules: `:=` declares a new variable unless the name already exists in the same
//! scope (then it assigns, and a Go variable never changes type); every other binding (range
//! variables, receive statements, closure parameters without a named type) is unknown and
//! shadows; type parameters, builtin types, slices, maps, channels and function types have no
//! type here.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use tree_sitter::{Node as TsNode, TreeCursor};

use super::{is_builtin_type, named_children, Go, GoCallee, GoMethodCall, GoTy, GoWritten};
use crate::extract::Ctx;

/// A `recv.M(..)` site, before the file's in-file targets are known.
pub(super) struct RawGoCall {
    /// Index of the caller's node.
    caller: u32,
    recv: u32,
    name: String,
    line: u32,
}

/// Top-level packages of the Go standard library, sorted (an import path without a dot in its
/// first element that starts with one of them).
const STD_ROOTS: &[&str] = &[
    "archive",
    "bufio",
    "bytes",
    "cmp",
    "compress",
    "container",
    "context",
    "crypto",
    "database",
    "debug",
    "embed",
    "encoding",
    "errors",
    "expvar",
    "flag",
    "fmt",
    "go",
    "hash",
    "html",
    "image",
    "index",
    "io",
    "iter",
    "log",
    "maps",
    "math",
    "mime",
    "net",
    "os",
    "path",
    "plugin",
    "reflect",
    "regexp",
    "runtime",
    "slices",
    "sort",
    "strconv",
    "strings",
    "sync",
    "syscall",
    "testing",
    "text",
    "time",
    "unicode",
    "unique",
    "unsafe",
    "weak",
];

/// Nesting beyond which a body is walked for its calls only (no inference), on an explicit
/// stack.
const MAX_DEPTH: u32 = 512;

/// The node kinds the body walk dispatches on, as grammar ids (every node of every body is
/// visited, so string comparisons would dominate).
struct Kinds {
    call: u16,
    func_literal: u16,
    type_switch: u16,
    short_var: u16,
    var_spec: u16,
    range_clause: u16,
    receive: u16,
    /// Kinds that open a scope.
    scopes: [u16; 8],
}

static KINDS: std::sync::LazyLock<Kinds> = std::sync::LazyLock::new(|| {
    let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    let id = |k: &str| lang.id_for_node_kind(k, true);
    Kinds {
        call: id("call_expression"),
        func_literal: id("func_literal"),
        type_switch: id("type_switch_statement"),
        short_var: id("short_var_declaration"),
        var_spec: id("var_spec"),
        range_clause: id("range_clause"),
        receive: id("receive_statement"),
        scopes: [
            id("block"),
            id("if_statement"),
            id("for_statement"),
            id("expression_switch_statement"),
            id("expression_case"),
            id("default_case"),
            id("communication_case"),
            id("select_statement"),
        ],
    }
});

/// A multiplicative hash for tree-sitter node ids (the memo's keys), cheaper than SipHash.
#[derive(Default)]
struct NodeHasher(u64);

impl Hasher for NodeHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }

    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Child lists while walking a body: one buffer used as a stack and one cursor, so visiting a
/// node allocates nothing.
struct Kids<'t> {
    buf: Vec<TsNode<'t>>,
    cursor: TreeCursor<'t>,
}

struct Scope<'a> {
    /// (name, type when fixed, scope level), innermost last.
    vars: Vec<(&'a str, Option<u32>, u32)>,
    level: u32,
    /// Types by expression node id.
    memo: HashMap<usize, Option<u32>, BuildHasherDefault<NodeHasher>>,
    /// Type parameters in scope: never a type here.
    type_params: Vec<String>,
    caller: String,
    /// Index of the caller's node.
    caller_node: u32,
    /// The caller's definition holds its id in the file: only then do its method calls count
    /// (a method sharing an interface method's id would start edges from the interface).
    record: bool,
}

impl<'a> Scope<'a> {
    fn var(&self, name: &str) -> Option<Option<u32>> {
        self.vars
            .iter()
            .rev()
            .find(|(n, _, _)| *n == name)
            .map(|(_, t, _)| *t)
    }

    fn declare(&mut self, name: &'a str, ty: Option<u32>) {
        if name == "_" {
            return;
        }
        self.vars.push((name, ty, self.level));
    }

    /// `:=`: a name already declared in this scope is assigned, not redeclared.
    fn short_declare(&mut self, name: &'a str, ty: Option<u32>) {
        let here = self
            .vars
            .iter()
            .rev()
            .take_while(|(_, _, l)| *l == self.level)
            .any(|(n, _, _)| *n == name);
        if !here {
            self.declare(name, ty);
        }
    }

    fn open(&mut self) -> usize {
        self.level += 1;
        self.vars.len()
    }

    fn close(&mut self, mark: usize) {
        self.level -= 1;
        self.vars.truncate(mark);
    }
}

impl<'a> Go<'_, 'a> {
    /// The named type a type expression denotes once pointers and type arguments are peeled:
    /// `T`, `*T`, `pkg.T`, `T[U]`. Type parameters and builtin types are not named types here.
    pub(super) fn written_ty(&self, ty: TsNode, type_params: &[String]) -> Option<GoWritten> {
        match ty.kind() {
            "pointer_type" | "parenthesized_type" => {
                self.written_ty(ty.named_child(0)?, type_params)
            }
            "generic_type" => self.written_ty(ty.child_by_field_name("type")?, type_params),
            "type_identifier" => {
                let name = self.text(ty);
                if is_builtin_type(name) || type_params.iter().any(|p| p == name) {
                    return None;
                }
                Some(GoWritten {
                    qual: None,
                    name: name.to_string(),
                })
            }
            "qualified_type" => {
                let q = self.text(ty.child_by_field_name("package")?);
                if self.std_qualifier(q) {
                    return None;
                }
                Some(GoWritten {
                    qual: Some(q.to_string()),
                    name: self.text(ty.child_by_field_name("name")?).to_string(),
                })
            }
            _ => None,
        }
    }

    /// One written type per result of a signature (`(a, b T, err error)` gives three).
    pub(super) fn results(&self, decl: TsNode, type_params: &[String]) -> Vec<Option<GoWritten>> {
        let Some(result) = decl.child_by_field_name("result") else {
            return Vec::new();
        };
        if result.kind() != "parameter_list" {
            return vec![self.written_ty(result, type_params)];
        }
        let mut out = Vec::new();
        for p in named_children(result) {
            let ty = p
                .child_by_field_name("type")
                .and_then(|t| self.written_ty(t, type_params));
            let mut c = p.walk();
            let names = p.children_by_field_name("name", &mut c).count().max(1);
            out.extend(std::iter::repeat_n(ty, names));
        }
        out
    }

    /// An import of the standard library (`testing`, `net/http`): nothing it declares is in
    /// the repository, so nothing reached through it is recorded.
    fn std_qualifier(&self, q: &str) -> bool {
        self.qualifiers.get(q).is_some_and(|p| {
            let root = p.split('/').next().unwrap_or(p);
            STD_ROOTS.binary_search(&root).is_ok()
        })
    }

    fn alloc(&mut self, t: GoTy) -> u32 {
        self.tys.push(t);
        (self.tys.len() - 1) as u32
    }

    fn alloc_written(&mut self, w: Option<GoWritten>) -> Option<u32> {
        w.map(|w| self.alloc(GoTy::Written(w)))
    }

    /// Bind a parameter list's names (`a, b T`) to their written type.
    fn bind_params(&mut self, s: &mut Scope<'a>, list: Option<TsNode>) {
        let Some(list) = list else { return };
        if list.kind() != "parameter_list" {
            return;
        }
        for p in named_children(list) {
            let ty = match p.kind() {
                "parameter_declaration" => p
                    .child_by_field_name("type")
                    .and_then(|t| self.written_ty(t, &s.type_params)),
                // `xs ...T` is a slice.
                _ => None,
            };
            let ty = self.alloc_written(ty);
            let mut c = p.walk();
            let names: Vec<TsNode> = p.children_by_field_name("name", &mut c).collect();
            for n in names {
                s.declare(self.text(n), ty);
            }
        }
    }

    /// Walk one function or method body: record its calls (as the plain walk did, in source
    /// order) and, for typed receivers, its method calls. `node` is the index of the caller's
    /// node when its definition holds its id (only then are method calls recorded).
    pub(super) fn walk_body(
        &mut self,
        caller: &str,
        node: Option<u32>,
        decl: TsNode,
        body: TsNode,
        type_params: &[String],
        receiver: Option<&str>,
    ) {
        let mut s = Scope {
            caller_node: node.unwrap_or(0),
            record: node.is_some(),
            vars: Vec::new(),
            level: 0,
            memo: HashMap::default(),
            type_params: type_params.to_vec(),
            caller: caller.to_string(),
        };
        if let (Some(recv), Some(list)) = (receiver, decl.child_by_field_name("receiver")) {
            let base = recv.split('[').next().unwrap_or(recv);
            let ty = Some(self.alloc(GoTy::Written(GoWritten {
                qual: None,
                name: base.to_string(),
            })));
            for p in named_children(list) {
                if let Some(n) = p.child_by_field_name("name") {
                    s.declare(self.text(n), ty);
                }
            }
        }
        self.bind_params(&mut s, decl.child_by_field_name("parameters"));
        self.bind_params(&mut s, decl.child_by_field_name("result"));
        let mut k = Kids {
            buf: Vec::new(),
            cursor: body.walk(),
        };
        self.walk(&mut s, &mut k, body, 0);
    }

    fn walk_children<'t>(
        &mut self,
        s: &mut Scope<'a>,
        k: &mut Kids<'t>,
        n: TsNode<'t>,
        depth: u32,
    ) {
        let start = k.buf.len();
        k.buf.extend(n.named_children(&mut k.cursor));
        for i in start..k.buf.len() {
            let c = k.buf[i];
            self.walk(s, k, c, depth + 1);
        }
        k.buf.truncate(start);
    }

    fn walk<'t>(&mut self, s: &mut Scope<'a>, k: &mut Kids<'t>, n: TsNode<'t>, depth: u32) {
        if depth > MAX_DEPTH {
            let caller = std::mem::take(&mut s.caller);
            self.plain_calls(&caller, n);
            s.caller = caller;
            return;
        }
        let kinds = &*KINDS;
        match n.kind_id() {
            id if id == kinds.call => {
                if let Some(f) = n.child_by_field_name("function") {
                    let line = Ctx::line(n);
                    let caller = std::mem::take(&mut s.caller);
                    self.call(&caller, f, line);
                    s.caller = caller;
                    self.method_site(s, f, line);
                }
                self.walk_children(s, k, n, depth);
            }
            id if id == kinds.func_literal => {
                let mark = s.open();
                self.bind_params(s, n.child_by_field_name("parameters"));
                self.bind_params(s, n.child_by_field_name("result"));
                self.walk_children(s, k, n, depth);
                s.close(mark);
            }
            id if kinds.scopes.contains(&id) => {
                let mark = s.open();
                self.walk_children(s, k, n, depth);
                s.close(mark);
            }
            id if id == kinds.type_switch => {
                let mark = s.open();
                let alias: Vec<&'a str> = n
                    .child_by_field_name("alias")
                    .map(|a| {
                        named_children(a)
                            .into_iter()
                            .filter(|i| i.kind() == "identifier")
                            .map(|i| self.text(i))
                            .collect()
                    })
                    .unwrap_or_default();
                for c in named_children(n) {
                    match c.kind() {
                        "type_case" | "default_case" => {
                            let case = s.open();
                            // `case *T:` fixes the alias's type; several types or `nil` do not.
                            let mut c2 = c.walk();
                            let types: Vec<TsNode> =
                                c.children_by_field_name("type", &mut c2).collect();
                            let ty = match types.as_slice() {
                                [t] => {
                                    let w = self.written_ty(*t, &s.type_params);
                                    self.alloc_written(w)
                                }
                                _ => None,
                            };
                            for a in &alias {
                                s.declare(a, ty);
                            }
                            self.walk_children(s, k, c, depth);
                            s.close(case);
                        }
                        _ => self.walk(s, k, c, depth + 1),
                    }
                }
                s.close(mark);
            }
            id if id == kinds.short_var => {
                // Every child, as the plain call walk did (a recovered parse can put calls
                // anywhere); the names bind afterwards.
                self.walk_children(s, k, n, depth);
                let names = self.left_names(n.child_by_field_name("left"));
                let types = self.right_types(s, n.child_by_field_name("right"), names.len());
                for (name, ty) in names.into_iter().zip(types) {
                    if let Some(name) = name {
                        s.short_declare(name, ty);
                    }
                }
            }
            id if id == kinds.var_spec => {
                self.walk_children(s, k, n, depth);
                let mut c = n.walk();
                let names: Vec<&'a str> = n
                    .children_by_field_name("name", &mut c)
                    .map(|i| self.text(i))
                    .collect();
                let types = match n.child_by_field_name("type") {
                    Some(t) => {
                        let w = self.written_ty(t, &s.type_params);
                        vec![self.alloc_written(w); names.len()]
                    }
                    None => self.right_types(s, n.child_by_field_name("value"), names.len()),
                };
                for (name, ty) in names.into_iter().zip(types) {
                    s.declare(name, ty);
                }
            }
            id if id == kinds.range_clause || id == kinds.receive => {
                self.walk_children(s, k, n, depth);
                // `for k, v := range xs`, `v := <-ch`: element types are not tracked, and an
                // unknown binding shadows whatever the name meant (for `=` too, conservatively).
                for name in self
                    .left_names(n.child_by_field_name("left"))
                    .into_iter()
                    .flatten()
                {
                    s.declare(name, None);
                }
            }
            _ => self.walk_children(s, k, n, depth),
        }
    }

    /// Package-level `var` declarations: each name's symbolic type (no calls are recorded).
    pub(super) fn package_vars(&mut self, decl: TsNode) {
        let mut s = Scope {
            caller_node: 0,
            record: false,
            vars: Vec::new(),
            level: 0,
            memo: HashMap::default(),
            type_params: Vec::new(),
            caller: String::new(),
        };
        let mut specs = Vec::new();
        for c in named_children(decl) {
            match c.kind() {
                "var_spec" => specs.push(c),
                "var_spec_list" => specs.extend(
                    named_children(c)
                        .into_iter()
                        .filter(|x| x.kind() == "var_spec"),
                ),
                _ => {}
            }
        }
        for spec in specs {
            let mut c = spec.walk();
            let names: Vec<&'a str> = spec
                .children_by_field_name("name", &mut c)
                .map(|i| self.text(i))
                .collect();
            let types = match spec.child_by_field_name("type") {
                Some(t) => {
                    let w = self.written_ty(t, &[]);
                    vec![self.alloc_written(w); names.len()]
                }
                None => self.right_types(&mut s, spec.child_by_field_name("value"), names.len()),
            };
            for (name, ty) in names.into_iter().zip(types) {
                if let (Some(ty), false) = (ty, name == "_") {
                    self.facts.vars.push((name.to_string(), ty));
                }
            }
        }
    }

    /// The explicit-stack call walk for very deep subtrees (no inference).
    fn plain_calls(&mut self, caller: &str, root: TsNode) {
        let mut stack = vec![root];
        let mut cursor = root.walk();
        while let Some(node) = stack.pop() {
            if node.kind() == "call_expression" {
                if let Some(f) = node.child_by_field_name("function") {
                    self.call(caller, f, Ctx::line(node));
                }
            }
            let start = stack.len();
            stack.extend(node.named_children(&mut cursor));
            stack[start..].reverse();
        }
    }

    /// The identifiers on the left of `:=` / `range` (`None` for anything else).
    fn left_names(&self, left: Option<TsNode>) -> Vec<Option<&'a str>> {
        let Some(left) = left else {
            return Vec::new();
        };
        named_children(left)
            .into_iter()
            .map(|c| (c.kind() == "identifier").then(|| self.text(c)))
            .collect()
    }

    /// Types of `n` values on the right of a declaration: one expression per name, or one
    /// multi-valued call (`a, err := f()`) or type assertion (`v, ok := x.(T)`).
    fn right_types(
        &mut self,
        s: &mut Scope<'a>,
        right: Option<TsNode>,
        n: usize,
    ) -> Vec<Option<u32>> {
        let Some(right) = right else {
            return vec![None; n];
        };
        let exprs = named_children(right);
        if exprs.len() == n {
            return exprs.into_iter().map(|e| self.expr_ty(s, e)).collect();
        }
        let mut out = vec![None; n];
        if let [e] = exprs.as_slice() {
            match e.kind() {
                "call_expression" => {
                    for (i, slot) in out.iter_mut().enumerate() {
                        *slot = self.call_ty(s, *e, i as u32);
                    }
                }
                "type_assertion_expression" => {
                    if let Some(first) = out.first_mut() {
                        *first = self.expr_ty(s, *e);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Record `recv.M(..)` when the receiver has a type (an import qualifier is a package).
    fn method_site(&mut self, s: &mut Scope<'a>, f: TsNode, line: u32) {
        if !s.record || f.kind() != "selector_expression" {
            return;
        }
        let (Some(operand), Some(field)) = (
            f.child_by_field_name("operand"),
            f.child_by_field_name("field"),
        ) else {
            return;
        };
        if operand.kind() == "identifier" && self.qualifiers.contains_key(self.text(operand)) {
            return;
        }
        let Some(recv) = self.expr_ty(s, operand) else {
            return;
        };
        self.method_calls_raw.push(RawGoCall {
            caller: s.caller_node,
            recv,
            name: self.text(field).to_string(),
            line,
        });
    }

    fn expr_ty(&mut self, s: &mut Scope<'a>, n: TsNode) -> Option<u32> {
        if let Some(&t) = s.memo.get(&n.id()) {
            return t;
        }
        let t = self.expr_ty_uncached(s, n);
        s.memo.insert(n.id(), t);
        t
    }

    fn expr_ty_uncached(&mut self, s: &mut Scope<'a>, n: TsNode) -> Option<u32> {
        match n.kind() {
            "identifier" => {
                let name = self.text(n);
                match s.var(name) {
                    Some(t) => t,
                    // A package-level variable (of this package; an import name is no value).
                    None if !self.qualifiers.contains_key(name) && name != "_" => {
                        Some(self.alloc(GoTy::PkgVar(name.to_string())))
                    }
                    None => None,
                }
            }
            "parenthesized_expression" => {
                let inner = named_children(n).into_iter().next()?;
                self.expr_ty(s, inner)
            }
            "unary_expression" => {
                let op = n.child_by_field_name("operator").map(|o| self.text(o));
                if !matches!(op, Some("&" | "*")) {
                    return None;
                }
                self.expr_ty(s, n.child_by_field_name("operand")?)
            }
            "composite_literal" => {
                let w = self.written_ty(n.child_by_field_name("type")?, &s.type_params);
                self.alloc_written(w)
            }
            "type_assertion_expression" | "type_conversion_expression" => {
                let w = self.written_ty(n.child_by_field_name("type")?, &s.type_params);
                self.alloc_written(w)
            }
            "selector_expression" => {
                let operand = n.child_by_field_name("operand")?;
                if operand.kind() == "identifier"
                    && s.var(self.text(operand)).is_none()
                    && self.qualifiers.contains_key(self.text(operand))
                {
                    let q = self.text(operand);
                    if self.std_qualifier(q) {
                        return None;
                    }
                    let name = self.text(n.child_by_field_name("field")?).to_string();
                    return Some(self.alloc(GoTy::QualVar(q.to_string(), name)));
                }
                let recv = self.expr_ty(s, operand)?;
                let name = self.text(n.child_by_field_name("field")?).to_string();
                Some(self.alloc(GoTy::Field { recv, name }))
            }
            "call_expression" => self.call_ty(s, n, 0),
            _ => None,
        }
    }

    /// The type of result `index` of a call.
    fn call_ty(&mut self, s: &mut Scope<'a>, call: TsNode, index: u32) -> Option<u32> {
        let f = call.child_by_field_name("function")?;
        if let Some((q, name)) = self.instantiated(f) {
            // `F[T](..)` / `pkg.F[T](..)`; an index into a local value is no instantiation.
            let callee = match q {
                Some(q) if s.var(q).is_none() && self.qualifiers.contains_key(q) => {
                    if self.std_qualifier(q) {
                        return None;
                    }
                    GoCallee::Qual(q.to_string(), name.to_string())
                }
                None if s.var(name).is_none() && !s.type_params.iter().any(|p| p == name) => {
                    GoCallee::Bare(name.to_string())
                }
                _ => return None,
            };
            return Some(self.alloc(GoTy::Call { callee, index }));
        }
        let callee = match f.kind() {
            "identifier" => {
                let name = self.text(f);
                if s.var(name).is_some() {
                    return None;
                }
                if name == "new" && index == 0 {
                    let arg = named_children(call.child_by_field_name("arguments")?)
                        .into_iter()
                        .next()?;
                    let w = self.written_ty(arg, &s.type_params);
                    return self.alloc_written(w);
                }
                if s.type_params.iter().any(|p| p == name) {
                    return None;
                }
                GoCallee::Bare(name.to_string())
            }
            "selector_expression" => {
                let operand = f.child_by_field_name("operand")?;
                let name = self.text(f.child_by_field_name("field")?).to_string();
                let q = self.text(operand);
                if operand.kind() == "identifier"
                    && s.var(q).is_none()
                    && self.qualifiers.contains_key(q)
                {
                    if self.std_qualifier(q) {
                        return None;
                    }
                    GoCallee::Qual(q.to_string(), name)
                } else {
                    GoCallee::Method(self.expr_ty(s, operand)?, name)
                }
            }
            _ => return None,
        };
        Some(self.alloc(GoTy::Call { callee, index }))
    }
}

/// The typed method calls of a file with their in-file targets (`table` is the G5 name table),
/// and the symbolic types they reach (unreachable entries dropped, indices remapped in order).
pub(super) fn finish_inference(
    raw: Vec<RawGoCall>,
    mut tys: Vec<GoTy>,
    vars: &mut [(String, u32)],
    table: &HashMap<&str, (&str, usize)>,
) -> (Vec<GoTy>, Vec<GoMethodCall>) {
    // One call per (caller, receiver, name), at its first site.
    let mut seen: std::collections::HashSet<(u32, u32, &str)> = std::collections::HashSet::new();
    let mut calls = Vec::new();
    for c in &raw {
        if !seen.insert((c.caller, c.recv, c.name.as_str())) {
            continue;
        }
        let in_file = if super::is_unbound_name(&c.name, false) {
            None
        } else {
            table.get(c.name.as_str()).map(|&(_, i)| i as u32)
        };
        calls.push(GoMethodCall {
            caller: c.caller,
            recv: c.recv,
            name: c.name.clone(),
            line: c.line,
            in_file,
        });
    }
    let mut keep = vec![false; tys.len()];
    let mut stack: Vec<u32> = calls.iter().map(|c| c.recv).collect();
    stack.extend(vars.iter().map(|(_, t)| *t));
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut keep[i as usize], true) {
            continue;
        }
        match &tys[i as usize] {
            GoTy::Field { recv, .. }
            | GoTy::Call {
                callee: GoCallee::Method(recv, _),
                ..
            } => stack.push(*recv),
            _ => {}
        }
    }
    let mut remap = vec![u32::MAX; tys.len()];
    let mut out = Vec::new();
    for (i, t) in tys.drain(..).enumerate() {
        if keep[i] {
            remap[i] = out.len() as u32;
            out.push(t);
        }
    }
    for t in &mut out {
        match t {
            GoTy::Field { recv, .. }
            | GoTy::Call {
                callee: GoCallee::Method(recv, _),
                ..
            } => *recv = remap[*recv as usize],
            _ => {}
        }
    }
    for c in &mut calls {
        c.recv = remap[c.recv as usize];
    }
    for (_, t) in vars.iter_mut() {
        *t = remap[*t as usize];
    }
    (out, calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std_roots_are_sorted_for_binary_search() {
        assert!(STD_ROOTS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn method_calls_record_typed_receivers_only() {
        let f = crate::extract::extract_file(
            crate::lang::Lang::Go,
            "a/a.go",
            b"package a\n\nimport \"testing\"\n\nfunc F(t *testing.T, x T, y interface{ M() }) {\n\
              \tt.Errorf(\"\")\n\tx.M()\n\ty.M()\n\tfor _, x := range y.L() {\n\t\tx.M()\n\t}\n}\n",
        );
        let crate::facts::LangFacts::Go(g) = &f.lang else {
            panic!()
        };
        // `t` is from the standard library and `y` has no named type; the range variable
        // shadows `x`. Only `x.M()` on the parameter remains.
        let names: Vec<(&str, u32)> = g
            .method_calls
            .iter()
            .map(|c| (c.name.as_str(), c.line))
            .collect();
        assert_eq!(names, [("M", 7)]);
    }
}

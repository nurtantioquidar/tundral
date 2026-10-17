//! Local type inference for method calls (dijkstral-native, Q23 not reproduced).
//!
//! One scoped walk per function body records, for every `recv.m(..)` whose receiver is not
//! `self` or a `self.a.b` field chain, a symbolic type of the receiver ([`RsTy`]) when a
//! declaration fixes it: a parameter's or `let`'s written type, a struct literal, `self`, or what a
//! path call, bare call, method call, field, `?` or `.await` yields. The cross-file pass evaluates
//! it against the repository's declared return and field types (`crate::resolve::rust`).
//!
//! Soundness rules: a binding is visible from its `let` to the end of its block and is shadowed by
//! any later binding of the name (patterns, closure parameters, `for`, `match` and `if let` arms
//! bind every name they mention as unknown, except `Some(x)` / `Ok(x)`, which unwrap the
//! scrutinee); a bare call to a local binding or a path through an item declared in the body is
//! unknown; generic parameters, `impl Trait`, `dyn Trait`, tuples and closures have no type.
//! Rust variables never change type on assignment, so reassignment needs no invalidation.

use std::collections::HashMap;

use tree_sitter::Node as TsNode;

use super::{call_denied, named_children, MethodCall, RsPathTy, RsReturn, RsTy, Walker};
use crate::extract::Ctx;

/// A `recv.name(..)` site, before the file's methods are all known.
pub(super) struct RawMethodCall<'a> {
    caller: String,
    recv: u32,
    name: &'a str,
    depth: u32,
    line: u32,
    /// Written with a turbofish (`x.m::<T>()`): in-file resolution never sees it.
    turbofish: bool,
}

/// Bindings while walking one function body.
struct Scope<'a> {
    /// (name, its type when a declaration fixes it), innermost last.
    vars: Vec<(&'a str, Option<u32>)>,
    /// Items declared in the enclosing blocks: a call or path through one is unknown.
    items: Vec<&'a str>,
    /// Types by expression node id: a chained receiver is typed once.
    memo: HashMap<usize, Option<u32>>,
    self_ty: Option<u32>,
    caller: String,
    depth: u32,
}

impl<'a> Scope<'a> {
    fn var(&self, name: &str) -> Option<Option<u32>> {
        self.vars
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map(|(_, t)| *t)
    }

    fn local_item(&self, name: &str) -> bool {
        self.items.contains(&name)
    }
}

/// Item kinds a block may declare (they are visible in the whole block).
fn is_item(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "impl_item"
            | "trait_item"
            | "mod_item"
            | "macro_definition"
            | "foreign_mod_item"
            | "struct_item"
            | "enum_item"
            | "union_item"
            | "const_item"
            | "static_item"
            | "type_item"
            | "use_declaration"
            | "extern_crate_declaration"
    )
}

impl<'c, 'a> Walker<'c, 'a> {
    /// The type `ty` names as written; `None` for anything but a path to a concrete type
    /// (generic parameters, `dyn`/`impl Trait`, tuples, arrays, function pointers, primitives).
    pub(super) fn written_ty(&self, ty: TsNode) -> Option<RsPathTy> {
        self.written(ty, false)
    }

    fn written(&self, ty: TsNode, wrapped: bool) -> Option<RsPathTy> {
        let plain = |path: Vec<&str>| RsPathTy {
            path: path.into_iter().map(str::to_string).collect(),
            wrapped,
            inner: None,
        };
        match ty.kind() {
            "reference_type" => self.written(ty.child_by_field_name("type")?, wrapped),
            "type_identifier" => match self.text(ty) {
                "Self" => (!self.self_path.is_empty()).then(|| plain(self.self_path.clone())),
                name if self.generics.contains(&name) => None,
                name => Some(plain(vec![name])),
            },
            "scoped_type_identifier" => {
                let mut segs = Vec::new();
                if let Some(p) = ty.child_by_field_name("path") {
                    if !self.path_segments(p, &mut segs) {
                        return None;
                    }
                }
                segs.push(self.field_text(ty, "name")?);
                // `Self::Item`, `T::Output`: associated types are not tracked.
                if segs
                    .first()
                    .is_some_and(|s| *s == "Self" || self.generics.contains(s))
                {
                    return None;
                }
                Some(plain(segs))
            }
            "generic_type" => {
                let base = ty.child_by_field_name("type")?;
                let args: Vec<TsNode> = named_children(ty.child_by_field_name("type_arguments")?)
                    .filter(|a| a.kind() != "lifetime")
                    .collect();
                if base.kind() == "type_identifier"
                    && matches!(self.text(base), "Box" | "Arc" | "Rc")
                    && args.len() == 1
                {
                    return self.written(args[0], true);
                }
                let mut t = self.written(base, wrapped)?;
                t.inner = args
                    .first()
                    .and_then(|a| self.written(*a, false))
                    .map(Box::new);
                Some(t)
            }
            _ => None,
        }
    }

    fn alloc(&mut self, t: RsTy) -> u32 {
        self.tys.push(t);
        (self.tys.len() - 1) as u32
    }

    /// The declared return type of function `id` (`f` is its item), while its generic
    /// parameters are in scope.
    pub(super) fn record_return(&mut self, id: &str, f: TsNode) {
        let Some(t) = f
            .child_by_field_name("return_type")
            .and_then(|t| self.written_ty(t))
        else {
            return;
        };
        let is_async = named_children(f)
            .any(|c| c.kind() == "function_modifiers" && self.text(c).contains("async"));
        self.returns.push(RsReturn {
            id: id.to_string(),
            depth: self.mod_path.len() as u32,
            ty: t,
            is_async,
        });
    }

    /// Record the typed method calls of function `caller` (item `f`, body `body`); `in_impl`
    /// when `self` is the impl's type.
    pub(super) fn infer_fn(&mut self, caller: &str, f: TsNode, body: TsNode, in_impl: bool) {
        let mark = self.push_generics(f.child_by_field_name("type_parameters"));
        let mut s = Scope {
            vars: Vec::new(),
            items: Vec::new(),
            memo: HashMap::new(),
            self_ty: None,
            caller: caller.to_string(),
            depth: self.mod_path.len() as u32,
        };
        if in_impl && !self.self_path.is_empty() {
            let t = RsTy::Written(RsPathTy {
                path: self.self_path.iter().map(|s| s.to_string()).collect(),
                wrapped: false,
                inner: None,
            });
            s.self_ty = Some(self.alloc(t));
        }
        if let Some(params) = f.child_by_field_name("parameters") {
            for p in named_children(params) {
                if p.kind() != "parameter" {
                    continue;
                }
                let Some(pat) = p.child_by_field_name("pattern") else {
                    continue;
                };
                let ty = p
                    .child_by_field_name("type")
                    .and_then(|t| self.written_ty(t))
                    .map(|t| self.alloc(RsTy::Written(t)));
                if pat.kind() == "self" {
                    // `self: Box<Self>` and friends.
                    s.self_ty = if in_impl { ty } else { None };
                } else {
                    self.bind(&mut s, pat, ty);
                }
            }
        }
        self.infer_node(&mut s, body);
        self.generics.truncate(mark);
    }

    /// Bind the names a pattern introduces: a plain `x` / `mut x` / `ref x` to `ty`, `Some(x)` /
    /// `Ok(x)` to what unwrapping `ty` gives, every other name to unknown.
    fn bind(&mut self, s: &mut Scope<'a>, pat: TsNode, ty: Option<u32>) {
        match pat.kind() {
            "identifier" => {
                let name = self.text(pat);
                s.vars.push((name, ty));
            }
            "mut_pattern" | "ref_pattern" => {
                if let Some(inner) = named_children(pat).find(|c| c.kind() != "mutable_specifier") {
                    self.bind(s, inner, ty);
                }
            }
            "tuple_struct_pattern" => {
                let ctor = pat.child_by_field_name("type").map(|t| self.text(t));
                let args: Vec<TsNode> = named_children(pat)
                    .filter(|c| Some(c.id()) != pat.child_by_field_name("type").map(|t| t.id()))
                    .collect();
                if matches!(ctor, Some("Some" | "Ok")) && args.len() == 1 {
                    let inner = ty.map(|t| self.alloc(RsTy::Unwrap(t)));
                    self.bind(s, args[0], inner);
                } else {
                    self.bind_unknown(s, pat);
                }
            }
            _ => self.bind_unknown(s, pat),
        }
    }

    /// Every identifier in a pattern becomes an unknown binding (over-approximated: a path or
    /// constant in the pattern only shadows more).
    fn bind_unknown(&mut self, s: &mut Scope<'a>, pat: TsNode) {
        let mut stack = vec![pat];
        while let Some(n) = stack.pop() {
            match n.kind() {
                "identifier" | "shorthand_field_identifier" => s.vars.push((self.text(n), None)),
                _ => stack.extend(named_children(n)),
            }
        }
    }

    /// `if let` / `while let` conditions, chains included.
    fn condition(&mut self, s: &mut Scope<'a>, c: TsNode) {
        match c.kind() {
            "let_condition" => {
                let value = c.child_by_field_name("value");
                if let Some(v) = value {
                    self.infer_node(s, v);
                }
                let ty = value.and_then(|v| self.expr_ty(s, v));
                if let Some(p) = c.child_by_field_name("pattern") {
                    self.bind(s, p, ty);
                }
            }
            "let_chain" => {
                for x in named_children(c) {
                    self.condition(s, x);
                }
            }
            _ => self.infer_node(s, c),
        }
    }

    fn infer_node(&mut self, s: &mut Scope<'a>, n: TsNode) {
        let kind = n.kind();
        if is_item(kind) || kind == "macro_invocation" {
            return;
        }
        let (vars, items) = (s.vars.len(), s.items.len());
        match kind {
            "block" => {
                for c in named_children(n) {
                    if is_item(c.kind()) {
                        if let Some(name) = self.field_text(c, "name") {
                            s.items.push(name);
                        }
                    }
                }
                for c in named_children(n) {
                    self.infer_node(s, c);
                }
            }
            "let_declaration" => {
                let value = n.child_by_field_name("value");
                if let Some(v) = value {
                    self.infer_node(s, v);
                }
                if let Some(alt) = n.child_by_field_name("alternative") {
                    self.infer_node(s, alt);
                }
                let ty = match n.child_by_field_name("type") {
                    Some(t) => self.written_ty(t).map(|t| self.alloc(RsTy::Written(t))),
                    None => value.and_then(|v| self.expr_ty(s, v)),
                };
                if let Some(p) = n.child_by_field_name("pattern") {
                    self.bind(s, p, ty);
                }
                // The binding outlives this statement: keep it.
                return;
            }
            "closure_expression" => {
                if let Some(params) = n.child_by_field_name("parameters") {
                    for p in named_children(params) {
                        if p.kind() == "parameter" {
                            let ty = p
                                .child_by_field_name("type")
                                .and_then(|t| self.written_ty(t))
                                .map(|t| self.alloc(RsTy::Written(t)));
                            if let Some(pat) = p.child_by_field_name("pattern") {
                                self.bind(s, pat, ty);
                            }
                        } else {
                            self.bind_unknown(s, p);
                        }
                    }
                }
                if let Some(body) = n.child_by_field_name("body") {
                    self.infer_node(s, body);
                }
            }
            "for_expression" => {
                if let Some(v) = n.child_by_field_name("value") {
                    self.infer_node(s, v);
                }
                if let Some(p) = n.child_by_field_name("pattern") {
                    self.bind_unknown(s, p);
                }
                if let Some(b) = n.child_by_field_name("body") {
                    self.infer_node(s, b);
                }
            }
            "if_expression" | "while_expression" => {
                if let Some(c) = n.child_by_field_name("condition") {
                    self.condition(s, c);
                }
                for f in ["consequence", "body"] {
                    if let Some(b) = n.child_by_field_name(f) {
                        self.infer_node(s, b);
                    }
                }
                // The `else` branch sees the condition's bindings too: they only shadow.
                if let Some(a) = n.child_by_field_name("alternative") {
                    self.infer_node(s, a);
                }
            }
            "match_expression" => {
                let value = n.child_by_field_name("value");
                if let Some(v) = value {
                    self.infer_node(s, v);
                }
                let scrutinee = value.and_then(|v| self.expr_ty(s, v));
                if let Some(body) = n.child_by_field_name("body") {
                    for arm in named_children(body).filter(|a| a.kind() == "match_arm") {
                        let mark = s.vars.len();
                        if let Some(p) = arm.child_by_field_name("pattern") {
                            for c in named_children(p) {
                                if p.child_by_field_name("condition").map(|x| x.id())
                                    == Some(c.id())
                                {
                                    continue;
                                }
                                self.bind(s, c, scrutinee);
                            }
                            if let Some(cond) = p.child_by_field_name("condition") {
                                self.condition(s, cond);
                            }
                        }
                        if let Some(v) = arm.child_by_field_name("value") {
                            self.infer_node(s, v);
                        }
                        s.vars.truncate(mark);
                    }
                }
            }
            "call_expression" => {
                if let Some(f) = n.child_by_field_name("function") {
                    self.method_site(s, f, Ctx::line(n));
                }
                for c in named_children(n) {
                    self.infer_node(s, c);
                }
            }
            _ => {
                for c in named_children(n) {
                    self.infer_node(s, c);
                }
            }
        }
        s.vars.truncate(vars);
        s.items.truncate(items);
    }

    /// Record `recv.name(..)` when its receiver has a type and is not `self` or a `self` field
    /// chain (already [`super::SelfCall`]s and [`super::FieldCall`]s).
    fn method_site(&mut self, s: &mut Scope<'a>, f: TsNode, line: u32) {
        let (f, turbofish) = match f.kind() {
            "generic_function" => match f.child_by_field_name("function") {
                Some(inner) => (inner, true),
                None => return,
            },
            _ => (f, false),
        };
        if f.kind() != "field_expression" {
            return;
        }
        let (Some(value), Some(field)) = (
            f.child_by_field_name("value"),
            f.child_by_field_name("field"),
        ) else {
            return;
        };
        if field.kind() != "field_identifier" {
            return;
        }
        if value.kind() == "self" || !self.receiver_fields(f).is_empty() {
            return;
        }
        let Some(recv) = self.expr_ty(s, value) else {
            return;
        };
        self.method_calls_raw.push(RawMethodCall {
            caller: s.caller.clone(),
            recv,
            name: self.text(field),
            depth: s.depth,
            line,
            turbofish,
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
            "identifier" => s.var(self.text(n)).flatten(),
            "self" => s.self_ty,
            "parenthesized_expression" => {
                let inner = named_children(n).next()?;
                self.expr_ty(s, inner)
            }
            "reference_expression" => {
                let inner = n.child_by_field_name("value")?;
                self.expr_ty(s, inner)
            }
            "try_expression" => {
                let inner = self.expr_ty(s, named_children(n).next()?)?;
                Some(self.alloc(RsTy::Unwrap(inner)))
            }
            "await_expression" => {
                let inner = self.expr_ty(s, named_children(n).next()?)?;
                Some(self.alloc(RsTy::Await(inner)))
            }
            "field_expression" => {
                let field = n.child_by_field_name("field")?;
                if !matches!(field.kind(), "field_identifier" | "integer_literal") {
                    return None;
                }
                let recv = self.expr_ty(s, n.child_by_field_name("value")?)?;
                let name = self.text(field).to_string();
                Some(self.alloc(RsTy::Field { recv, name }))
            }
            "struct_expression" => {
                let mut name = n.child_by_field_name("name")?;
                if name.kind() == "generic_type_with_turbofish" {
                    name = name.child_by_field_name("type")?;
                }
                let t = match name.kind() {
                    "scoped_identifier" => {
                        let mut segs = Vec::new();
                        if !self.path_segments(name, &mut segs) {
                            return None;
                        }
                        self.written_path(segs)?
                    }
                    _ => self.written_ty(name)?,
                };
                if t.path.first().is_some_and(|f| s.local_item(f)) {
                    return None;
                }
                Some(self.alloc(RsTy::Written(t)))
            }
            "call_expression" => {
                let mut f = n.child_by_field_name("function")?;
                if f.kind() == "generic_function" {
                    f = f.child_by_field_name("function")?;
                }
                match f.kind() {
                    "identifier" => {
                        let name = self.text(f);
                        if s.var(name).is_some() || s.local_item(name) {
                            return None;
                        }
                        Some(self.alloc(RsTy::BareRet(name.to_string())))
                    }
                    "scoped_identifier" => {
                        let mut segs = Vec::new();
                        if !self.path_segments(f, &mut segs) || segs.len() < 2 {
                            return None;
                        }
                        let name = segs.pop()?.to_string();
                        let qual = self.written_path(segs)?.path;
                        if qual.first().is_some_and(|q| s.local_item(q)) {
                            return None;
                        }
                        let pointer =
                            matches!(qual.last().map(String::as_str), Some("Box" | "Arc" | "Rc"));
                        if pointer && name == "new" {
                            let args = n.child_by_field_name("arguments")?;
                            let mut args = named_children(args);
                            let (Some(arg), None) = (args.next(), args.next()) else {
                                return None;
                            };
                            let inner = self.expr_ty(s, arg)?;
                            return Some(self.alloc(RsTy::Wrap(inner)));
                        }
                        Some(self.alloc(RsTy::PathRet { qual, name }))
                    }
                    "field_expression" => {
                        let field = f.child_by_field_name("field")?;
                        if field.kind() != "field_identifier" {
                            return None;
                        }
                        let recv = self.expr_ty(s, f.child_by_field_name("value")?)?;
                        let name = self.text(field).to_string();
                        Some(self.alloc(RsTy::MethodRet { recv, name }))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// A path written in a body (`a::B`, `Self::new`'s `Self`): `Self` becomes the impl's type,
    /// and a path through a generic parameter is unknown.
    fn written_path(&self, segs: Vec<&str>) -> Option<RsPathTy> {
        let first = *segs.first()?;
        if self.generics.contains(&first) || first.is_empty() && segs.len() < 2 {
            return None;
        }
        let path: Vec<String> = if first == "Self" {
            if self.self_path.is_empty() {
                return None;
            }
            self.self_path
                .iter()
                .chain(&segs[1..])
                .map(|s| s.to_string())
                .collect()
        } else {
            segs.iter().map(|s| s.to_string()).collect()
        };
        Some(RsPathTy {
            path,
            wrapped: false,
            inner: None,
        })
    }

    /// The typed method calls of the file with their in-file targets, and the symbolic types
    /// they reach (unreachable entries dropped, indices remapped in order).
    pub(super) fn finish_inference(&mut self) -> (Vec<RsTy>, Vec<MethodCall>) {
        let raw = std::mem::take(&mut self.method_calls_raw);
        let mut tys = std::mem::take(&mut self.tys);
        let mut seen = std::collections::HashSet::new();
        let mut calls = Vec::new();
        for c in raw {
            if !seen.insert((c.caller.clone(), c.recv, c.name)) {
                continue;
            }
            let in_file = if c.turbofish || call_denied(c.name) {
                None
            } else {
                self.methods.get(c.name).cloned()
            };
            calls.push(MethodCall {
                caller: c.caller,
                recv: c.recv,
                name: c.name.to_string(),
                depth: c.depth,
                line: c.line,
                in_file,
            });
        }
        // Keep only what the calls reach.
        let mut keep = vec![false; tys.len()];
        let mut stack: Vec<u32> = calls.iter().map(|c| c.recv).collect();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut keep[i as usize], true) {
                continue;
            }
            match &tys[i as usize] {
                RsTy::MethodRet { recv, .. } | RsTy::Field { recv, .. } => stack.push(*recv),
                RsTy::Unwrap(r) | RsTy::Await(r) | RsTy::Wrap(r) => stack.push(*r),
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
                RsTy::MethodRet { recv, .. } | RsTy::Field { recv, .. } => {
                    *recv = remap[*recv as usize]
                }
                RsTy::Unwrap(r) | RsTy::Await(r) | RsTy::Wrap(r) => *r = remap[*r as usize],
                _ => {}
            }
        }
        for c in &mut calls {
            c.recv = remap[c.recv as usize];
        }
        (out, calls)
    }
}

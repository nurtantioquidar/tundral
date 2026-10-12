//! Cross-file pass for TypeScript and JavaScript (§10). See `docs/graphify-format.md`.
//!
//! Module specifiers resolve like a bundler: relative paths (exact, plus each code extension,
//! `.js`→`.ts`, directory `index`), the nearest `tsconfig.json`/`jsconfig.json` `paths` and
//! `baseUrl` (following relative `extends`), workspace packages by `package.json` name and
//! `exports`/`main`, and finally `@/x` relative to the importing directory (§10.3). The configs are
//! the discovered JSON files, read from disk here (the scan root is derived from any view's
//! absolute path), so no other language's facts are needed. Imported names are followed through
//! re-export chains (`export {a as b} from`, `export *`, local `export {…}` lists, defaults).
//!
//! Edges are emitted in Graphify's order: each file's import block in file order, then
//! `re_exports` to modules, import-scoped calls (credited to the caller and to its enclosing
//! module-level function, as Graphify does), heritage, type references, function references,
//! remaining calls and member calls (§2.4, §10).
//!
//! Not reproduced (§15): barrel imports never leave a stub beside the real edge (Q9); a name that
//! cannot be resolved still gets the `module_name` stub. Dynamic imports of packages point at the
//! package's `ref_*` external instead of a phantom code node (Q9). Calls resolve through import
//! bindings, so aliased imports (`import { a as b }`) resolve where Graphify's by-name lookup
//! misses them or picks a same-named method of another module.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Index, Resolution, View};
use crate::extract::script::{
    CallKind, DefKind, DefaultExport, ImportKind, Pending, Receiver, RefContext, ScriptFacts,
    TopDef,
};
use crate::extract::{external_node, origin, slug};
use crate::facts::{Attr, Confidence, Edge, LangFacts, Loc};

/// Extensions tried after a specifier, in order. Declaration-only `.d.ts` targets are not
/// probed (Graphify leaves such imports unresolved).
const EXTS: [&str; 8] = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Target {
    /// A file in the graph (view index).
    Local(usize),
    /// An external node id.
    External(String),
}

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let mut r = Resolver::new(files, index);
    if r.script.is_empty() {
        return Resolution::default();
    }
    r.run();
    r.res
}

/// A class's own methods: (source name, local id).
type Methods<'a> = &'a [(String, String)];

struct TsConfig {
    /// Directory paths are resolved against (`baseUrl`, else the config's directory).
    base: String,
    has_base_url: bool,
    /// (`pattern`, targets) in declaration order.
    paths: Vec<(String, Vec<String>)>,
}

struct Package {
    name: String,
    dir: String,
    /// `exports` subpath → target, plus `.` for `main`-style fields.
    exports: Vec<(String, String)>,
    /// The package declares `exports` (unlisted subpaths are private).
    has_exports: bool,
    /// The package declares an entry (`main`, `module`…).
    has_main: bool,
}

struct Resolver<'a> {
    views: &'a [View<'a>],
    index: &'a Index,
    facts: Vec<Option<&'a ScriptFacts>>,
    script: Vec<usize>,
    /// Relative path → view, for files that have a file node.
    by_rel: HashMap<&'a str, usize>,
    root: Option<PathBuf>,
    tsconfigs: HashMap<String, Option<TsConfig>>,
    packages: Vec<Package>,
    tops: Vec<HashMap<&'a str, &'a TopDef>>,
    /// Local name → (specifier, imported name), module-level imports and requires.
    bindings: Vec<HashMap<&'a str, (&'a str, &'a str, bool)>>,
    /// Classes and interfaces by final id → (view, own methods by name).
    classes: HashMap<String, (usize, Methods<'a>)>,
    spec_cache: RefCell<HashMap<(String, String), Target>>,
    export_memo: RefCell<HashMap<(usize, String), Option<String>>>,
    externals: HashSet<String>,
    res: Resolution,
}

impl<'a> Resolver<'a> {
    fn new(views: &'a [View<'a>], index: &'a Index) -> Resolver<'a> {
        let facts: Vec<Option<&ScriptFacts>> = views
            .iter()
            .map(|v| match &v.facts.lang {
                LangFacts::Script(s) => Some(s),
                _ => None,
            })
            .collect();
        let script = (0..views.len()).filter(|&i| facts[i].is_some()).collect();
        let by_rel = views
            .iter()
            .enumerate()
            .filter(|(_, v)| index.file_ids.contains_key(&v.file.rel))
            .map(|(i, v)| (v.file.rel.as_str(), i))
            .collect();
        let root = views.first().and_then(|v| {
            let depth = v.file.rel.split('/').count();
            v.file.abs.ancestors().nth(depth).map(Path::to_path_buf)
        });
        let mut tops = Vec::with_capacity(views.len());
        let mut bindings = Vec::with_capacity(views.len());
        let mut classes = HashMap::new();
        for (i, f) in facts.iter().enumerate() {
            let mut top: HashMap<&str, &TopDef> = HashMap::new();
            let mut bind: HashMap<&str, (&str, &str, bool)> = HashMap::new();
            if let Some(f) = f {
                for d in &f.top {
                    top.entry(d.name.as_str()).or_insert(d);
                }
                for imp in &f.imports {
                    if matches!(imp.kind, ImportKind::Import | ImportKind::Require)
                        && imp.owner.is_none()
                    {
                        for n in &imp.names {
                            bind.entry(n.local.as_str()).or_insert((
                                imp.spec.as_str(),
                                n.imported.as_str(),
                                imp.kind == ImportKind::Require,
                            ));
                        }
                    }
                }
                for c in &f.classes {
                    classes
                        .entry(views[i].id(&c.id).to_string())
                        .or_insert((i, c.methods.as_slice()));
                }
            }
            tops.push(top);
            bindings.push(bind);
        }
        let mut r = Resolver {
            views,
            index,
            facts,
            script,
            by_rel,
            root,
            tsconfigs: HashMap::new(),
            packages: Vec::new(),
            tops,
            bindings,
            classes,
            spec_cache: RefCell::new(HashMap::new()),
            export_memo: RefCell::new(HashMap::new()),
            externals: HashSet::new(),
            res: Resolution::default(),
        };
        if !r.script.is_empty() {
            r.load_configs();
        }
        r
    }

    /// Read every discovered `tsconfig.json`/`jsconfig.json` and `package.json` from disk.
    fn load_configs(&mut self) {
        let Some(root) = self.root.clone() else {
            return;
        };
        for v in self.views {
            let rel = v.file.rel.as_str();
            let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
            match name {
                "tsconfig.json" | "jsconfig.json" => {
                    let cfg = load_tsconfig(&root, rel, 0);
                    // A tsconfig wins over a jsconfig in the same directory.
                    if name == "tsconfig.json" || !self.tsconfigs.contains_key(dir) {
                        self.tsconfigs.insert(dir.to_string(), cfg);
                    }
                }
                "package.json" => {
                    if let Some(p) = load_package(&root.join(rel), dir) {
                        self.packages.push(p);
                    }
                }
                _ => {}
            }
        }
        // Longest names first so `@scope/a-b` wins over `@scope/a`.
        self.packages
            .sort_by(|a, b| b.name.len().cmp(&a.name.len()).then(a.name.cmp(&b.name)));
    }

    fn view_facts(&self, vi: usize) -> Option<&'a ScriptFacts> {
        self.facts[vi]
    }

    fn file_id(&self, vi: usize) -> &'a str {
        let rel = &self.views[vi].file.rel;
        match self.index.file_ids.get(rel) {
            Some(id) => id,
            None => self.views[vi]
                .facts
                .nodes
                .first()
                .map_or("", |n| n.id.as_str()),
        }
    }

    fn local_id(&self, vi: usize, local: &'a str) -> &'a str {
        self.views[vi].id(local)
    }

    // ---- specifier resolution -------------------------------------------------------------

    fn resolve_spec(&self, vi: usize, spec: &str) -> Target {
        let rel = &self.views[vi].file.rel;
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
        let key = (dir.to_string(), spec.to_string());
        if let Some(t) = self.spec_cache.borrow().get(&key) {
            return t.clone();
        }
        let t = self.resolve_spec_uncached(dir, spec);
        self.spec_cache.borrow_mut().insert(key, t.clone());
        t
    }

    fn resolve_spec_uncached(&self, dir: &str, spec: &str) -> Target {
        if spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == ".." {
            let Some(path) = join(dir, spec) else {
                return Target::External(missing_relative(spec));
            };
            if let Some(vi) = self.probe(&path) {
                return Target::Local(vi);
            }
            let exists = self.root.as_ref().is_some_and(|r| r.join(&path).is_file());
            return Target::External(if exists {
                slug(strip_ext(&path))
            } else {
                missing_relative(spec)
            });
        }
        if let Some(vi) = self.via_tsconfig(dir, spec) {
            return Target::Local(vi);
        }
        if let Some(vi) = self.via_package(spec) {
            return Target::Local(vi);
        }
        if let Some(rest) = spec.strip_prefix("@/") {
            if let Some(vi) = join(dir, &format!("./{rest}")).and_then(|p| self.probe(&p)) {
                return Target::Local(vi);
            }
        }
        Target::External(format!("ref_{}", slug(package_name(spec))))
    }

    /// Exact path, path plus each extension, `.js`→`.ts`, then the directory index.
    fn probe(&self, path: &str) -> Option<usize> {
        if let Some(&vi) = self.by_rel.get(path) {
            return Some(vi);
        }
        let mut buf = String::with_capacity(path.len() + 12);
        for ext in EXTS {
            buf.clear();
            buf.push_str(path);
            buf.push_str(ext);
            if let Some(&vi) = self.by_rel.get(buf.as_str()) {
                return Some(vi);
            }
        }
        for (js, ts) in [
            (".js", ".ts"),
            (".js", ".tsx"),
            (".jsx", ".tsx"),
            (".mjs", ".mts"),
            (".cjs", ".cts"),
        ] {
            if let Some(stem) = path.strip_suffix(js) {
                buf.clear();
                buf.push_str(stem);
                buf.push_str(ts);
                if let Some(&vi) = self.by_rel.get(buf.as_str()) {
                    return Some(vi);
                }
            }
        }
        for ext in EXTS {
            buf.clear();
            if !path.is_empty() {
                buf.push_str(path);
                buf.push('/');
            }
            buf.push_str("index");
            buf.push_str(ext);
            if let Some(&vi) = self.by_rel.get(buf.as_str()) {
                return Some(vi);
            }
        }
        None
    }

    /// The nearest config in `dir` or its ancestors.
    fn tsconfig_for(&self, dir: &str) -> Option<&TsConfig> {
        let mut d = dir;
        loop {
            if let Some(cfg) = self.tsconfigs.get(d) {
                return cfg.as_ref();
            }
            if d.is_empty() {
                return None;
            }
            d = d.rsplit_once('/').map_or("", |(p, _)| p);
        }
    }

    fn via_tsconfig(&self, dir: &str, spec: &str) -> Option<usize> {
        let cfg = self.tsconfig_for(dir)?;
        // The most specific pattern (longest prefix before `*`) wins, as in TypeScript.
        let mut best: Option<(usize, &str, &Vec<String>)> = None;
        for (pat, targets) in &cfg.paths {
            let (matched, capture, weight) = match pat.split_once('*') {
                Some((pre, suf)) => {
                    if spec.len() >= pre.len() + suf.len()
                        && spec.starts_with(pre)
                        && spec.ends_with(suf)
                    {
                        (true, &spec[pre.len()..spec.len() - suf.len()], pre.len())
                    } else {
                        (false, "", 0)
                    }
                }
                None => (pat == spec, "", usize::MAX),
            };
            if matched && best.is_none_or(|(w, _, _)| weight > w) {
                best = Some((weight, capture, targets));
            }
        }
        if let Some((_, capture, targets)) = best {
            for t in targets {
                let sub = t.replace('*', capture);
                if let Some(vi) = join(&cfg.base, &dot(&sub)).and_then(|p| self.probe(&p)) {
                    return Some(vi);
                }
            }
        }
        if cfg.has_base_url {
            return join(&cfg.base, &dot(spec)).and_then(|p| self.probe(&p));
        }
        None
    }

    fn via_package(&self, spec: &str) -> Option<usize> {
        let p = self.packages.iter().find(|p| {
            spec == p.name
                || (spec.starts_with(&p.name) && spec.as_bytes().get(p.name.len()) == Some(&b'/'))
        })?;
        let sub = format!(".{}", &spec[p.name.len()..]);
        for (key, target) in &p.exports {
            let resolved = match key.split_once('*') {
                Some((pre, suf)) if sub.starts_with(pre) && sub.ends_with(suf) => {
                    let cap = &sub[pre.len()..sub.len() - suf.len()];
                    Some(target.replace('*', cap))
                }
                None if *key == sub => Some(target.clone()),
                _ => None,
            };
            if let Some(t) = resolved {
                if let Some(vi) = join(&p.dir, &dot(&t)).and_then(|q| self.probe(&q)) {
                    return Some(vi);
                }
            }
        }
        // Without declared entry points, node resolves the directory itself.
        let open = if sub == "." {
            !p.has_exports && !p.has_main
        } else {
            !p.has_exports
        };
        if !open {
            return None;
        }
        join(&p.dir, &sub).and_then(|q| self.probe(&q))
    }

    // ---- name resolution ------------------------------------------------------------------

    /// The definition a module exports under `name` (`default` included), following re-exports.
    fn resolve_export(&self, vi: usize, name: &str) -> Option<String> {
        let key = (vi, name.to_string());
        if let Some(r) = self.export_memo.borrow().get(&key) {
            return r.clone();
        }
        // In progress: a cycle resolves to nothing.
        self.export_memo.borrow_mut().insert(key.clone(), None);
        let r = self.resolve_export_uncached(vi, name);
        self.export_memo.borrow_mut().insert(key, r.clone());
        r
    }

    fn resolve_export_uncached(&self, vi: usize, name: &str) -> Option<String> {
        let f = self.view_facts(vi)?;
        if name == "default" {
            match &f.default_export {
                Some(DefaultExport::Def(id)) => return Some(self.local_id(vi, id).to_string()),
                Some(DefaultExport::Name(n)) => {
                    if let Some(r) = self.resolve_local_name(vi, n) {
                        return Some(r);
                    }
                }
                None => {}
            }
        } else if let Some(d) = self.tops[vi].get(name) {
            return Some(self.local_id(vi, &d.id).to_string());
        }
        for (exported, local, _) in &f.local_exports {
            if exported == name {
                if let Some(r) = self.resolve_local_name(vi, local) {
                    return Some(r);
                }
            }
        }
        for imp in &f.imports {
            if imp.kind != ImportKind::ReExport {
                continue;
            }
            for n in &imp.names {
                if n.local == name {
                    if let Target::Local(ti) = self.resolve_spec(vi, &imp.spec) {
                        if let Some(r) = self.resolve_export(ti, &n.imported) {
                            return Some(r);
                        }
                    }
                }
            }
        }
        if name == "default" {
            return None;
        }
        for imp in &f.imports {
            if imp.kind == ImportKind::ExportStar {
                if let Target::Local(ti) = self.resolve_spec(vi, &imp.spec) {
                    if let Some(r) = self.resolve_export(ti, name) {
                        return Some(r);
                    }
                }
            }
        }
        None
    }

    /// A name in a module's own scope: a module-level definition or an import binding.
    fn resolve_local_name(&self, vi: usize, name: &str) -> Option<String> {
        if let Some(d) = self.tops[vi].get(name) {
            return Some(self.local_id(vi, &d.id).to_string());
        }
        self.binding(vi, name)
    }

    /// What an imported local name refers to (namespace imports refer to no definition).
    fn binding(&self, vi: usize, local: &str) -> Option<String> {
        let &(spec, imported, _) = self.bindings[vi].get(local)?;
        if imported == "*" {
            return None;
        }
        match self.resolve_spec(vi, spec) {
            Target::Local(ti) => self.resolve_export(ti, imported),
            Target::External(_) => None,
        }
    }

    /// `true` when `id` is a function (not a class) in the graph.
    fn is_function(&self, id: &str) -> bool {
        self.index.get(self.views, id).is_some_and(|n| {
            n.attr("_callable").is_some() && n.attr("_callable_class").is_none() && {
                // Methods are callable too but are never passed by bare name.
                !n.label.starts_with('.')
            }
        })
    }

    // ---- emission -------------------------------------------------------------------------

    fn target_id(&mut self, t: &Target) -> String {
        match t {
            Target::Local(ti) => self.file_id(*ti).to_string(),
            Target::External(id) => {
                self.external(id);
                id.clone()
            }
        }
    }

    fn external(&mut self, id: &str) {
        if !self.index.contains(id) && self.externals.insert(id.to_string()) {
            self.res.nodes.push(external_node(id.to_string()));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn edge(
        &mut self,
        vi: usize,
        source: &str,
        target: &str,
        relation: &str,
        loc: Loc,
        context: Option<&str>,
        confidence: Confidence,
        score: Option<f64>,
    ) -> &mut Edge {
        self.res.edges.push(Edge {
            source: source.to_string(),
            target: target.to_string(),
            relation: relation.to_string(),
            confidence,
            score,
            context: context.map(str::to_string),
            source_file: self.views[vi].file.rel.clone(),
            loc,
            weight: Some(1.0),
            attrs: vec![origin()],
        });
        self.res.edges.last_mut().expect("just pushed")
    }

    fn extracted(
        &mut self,
        vi: usize,
        source: &str,
        target: &str,
        relation: &str,
        line: u32,
        context: Option<&str>,
    ) -> &mut Edge {
        self.edge(
            vi,
            source,
            target,
            relation,
            Loc::Line(line),
            context,
            Confidence::Extracted,
            None,
        )
    }

    fn run(&mut self) {
        let script = self.script.clone();
        for &vi in &script {
            self.import_block(vi);
        }
        // `re_exports` to modules: `export *` forms first, then `export {…} from`.
        let mut reexported: HashSet<(usize, usize)> = HashSet::new();
        for star in [true, false] {
            for &vi in &script {
                let f = self.facts[vi].expect("script view");
                for imp in &f.imports {
                    let is_star =
                        matches!(imp.kind, ImportKind::ExportStar | ImportKind::ExportStarAs);
                    if is_star != star || !(is_star || imp.kind == ImportKind::ReExport) {
                        continue;
                    }
                    if let Target::Local(ti) = self.resolve_spec(vi, &imp.spec) {
                        if reexported.insert((vi, ti)) {
                            let (file, target) = (self.file_id(vi), self.file_id(ti));
                            self.extracted(
                                vi,
                                file,
                                target,
                                "re_exports",
                                imp.line,
                                Some("export"),
                            );
                        }
                    }
                }
                if star {
                    continue;
                }
                // `import { X } from "./m"; export { X };` re-exports module `m`.
                for (_, local, line) in &f.local_exports {
                    let Some(&(spec, _, false)) = self.bindings[vi].get(local.as_str()) else {
                        continue;
                    };
                    if let Target::Local(ti) = self.resolve_spec(vi, spec) {
                        if reexported.insert((vi, ti)) {
                            let (file, target) = (self.file_id(vi), self.file_id(ti));
                            self.extracted(vi, file, target, "re_exports", *line, Some("export"));
                        }
                    }
                }
            }
        }
        // Calls and function references: the first occurrence per (caller, callee) decides the
        // relation, as in the per-file pass.
        let mut import_calls: Vec<(usize, &str, String, u32)> = Vec::new();
        let mut other_calls: Vec<(usize, &str, String, u32)> = Vec::new();
        let mut fn_refs: Vec<(usize, &str, String, u32, RefContext)> = Vec::new();
        let mut called: HashSet<(String, String)> = HashSet::new();
        for &vi in &script {
            let f = self.facts[vi].expect("script view");
            let mut seen: HashSet<(&str, String)> = HashSet::new();
            for p in &f.pending {
                match p {
                    Pending::Call {
                        caller,
                        top,
                        root,
                        name,
                        kind,
                        line,
                    } => {
                        // The import-scoped pass also credits the enclosing module-level function.
                        let bound = *kind == CallKind::Bare
                            && self.bindings[vi].contains_key(name.as_str());
                        if let (Some(r), true) = (root, bound) {
                            if let Some(t) = self.binding(vi, name) {
                                let r = self.local_id(vi, r);
                                if seen.insert((r, t.clone())) {
                                    called.insert((r.to_string(), t.clone()));
                                    import_calls.push((vi, r, t, *line));
                                }
                            }
                        }
                        let scoped = *top
                            && *kind == CallKind::Bare
                            && self.bindings[vi].contains_key(name.as_str());
                        let target = if scoped {
                            self.binding(vi, name)
                        } else {
                            self.binding(vi, name)
                                .or_else(|| self.in_imported_module(vi, name))
                        };
                        let Some(t) = target else { continue };
                        let src = self.local_id(vi, caller);
                        if seen.insert((src, t.clone())) {
                            called.insert((src.to_string(), t.clone()));
                            let list = if scoped {
                                &mut import_calls
                            } else {
                                &mut other_calls
                            };
                            list.push((vi, src, t, *line));
                        }
                    }
                    Pending::Ref {
                        owner,
                        name,
                        context,
                        line,
                    } => {
                        let Some(t) = self.binding(vi, name).filter(|t| self.is_function(t)) else {
                            continue;
                        };
                        let src = match owner {
                            Some(o) => self.local_id(vi, o),
                            None => self.file_id(vi),
                        };
                        if seen.insert((src, t.clone())) {
                            fn_refs.push((vi, src, t, *line, *context));
                        }
                    }
                    Pending::Member { .. } => {}
                }
            }
        }
        // Calls from module-level functions through import bindings.
        for (vi, src, t, line) in import_calls {
            self.extracted(vi, src, &t, "calls", line, Some("call"));
        }
        for &vi in &script {
            let f = self.facts[vi].expect("script view");
            for h in &f.heritage {
                if let Some(t) = self.resolve_local_name(vi, &h.name) {
                    let src = self.local_id(vi, &h.source);
                    let rel = if h.implements {
                        "implements"
                    } else {
                        "inherits"
                    };
                    self.extracted(vi, src, &t, rel, h.line, Some("type"));
                }
            }
        }
        // Type references count only for types imported from another file (§10.2).
        for &vi in &script {
            let f = self.facts[vi].expect("script view");
            for r in &f.type_refs {
                if self.tops[vi].contains_key(r.name.as_str()) {
                    continue;
                }
                if let Some(t) = self.binding(vi, &r.name) {
                    let src = self.local_id(vi, &r.source);
                    self.extracted(vi, src, &t, "references", r.line, Some(r.context.as_str()));
                }
            }
        }
        for (vi, src, t, line, context) in fn_refs {
            self.edge(
                vi,
                src,
                &t,
                "indirect_call",
                Loc::Line(line),
                Some(context.as_str()),
                Confidence::Inferred,
                Some(0.85),
            );
        }
        for (vi, src, t, line) in other_calls {
            self.edge(
                vi,
                src,
                &t,
                "calls",
                Loc::Line(line),
                Some("call"),
                Confidence::Extracted,
                Some(1.0),
            );
        }
        for &vi in &script {
            let f = self.facts[vi].expect("script view");
            for p in &f.pending {
                let Pending::Member {
                    caller,
                    recv,
                    method,
                    line,
                } = p
                else {
                    continue;
                };
                let (class_name, confidence, score) = match recv {
                    Receiver::Name(n) => (n, Confidence::Extracted, 1.0),
                    Receiver::Typed(t) => (t, Confidence::Inferred, 0.85),
                };
                // Static and typed member calls follow ESM bindings only (Graphify drops calls
                // through destructured `require` aliases, §10.4).
                if self.bindings[vi]
                    .get(class_name.as_str())
                    .is_some_and(|b| b.2)
                {
                    continue;
                }
                let Some(class) = self.binding(vi, class_name) else {
                    continue;
                };
                let Some(&(_, methods)) = self.classes.get(&class) else {
                    continue;
                };
                let Some((_, mid)) = methods.iter().find(|(n, _)| n == method) else {
                    continue;
                };
                let cvi = self.classes[&class].0;
                let t = self.local_id(cvi, mid).to_string();
                let src = self.local_id(vi, caller);
                if called.insert((src.to_string(), t.clone())) {
                    self.edge(
                        vi,
                        src,
                        &t,
                        "calls",
                        Loc::Line(*line),
                        Some("call"),
                        confidence,
                        Some(score),
                    );
                }
            }
        }
    }

    /// A function or class defined directly in a module this file imports (§10.4).
    fn in_imported_module(&self, vi: usize, name: &str) -> Option<String> {
        let f = self.facts[vi]?;
        for imp in &f.imports {
            if let Target::Local(ti) = self.resolve_spec(vi, &imp.spec) {
                if let Some(d) = self.tops[ti]
                    .get(name)
                    .filter(|d| d.kind.callable() || d.kind == DefKind::Value)
                {
                    return Some(self.local_id(ti, &d.id).to_string());
                }
            }
        }
        None
    }

    /// One file's import edges, decorators and dynamic imports, in source order.
    fn import_block(&mut self, vi: usize) {
        let f = self.facts[vi].expect("script view");
        let file = self.file_id(vi);
        // `type_only` marks a file→module edge only when every statement behind it is
        // `import type`.
        let mut value_imports: HashSet<(Option<&str>, Target)> = HashSet::new();
        for imp in &f.imports {
            if !(imp.type_only && imp.kind == ImportKind::Import) {
                let t = self.resolve_spec(vi, &imp.spec);
                value_imports.insert((imp.owner.as_deref(), t));
            }
        }
        for imp in &f.imports {
            let target = self.resolve_spec(vi, &imp.spec);
            if matches!(&target, Target::External(id) if id == file) {
                // `import "./App.scss"` from `App.tsx`: the asset's id is the file's own.
                continue;
            }
            let tid = self.target_id(&target);
            let owner = imp.owner.as_deref().map_or(file, |o| self.local_id(vi, o));
            let context = match imp.kind {
                ImportKind::ReExport | ImportKind::ExportStar | ImportKind::ExportStarAs => {
                    "re-export"
                }
                _ => "import",
            };
            let e = self.extracted(vi, owner, &tid, "imports_from", imp.line, Some(context));
            if imp.type_only
                && imp.kind == ImportKind::Import
                && !value_imports.contains(&(imp.owner.as_deref(), target.clone()))
            {
                e.attrs.push(("type_only".into(), Attr::Bool(true)));
            }
            if imp.kind == ImportKind::Dynamic {
                e.attrs.push(("deferred".into(), Attr::Bool(true)));
            }
            let (Target::Local(ti), None) = (&target, &imp.owner) else {
                continue;
            };
            let ti = *ti;
            match imp.kind {
                ImportKind::Import | ImportKind::Require => {
                    for n in &imp.names {
                        if n.imported == "*" {
                            continue;
                        }
                        match self.resolve_export(ti, &n.imported) {
                            Some(t) => {
                                self.extracted(vi, file, &t, "imports", imp.line, Some("import"));
                            }
                            None if n.imported != "default" => {
                                let stub = format!("{tid}_{}", slug(&n.imported));
                                self.external(&stub);
                                self.extracted(
                                    vi,
                                    file,
                                    &stub,
                                    "imports",
                                    imp.line,
                                    Some("import"),
                                );
                            }
                            None => {}
                        }
                    }
                }
                ImportKind::ReExport => {
                    for n in &imp.names {
                        if n.imported == "default" {
                            continue;
                        }
                        if let Some(t) = self.resolve_export(ti, &n.imported) {
                            let e = self.extracted(
                                vi,
                                file,
                                &t,
                                "re_exports",
                                imp.line,
                                Some("re-export"),
                            );
                            if imp.type_only {
                                e.attrs.push(("type_only".into(), Attr::Bool(true)));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for d in &f.decorators {
            let target = match self.tops[vi].get(d.name.as_str()) {
                Some(def) if def.kind == DefKind::Function => {
                    Some(self.local_id(vi, &def.id).to_string())
                }
                Some(_) => None,
                None => self.binding(vi, &d.name),
            };
            if let Some(t) = target {
                let src = self.local_id(vi, &d.source);
                self.extracted(vi, src, &t, "references", d.line, Some("decorator"));
            }
        }
        for spec in &f.dynamic {
            let target = self.resolve_spec(vi, spec);
            let tid = self.target_id(&target);
            let e = self.edge(
                vi,
                file,
                &tid,
                "dynamic_import",
                Loc::Absent,
                None,
                Confidence::Extracted,
                None,
            );
            e.weight = None;
        }
    }
}

/// `ref_<slug(specifier without leading ./ and ../)>` for a relative import of a missing file.
fn missing_relative(spec: &str) -> String {
    let mut s = spec;
    loop {
        if let Some(r) = s.strip_prefix("./") {
            s = r;
        } else if let Some(r) = s.strip_prefix("../") {
            s = r;
        } else {
            break;
        }
    }
    format!("ref_{}", slug(s))
}

/// `react` → `react`; `@scope/pkg/sub` → `@scope/pkg`; `node:path` → `node:path`.
fn package_name(spec: &str) -> &str {
    let mut parts = spec.splitn(3, '/');
    let first = parts.next().unwrap_or(spec);
    if first.starts_with('@') {
        match parts.next() {
            Some(second) => &spec[..first.len() + 1 + second.len()],
            None => first,
        }
    } else {
        first
    }
}

/// Drop the final extension of the last path segment.
fn strip_ext(path: &str) -> &str {
    let name_start = path.rfind('/').map_or(0, |i| i + 1);
    match path[name_start..].rfind('.') {
        Some(dot) if dot > 0 => &path[..name_start + dot],
        _ => path,
    }
}

/// Make a config-relative path explicit (`src/x` → `./src/x`) for [`join`].
fn dot(p: &str) -> String {
    if p.starts_with("./") || p.starts_with("../") || p == "." || p == ".." {
        p.to_string()
    } else if let Some(rest) = p.strip_prefix('/') {
        format!("./{rest}")
    } else {
        format!("./{p}")
    }
}

/// Join a root-relative directory and a relative specifier; `None` when it leaves the root.
fn join(dir: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// Remove `//` and `/* */` comments and trailing commas so tsconfig-style JSON parses.
fn strip_jsonc(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c as char);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_str = true;
                out.push('"');
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b',' => {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if !matches!(b.get(j), Some(b'}') | Some(b']')) {
                    out.push(',');
                }
                i += 1;
            }
            _ => {
                // Copy whole UTF-8 sequences untouched.
                let len = utf8_len(c);
                out.push_str(std::str::from_utf8(&b[i..(i + len).min(b.len())]).unwrap_or(""));
                i += len;
            }
        }
    }
    out
}

fn utf8_len(first: u8) -> usize {
    match first {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text)
        .ok()
        .or_else(|| serde_json::from_str(&strip_jsonc(&text)).ok())
}

/// A tsconfig's `baseUrl` and `paths`, following relative `extends` (child settings win).
fn load_tsconfig(root: &Path, rel: &str, depth: u32) -> Option<TsConfig> {
    let json = read_json(&root.join(rel))?;
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let mut cfg = match json.get("extends").and_then(Value::as_str) {
        Some(ext) if depth < 5 && (ext.starts_with("./") || ext.starts_with("../")) => {
            let mut target = join(dir, ext)?;
            if !target.ends_with(".json") {
                target.push_str(".json");
            }
            load_tsconfig(root, &target, depth + 1)
        }
        _ => None,
    }
    .unwrap_or(TsConfig {
        base: dir.to_string(),
        has_base_url: false,
        paths: Vec::new(),
    });
    let opts = json.get("compilerOptions");
    if let Some(base) = opts.and_then(|o| o.get("baseUrl")).and_then(Value::as_str) {
        cfg.base = join(dir, &dot(base))?;
        cfg.has_base_url = true;
    }
    if let Some(paths) = opts.and_then(|o| o.get("paths")).and_then(Value::as_object) {
        if !cfg.has_base_url {
            cfg.base = dir.to_string();
        }
        cfg.paths = paths
            .iter()
            .map(|(k, v)| {
                let targets = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                (k.clone(), targets)
            })
            .collect();
    }
    Some(cfg)
}

fn load_package(path: &Path, dir: &str) -> Option<Package> {
    let json = read_json(path)?;
    let name = json.get("name")?.as_str()?.to_string();
    let mut exports = Vec::new();
    let has_exports = json.get("exports").is_some();
    let mut has_main = false;
    match json.get("exports") {
        Some(Value::String(s)) => exports.push((".".to_string(), s.clone())),
        Some(Value::Object(map)) => {
            let subpaths = map.keys().all(|k| k.starts_with('.'));
            if subpaths {
                for (k, v) in map {
                    if let Some(t) = export_target(v) {
                        exports.push((k.clone(), t));
                    }
                }
            } else if let Some(t) = export_target(&Value::Object(map.clone())) {
                exports.push((".".to_string(), t));
            }
        }
        _ => {}
    }
    for field in ["source", "module", "main", "types", "typings"] {
        if let Some(t) = json.get(field).and_then(Value::as_str) {
            has_main = true;
            exports.push((".".to_string(), t.to_string()));
        }
    }
    Some(Package {
        name,
        dir: dir.to_string(),
        exports,
        has_exports,
        has_main,
    })
}

/// A string export target, preferring source-like conditions.
fn export_target(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => {
            for cond in [
                "source", "types", "import", "module", "default", "require", "node",
            ] {
                if let Some(t) = map.get(cond).and_then(export_target) {
                    return Some(t);
                }
            }
            map.values().find_map(export_target)
        }
        Value::Array(a) => a.iter().find_map(export_target),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_and_missing_paths() {
        assert_eq!(package_name("react"), "react");
        assert_eq!(package_name("react-dom/client"), "react-dom");
        assert_eq!(package_name("@scope/other-pkg/x"), "@scope/other-pkg");
        assert_eq!(package_name("@/lib/b"), "@/lib");
        assert_eq!(package_name("node:path"), "node:path");
        assert_eq!(missing_relative("../../x/y"), "ref_x_y");
        assert_eq!(missing_relative("./polyfill"), "ref_polyfill");
        assert_eq!(strip_ext("a.b/styles.module.css"), "a.b/styles.module");
        assert_eq!(strip_ext("a/b"), "a/b");
    }

    #[test]
    fn joins_stay_inside_the_root() {
        assert_eq!(join("src/deep", "../lib/b").as_deref(), Some("src/lib/b"));
        assert_eq!(join("", "./a").as_deref(), Some("a"));
        assert_eq!(join("src", "../../x"), None);
        assert_eq!(dot("src/*"), "./src/*");
    }

    #[test]
    fn jsonc_comments_and_trailing_commas() {
        let v: Value = serde_json::from_str(&strip_jsonc(
            "{ // c\n \"a\": \"//not\", /* b */ \"p\": [\"x\",],\n}",
        ))
        .unwrap();
        assert_eq!(v["a"], "//not");
        assert_eq!(v["p"][0], "x");
    }
}

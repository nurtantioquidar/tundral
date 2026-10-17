//! Rust module resolution for `use` paths (dijkstral-native; Graphify has none, §11.3 / Q7).
//!
//! The module tree is rebuilt from the files themselves: crate roots are the Cargo target
//! conventions under a directory holding a `Cargo.toml` (`src/lib.rs`, `src/main.rs`,
//! `src/bin/*`, `tests/*.rs`, `examples/*`, `benches/*.rs`, `build.rs`), and every file reached
//! from a root through `mod name;` (`#[path]` included) belongs to that root's crate. A file no
//! declaration reaches is a root of its own (a `[[bin]] path` target such as ripgrep's
//! `crates/core/main.rs`).
//!
//! Crate names come from the manifests: a package's library (`[lib] path`, else `src/lib.rs`) is
//! named by `[lib] name`, else its package name, with `-` replaced by `_`. Inside a package, a
//! crate name means what the package's own dependency tables bind it to: a `path` dependency
//! (directly, or inherited through `workspace = true` from the workspace root's
//! `[workspace.dependencies]`) names that workspace library, under its key when renamed with
//! `package = …`, else under the library's name; the package's own library is known to its other
//! targets by its name; any other crate is outside the repository, even when a workspace package
//! happens to share its name. Files outside every package fall back
//! to the library names of the whole repository.
//!
//! A `use` path then resolves segment by segment: `crate`, `self`, `super` (which stays in the
//! file inside an inline `mod` block), child modules, items the file declares (inline modules are
//! flattened into their file, like the graph does), names the module itself imports (`pub use`
//! re-exports and globs are followed), `extern crate … as` aliases and workspace crates. Anything
//! else (`std`, a third-party crate) is external.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::extract::data::CargoManifest;
use crate::extract::rust::{RsMod, RsUse, RustFacts};
use crate::facts::LangFacts;
use crate::lang::Family;
use crate::resolve::View;

/// What a path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Res<'v> {
    /// A module: the file that holds it.
    File(usize),
    /// An item declared in a file (or anything below it, such as an enum variant), with the name
    /// to look up there: the item's own name once `use … as` aliases are followed, or the path's
    /// last segment when the path continues below the item (`Kind::Round`).
    Item(usize, &'v str),
    /// Outside the repository.
    External,
}

impl Res<'_> {
    /// The file a path leads to, if it is in the repository.
    pub(super) fn file(self) -> Option<usize> {
        match self {
            Res::File(f) | Res::Item(f, _) => Some(f),
            Res::External => None,
        }
    }
}

pub(super) struct Modules<'v, 'a> {
    /// Rust files, in view order.
    pub(super) views: Vec<&'v View<'a>>,
    facts: Vec<Option<&'v RustFacts>>,
    parent: Vec<Option<usize>>,
    root: Vec<usize>,
    /// Out-of-line child modules by inline path and name (`name`, `inline::name`).
    children: Vec<HashMap<String, usize>>,
    /// Item names, and whether a path continues through each (a module or a type).
    items: Vec<HashMap<&'v str, bool>>,
    /// Inline `mod name { … }` blocks per file: flattened, a path continues in the file itself.
    inline: Vec<HashSet<&'v str>>,
    /// Library roots by crate name, over every package (files outside any package).
    libs: HashMap<String, usize>,
    /// Per package directory: the crate names its manifest binds, and its own library's name.
    crates: HashMap<&'v str, Crates>,
    /// Package directories (manifests with `[package]`).
    packages: HashSet<&'v str>,
    memo: RefCell<HashMap<(usize, String, bool), Option<Res<'v>>>>,
}

/// The crate names one package can use.
#[derive(Default)]
struct Crates {
    /// Dependency key (`-` → `_`) → the workspace library it names, `None` outside the repo.
    deps: HashMap<String, Option<usize>>,
    /// The package's own library: its crate name and root file.
    own: Option<(String, usize)>,
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn join(dir: &str, rest: &str) -> String {
    if dir.is_empty() {
        rest.to_string()
    } else {
        format!("{dir}/{rest}")
    }
}

/// `a/b/../c/./d.rs` → `a/c/d.rs`.
fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            p => out.push(p),
        }
    }
    out.join("/")
}

/// The files `mod` declaration `m` in file `rel` may name, read as a crate root's (or `mod.rs`)
/// declaration when `as_root`, else as a plain module file's (`a.rs` → `a/name.rs`).
fn mod_candidates(rel: &str, as_root: bool, m: &RsMod) -> Vec<String> {
    let dir = dir_of(rel);
    let stem = rel
        .rsplit('/')
        .next()
        .unwrap_or(rel)
        .trim_end_matches(".rs");
    let mdir = if as_root || stem == "mod" {
        dir.to_string()
    } else {
        join(dir, stem)
    };
    match &m.path {
        Some(p) if m.inline.is_empty() => vec![normalize(&join(dir, p))],
        Some(p) => vec![normalize(&join(&join(&mdir, &m.inline.join("/")), p))],
        None => {
            let base = if m.inline.is_empty() {
                mdir
            } else {
                join(&mdir, &m.inline.join("/"))
            };
            vec![
                join(&base, &format!("{}.rs", m.name)),
                join(&base, &format!("{}/mod.rs", m.name)),
            ]
        }
    }
}

/// The directory of the nearest `Cargo.toml` package above `rel`.
fn package_dir<'p>(packages: &HashSet<&str>, rel: &'p str) -> Option<&'p str> {
    let mut dir = dir_of(rel);
    loop {
        if packages.contains(dir) {
            return Some(dir);
        }
        if dir.is_empty() {
            return None;
        }
        dir = dir_of(dir);
    }
}

/// A Cargo target root relative to its package directory.
fn conventional_root(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.as_slice() {
        ["build.rs"] => true,
        ["src", "lib.rs" | "main.rs"] => true,
        ["src", "bin", f] | ["tests" | "examples" | "benches", f] => f.ends_with(".rs"),
        ["src", "bin", _, "main.rs"] | ["examples", _, "main.rs"] => true,
        _ => false,
    }
}

impl<'v, 'a> Modules<'v, 'a> {
    pub(super) fn new(all: &'v [View<'a>]) -> Self {
        let views: Vec<&View> = all.iter().filter(|v| v.family() == Family::Rust).collect();
        let facts: Vec<Option<&RustFacts>> = views
            .iter()
            .map(|v| match &v.facts.lang {
                LangFacts::Rust(f) => Some(f),
                _ => None,
            })
            .collect();
        let by_path: HashMap<&str, usize> = views
            .iter()
            .enumerate()
            .map(|(i, v)| (v.file.rel.as_str(), i))
            .collect();

        // Cargo manifests by directory; packages are the ones with `[package] name`.
        let mut manifests: HashMap<&str, &CargoManifest> = HashMap::new();
        for v in all {
            let rel = v.file.rel.as_str();
            if rel != "Cargo.toml" && !rel.ends_with("/Cargo.toml") {
                continue;
            }
            if let LangFacts::Data(d) = &v.facts.lang {
                if let Some(m) = &d.cargo {
                    manifests.insert(dir_of(rel), m);
                }
            }
        }
        let packages: HashSet<&str> = manifests
            .iter()
            .filter(|(_, m)| m.package.is_some())
            .map(|(d, _)| *d)
            .collect();

        let n = views.len();
        let mut parent = vec![None; n];
        let mut root: Vec<Option<usize>> = vec![None; n];
        let mut children: Vec<HashMap<String, usize>> = vec![HashMap::new(); n];
        let mut is_root = vec![false; n];

        // Files some `mod` declaration may name (either reading of its parent): a conventional
        // target path that is declared (`tests/feature.rs` under `tests/tests.rs` with
        // `autotests = false`) is a module, not a root.
        let mut declared: HashSet<usize> = HashSet::new();
        for (f, fx) in facts.iter().enumerate() {
            let Some(fx) = fx else { continue };
            let rel = views[f].file.rel.as_str();
            for m in &fx.mods {
                for as_root in [true, false] {
                    for p in mod_candidates(rel, as_root, m) {
                        if let Some(&c) = by_path.get(p.as_str()) {
                            if c != f {
                                declared.insert(c);
                            }
                        }
                    }
                }
            }
        }
        let mut order: Vec<usize> = (0..n)
            .filter(|&i| {
                let rel = views[i].file.rel.as_str();
                let main = matches!(rel.rsplit('/').next(), Some("main.rs" | "lib.rs"));
                if declared.contains(&i) && !main {
                    return false;
                }
                package_dir(&packages, rel).is_some_and(|d| {
                    let inner = if d.is_empty() {
                        rel
                    } else {
                        &rel[d.len() + 1..]
                    };
                    conventional_root(inner)
                })
            })
            .collect();
        order.sort_by(|&a, &b| views[a].file.rel.cmp(&views[b].file.rel));
        // Files no declaration reaches come after, `main.rs`/`lib.rs` and shallow paths first.
        let mut rest: Vec<usize> = (0..n).collect();
        rest.sort_by_key(|&i| {
            let rel = views[i].file.rel.as_str();
            let stem = rel.rsplit('/').next().unwrap_or(rel);
            (
                !matches!(stem, "main.rs" | "lib.rs"),
                rel.matches('/').count(),
                rel,
            )
        });
        order.extend(rest);

        for start in order {
            if root[start].is_some() {
                continue;
            }
            root[start] = Some(start);
            is_root[start] = true;
            let mut queue = vec![start];
            while let Some(f) = queue.pop() {
                let Some(fx) = facts[f] else { continue };
                let rel = views[f].file.rel.as_str();
                for m in &fx.mods {
                    let candidates = mod_candidates(rel, is_root[f], m);
                    let Some(&c) = candidates.iter().find_map(|p| by_path.get(p.as_str())) else {
                        continue;
                    };
                    if root[c].is_some() {
                        continue;
                    }
                    root[c] = root[f];
                    parent[c] = Some(f);
                    let key = if m.inline.is_empty() {
                        m.name.clone()
                    } else {
                        format!("{}::{}", m.inline.join("::"), m.name)
                    };
                    children[f].entry(key).or_insert(c);
                    queue.push(c);
                }
            }
        }
        let root: Vec<usize> = root
            .into_iter()
            .enumerate()
            .map(|(i, r)| r.unwrap_or(i))
            .collect();

        // Each package's library: (crate name, root file).
        let mut dirs: Vec<&str> = packages.iter().copied().collect();
        dirs.sort_unstable();
        let lib_of = |dir: &str| -> Option<(String, usize)> {
            let m = manifests.get(dir)?;
            let name = m.lib_name.as_deref().or(m.package.as_deref())?;
            let path = match &m.lib_path {
                Some(p) => normalize(&join(dir, p)),
                None => join(dir, "src/lib.rs"),
            };
            let &f = by_path.get(path.as_str())?;
            (root[f] == f).then(|| (name.replace('-', "_"), f))
        };
        let mut libs = HashMap::new();
        for dir in &dirs {
            if let Some((name, f)) = lib_of(dir) {
                libs.entry(name).or_insert(f);
            }
        }
        // What each package's dependency keys name. A `workspace = true` entry takes its `path`
        // and `package` from the nearest manifest above that lists the key in
        // `[workspace.dependencies]`.
        let mut crates: HashMap<&str, Crates> = HashMap::new();
        for dir in &dirs {
            let m = manifests[dir];
            let mut c = Crates {
                own: lib_of(dir),
                ..Default::default()
            };
            for dep in &m.deps {
                let (base, spec) = if dep.workspace {
                    let mut up = Some(*dir);
                    let mut found = None;
                    while let Some(d) = up {
                        if let Some(w) = manifests
                            .get(d)
                            .and_then(|w| w.workspace_deps.iter().find(|w| w.key == dep.key))
                        {
                            found = Some((d, w));
                            break;
                        }
                        up = (!d.is_empty()).then(|| dir_of(d));
                    }
                    found.unwrap_or((*dir, dep))
                } else {
                    (*dir, dep)
                };
                let renamed = spec.package.as_deref().or(dep.package.as_deref());
                let package = renamed.unwrap_or(&dep.key);
                let target = spec.path.as_ref().and_then(|p| {
                    let d = normalize(&join(base, p));
                    let t = manifests.get(d.as_str())?;
                    if t.package.as_deref() != Some(package) {
                        return None;
                    }
                    lib_of(&d)
                });
                // Code names a dependency by its key when renamed, else by its library's name.
                let name = match (&target, renamed) {
                    (Some((lib, _)), None) => lib.clone(),
                    _ => dep.key.replace('-', "_"),
                };
                c.deps.entry(name).or_insert(target.map(|(_, f)| f));
            }
            crates.insert(dir, c);
        }

        let items = facts
            .iter()
            .map(|f| {
                f.map(|f| f.items.iter().map(|(n, p)| (n.as_str(), *p)).collect())
                    .unwrap_or_default()
            })
            .collect();
        let inline = facts
            .iter()
            .map(|f| {
                f.map(|f| f.inline_mods.iter().map(String::as_str).collect())
                    .unwrap_or_default()
            })
            .collect();
        Modules {
            inline,
            views,
            facts,
            parent,
            root,
            children,
            items,
            libs,
            crates,
            packages,
            memo: RefCell::new(HashMap::new()),
        }
    }

    pub(super) fn uses(&self, f: usize) -> &'v [RsUse] {
        self.facts[f].map_or(&[], |x| x.uses.as_slice())
    }

    /// What a `use` leaf of file `f` names.
    pub(super) fn resolve_use(&self, f: usize, u: &'v RsUse) -> Option<Res<'v>> {
        self.resolve_path(f, u.depth, &u.path)
    }

    /// The crate root's `extern crate … as name`, else [`Self::crate_lib`].
    fn crate_named(&self, f: usize, name: &str) -> Res<'v> {
        let root = self.root[f];
        if let Some(fx) = self.facts[root] {
            if let Some((krate, _)) = fx.extern_crates.iter().find(|(_, b)| b == name) {
                return self.crate_lib(f, krate);
            }
        }
        self.crate_lib(f, name)
    }

    /// The library crate `name` means in file `f`'s package (module docs).
    fn crate_lib(&self, f: usize, name: &str) -> Res<'v> {
        let root = self.root[f];
        let lib = match package_dir(&self.packages, &self.views[root].file.rel)
            .and_then(|d| self.crates.get(d))
        {
            Some(c) => match c.deps.get(name) {
                Some(t) => *t,
                None => c
                    .own
                    .as_ref()
                    .filter(|(own, l)| own == name && *l != root)
                    .map(|(_, l)| *l),
            },
            None => self.libs.get(name).copied(),
        };
        lib.map_or(Res::External, Res::File)
    }

    fn resolve_path(&self, f: usize, depth: u32, segs: &'v [String]) -> Option<Res<'v>> {
        self.walk(f, depth, segs, false)
    }

    /// The `use` declarations of file `f` that bind names for code in function `func` (local
    /// id): the function body's own, then the file's and its inline modules'.
    pub(super) fn uses_in(&self, f: usize, func: &str) -> Vec<&'v RsUse> {
        let uses = self.uses(f);
        let own = uses.iter().filter(|u| u.func.as_deref() == Some(func));
        own.chain(uses.iter().filter(|u| !u.in_fn)).collect()
    }

    /// What the qualifier of a path call names (`a::b` of `a::b::f()`, `T` of `T::new()`): the
    /// whole path must resolve, not stop at an item part-way.
    pub(super) fn resolve_qualifier<'s>(
        &self,
        f: usize,
        depth: u32,
        segs: &'s [String],
    ) -> Option<Res<'s>>
    where
        'v: 's,
    {
        self.walk(f, depth, segs, true)
    }

    fn walk<'s>(&self, f: usize, depth: u32, segs: &'s [String], whole: bool) -> Option<Res<'s>>
    where
        'v: 's,
    {
        let last = segs.last()?.as_str();
        // An inline module is flattened into its file.
        let flat = |r: Res<'v>| match r {
            Res::Item(t, n) if self.inline[t].contains(n) => Res::File(t),
            r => r,
        };
        // An item reached before the last segment: the rest of the path is below it.
        let early = |r: Res<'s>, i: usize| match r {
            Res::Item(t, _) if i + 1 < segs.len() => (!whole).then_some(Res::Item(t, last)),
            r => Some(r),
        };
        let first = segs.first()?;
        let (mut cur, mut i) = match first.as_str() {
            "crate" => (self.root[f], 1),
            "self" => (f, 1),
            "super" => (f, 0),
            "" => match self.crate_named(f, segs.get(1)?) {
                Res::File(l) => (l, 2),
                other => return Some(other),
            },
            name => match self.lookup(f, name, segs.len() > 1).map(flat) {
                Some(Res::File(c)) => (c, 1),
                Some(r) => return early(r, 0),
                None => match self.crate_named(f, name) {
                    Res::File(l) => (l, 1),
                    other => return Some(other),
                },
            },
        };
        let mut depth = depth;
        while i < segs.len() {
            let seg = segs[i].as_str();
            match seg {
                "self" => i += 1,
                "super" => {
                    if cur == f && depth > 0 {
                        depth -= 1;
                    } else {
                        cur = self.parent[cur]?;
                    }
                    i += 1;
                }
                _ => {
                    // An out-of-line module declared inside inline ones (`a::b` for
                    // `mod a { mod b; }`), longest first; then any name the module exposes.
                    let child = (i + 1..=segs.len()).rev().find_map(|end| {
                        self.children[cur]
                            .get(&segs[i..end].join("::"))
                            .map(|&c| (c, end))
                    });
                    if let Some((c, end)) = child {
                        cur = c;
                        i = end;
                        continue;
                    }
                    match flat(self.lookup(cur, seg, i + 1 < segs.len())?) {
                        Res::File(c) => {
                            cur = c;
                            i += 1;
                        }
                        r => return early(r, i),
                    }
                }
            }
        }
        Some(Res::File(cur))
    }

    /// What module file `f` exposes as `name` at its top level: a child module, an item, a name
    /// it imports (followed), an `extern crate` alias, or a name a glob import brings in. With
    /// `prefix`, the path continues, so a function or value of that name does not count.
    pub(super) fn lookup(&self, f: usize, name: &str, prefix: bool) -> Option<Res<'v>> {
        let key = (f, name.to_string(), prefix);
        if let Some(hit) = self.memo.borrow().get(&key) {
            return *hit;
        }
        // Guard against import cycles while this lookup is in progress.
        self.memo.borrow_mut().insert(key.clone(), None);
        let found = self.lookup_uncached(f, name, prefix);
        self.memo.borrow_mut().insert(key, found);
        found
    }

    /// What module file `f` exposes as `name` in the value namespace (a function, constant or
    /// tuple struct): like [`Self::lookup`], but a child module of that name does not hide a
    /// function re-exported beside it (`mod hostname; pub use hostname::hostname;`).
    pub(super) fn lookup_value(&self, f: usize, name: &str) -> Option<Res<'v>> {
        self.lookup_value_in(f, name, 0)
    }

    fn lookup_value_in(&self, f: usize, name: &str, hops: u32) -> Option<Res<'v>> {
        // `mod name;` is an item of the file too, but not a value.
        if !self.children[f].contains_key(name) {
            if let Some((&key, _)) = self.items[f].get_key_value(name) {
                return Some(Res::Item(f, key));
            }
        }
        let fx = self.facts[f]?;
        let top = |u: &&RsUse| u.depth == 0 && !u.in_fn;
        for u in fx.uses.iter().filter(top).filter(|u| !u.glob) {
            if u.bound.as_deref() == Some(name) {
                if let Some(r @ (Res::Item(..) | Res::External)) = self.resolve_use(f, u) {
                    return Some(r);
                }
            }
        }
        // Globs, a bounded number of hops deep (glob cycles are legal).
        if hops < 8 {
            for u in fx.uses.iter().filter(top).filter(|u| u.glob) {
                if let Some(Res::File(g)) = self.resolve_use(f, u) {
                    if g != f {
                        if let Some(r) = self.lookup_value_in(g, name, hops + 1) {
                            return Some(r);
                        }
                    }
                }
            }
        }
        None
    }

    fn lookup_uncached(&self, f: usize, name: &str, prefix: bool) -> Option<Res<'v>> {
        if let Some(&c) = self.children[f].get(name) {
            return Some(Res::File(c));
        }
        if let Some((&key, &p)) = self.items[f].get_key_value(name) {
            if p || !prefix {
                return Some(Res::Item(f, key));
            }
        }
        let fx = self.facts[f]?;
        // A `use` in a function body binds the name in that body only.
        for u in fx
            .uses
            .iter()
            .filter(|u| u.depth == 0 && !u.glob && !u.in_fn)
        {
            if u.bound.as_deref() == Some(name) {
                if let Some(r) = self.resolve_use(f, u) {
                    return Some(r);
                }
            }
        }
        if let Some((krate, _)) = fx.extern_crates.iter().find(|(_, b)| b == name) {
            return Some(self.crate_lib(f, krate));
        }
        for u in fx
            .uses
            .iter()
            .filter(|u| u.depth == 0 && u.glob && !u.in_fn)
        {
            if let Some(Res::File(g)) = self.resolve_use(f, u) {
                if g != f {
                    if let Some(r) = self.lookup(g, name, prefix) {
                        return Some(r);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_target_roots() {
        assert_eq!(normalize("a/b/../c/./d.rs"), "a/c/d.rs");
        assert!(conventional_root("src/lib.rs"));
        assert!(conventional_root("tests/it.rs"));
        assert!(conventional_root("src/bin/tool/main.rs"));
        assert!(!conventional_root("src/model.rs"));
        assert!(!conventional_root("tests/common/mod.rs"));
    }
}

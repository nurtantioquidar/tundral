//! Rust module resolution for `use` paths (dijkstral-native; Graphify has none, §11.3 / Q7).
//!
//! The module tree is rebuilt from the files themselves: crate roots are the Cargo target
//! conventions under a directory holding a `Cargo.toml` (`src/lib.rs`, `src/main.rs`,
//! `src/bin/*`, `tests/*.rs`, `examples/*`, `benches/*.rs`, `build.rs`), and every file reached
//! from a root through `mod name;` (`#[path]` included) belongs to that root's crate. A file no
//! declaration reaches is a root of its own (a `[[bin]] path` target such as ripgrep's
//! `crates/core/main.rs`). A library root is known to other crates by its package name with `-`
//! replaced by `_`.
//!
//! A `use` path then resolves segment by segment: `crate`, `self`, `super` (which stays in the
//! file inside an inline `mod` block), child modules, items the file declares (inline modules are
//! flattened into their file, like the graph does), names the module itself imports (`pub use`
//! re-exports and globs are followed), `extern crate … as` aliases and workspace crates. Anything
//! else (`std`, a third-party crate) is external.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::extract::rust::{RsMod, RsUse, RustFacts};
use crate::facts::{Attr, LangFacts};
use crate::lang::Family;
use crate::resolve::View;

/// What a path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Res {
    /// A module: the file that holds it.
    File(usize),
    /// An item declared in a file (or anything below it, such as an enum variant).
    Item(usize),
    /// Outside the repository.
    External,
}

impl Res {
    /// The file a path leads to, if it is in the repository.
    pub(super) fn file(self) -> Option<usize> {
        match self {
            Res::File(f) | Res::Item(f) => Some(f),
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
    /// Library roots by crate name.
    libs: HashMap<String, usize>,
    memo: RefCell<HashMap<(usize, String, bool), Option<Res>>>,
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
fn package_dir<'p>(packages: &HashMap<&str, &str>, rel: &'p str) -> Option<&'p str> {
    let mut dir = dir_of(rel);
    loop {
        if packages.contains_key(dir) {
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

        // Cargo packages: directory → package name.
        let mut packages: HashMap<&str, &str> = HashMap::new();
        for v in all {
            let rel = v.file.rel.as_str();
            if rel != "Cargo.toml" && !rel.ends_with("/Cargo.toml") {
                continue;
            }
            for n in &v.facts.nodes {
                if matches!(n.attr("ecosystem"), Some(Attr::Str(e)) if e == "cargo") {
                    packages.insert(dir_of(rel), n.label.as_str());
                }
            }
        }

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

        let mut libs = HashMap::new();
        for (dir, name) in &packages {
            if let Some(&f) = by_path.get(join(dir, "src/lib.rs").as_str()) {
                if root[f] == f {
                    libs.entry(name.replace('-', "_")).or_insert(f);
                }
            }
        }

        let items = facts
            .iter()
            .map(|f| {
                f.map(|f| f.items.iter().map(|(n, p)| (n.as_str(), *p)).collect())
                    .unwrap_or_default()
            })
            .collect();
        Modules {
            views,
            facts,
            parent,
            root,
            children,
            items,
            libs,
            memo: RefCell::new(HashMap::new()),
        }
    }

    pub(super) fn uses(&self, f: usize) -> &'v [RsUse] {
        self.facts[f].map_or(&[], |x| x.uses.as_slice())
    }

    /// What a `use` leaf of file `f` names.
    pub(super) fn resolve_use(&self, f: usize, u: &RsUse) -> Option<Res> {
        self.resolve_path(f, u.depth, &u.path)
    }

    fn crate_named(&self, f: usize, name: &str) -> Res {
        let root = self.root[f];
        if let Some(fx) = self.facts[root] {
            if let Some((krate, _)) = fx.extern_crates.iter().find(|(_, b)| b == name) {
                return self
                    .libs
                    .get(krate)
                    .map_or(Res::External, |&l| Res::File(l));
            }
        }
        self.libs.get(name).map_or(Res::External, |&l| Res::File(l))
    }

    fn resolve_path(&self, f: usize, depth: u32, segs: &[String]) -> Option<Res> {
        let first = segs.first()?;
        let (mut cur, mut i) = match first.as_str() {
            "crate" => (self.root[f], 1),
            "self" => (f, 1),
            "super" => (f, 0),
            "" => match self.crate_named(f, segs.get(1)?) {
                Res::File(l) => (l, 2),
                other => return Some(other),
            },
            name => match self.lookup(f, name, segs.len() > 1) {
                Some(Res::File(c)) => (c, 1),
                Some(r) => return Some(r),
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
                    match self.lookup(cur, seg, i + 1 < segs.len())? {
                        Res::File(c) => {
                            cur = c;
                            i += 1;
                        }
                        r => return Some(r),
                    }
                }
            }
        }
        Some(Res::File(cur))
    }

    /// What module file `f` exposes as `name` at its top level: a child module, an item, a name
    /// it imports (followed), an `extern crate` alias, or a name a glob import brings in. With
    /// `prefix`, the path continues, so a function or value of that name does not count.
    pub(super) fn lookup(&self, f: usize, name: &str, prefix: bool) -> Option<Res> {
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

    fn lookup_uncached(&self, f: usize, name: &str, prefix: bool) -> Option<Res> {
        if let Some(&c) = self.children[f].get(name) {
            return Some(Res::File(c));
        }
        if let Some(&p) = self.items[f].get(name) {
            if p || !prefix {
                return Some(Res::Item(f));
            }
        }
        let fx = self.facts[f]?;
        for u in fx.uses.iter().filter(|u| u.depth == 0 && !u.glob) {
            if u.bound.as_deref() == Some(name) {
                if let Some(r) = self.resolve_use(f, u) {
                    return Some(r);
                }
            }
        }
        if let Some((krate, _)) = fx.extern_crates.iter().find(|(_, b)| b == name) {
            return Some(
                self.libs
                    .get(krate)
                    .map_or(Res::External, |&l| Res::File(l)),
            );
        }
        for u in fx.uses.iter().filter(|u| u.depth == 0 && u.glob) {
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

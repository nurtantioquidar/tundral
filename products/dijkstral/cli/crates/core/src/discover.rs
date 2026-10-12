//! Corpus discovery: which files under a root are indexed, and as what language.
//!
//! The walk is parallel and scopes every ignore file to its own directory (unlike a walk that
//! accumulates patterns as it descends). `.gitignore`, `.git/info/exclude`, `.graphifyignore` and
//! `.dijkstralignore` are honoured; the user's global git excludes are not, so the corpus does not
//! depend on the machine. The default skip lists match Graphify so both tools index the same files.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ignore::{WalkBuilder, WalkState};

use crate::lang::Lang;

/// Directory names that are never descended into, at any depth (§14.5). Hidden directories in
/// general, `vendor`, `third_party` and the like are indexed, as Graphify does.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
    "site-packages",
    ".tox",
    ".nox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".cache",
    ".idea",
    ".vscode",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".angular",
    ".turbo",
    ".parcel-cache",
    ".serverless",
    ".terraform",
    ".claude",
    "__snapshots__",
    "storybook-static",
    ".eggs",
    "graphify-out",
    "dijkstral-out",
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceFile {
    /// Repo-relative path with `/` separators.
    pub rel: String,
    pub abs: PathBuf,
    pub lang: Lang,
}

/// Every indexable file under `root`, sorted by relative path.
pub fn discover(root: &Path, threads: usize) -> Vec<SourceFile> {
    let found = Mutex::new(Vec::new());
    WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .require_git(false)
        .parents(false)
        .add_custom_ignore_filename(".graphifyignore")
        .add_custom_ignore_filename(".dijkstralignore")
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            if e.file_type().is_some_and(|t| t.is_dir()) {
                !SKIP_DIRS.contains(&name.as_ref()) && !name.ends_with(".egg-info")
            } else {
                !is_sensitive(&name)
            }
        })
        .threads(threads)
        .build_parallel()
        .run(|| {
            let found = &found;
            Box::new(move |entry| {
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                let path = entry.path();
                if let Some(lang) = Lang::classify(path) {
                    if let Some(rel) = relative(root, path) {
                        found.lock().unwrap().push(SourceFile {
                            rel,
                            abs: path.to_path_buf(),
                            lang,
                        });
                    }
                }
                WalkState::Continue
            })
        });
    let mut files = found.into_inner().unwrap();
    files.sort();
    files
}

/// Files that may hold secrets are never read (§14.5): `.env`, `.env.*`, `*.pem`.
fn is_sensitive(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.") || name.ends_with(".pem")
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn honours_skip_lists_and_scoped_ignore_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for p in [
            "src/a.py",
            "src/b.ts",
            "node_modules/x/index.js",
            "pkg/gen/c.go",
            "pkg/d.go",
            "other/gen/e.go",
            "secrets/server.pem",
            "README.txt",
            "package-lock.json",
            "vendor/v.go",
        ] {
            let path = root.join(p);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        // An ignore file only applies beneath its own directory.
        fs::write(root.join("pkg/.graphifyignore"), "gen/\n").unwrap();
        let rels: Vec<_> = discover(root, 2).into_iter().map(|f| f.rel).collect();
        assert_eq!(
            rels,
            [
                "other/gen/e.go",
                "pkg/d.go",
                "src/a.py",
                "src/b.ts",
                "vendor/v.go"
            ]
        );
    }
}

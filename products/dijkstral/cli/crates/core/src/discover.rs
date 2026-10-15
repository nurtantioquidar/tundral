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
pub const SKIP_DIRS: &[&str] = &[
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

/// Saved results (`dijkstral save-result`), relative to the root. Although the output directory is
/// skipped, this subtree is walked as corpus content, as Graphify walks `graphify-out/memory`
/// (`docs/graphify-memory.md` §3); only ignore files inside it apply.
pub const MEMORY_DIR: &str = "dijkstral-out/memory";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceFile {
    /// Repo-relative path with `/` separators.
    pub rel: String,
    pub abs: PathBuf,
    pub lang: Lang,
}

/// Every indexable file under `root`, sorted by relative path.
pub fn discover(root: &Path, threads: usize) -> Vec<SourceFile> {
    discover_all(root, threads).files
}

/// What a walk of the corpus found.
#[derive(Debug, Default)]
pub struct Discovery {
    /// Indexable files, sorted by relative path.
    pub files: Vec<SourceFile>,
    /// Relative paths, sorted, of the walked files that are neither code nor a document, paper,
    /// image or media file Graphify's `update` would index, nor a lockfile or sensitive file it
    /// drops silently: the report's `Unclassified` line (§17.5).
    pub unclassified: Vec<String>,
    /// Relative paths, sorted, of the walked files that are not code but whose extension
    /// `update` indexes (documents, papers, images, media, office and Google shortcuts): the
    /// candidates of the semantic pass and of the markdown structure (`docs/graphify-semantic.md`
    /// §2, §3), after the same ignore rules, skipped directories and sensitive-file filter.
    pub documents: Vec<String>,
}

/// Extensions `update` indexes as documents, papers, images or media (probed one by one).
const INDEXED_NON_CODE: &[&str] = &[
    "md", "mdx", "rst", "txt", "html", "yaml", "yml", "pdf", "png", "svg", "jpg", "jpeg", "gif",
    "webp", "mp4", "mov", "webm", "m4v", "mkv", "avi", "mp3", "wav", "ogg", "m4a", "docx", "xlsx",
    "gdoc", "gsheet", "gslides",
];

/// Files Graphify skips without counting: lockfiles and likely secrets (probed).
fn dropped_silently(name: &str) -> bool {
    matches!(
        name,
        "package-lock.json"
            | "Cargo.lock"
            | "yarn.lock"
            | "composer.lock"
            | "poetry.lock"
            | "Gemfile.lock"
            | "go.sum"
            | "id_rsa"
            | ".npmrc"
            | ".netrc"
    ) || [".key", ".crt", ".p12", ".pfx"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// How a walked file that is not code is counted.
#[derive(PartialEq)]
enum NonCode {
    /// Dropped without counting (lockfiles, likely secrets).
    Dropped,
    /// A document, paper, image or media file `update` indexes.
    Indexed,
    /// The report's `Unclassified` line.
    Unclassified,
}

fn non_code(name: &str) -> NonCode {
    if dropped_silently(name) {
        return NonCode::Dropped;
    }
    match name.rfind('.') {
        Some(i)
            if i > 0
                && i < name.len() - 1
                && INDEXED_NON_CODE.contains(&name[i + 1..].to_ascii_lowercase().as_str()) =>
        {
            NonCode::Indexed
        }
        _ => NonCode::Unclassified,
    }
}

/// [`discover`], also collecting the unclassified files.
pub fn discover_all(root: &Path, threads: usize) -> Discovery {
    discover_all_with(root, threads, true)
}

/// [`discover_all`]; with `gitignore` false, `.gitignore` files and `.git/info/exclude` are not
/// read (`extract --no-gitignore`, `docs/graphify-inputs.md` §5) while `.graphifyignore` and
/// `.dijkstralignore` still apply.
pub fn discover_all_with(root: &Path, threads: usize, gitignore: bool) -> Discovery {
    let found = Mutex::new(Vec::new());
    let unclassified = Mutex::new(Vec::new());
    let documents = Mutex::new(Vec::new());
    let sinks = (&found, &unclassified, &documents);
    walk(root, root, threads, gitignore, sinks);
    let memory = root.join(MEMORY_DIR);
    if memory.is_dir() {
        walk(&memory, root, threads, gitignore, sinks);
    }
    let mut files = found.into_inner().unwrap();
    files.sort();
    files.dedup();
    let mut unclassified = unclassified.into_inner().unwrap();
    unclassified.sort();
    unclassified.dedup();
    let mut documents = documents.into_inner().unwrap();
    documents.sort();
    documents.dedup();
    Discovery {
        files,
        unclassified,
        documents,
    }
}

type Sinks<'a> = (
    &'a Mutex<Vec<SourceFile>>,
    &'a Mutex<Vec<String>>,
    &'a Mutex<Vec<String>>,
);

/// Walk `start` (the root, or a subtree of it), naming files relative to `root`.
fn walk(
    start: &Path,
    root: &Path,
    threads: usize,
    gitignore: bool,
    (found, unclassified, documents): Sinks,
) {
    WalkBuilder::new(start)
        .hidden(false)
        .git_ignore(gitignore)
        .git_exclude(gitignore)
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
            Box::new(move |entry| {
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                let path = entry.path();
                let Some(rel) = relative(root, path) else {
                    return WalkState::Continue;
                };
                if let Some(lang) = Lang::classify(path) {
                    found.lock().unwrap().push(SourceFile {
                        rel,
                        abs: path.to_path_buf(),
                        lang,
                    });
                } else {
                    match non_code(&entry.file_name().to_string_lossy()) {
                        NonCode::Unclassified => unclassified.lock().unwrap().push(rel),
                        NonCode::Indexed => documents.lock().unwrap().push(rel),
                        NonCode::Dropped => {}
                    }
                }
                WalkState::Continue
            })
        });
}

/// Files that may hold secrets are never read (§14.5): `.env`, `.env.*`, `*.pem`.
pub fn is_sensitive(name: &str) -> bool {
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
        let found = discover_all(root, 2);
        assert_eq!(found.documents, ["README.txt"]);
        let rels: Vec<_> = found.files.into_iter().map(|f| f.rel).collect();
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

    /// `docs/graphify-inputs.md` §5: `--no-gitignore` drops `.gitignore` (at any depth) and
    /// `.git/info/exclude`; `.graphifyignore` still applies and its negations win over git's.
    #[test]
    fn gitignore_toggle_keeps_graphifyignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for p in [
            "gen/g.py",
            "ex/e.py",
            "keep/k.py",
            "deep/sub/d.py",
            "top.py",
        ] {
            let path = root.join(p);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x = 1\n").unwrap();
        }
        fs::create_dir_all(root.join(".git/info")).unwrap();
        fs::write(root.join(".git/info/exclude"), "ex/\n").unwrap();
        fs::write(root.join(".gitignore"), "gen/\n").unwrap();
        fs::write(root.join("deep/.gitignore"), "sub/\n").unwrap();
        fs::write(root.join(".graphifyignore"), "keep/\n").unwrap();
        let rels = |gitignore| -> Vec<String> {
            discover_all_with(root, 2, gitignore)
                .files
                .into_iter()
                .map(|f| f.rel)
                .collect()
        };
        assert_eq!(rels(true), ["top.py"]);
        assert_eq!(
            rels(false),
            ["deep/sub/d.py", "ex/e.py", "gen/g.py", "top.py"]
        );
        fs::write(root.join(".graphifyignore"), "keep/\n!gen/\n").unwrap();
        assert_eq!(rels(true), ["gen/g.py", "top.py"]);
    }

    #[test]
    fn walks_saved_results_inside_the_output_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for p in [
            "a.py",
            "dijkstral-out/graph.json",
            "dijkstral-out/reflections/LESSONS.md",
            "dijkstral-out/memory/query_x.md",
            "dijkstral-out/memory/sub/nested.md",
            "dijkstral-out/memory/helper.py",
            "dijkstral-out/memory/skip.md",
            "graphify-out/memory/theirs.md",
        ] {
            let path = root.join(p);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        // The project ignores the output directory; the memory walk still happens, and an ignore
        // file inside the memory directory applies there.
        fs::write(root.join(".gitignore"), "dijkstral-out/\n").unwrap();
        fs::write(root.join("dijkstral-out/memory/.gitignore"), "skip.md\n").unwrap();
        let found = discover_all(root, 2);
        assert_eq!(
            found.documents,
            [
                "dijkstral-out/memory/query_x.md",
                "dijkstral-out/memory/sub/nested.md"
            ]
        );
        let rels: Vec<_> = found.files.into_iter().map(|f| f.rel).collect();
        assert_eq!(rels, ["a.py", "dijkstral-out/memory/helper.py"]);
    }
}

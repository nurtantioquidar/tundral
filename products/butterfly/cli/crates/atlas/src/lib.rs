//! Atlas: Butterfly's native code graph. Deterministic tree-sitter extraction
//! over the workspace — symbol definitions, name references, imports — with
//! blake3 content-hash incremental refresh. No LLM, no network, no external
//! binaries. Everything Atlas emits flows through tool results (appended
//! messages), never the cached prompt prefix.

mod extract;
mod rank;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use extract::{DefKind, Definition, Extraction, Lang, Reference};

/// Files larger than this are skipped (generated bundles, vendored blobs).
const MAX_FILE_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone)]
pub struct FileIndex {
    pub language: Lang,
    hash: blake3::Hash,
    stamp: Option<(SystemTime, u64)>,
    pub extraction: Extraction,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RefreshStats {
    pub total_files: usize,
    pub parsed: usize,
    pub removed: usize,
}

/// The in-memory index. Built lazily on first use; `refresh()` re-parses only
/// files whose content hash changed.
pub struct Atlas {
    root: PathBuf,
    files: HashMap<PathBuf, FileIndex>,
}

impl Atlas {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: HashMap::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn files(&self) -> impl Iterator<Item = (&PathBuf, &FileIndex)> {
        self.files.iter()
    }

    /// Drop a path from the index (called when the agent writes it); the next
    /// refresh re-parses it.
    pub fn invalidate(&mut self, relative: &Path) {
        self.files.remove(relative);
    }

    /// Walk the workspace (`.gitignore`-aware), re-parse changed files, drop
    /// deleted ones.
    pub fn refresh(&mut self) -> RefreshStats {
        let mut stats = RefreshStats::default();
        let mut seen: Vec<PathBuf> = Vec::new();
        let walker = ignore::WalkBuilder::new(&self.root)
            .hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .require_git(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let absolute = entry.path();
            let Some(language) = Lang::from_path(absolute) else {
                continue;
            };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            let relative = absolute
                .strip_prefix(&self.root)
                .unwrap_or(absolute)
                .to_path_buf();
            seen.push(relative.clone());
            let stamp = metadata.modified().ok().map(|m| (m, metadata.len()));
            // Fast path: unchanged mtime+size means unchanged content.
            if let Some(existing) = self.files.get(&relative) {
                if existing.stamp.is_some() && existing.stamp == stamp {
                    continue;
                }
            }
            let Ok(source) = std::fs::read_to_string(absolute) else {
                continue;
            };
            let hash = blake3::hash(source.as_bytes());
            if let Some(existing) = self.files.get_mut(&relative) {
                if existing.hash == hash {
                    existing.stamp = stamp;
                    continue;
                }
            }
            let extraction = extract::extract(language, &source);
            stats.parsed += 1;
            self.files.insert(
                relative,
                FileIndex {
                    language,
                    hash,
                    stamp,
                    extraction,
                },
            );
        }
        let seen_set: std::collections::HashSet<&PathBuf> = seen.iter().collect();
        let before = self.files.len();
        self.files.retain(|path, _| seen_set.contains(path));
        stats.removed = before - self.files.len();
        stats.total_files = self.files.len();
        stats
    }

    /// All definitions of `name`, exact match, sorted by path for determinism.
    pub fn definitions(&self, name: &str) -> Vec<(&Path, &Definition)> {
        let mut hits: Vec<(&Path, &Definition)> = self
            .files
            .iter()
            .flat_map(|(path, index)| {
                index
                    .extraction
                    .definitions
                    .iter()
                    .filter(|def| def.name == name)
                    .map(move |def| (path.as_path(), def))
            })
            .collect();
        hits.sort_by(|a, b| a.0.cmp(b.0).then(a.1.line.cmp(&b.1.line)));
        hits
    }

    /// All name-based references to `name` (call sites), sorted by path.
    /// Heuristic: same-name symbols from other scopes may be included.
    pub fn references(&self, name: &str) -> Vec<(&Path, usize)> {
        let mut hits: Vec<(&Path, usize)> = self
            .files
            .iter()
            .flat_map(|(path, index)| {
                index
                    .extraction
                    .references
                    .iter()
                    .filter(|reference| reference.name == name)
                    .map(move |reference| (path.as_path(), reference.line))
            })
            .collect();
        hits.sort();
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("web")).unwrap();
        std::fs::write(
            dir.path().join("web/app.ts"),
            "export function render() { return alpha(); }\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn refresh_is_incremental_by_content() {
        let dir = workspace();
        let mut atlas = Atlas::new(dir.path().to_path_buf());
        let first = atlas.refresh();
        assert_eq!(first.parsed, 2);
        assert_eq!(first.total_files, 2);
        // Nothing changed: zero parses.
        let second = atlas.refresh();
        assert_eq!(second.parsed, 0);
        // Touch without content change (rewrite same bytes, new mtime): the
        // hash check prevents a re-parse from being counted... rewrite with
        // NEW content and expect exactly one parse.
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn alpha() {}\npub fn gamma() { alpha(); }\n",
        )
        .unwrap();
        let third = atlas.refresh();
        assert_eq!(third.parsed, 1);
        assert_eq!(atlas.definitions("gamma").len(), 1);
        assert!(atlas.definitions("beta").is_empty());
        // Deleted files leave the index.
        std::fs::remove_file(dir.path().join("web/app.ts")).unwrap();
        let fourth = atlas.refresh();
        assert_eq!(fourth.removed, 1);
        assert_eq!(fourth.total_files, 1);
    }

    #[test]
    fn cross_language_lookups_are_deterministic() {
        let dir = workspace();
        let mut atlas = Atlas::new(dir.path().to_path_buf());
        atlas.refresh();
        let defs = atlas.definitions("alpha");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].0, Path::new("lib.rs"));
        let refs = atlas.references("alpha");
        assert_eq!(refs.len(), 2, "rust call + ts call: {refs:?}");
        assert_eq!(refs[0].0, Path::new("lib.rs"));
        assert_eq!(refs[1].0, Path::new("web/app.ts"));
    }

    #[test]
    fn invalidate_forces_reparse() {
        let dir = workspace();
        let mut atlas = Atlas::new(dir.path().to_path_buf());
        atlas.refresh();
        atlas.invalidate(Path::new("lib.rs"));
        let stats = atlas.refresh();
        assert_eq!(stats.parsed, 1);
    }
}

//! The per-file semantic cache (§9).
//!
//! Layout under `dijkstral-out/semantic/`:
//!
//! ```text
//! cache/<normal|deep>/<scope>/<key>.json   one entry per file
//! index.json                               rel path -> key, node count, size, mtime
//! ```
//!
//! `key` is Graphify's content key: `sha256(body + b"\0" + lower(rel path))`, where `body` is the
//! file's bytes with markdown front matter removed (so editing only the front matter is a hit).
//! `scope` is the first 12 hex digits of `sha256(backend \0 model \0 PROMPT_VERSION \0 mode)`.
//! **Deliberate difference:** Graphify's cache directory changes with the prompt only, so a
//! different model or backend reuses replies another model wrote; dijkstral's scope includes
//! the backend and the model, so changing either re-asks (a reply is only as good as the model
//! that wrote it), and switching back reuses the earlier entries.
//!
//! An entry is `{"nodes", "edges", "hyperedges"}` with the reply items attributed to that file,
//! plus `"partial": true` for files of a hollow or truncated chunk (a miss next run). A file the
//! model left out of a non-empty reply gets an empty partial entry with an `"omitted"` count; the
//! second omission is cached as complete, so a file with nothing to say does not cost a model call
//! on every run (Graphify retries it forever).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::types::{DocKind, SourceDoc};

use super::prompt::PROMPT_VERSION;
use super::Mode;

/// `dijkstral-out/semantic`.
pub fn semantic_dir(out_dir: &Path) -> PathBuf {
    out_dir.join("semantic")
}

/// Omissions after which an empty entry counts as complete.
const OMIT_LIMIT: u64 = 2;

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Graphify's per-file content key (§9.1).
pub fn content_key(doc: &SourceDoc, root: Option<&Path>) -> String {
    let rel_lower = doc.rel.to_lowercase();
    let office = rel_lower.ends_with(".docx") || rel_lower.ends_with(".xlsx");
    let on_disk = if office {
        None
    } else {
        root.and_then(|r| std::fs::read(r.join(&doc.rel)).ok())
    };
    let bytes: Vec<u8> = match on_disk {
        Some(b) => b,
        None => match (&doc.kind, &doc.image) {
            (DocKind::RasterImage, Some((_, b))) => b.clone(),
            _ => doc.text.as_bytes().to_vec(),
        },
    };
    let markdown = [".md", ".mdx", ".markdown"]
        .iter()
        .any(|e| rel_lower.ends_with(e));
    let body = if markdown && !office {
        strip_front_matter(&bytes)
    } else {
        &bytes[..]
    };
    let mut h = Sha256::new();
    h.update(body);
    h.update(b"\0");
    h.update(rel_lower.as_bytes());
    hex(&h.finalize())
}

/// The bytes after a front matter block (line 1 exactly `---`, closed by a later line exactly
/// `---`), starting at the newline after the closing line; the input unchanged otherwise.
pub fn strip_front_matter(bytes: &[u8]) -> &[u8] {
    let mut lines = bytes.split_inclusive(|b| *b == b'\n');
    let is_dashes = |l: &[u8]| {
        let l = l.strip_suffix(b"\n").unwrap_or(l);
        let l = l.strip_suffix(b"\r").unwrap_or(l);
        l == b"---"
    };
    let Some(first) = lines.next() else {
        return bytes;
    };
    if !is_dashes(first) {
        return bytes;
    }
    let mut pos = first.len();
    for line in lines {
        if is_dashes(line) {
            let no_nl = line.strip_suffix(b"\n").unwrap_or(line);
            return &bytes[pos + no_nl.len()..];
        }
        pos += line.len();
    }
    bytes
}

/// One backend/model/prompt/mode's cache directory.
#[derive(Debug, Clone)]
pub struct Scope {
    pub dir: PathBuf,
}

impl Scope {
    pub fn new(out_dir: &Path, backend: &str, model: &str, mode: Mode) -> Scope {
        let id =
            sha256_hex(format!("{backend}\0{model}\0{PROMPT_VERSION}\0{}", mode.name()).as_bytes());
        Scope {
            dir: semantic_dir(out_dir)
                .join("cache")
                .join(mode.name())
                .join(&id[..12]),
        }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    pub fn read(&self, key: &str) -> Option<Entry> {
        let bytes = std::fs::read(self.path(key)).ok()?;
        match serde_json::from_slice::<Value>(&bytes).ok()? {
            Value::Object(map) => Some(Entry { map }),
            _ => None,
        }
    }

    pub fn write(&self, key: &str, entry: &Entry) -> Result<()> {
        let bytes = serde_json::to_vec(&Value::Object(entry.map.clone()))?;
        write_atomic(&self.path(key), &bytes)
    }
}

/// A cache entry: `nodes`, `edges`, `hyperedges` and the optional `partial` / `omitted` marks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Entry {
    pub map: Map<String, Value>,
}

impl Entry {
    pub fn new(nodes: Vec<Value>, edges: Vec<Value>, hyperedges: Vec<Value>) -> Entry {
        let mut map = Map::new();
        map.insert("nodes".into(), Value::Array(nodes));
        map.insert("edges".into(), Value::Array(edges));
        map.insert("hyperedges".into(), Value::Array(hyperedges));
        Entry { map }
    }

    pub fn mark_partial(&mut self) {
        self.map.insert("partial".into(), Value::Bool(true));
    }

    pub fn omitted(&self) -> u64 {
        self.map.get("omitted").and_then(Value::as_u64).unwrap_or(0)
    }

    /// An empty entry for a file left out of a reply, after `previous` earlier omissions.
    pub fn omission(previous: u64) -> Entry {
        let mut e = Entry::new(Vec::new(), Vec::new(), Vec::new());
        let n = previous + 1;
        e.map.insert("omitted".into(), Value::from(n));
        if n < OMIT_LIMIT {
            e.mark_partial();
        }
        e
    }

    pub fn is_partial(&self) -> bool {
        self.map.get("partial").and_then(Value::as_bool) == Some(true)
    }

    pub fn node_count(&self) -> usize {
        self.map
            .get("nodes")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    }

    /// `nodes`, `edges` and `hyperedges` only.
    pub fn reply_map(&self) -> Map<String, Value> {
        let mut m = Map::new();
        for k in ["nodes", "edges", "hyperedges"] {
            m.insert(
                k.into(),
                self.map
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new())),
            );
        }
        m
    }
}

/// `index.json`: what the last accepted extraction recorded per file, for the shrink guard and
/// `check-update`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub files: BTreeMap<String, IndexEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub key: String,
    pub nodes: usize,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub mtime_ns: Option<u128>,
}

impl Index {
    fn path(out_dir: &Path) -> PathBuf {
        semantic_dir(out_dir).join("index.json")
    }

    pub fn load(out_dir: &Path) -> Index {
        std::fs::read(Self::path(out_dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, out_dir: &Path) -> Result<()> {
        write_atomic(&Self::path(out_dir), &serde_json::to_vec_pretty(self)?)
    }

    /// Records `rel` as accepted with `nodes` nodes, stamping its size and mtime from `root`.
    pub fn record(&mut self, rel: &str, key: &str, nodes: usize, root: Option<&Path>) {
        let (size, mtime_ns) = root.map_or((None, None), |r| stat(&r.join(rel)));
        self.files.insert(
            rel.to_string(),
            IndexEntry {
                key: key.to_string(),
                nodes,
                size,
                mtime_ns,
            },
        );
    }
}

fn stat(path: &Path) -> (Option<u64>, Option<u128>) {
    match std::fs::metadata(path) {
        Ok(m) => (
            Some(m.len()),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos()),
        ),
        Err(_) => (None, None),
    }
}

/// `dijkstral-out/needs_update`: written when non-code files change outside an extraction
/// (`watch`, `update`), cleared by a complete extraction.
pub fn needs_update_path(out_dir: &Path) -> PathBuf {
    out_dir.join("needs_update")
}

pub fn mark_needs_update(out_dir: &Path) -> Result<()> {
    write_atomic(&needs_update_path(out_dir), b"1")
}

pub fn clear_needs_update(out_dir: &Path) {
    let _ = std::fs::remove_file(needs_update_path(out_dir));
}

/// Why `root` has non-code changes the semantic pass has not seen, if it has: the
/// `needs_update` flag, or a file the index recorded that changed size or mtime or disappeared.
pub fn pending_changes(root: &Path, out_dir: &Path) -> Option<String> {
    if needs_update_path(out_dir).exists() {
        return Some("needs_update is set".into());
    }
    let index = Index::load(out_dir);
    for (rel, e) in &index.files {
        if e.size.is_none() {
            continue;
        }
        let now = stat(&root.join(rel));
        if now.0.is_none() {
            return Some(format!("{rel} was removed"));
        }
        if now != (e.size, e.mtime_ns) {
            return Some(format!("{rel} changed"));
        }
    }
    None
}

/// Writes `bytes` to `path` through a temporary file and a rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("path has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("x"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantic")
    }

    fn doc(rel: &str, kind: DocKind, text: &str) -> SourceDoc {
        SourceDoc {
            rel: rel.into(),
            kind,
            text: text.into(),
            image: None,
            sha256: String::new(),
            tokens: 0,
            converted: None,
        }
    }

    #[test]
    fn keys_match_graphify() {
        let keys: Value = serde_json::from_slice(
            &std::fs::read(fixture().join("mixed.cache_keys.json")).unwrap(),
        )
        .unwrap();
        let root = fixture().join("mixed");
        for (rel, kind) in [
            ("README.md", DocKind::Document),
            ("docs/guide.md", DocKind::Document),
            ("docs/glossary.txt", DocKind::Document),
            ("config/queue.yaml", DocKind::Document),
            ("papers/write_through.pdf", DocKind::Paper),
            ("images/architecture.png", DocKind::RasterImage),
        ] {
            let want = keys["entries"][rel][rel.to_lowercase()].as_str().unwrap();
            // From disk, and from the text ingest sends (documents only).
            assert_eq!(content_key(&doc(rel, kind, ""), Some(&root)), want, "{rel}");
            if kind == DocKind::Document {
                let text = std::fs::read_to_string(root.join(rel)).unwrap();
                assert_eq!(content_key(&doc(rel, kind, &text), None), want, "{rel}");
            }
        }
    }

    #[test]
    fn front_matter_only_edits_keep_the_key() {
        let a = doc("a.md", DocKind::Document, "---\ntitle: A\n---\n# Body\n");
        let b = doc(
            "a.md",
            DocKind::Document,
            "---\ntitle: B\nx: y\n---\n# Body\n",
        );
        let c = doc("a.md", DocKind::Document, "---\ntitle: B\n---\n# Body!\n");
        assert_eq!(content_key(&a, None), content_key(&b, None));
        assert_ne!(content_key(&a, None), content_key(&c, None));
        assert_eq!(strip_front_matter(b"---\nunclosed\n"), b"---\nunclosed\n");
        assert_eq!(strip_front_matter(b"---\nk: v\n---"), b"");
    }

    #[test]
    fn scope_changes_with_backend_model_and_mode() {
        let out = Path::new("/o");
        let a = Scope::new(out, "codex", "m1", Mode::Normal).dir;
        assert!(a.starts_with("/o/semantic/cache/normal"));
        assert_ne!(a, Scope::new(out, "codex", "m2", Mode::Normal).dir);
        assert_ne!(a, Scope::new(out, "openai", "m1", Mode::Normal).dir);
        assert!(Scope::new(out, "codex", "m1", Mode::Deep)
            .dir
            .starts_with("/o/semantic/cache/deep"));
    }

    #[test]
    fn omissions_become_complete() {
        let e = Entry::omission(0);
        assert!(e.is_partial());
        let e = Entry::omission(e.omitted());
        assert!(!e.is_partial());
    }

    #[test]
    fn check_update_sees_flag_and_changes() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("dijkstral-out");
        std::fs::write(root.path().join("a.md"), "x").unwrap();
        assert_eq!(pending_changes(root.path(), &out), None);
        let mut index = Index::default();
        index.record("a.md", "k", 1, Some(root.path()));
        index.save(&out).unwrap();
        assert_eq!(pending_changes(root.path(), &out), None);
        mark_needs_update(&out).unwrap();
        assert!(pending_changes(root.path(), &out).is_some());
        clear_needs_update(&out);
        std::fs::write(root.path().join("a.md"), "longer").unwrap();
        assert_eq!(
            pending_changes(root.path(), &out).as_deref(),
            Some("a.md changed")
        );
    }
}

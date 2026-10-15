//! STAND-IN for `ingest` (track 1) until it lands: the pass in `pass.rs` calls only
//! [`collect`] and [`chunk`], with `ingest`'s contract types, so swapping in the real stage
//! changes those two call sites.
//!
//! What it does (a subset of §2 and §4.1–4.2 of `docs/graphify-semantic.md`):
//!
//! - walks the corpus with the code walk's ignore files, skipped directories and sensitive-file
//!   filter;
//! - sends `.md .mdx .rst .txt .html .yaml .yml` as text, `.svg` as text, `.png .jpg .jpeg .gif
//!   .webp` as image parts; skips PDFs and office files (no text extraction here) and lists them
//!   in [`Collected::skipped`];
//! - orders documents and SVGs by path, then raster images;
//! - estimates a text at `chars / 4 + 40` tokens (not cl100k) and an image at 1600;
//! - packs greedily up to the budget and splits a file over the budget into 20,000-character
//!   pieces.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use dijkstral_core::discover::{is_sensitive, SKIP_DIRS};
use ignore::{WalkBuilder, WalkState};
use sha2::{Digest, Sha256};

use crate::types::{Chunk, DocKind, Part, SourceDoc};

/// What the walk found.
#[derive(Debug, Default)]
pub struct Collected {
    pub docs: Vec<SourceDoc>,
    /// Files the real `ingest` would send but this stand-in cannot read (PDF, office).
    pub skipped: Vec<String>,
}

const TEXT: &[&str] = &["md", "mdx", "rst", "txt", "html", "yaml", "yml"];
const IMAGES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
];
const UNREAD: &[&str] = &["pdf", "docx", "xlsx"];
const IMAGE_TOKENS: usize = 1600;
const PIECE_CHARS: usize = 20_000;

pub fn collect(root: &Path) -> Result<Collected> {
    let found: Mutex<Vec<(String, std::path::PathBuf)>> = Mutex::new(Vec::new());
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
        .build_parallel()
        .run(|| {
            let found = &found;
            Box::new(move |entry| {
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    if let Ok(rel) = entry.path().strip_prefix(root) {
                        let rel: Vec<String> = rel
                            .components()
                            .map(|c| c.as_os_str().to_string_lossy().into_owned())
                            .collect();
                        found
                            .lock()
                            .unwrap()
                            .push((rel.join("/"), entry.path().to_path_buf()));
                    }
                }
                WalkState::Continue
            })
        });
    let mut found = found.into_inner().unwrap();
    found.sort();
    let mut out = Collected::default();
    let mut images = Vec::new();
    for (rel, abs) in found {
        let ext = rel
            .rsplit('/')
            .next()
            .and_then(|n| {
                n.rfind('.')
                    .filter(|&i| i > 0)
                    .map(|i| n[i + 1..].to_string())
            })
            .unwrap_or_default()
            .to_ascii_lowercase();
        if UNREAD.contains(&ext.as_str()) {
            out.skipped.push(rel);
            continue;
        }
        let image = IMAGES.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m);
        if !TEXT.contains(&ext.as_str()) && ext != "svg" && image.is_none() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&abs) else {
            continue;
        };
        let sha256 = hex(&Sha256::digest(&bytes));
        match image {
            Some(mime) => images.push(SourceDoc {
                rel,
                kind: DocKind::RasterImage,
                text: String::new(),
                image: Some((mime.to_string(), bytes)),
                sha256,
                tokens: IMAGE_TOKENS,
            }),
            None => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let tokens = text.chars().count() / 4 + 40;
                out.docs.push(SourceDoc {
                    rel,
                    kind: if ext == "svg" {
                        DocKind::VectorImage
                    } else {
                        DocKind::Document
                    },
                    text,
                    image: None,
                    sha256,
                    tokens,
                });
            }
        }
    }
    out.docs.extend(images);
    Ok(out)
}

/// Greedy packing of `docs[i]` for `i` in `indices`, in that order (§4.2).
pub fn chunk(docs: &[SourceDoc], indices: &[usize], budget: usize) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut cur: Vec<Part> = Vec::new();
    let mut used = 0;
    let push = |parts: Vec<Part>, chunks: &mut Vec<Chunk>| {
        if !parts.is_empty() {
            chunks.push(Chunk {
                index: chunks.len(),
                parts,
            });
        }
    };
    for &i in indices {
        let d = &docs[i];
        if d.tokens > budget && d.kind != DocKind::RasterImage {
            push(std::mem::take(&mut cur), &mut chunks);
            used = 0;
            let n = d.text.chars().count();
            let mut start = 0;
            while start < n {
                let end = (start + PIECE_CHARS).min(n);
                push(
                    vec![Part {
                        doc: i,
                        piece: Some((start, end)),
                    }],
                    &mut chunks,
                );
                start = end;
            }
            continue;
        }
        if used + d.tokens > budget && !cur.is_empty() {
            push(std::mem::take(&mut cur), &mut chunks);
            used = 0;
        }
        cur.push(Part {
            doc: i,
            piece: None,
        });
        used += d.tokens;
    }
    push(cur, &mut chunks);
    chunks
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(tokens: usize) -> SourceDoc {
        SourceDoc {
            rel: "a.md".into(),
            kind: DocKind::Document,
            text: "x".repeat(25_000),
            image: None,
            sha256: String::new(),
            tokens,
        }
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn packs_greedily_and_splits_oversize_files() {
        let docs = vec![doc(100), doc(100), doc(100), doc(1000)];
        let chunks = chunk(&docs, &[0, 1, 2, 3], 250);
        let shape: Vec<Vec<(usize, Option<(usize, usize)>)>> = chunks
            .iter()
            .map(|c| c.parts.iter().map(|p| (p.doc, p.piece)).collect())
            .collect();
        assert_eq!(
            shape,
            vec![
                vec![(0, None), (1, None)],
                vec![(2, None)],
                vec![(3, Some((0, 20_000)))],
                vec![(3, Some((20_000, 25_000)))],
            ]
        );
    }

    #[test]
    fn collects_documents_then_images_and_skips_unread_kinds() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        for (p, body) in [
            ("b.md", "# B"),
            ("a/z.yaml", "k: v"),
            ("img.png", "png"),
            ("p.pdf", "%PDF"),
            ("code.py", "x = 1"),
            (".env", "SECRET=1"),
            ("node_modules/x.md", "no"),
        ] {
            let path = r.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let c = collect(r).unwrap();
        let rels: Vec<&str> = c.docs.iter().map(|d| d.rel.as_str()).collect();
        assert_eq!(rels, ["a/z.yaml", "b.md", "img.png"]);
        assert_eq!(c.skipped, ["p.pdf"]);
    }
}

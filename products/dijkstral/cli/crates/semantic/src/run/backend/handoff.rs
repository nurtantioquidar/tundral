//! The skill-driven hand-off: no model call. Each chunk's request is written to
//! `dijkstral-out/semantic/pending/<n>.json` (system prompt, user text, image paths, the files and
//! their cache keys); the host assistant (Claude Code or Codex through the `/dijkstral` skill)
//! answers each with a JSON reply in `dijkstral-out/semantic/replies/<n>.json`; and
//! `dijkstral semantic import` validates the replies exactly like a model's and writes them to the
//! cache. The next `extract --backend handoff` then finds every file cached, makes no request and
//! merges the replies.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::{Backend, Request, Response};

use super::super::cache::{semantic_dir, write_atomic, Entry, Scope};
use super::super::extract::split_by_file;
use super::super::repair::{first_object, is_extraction, is_hollow, schema_issues, SchemaIssues};

/// The model name the hand-off is cached under: whatever the host assistant runs.
pub const HOST_MODEL: &str = "host";

pub struct Handoff;

/// The error a hand-off returns instead of a reply; the runner writes the request out.
#[derive(Debug)]
pub struct Pending;

impl fmt::Display for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("request handed off to the host assistant")
    }
}

impl std::error::Error for Pending {}

impl Backend for Handoff {
    fn name(&self) -> &str {
        "handoff"
    }

    fn model(&self) -> &str {
        HOST_MODEL
    }

    fn complete(&self, _req: &Request) -> Result<Response> {
        Err(Pending.into())
    }
}

pub fn pending_dir(out_dir: &Path) -> PathBuf {
    semantic_dir(out_dir).join("pending")
}

pub fn replies_dir(out_dir: &Path) -> PathBuf {
    semantic_dir(out_dir).join("replies")
}

/// One request waiting for the host assistant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingRequest {
    pub version: u32,
    pub request: usize,
    pub instructions: String,
    /// Where the reply goes (absolute).
    pub reply_file: String,
    /// The cache scope, relative to `dijkstral-out`.
    pub scope: String,
    pub files: Vec<PendingFile>,
    pub system: String,
    pub user_text: String,
    pub images: Vec<PendingImage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingFile {
    pub rel: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingImage {
    pub rel: String,
    pub mime: String,
    /// An absolute path the assistant can open.
    pub path: String,
}

/// Writes request `n` (from 1) for `files` (`(rel, cache key)`).
pub(crate) fn write_pending(
    out_dir: &Path,
    root: Option<&Path>,
    scope: &Scope,
    n: usize,
    files: &[(String, String)],
    req: &Request,
) -> Result<PathBuf> {
    let dir = pending_dir(out_dir);
    std::fs::create_dir_all(&dir)?;
    let reply = absolute(&replies_dir(out_dir).join(format!("{n}.json")));
    // A reply left from an earlier hand-off answers a different request.
    let _ = std::fs::remove_file(&reply);
    let mut images = Vec::new();
    for (i, (mime, bytes, rel)) in req.images.iter().enumerate() {
        let on_disk = root.map(|r| r.join(rel)).filter(|p| p.is_file());
        let path = match on_disk {
            Some(p) => p,
            None => {
                let ext = mime.rsplit('/').next().unwrap_or("png");
                let p = dir.join(format!("{n}-image{i}.{ext}"));
                std::fs::write(&p, bytes)?;
                p
            }
        };
        images.push(PendingImage {
            rel: rel.clone(),
            mime: mime.clone(),
            path: absolute(&path).display().to_string(),
        });
    }
    let p = PendingRequest {
        version: 1,
        request: n,
        instructions: format!(
            "Answer as the model: follow `system`, read `user_text` and look at every image in \
             `images` (in order), then write only the JSON reply object to {}. When every \
             pending request has a reply, run `dijkstral semantic import`.",
            reply.display()
        ),
        reply_file: reply.display().to_string(),
        scope: scope
            .dir
            .strip_prefix(out_dir)
            .unwrap_or(&scope.dir)
            .display()
            .to_string(),
        files: files
            .iter()
            .map(|(rel, key)| PendingFile {
                rel: rel.clone(),
                key: key.clone(),
            })
            .collect(),
        system: req.system.clone(),
        user_text: req.user_text.clone(),
        images,
    };
    let path = dir.join(format!("{n}.json"));
    write_atomic(&path, &serde_json::to_vec_pretty(&p)?)?;
    Ok(path)
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The pending requests, by number.
pub fn list_pending(out_dir: &Path) -> Result<Vec<PendingRequest>> {
    let dir = pending_dir(out_dir);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    for e in entries.filter_map(|e| e.ok()) {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let p: PendingRequest = serde_json::from_slice(&std::fs::read(&path)?)
            .with_context(|| format!("parsing {}", path.display()))?;
        out.push(p);
    }
    out.sort_by_key(|p| p.request);
    Ok(out)
}

/// What `import` did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportReport {
    /// Requests whose reply was accepted and cached.
    pub imported: Vec<usize>,
    /// Requests with no reply file yet.
    pub missing: Vec<usize>,
    /// Requests whose reply was unusable, with the reason; they stay pending.
    pub rejected: Vec<(usize, String)>,
    /// Files cached.
    pub files: usize,
    /// Files the replies left out (cached as omitted, asked again next run).
    pub omitted: Vec<String>,
    pub schema: SchemaIssues,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "imported {} repl{} ({} file(s)); {} missing; {} rejected",
            self.imported.len(),
            if self.imported.len() == 1 { "y" } else { "ies" },
            self.files,
            self.missing.len(),
            self.rejected.len()
        );
        for (n, why) in &self.rejected {
            s.push_str(&format!("\n  request {n}: {why}"));
        }
        if let Some(w) = self.schema.summary() {
            s.push('\n');
            s.push_str(&w);
        }
        s
    }
}

/// Validates the replies in `replies` (`<n>.json` per pending request `n`) and caches the usable
/// ones; their pending requests are removed.
pub fn import(out_dir: &Path, replies: &Path) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    for p in list_pending(out_dir)? {
        let reply_path = replies.join(format!("{}.json", p.request));
        let Ok(text) = std::fs::read_to_string(&reply_path) else {
            report.missing.push(p.request);
            continue;
        };
        let map = match first_object(&text, is_extraction) {
            Ok(m) => m,
            Err(e) => {
                report.rejected.push((p.request, e.to_string()));
                continue;
            }
        };
        if is_hollow(&map) {
            report
                .rejected
                .push((p.request, "the reply has no nodes and no edges".into()));
            continue;
        }
        let rels: Vec<String> = p.files.iter().map(|f| f.rel.clone()).collect();
        let (entries, omitted) = split_by_file(&map, &rels);
        if entries.is_empty() {
            report.rejected.push((
                p.request,
                format!(
                    "no node has a source_file of this request ({})",
                    rels.join(", ")
                ),
            ));
            continue;
        }
        report.schema.add(&schema_issues(&map));
        let scope = Scope {
            dir: out_dir.join(&p.scope),
        };
        for f in &p.files {
            let entry = match entries.get(&f.rel) {
                Some(e) => e.clone(),
                None => Entry::omission(scope.read(&f.key).map_or(0, |e| e.omitted())),
            };
            scope.write(&f.key, &entry)?;
            report.files += 1;
        }
        report.omitted.extend(omitted);
        std::fs::remove_file(pending_dir(out_dir).join(format!("{}.json", p.request)))?;
        report.imported.push(p.request);
    }
    Ok(report)
}

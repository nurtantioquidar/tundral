//! The extraction runner: cache lookup, requests with retries and bisection, per-file cache
//! writes, the shrink guard and usage (§4.4, §6, §9).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Map, Value};

use crate::types::{Backend, Chunk, Part, Reply, SourceDoc};

use super::backend::handoff::{self, Pending};
use super::cache::{self, content_key, Entry, Index, Scope};
use super::prompt::extraction_request;
use super::repair::{
    first_object, is_extraction, is_hollow, items, salvage, schema_issues, to_reply, SchemaIssues,
};
use super::usage::{UsageLog, UsageReport};
use super::Mode;

/// How `extract` runs.
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    /// `dijkstral-out` (cache, usage, pending requests).
    pub out_dir: PathBuf,
    /// The corpus root: cache keys from the files' bytes, `index.json` stats, image paths for the
    /// hand-off. `None` keys on the text `ingest` produced.
    pub root: Option<PathBuf>,
    pub mode: Mode,
    /// `--max-concurrency`: requests in flight (default 4; 1 sends them strictly in order).
    pub max_concurrency: usize,
    /// `--force`: ignore cache entries and ask again.
    pub force: bool,
    /// `--allow-partial`: accept files whose node count shrank.
    pub allow_partial: bool,
    /// Waits before the 2nd, 3rd, … attempt of a hollow, invalid or failed request (2 s, 8 s:
    /// three attempts in all).
    pub retry_delays: Vec<Duration>,
    /// Print one line per request to stderr.
    pub progress: bool,
}

impl ExtractOptions {
    pub fn new(out_dir: impl Into<PathBuf>) -> ExtractOptions {
        ExtractOptions {
            out_dir: out_dir.into(),
            root: None,
            mode: Mode::Normal,
            max_concurrency: 4,
            force: false,
            allow_partial: false,
            retry_delays: vec![Duration::from_secs(2), Duration::from_secs(8)],
            progress: false,
        }
    }
}

/// One reply to merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    /// Files the request covered (cache hits: the one file).
    pub files: Vec<String>,
    /// Read back from the cache (Graphify marks these `_origin: "semantic"`, §9.4).
    pub cached: bool,
    /// From a truncated request (kept, not cached as complete).
    pub partial: bool,
    pub reply: Reply,
    /// The reply object as received (fresh) or stored (cached), for raw output and pass-through
    /// keys.
    pub raw: Map<String, Value>,
}

/// What happened to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// Served from the cache, no request.
    Cached,
    /// Answered and cached.
    Complete,
    /// Hollow after every attempt, or truncated: kept if anything parsed, asked again next run.
    Partial,
    /// The reply left it out: asked again next run (once).
    Omitted,
    /// The backend failed on every attempt: not cached.
    Failed,
    /// Handed off to the host assistant.
    Pending,
}

/// A file whose new node count is below the accepted one (§6 shrink guard).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shrink {
    pub rel: String,
    pub before: usize,
    pub after: usize,
}

#[derive(Debug, Clone)]
pub struct ExtractOutcome {
    /// Cache hits in dispatch order, then fresh replies in request order.
    pub extracted: Vec<Extracted>,
    /// Every file of the run with its state, in dispatch order.
    pub files: Vec<(String, FileState)>,
    /// Requests written for the host assistant (`--backend handoff`).
    pub pending: Vec<PathBuf>,
    /// Files refused by the shrink guard (empty with `allow_partial`).
    pub shrunk: Vec<Shrink>,
    /// Log lines: invalid replies, retries, bisections, omissions, the schema summary.
    pub warnings: Vec<String>,
    pub schema: SchemaIssues,
    pub usage: UsageReport,
}

impl ExtractOutcome {
    pub fn count(&self, state: FileState) -> usize {
        self.files.iter().filter(|(_, s)| *s == state).count()
    }

    /// All replies (`extracted[..].reply`).
    pub fn replies(&self) -> Vec<Reply> {
        self.extracted.iter().map(|e| e.reply.clone()).collect()
    }

    /// The shrink guard's refusal, unless `--allow-partial`.
    pub fn shrink_error(&self) -> Option<String> {
        if self.shrunk.is_empty() {
            return None;
        }
        let list: Vec<String> = self
            .shrunk
            .iter()
            .map(|s| format!("'{}' ({} -> {} nodes)", s.rel, s.before, s.after))
            .collect();
        Some(format!(
            "unverified semantic shrink detected for {}; refusing to replace the earlier \
             extraction (pass --allow-partial to accept it)",
            list.join(", ")
        ))
    }

    /// `semantic cache: H hit / M miss` and the per-state counts that are not zero.
    pub fn summary(&self) -> String {
        let hits = self.count(FileState::Cached);
        let mut s = format!(
            "semantic cache: {hits} hit / {} miss",
            self.files.len() - hits
        );
        for (state, name) in [
            (FileState::Partial, "partial"),
            (FileState::Omitted, "omitted"),
            (FileState::Failed, "failed"),
            (FileState::Pending, "handed off"),
        ] {
            let n = self.count(state);
            if n > 0 {
                s.push_str(&format!(", {n} {name}"));
            }
        }
        s
    }
}

/// The documents of `docs` that need the model (no complete cache entry, or `force`), in index
/// order: chunk these to pack requests tightly.
pub fn cache_misses(
    docs: &[SourceDoc],
    backend: &dyn Backend,
    opts: &ExtractOptions,
) -> Vec<usize> {
    let scope = Scope::new(&opts.out_dir, backend.name(), backend.model(), opts.mode);
    (0..docs.len())
        .filter(|&i| {
            opts.force
                || scope
                    .read(&content_key(&docs[i], opts.root.as_deref()))
                    .is_none_or(|e| e.is_partial())
        })
        .collect()
}

#[derive(Debug, Clone)]
enum Status {
    Complete,
    Truncated,
    Hollow,
    Failed,
    Pending(PathBuf),
}

#[derive(Debug, Clone)]
struct Piece {
    parts: Vec<Part>,
    status: Status,
    map: Option<Map<String, Value>>,
}

/// Per-file accumulation while its requests finish.
#[derive(Default)]
struct FileAcc {
    remaining: usize,
    nodes: Vec<Value>,
    edges: Vec<Value>,
    hyperedges: Vec<Value>,
    answered: bool,
    omitted: bool,
    partial: bool,
    failed: bool,
    pending: bool,
}

struct Run<'a> {
    docs: &'a [SourceDoc],
    backend: &'a dyn Backend,
    opts: &'a ExtractOptions,
    scope: Scope,
    keys: HashMap<usize, String>,
    usage: UsageLog,
    warnings: Mutex<Vec<(usize, usize, String)>>,
    jobs_total: usize,
}

impl Run<'_> {
    fn warn(&self, job: usize, msg: String) {
        let mut w = self.warnings.lock().expect("warnings lock");
        let seq = w.len();
        if self.opts.progress {
            eprintln!("[dijkstral] {msg}");
        }
        w.push((job, seq, msg));
    }

    fn files_of(&self, parts: &[Part]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in parts {
            let rel = self.docs[p.doc].dispatched();
            if !out.iter().any(|f| f == rel) {
                out.push(rel.to_string());
            }
        }
        out
    }

    /// Runs request `job` over `parts`, bisecting on truncation.
    fn request(
        &self,
        job: usize,
        parts: &[Part],
        depth: usize,
        attempt_no: &mut usize,
    ) -> Vec<Piece> {
        let files = self.files_of(parts);
        let req = extraction_request(self.docs, parts, self.opts.mode);
        let attempts = self.opts.retry_delays.len() + 1;
        let mut failed = false;
        for attempt in 1..=attempts {
            *attempt_no += 1;
            let started = Instant::now();
            let result = self.backend.complete(&req);
            let wall = started.elapsed();
            let resp = match result {
                Ok(r) => r,
                Err(e) if e.is::<Pending>() => {
                    self.usage.add(
                        "extract",
                        job + 1,
                        *attempt_no,
                        files.len(),
                        (None, None),
                        wall,
                        "pending",
                    );
                    let file_keys: Vec<(String, String)> = parts
                        .iter()
                        .map(|p| {
                            (
                                self.docs[p.doc].dispatched().to_string(),
                                self.keys[&p.doc].clone(),
                            )
                        })
                        .fold(Vec::new(), |mut v, fk| {
                            if !v.contains(&fk) {
                                v.push(fk);
                            }
                            v
                        });
                    let status = match handoff::write_pending(
                        &self.opts.out_dir,
                        self.opts.root.as_deref(),
                        &self.scope,
                        job + 1,
                        &file_keys,
                        &req,
                    ) {
                        Ok(path) => Status::Pending(path),
                        Err(e) => {
                            self.warn(job, format!("writing pending request {}: {e:#}", job + 1));
                            Status::Failed
                        }
                    };
                    return vec![Piece {
                        parts: parts.to_vec(),
                        status,
                        map: None,
                    }];
                }
                Err(e) => {
                    self.usage.add(
                        "extract",
                        job + 1,
                        *attempt_no,
                        files.len(),
                        (None, None),
                        wall,
                        "error",
                    );
                    self.warn(job, format!("request {} failed: {e:#}", job + 1));
                    failed = true;
                    self.pause(attempt, attempts);
                    continue;
                }
            };
            let tokens = (resp.input_tokens, resp.output_tokens);
            if resp.truncated {
                self.usage.add(
                    "extract",
                    job + 1,
                    *attempt_no,
                    files.len(),
                    tokens,
                    wall,
                    "truncated",
                );
                if parts.len() > 1 {
                    let mid = parts.len() / 2;
                    self.warn(
                        job,
                        format!(
                            "chunk of {} truncated at depth {depth}, splitting into halves of {} \
                             and {}",
                            parts.len(),
                            mid,
                            parts.len() - mid
                        ),
                    );
                    let mut out = self.request(job, &parts[..mid], depth + 1, attempt_no);
                    out.extend(self.request(job, &parts[mid..], depth + 1, attempt_no));
                    return out;
                }
                self.warn(
                    job,
                    format!(
                        "{} truncated: partial result kept (not cached as complete)",
                        files.join(", ")
                    ),
                );
                let map = first_object(&resp.text, is_extraction)
                    .ok()
                    .or_else(|| salvage(&resp.text));
                return vec![Piece {
                    parts: parts.to_vec(),
                    status: Status::Truncated,
                    map,
                }];
            }
            match first_object(&resp.text, is_extraction) {
                Ok(map) if !is_hollow(&map) => {
                    self.usage.add(
                        "extract",
                        job + 1,
                        *attempt_no,
                        files.len(),
                        tokens,
                        wall,
                        "ok",
                    );
                    if self.opts.progress {
                        eprintln!(
                            "[dijkstral] semantic request {}/{}: {} file(s) in {:.1} s",
                            job + 1,
                            self.jobs_total,
                            files.len(),
                            wall.as_secs_f64()
                        );
                    }
                    return vec![Piece {
                        parts: parts.to_vec(),
                        status: Status::Complete,
                        map: Some(map),
                    }];
                }
                Ok(_) => {
                    self.usage.add(
                        "extract",
                        job + 1,
                        *attempt_no,
                        files.len(),
                        tokens,
                        wall,
                        "hollow",
                    );
                    self.warn(
                        job,
                        format!(
                            "request {} returned no nodes/edges (attempt {attempt} of {attempts})",
                            job + 1
                        ),
                    );
                }
                Err(e) => {
                    self.usage.add(
                        "extract",
                        job + 1,
                        *attempt_no,
                        files.len(),
                        tokens,
                        wall,
                        "invalid",
                    );
                    self.warn(
                        job,
                        format!(
                            "LLM returned {e}, skipping chunk (attempt {attempt} of {attempts})"
                        ),
                    );
                }
            }
            failed = false;
            self.pause(attempt, attempts);
        }
        let status = match failed {
            true => Status::Failed,
            false => {
                self.warn(
                    job,
                    format!(
                        "chunk of {} still hollow after {attempts} attempt(s)",
                        files.len()
                    ),
                );
                Status::Hollow
            }
        };
        vec![Piece {
            parts: parts.to_vec(),
            status,
            map: None,
        }]
    }

    fn pause(&self, attempt: usize, attempts: usize) {
        if attempt < attempts {
            std::thread::sleep(self.opts.retry_delays[attempt - 1]);
        }
    }
}

/// Runs the semantic pass over `chunks` of `docs` with `backend`.
///
/// Files with a complete cache entry are not sent (their parts are removed from the chunks; a
/// chunk left empty is skipped), so a rerun with nothing changed makes no request. Each finished
/// file is cached as soon as its last request completes.
pub fn extract(
    chunks: &[Chunk],
    docs: &[SourceDoc],
    backend: &dyn Backend,
    opts: &ExtractOptions,
) -> Result<ExtractOutcome> {
    let scope = Scope::new(&opts.out_dir, backend.name(), backend.model(), opts.mode);
    let _ = std::fs::remove_dir_all(handoff::pending_dir(&opts.out_dir));
    let mut index = Index::load(&opts.out_dir);
    let root = opts.root.as_deref();

    // Dispatch order: first appearance in the chunks.
    let mut order: Vec<usize> = Vec::new();
    for c in chunks {
        for p in &c.parts {
            if !order.contains(&p.doc) {
                order.push(p.doc);
            }
        }
    }
    let keys: HashMap<usize, String> = order
        .iter()
        .map(|&d| (d, content_key(&docs[d], root)))
        .collect();

    let mut hits: HashMap<usize, Entry> = HashMap::new();
    let mut previous_omissions: HashMap<usize, u64> = HashMap::new();
    for &d in &order {
        if let Some(e) = scope.read(&keys[&d]) {
            if !opts.force && !e.is_partial() {
                hits.insert(d, e);
            } else {
                previous_omissions.insert(d, e.omitted());
            }
        }
    }
    let jobs: Vec<Vec<Part>> = chunks
        .iter()
        .map(|c| {
            c.parts
                .iter()
                .filter(|p| !hits.contains_key(&p.doc))
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter(|parts| !parts.is_empty())
        .collect();

    let mut accs: HashMap<usize, FileAcc> = HashMap::new();
    for parts in &jobs {
        let mut seen = Vec::new();
        for p in parts {
            if !seen.contains(&p.doc) {
                seen.push(p.doc);
                accs.entry(p.doc).or_default().remaining += 1;
            }
        }
    }

    let run = Run {
        docs,
        backend,
        opts,
        scope: scope.clone(),
        keys: keys.clone(),
        usage: UsageLog::new(),
        warnings: Mutex::new(Vec::new()),
        jobs_total: jobs.len(),
    };
    let results: Mutex<Vec<Option<Vec<Piece>>>> = Mutex::new(vec![None; jobs.len()]);
    let accs = Mutex::new(accs);
    let write_errors: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    let workers = opts.max_concurrency.max(1).min(jobs.len());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::SeqCst);
                if j >= jobs.len() {
                    break;
                }
                let mut attempt_no = 0;
                let pieces = run.request(j, &jobs[j], 0, &mut attempt_no);
                let mut accs = accs.lock().expect("accumulator lock");
                accumulate(&run, &pieces, &mut accs);
                for d in unique_docs(&jobs[j]) {
                    let acc = accs.get_mut(&d).expect("tracked file");
                    acc.remaining -= 1;
                    if acc.remaining == 0 {
                        let prev = previous_omissions.get(&d).copied().unwrap_or(0);
                        if let Err(e) = finalize(&run, d, acc, prev) {
                            write_errors.lock().expect("lock").push(format!("{e:#}"));
                        }
                    }
                }
                drop(accs);
                results.lock().expect("results lock")[j] = Some(pieces);
            });
        }
    });
    if let Some(e) = write_errors.into_inner().expect("lock").into_iter().next() {
        anyhow::bail!("semantic cache: {e}");
    }
    let accs = accs.into_inner().expect("accumulator lock");
    let results = results.into_inner().expect("results lock");

    let mut outcome = ExtractOutcome {
        extracted: Vec::new(),
        files: Vec::new(),
        pending: Vec::new(),
        shrunk: Vec::new(),
        warnings: Vec::new(),
        schema: SchemaIssues::default(),
        usage: UsageReport::default(),
    };
    for &d in &order {
        if let Some(e) = hits.get(&d) {
            let raw = e.reply_map();
            outcome.extracted.push(Extracted {
                files: vec![docs[d].dispatched().to_string()],
                cached: true,
                partial: false,
                reply: to_reply(&raw),
                raw,
            });
        }
    }
    for pieces in results.into_iter().flatten() {
        for piece in pieces {
            match &piece.status {
                Status::Pending(path) => outcome.pending.push(path.clone()),
                Status::Complete | Status::Truncated => {}
                Status::Hollow | Status::Failed => continue,
            }
            if let Some(map) = piece.map {
                outcome.schema.add(&schema_issues(&map));
                outcome.extracted.push(Extracted {
                    files: run.files_of(&piece.parts),
                    cached: false,
                    partial: matches!(piece.status, Status::Truncated),
                    reply: to_reply(&map),
                    raw: map,
                });
            }
        }
    }

    // File states, the shrink guard and the index.
    let mut all_complete = true;
    for &d in &order {
        let rel = docs[d].dispatched().to_string();
        let (state, nodes) = match (hits.get(&d), accs.get(&d)) {
            (Some(e), _) => (FileState::Cached, e.node_count()),
            (None, Some(a)) => (state_of(a), a.nodes.len()),
            (None, None) => (FileState::Failed, 0),
        };
        if matches!(state, FileState::Cached | FileState::Complete) {
            let before = index.files.get(&rel).map_or(0, |e| e.nodes);
            if nodes < before && !opts.allow_partial {
                outcome.shrunk.push(Shrink {
                    rel: rel.clone(),
                    before,
                    after: nodes,
                });
                all_complete = false;
            } else {
                index.record(&rel, &keys[&d], nodes, root);
            }
        } else {
            all_complete = false;
        }
        outcome.files.push((rel, state));
    }
    let omitted: Vec<&str> = outcome
        .files
        .iter()
        .filter(|(_, s)| *s == FileState::Omitted)
        .map(|(r, _)| r.as_str())
        .collect();
    let mut warnings = run.warnings.into_inner().expect("warnings lock");
    warnings.sort();
    outcome.warnings = warnings.into_iter().map(|(_, _, m)| m).collect();
    if !omitted.is_empty() {
        outcome.warnings.push(format!(
            "WARNING: {}/{} dispatched file(s) produced no nodes ({}); a re-run will retry them",
            omitted.len(),
            outcome.files.len() - outcome.count(FileState::Cached),
            omitted.join(", ")
        ));
    }
    if let Some(s) = outcome.schema.summary() {
        outcome.warnings.push(s);
    }
    outcome.usage = run.usage.report(backend.name(), backend.model());
    index.save(&opts.out_dir)?;
    outcome.usage.write(&opts.out_dir, "extract")?;
    if all_complete {
        cache::clear_needs_update(&opts.out_dir);
    }
    Ok(outcome)
}

fn state_of(a: &FileAcc) -> FileState {
    if a.pending {
        FileState::Pending
    } else if a.failed {
        FileState::Failed
    } else if a.partial {
        FileState::Partial
    } else if a.answered {
        FileState::Complete
    } else if a.omitted {
        FileState::Omitted
    } else {
        FileState::Failed
    }
}

fn accumulate(run: &Run<'_>, pieces: &[Piece], accs: &mut HashMap<usize, FileAcc>) {
    for piece in pieces {
        let files = run.files_of(&piece.parts);
        let split = piece.map.as_ref().map(|m| split_by_file(m, &files).0);
        for d in unique_docs(&piece.parts) {
            let rel = run.docs[d].dispatched();
            let acc = accs.get_mut(&d).expect("tracked file");
            match &piece.status {
                Status::Pending(_) => acc.pending = true,
                Status::Failed => acc.failed = true,
                Status::Hollow => acc.partial = true,
                Status::Truncated => acc.partial = true,
                Status::Complete => {}
            }
            match split.as_ref().and_then(|s| s.get(rel)) {
                Some(e) => {
                    acc.answered = true;
                    for (key, list) in [
                        ("nodes", &mut acc.nodes),
                        ("edges", &mut acc.edges),
                        ("hyperedges", &mut acc.hyperedges),
                    ] {
                        list.extend(items(&e.map, key).cloned());
                    }
                }
                None if matches!(piece.status, Status::Complete) => acc.omitted = true,
                None => {}
            }
        }
    }
}

/// Writes file `d`'s cache entry once all its requests are done.
fn finalize(run: &Run<'_>, d: usize, acc: &FileAcc, previous_omissions: u64) -> Result<()> {
    let key = &run.keys[&d];
    match state_of(acc) {
        FileState::Pending | FileState::Failed | FileState::Cached => Ok(()),
        FileState::Omitted => run.scope.write(key, &Entry::omission(previous_omissions)),
        state => {
            let mut e = Entry::new(acc.nodes.clone(), acc.edges.clone(), acc.hyperedges.clone());
            if state == FileState::Partial {
                e.mark_partial();
            }
            run.scope.write(key, &e)
        }
    }
}

fn unique_docs(parts: &[Part]) -> Vec<usize> {
    let mut out = Vec::new();
    for p in parts {
        if !out.contains(&p.doc) {
            out.push(p.doc);
        }
    }
    out
}

/// One file's nodes, edges and hyperedges.
type Items = (Vec<Value>, Vec<Value>, Vec<Value>);

/// `./README.md` and `docs\guide.md` name `README.md` and `docs/guide.md`.
fn normalize(path: &str) -> String {
    let mut p = path.replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    p
}

/// Splits a reply into cache entries for `files`: nodes by `source_file`; edges by
/// `source_file`, else by their source or target node's file, else the only file; hyperedges by
/// `source_file`, else their first member's file, else the only file. Items attributed to no file
/// of the request are not cached. Returns the entries of files with at least one node, and the
/// files without one.
pub(crate) fn split_by_file(
    map: &Map<String, Value>,
    files: &[String],
) -> (BTreeMap<String, Entry>, Vec<String>) {
    let in_scope = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map(normalize)
            .filter(|p| files.contains(p))
    };
    let only = (files.len() == 1).then(|| files[0].clone());
    let mut node_file: HashMap<String, String> = HashMap::new();
    let mut by: BTreeMap<String, Items> = BTreeMap::new();
    for n in items(map, "nodes") {
        if let Some(f) = in_scope(n.get("source_file")) {
            if let Some(id) = n.get("id").and_then(Value::as_str) {
                node_file.entry(id.to_string()).or_insert_with(|| f.clone());
            }
            by.entry(f).or_default().0.push(n.clone());
        }
    }
    let of_node = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .and_then(|id| node_file.get(id).cloned())
    };
    for e in items(map, "edges") {
        let f = in_scope(e.get("source_file"))
            .or_else(|| of_node(e.get("source")))
            .or_else(|| of_node(e.get("target")))
            .or_else(|| only.clone());
        if let Some(f) = f {
            by.entry(f).or_default().1.push(e.clone());
        }
    }
    for h in items(map, "hyperedges") {
        let first = h
            .get("nodes")
            .and_then(Value::as_array)
            .and_then(|m| m.first());
        let f = in_scope(h.get("source_file"))
            .or_else(|| of_node(first))
            .or_else(|| only.clone());
        if let Some(f) = f {
            by.entry(f).or_default().2.push(h.clone());
        }
    }
    let mut entries = BTreeMap::new();
    let mut omitted = Vec::new();
    for f in files {
        match by.remove(f) {
            Some((n, e, h)) if !n.is_empty() => {
                entries.insert(f.clone(), Entry::new(n, e, h));
            }
            _ => omitted.push(f.clone()),
        }
    }
    (entries, omitted)
}

/// `dijkstral-out` for a corpus root (the default output location).
pub fn default_out_dir(root: &Path) -> PathBuf {
    root.join(dijkstral_core::pipeline::OUT_DIR)
}

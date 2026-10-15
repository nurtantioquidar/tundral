//! The semantic pass as a build stage: `ingest` → `run::extract` → `merge`, plugged into
//! `dijkstral_core::pipeline` through its [`Semantic`] hook, plus the no-model helpers `update`
//! and `label` need.
//!
//! `ingest` is still the stand-in of `standin.rs`; the real stage replaces the two calls marked
//! `STAND-IN`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use dijkstral_core::graph::Graph;
use dijkstral_core::pipeline::{Semantic, SemanticReport};
use sha2::{Digest, Sha256};

use crate::merge::{merge, Dispatched, MergeInput};
use crate::run::{self, BackendSettings, Env, ExtractOptions, Mode};
use crate::standin;
use crate::types::{Backend, Chunk, DocKind, SourceDoc};

/// Default `--token-budget` (§4.2).
pub const DEFAULT_TOKEN_BUDGET: usize = 60_000;

/// The command-line options of the pass.
#[derive(Debug, Clone)]
pub struct PassOptions {
    pub backend: BackendSettings,
    pub mode: Mode,
    pub max_concurrency: usize,
    pub token_budget: usize,
    pub force: bool,
    pub allow_partial: bool,
    pub progress: bool,
}

impl Default for PassOptions {
    fn default() -> Self {
        PassOptions {
            backend: BackendSettings::default(),
            mode: Mode::Normal,
            max_concurrency: 4,
            token_budget: DEFAULT_TOKEN_BUDGET,
            force: false,
            allow_partial: false,
            progress: false,
        }
    }
}

/// A prepared pass: the corpus's non-code files and the backend that will read them.
pub struct SemanticPass {
    opts: PassOptions,
    docs: Vec<SourceDoc>,
    skipped: Vec<String>,
    backend: Box<dyn Backend>,
}

impl SemanticPass {
    /// The pass for `root`, or `None` when no file needs a model (then no backend is needed and
    /// none is selected, as Graphify asks for no key on a code-only corpus).
    pub fn prepare(root: &Path, opts: PassOptions, env: &Env) -> Result<Option<SemanticPass>> {
        // STAND-IN: `ingest`.
        let collected = standin::collect(root)?;
        if collected.docs.is_empty() {
            return Ok(None);
        }
        let backend = run::select(&opts.backend, env)?;
        Ok(Some(SemanticPass {
            opts,
            docs: collected.docs,
            skipped: collected.skipped,
            backend,
        }))
    }

    /// The pass over fixed documents (tests and benchmarks).
    pub fn with_docs(opts: PassOptions, docs: Vec<SourceDoc>, backend: Box<dyn Backend>) -> Self {
        SemanticPass {
            opts,
            docs,
            skipped: Vec::new(),
            backend,
        }
    }

    pub fn docs(&self) -> &[SourceDoc] {
        &self.docs
    }

    pub fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    fn extract_options(&self, root: &Path, out_dir: &Path) -> ExtractOptions {
        let mut eo = ExtractOptions::new(out_dir);
        eo.root = Some(root.to_path_buf());
        eo.mode = self.opts.mode;
        eo.max_concurrency = self.opts.max_concurrency.max(1);
        eo.force = self.opts.force;
        eo.allow_partial = self.opts.allow_partial;
        eo.progress = self.opts.progress;
        eo
    }

    /// Requests for the files that need the model, packed tightly, then one part per cached
    /// file (which `run::extract` serves from the cache without a request).
    fn chunks(&self, eo: &ExtractOptions) -> Vec<Chunk> {
        let misses = run::cache_misses(&self.docs, self.backend.as_ref(), eo);
        // STAND-IN: `ingest`'s chunking.
        let mut chunks = standin::chunk(&self.docs, &misses, self.opts.token_budget);
        for i in (0..self.docs.len()).filter(|i| !misses.contains(i)) {
            chunks.push(Chunk {
                index: chunks.len(),
                parts: vec![crate::types::Part {
                    doc: i,
                    piece: None,
                }],
            });
        }
        chunks
    }
}

impl Semantic for SemanticPass {
    fn fingerprint(&self, root: &Path, out_dir: &Path) -> Result<String> {
        let mut h = Sha256::new();
        for part in [
            run::PROMPT_VERSION.to_string(),
            self.backend.name().to_string(),
            self.backend.model().to_string(),
            self.opts.mode.name().to_string(),
            self.opts
                .backend
                .reasoning_effort
                .clone()
                .unwrap_or_default(),
        ] {
            h.update(part.as_bytes());
            h.update(b"\0");
        }
        for d in &self.docs {
            h.update(d.rel.as_bytes());
            h.update(b"\0");
            h.update(d.sha256.as_bytes());
            h.update(b"\0");
        }
        // While any file still needs the model (failed, omitted, partial or new), a build is
        // never "unchanged": the next run asks again.
        let eo = self.extract_options(root, out_dir);
        if !run::cache_misses(&self.docs, self.backend.as_ref(), &eo).is_empty() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            h.update(now.to_le_bytes());
        }
        Ok(hex(&h.finalize()))
    }

    fn force(&self) -> bool {
        self.opts.force
    }

    fn apply(&self, root: &Path, out_dir: &Path, graph: &mut Graph) -> Result<SemanticReport> {
        let eo = self.extract_options(root, out_dir);
        let chunks = self.chunks(&eo);
        let outcome = run::extract(&chunks, &self.docs, self.backend.as_ref(), &eo)?;
        for w in &outcome.warnings {
            eprintln!("[dijkstral extract] {w}");
        }
        outcome.usage.write(out_dir, "extract")?;
        if let Some(err) = outcome.shrink_error() {
            bail!("{err}");
        }
        let inputs: Vec<MergeInput> = outcome
            .extracted
            .iter()
            .map(|e| MergeInput::new(e.raw.clone(), e.cached))
            .collect();
        let stats = merge(graph, &dispatched(&self.docs), &inputs);
        let mut notes = vec![outcome.summary(), outcome.usage.summary()];
        if !outcome.pending.is_empty() {
            notes.push(format!(
                "{} request(s) written for your assistant under {}; answer them, then run \
                 `dijkstral semantic import`",
                outcome.pending.len(),
                run::backend::handoff::pending_dir(out_dir).display()
            ));
        }
        if !self.skipped.is_empty() {
            notes.push(format!(
                "not read yet (PDF and office text extraction pending): {}",
                self.skipped.join(", ")
            ));
        }
        notes.extend(stats.notes());
        notes.push(format!(
            "semantic: +{} nodes, +{} links, {} hyperedge(s) from {} of {} file(s)",
            stats.nodes_added,
            stats.edges_added,
            stats.hyperedges,
            stats.files_with_nodes,
            self.docs.len()
        ));
        Ok(SemanticReport {
            input_tokens: outcome.usage.input_tokens,
            output_tokens: outcome.usage.output_tokens,
            calls: outcome.usage.requests,
            nodes_added: stats.nodes_added,
            edges_added: stats.edges_added,
            notes,
        })
    }
}

/// The files of this run, with the text the evidence check searches.
pub fn dispatched(docs: &[SourceDoc]) -> Vec<Dispatched> {
    docs.iter()
        .map(|d| {
            let text =
                matches!(d.kind, DocKind::Document | DocKind::VectorImage).then(|| d.text.clone());
            Dispatched::new(d.rel.clone(), text)
        })
        .collect()
}

/// After a no-model `update`: flag the semantic pass as stale when a non-code file changed or
/// a new one appeared since it last ran (`check-update` then reports it, §9.3).
pub fn note_pending_changes(root: &Path, out_dir: &Path) -> Result<bool> {
    let index = run::cache::Index::load(out_dir);
    let new_file = || -> Result<bool> {
        // STAND-IN: `ingest`.
        let collected = standin::collect(root)?;
        Ok(collected
            .docs
            .iter()
            .any(|d| !index.files.contains_key(&d.rel)))
    };
    let pending =
        run::pending_changes(root, out_dir).is_some() || (!index.files.is_empty() && new_file()?);
    if pending {
        run::mark_needs_update(out_dir)?;
    }
    Ok(pending)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Graphify's community-names sidecar, next to the graph.
pub const LABELS_FILE: &str = ".graphify_labels.json";

/// `label`: re-cluster `<dir>/dijkstral-out/graph.json` (or `graph`) and name its communities
/// through `backend` (§10), rewriting the graph, analysis, report and `.graphify_labels.json`.
/// Returns the output directory, the community count and the naming outcome.
pub fn label(
    dir: &Path,
    graph: Option<&Path>,
    backend: &dyn Backend,
    opts: &run::LabelOptions,
) -> Result<(std::path::PathBuf, usize, run::LabelOutcome)> {
    use dijkstral_core::cluster::Communities;
    use std::cell::RefCell;

    let outcome: RefCell<run::LabelOutcome> = RefCell::default();
    let namer = |g: &Graph, c: &Communities, _fallback: &[String]| {
        let mut degree = vec![0usize; g.nodes.len()];
        let index: std::collections::HashMap<&str, usize> = g
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        for e in &g.edges {
            for end in [&e.source, &e.target] {
                if let Some(&i) = index.get(end.as_str()) {
                    degree[i] += 1;
                }
            }
        }
        // Members most connected first, ties by id.
        let communities: Vec<run::Community> = c
            .members
            .iter()
            .enumerate()
            .map(|(cid, members)| {
                let mut m: Vec<usize> = members.iter().map(|&i| i as usize).collect();
                m.sort_by(|&a, &b| {
                    degree[b]
                        .cmp(&degree[a])
                        .then(g.nodes[a].id.cmp(&g.nodes[b].id))
                });
                run::Community {
                    id: cid.to_string(),
                    labels: m.iter().map(|&i| g.nodes[i].label.clone()).collect(),
                }
            })
            .collect();
        let named = run::name_communities(&communities, backend, opts);
        let names = (0..c.members.len())
            .map(|cid| named.names.get(&cid.to_string()).cloned())
            .collect();
        let tokens = (named.usage.input_tokens, named.usage.output_tokens);
        *outcome.borrow_mut() = named;
        Ok((names, tokens))
    };
    let (out_dir, count, labels) =
        dijkstral_core::pipeline::cluster_only_files_named(dir, graph, Some(&namer))?;
    let outcome = outcome.into_inner();
    outcome.usage.write(&out_dir, "label")?;
    // `json.dumps` defaults: `", "` and `": "` separators, ASCII-escaped, one line.
    let body: Vec<String> = labels
        .iter()
        .enumerate()
        .map(|(c, name)| format!("\"{c}\": {}", ascii_json(name)))
        .collect();
    dijkstral_core::pipeline::write_atomic(
        &out_dir.join(LABELS_FILE),
        format!("{{{}}}", body.join(", ")).as_bytes(),
    )?;
    Ok((out_dir, count, outcome))
}

/// A JSON string with every non-ASCII character as `\uXXXX` (Python's `ensure_ascii`).
fn ascii_json(s: &str) -> String {
    let mut out = String::new();
    let mut buf = [0u16; 2];
    for c in serde_json::to_string(s).unwrap_or_default().chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            for unit in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

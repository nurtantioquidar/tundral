//! The semantic pass as `extract`'s build pass: `ingest` → `run::extract` → `merge`, plugged into
//! `dijkstral_core::pipeline` through its [`Documents`] seam (the one `update`'s markdown uses):
//! it adds no pages of its own and merges the model's replies into the resolved graph in
//! [`Documents::augment`]. Also the no-model helper `update` needs and `label`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use dijkstral_core::discover::{discover_all, discover_all_with, SourceFile};
use dijkstral_core::graph::Graph;
use dijkstral_core::pipeline::{DocumentFacts, Documents, SemanticReport, OUT_DIR};
use dijkstral_core::resolve::{Index, Resolution, View};
use sha2::{Digest, Sha256};

use crate::ingest::{self, IngestOptions};
use crate::merge::{merge, Dispatched, MergeInput};
use crate::run::{self, BackendSettings, Env, ExtractOptions, Mode};
use crate::types::{Backend, Chunk, DocKind, Part, SourceDoc};

pub use crate::ingest::DEFAULT_TOKEN_BUDGET;

/// Where converted office files are written and the path replies name them by (§2.2). Graphify
/// uses `graphify-out/converted`; dijkstral keeps its outputs in its own directory, so office
/// nodes are `dijkstral_out_converted_…` where Graphify's are `graphify_out_converted_…`, and
/// `update` does not index the conversions as pages (quirk S2 is not reproduced).
pub const CONVERTED_PREFIX: &str = "dijkstral-out/converted";

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
    /// Read `.gitignore` files when finding documents (off with `--no-gitignore`).
    pub gitignore: bool,
    /// Export Google Workspace shortcuts through `gws` (`--google-workspace`).
    pub google_workspace: bool,
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
            gitignore: true,
            google_workspace: false,
        }
    }
}

/// A prepared pass: the corpus's non-code files and the backend that will read them.
pub struct SemanticPass {
    opts: PassOptions,
    root: PathBuf,
    out_dir: PathBuf,
    docs: Vec<SourceDoc>,
    notes: Vec<String>,
    backend: Box<dyn Backend>,
}

/// `<out or root>/dijkstral-out`.
pub fn out_dir_for(root: &Path, out: Option<&Path>) -> PathBuf {
    out.unwrap_or(root).join(OUT_DIR)
}

impl SemanticPass {
    /// The pass for `root` (outputs in `out_dir`), or `None` when no file needs a model (then no
    /// backend is selected, as Graphify asks for no key on a code-only corpus).
    pub fn prepare(
        root: &Path,
        out_dir: &Path,
        opts: PassOptions,
        env: &Env,
    ) -> Result<Option<SemanticPass>> {
        let found = discover_all_with(root, 1, opts.gitignore);
        let converted_dir = out_dir.join("converted");
        let mut notes: Vec<String> = Vec::new();
        let gws = if opts.google_workspace {
            crate::gws::GwsCli::find()
        } else {
            None
        };
        if opts.google_workspace && gws.is_none() {
            let shortcuts = found
                .documents
                .iter()
                .filter(|rel| crate::gws::Kind::of(rel).is_some())
                .count();
            if shortcuts > 0 {
                notes.push(format!(
                    "google workspace: gws is not on PATH (or DIJKSTRAL_GWS); {shortcuts} \
                     shortcut(s) left out"
                ));
            }
        }
        let ingested = ingest::ingest(
            root,
            &found.documents,
            &IngestOptions {
                converted_prefix: CONVERTED_PREFIX,
                converted_dir: Some(&converted_dir),
                workspace: gws.as_ref().map(|g| g as &dyn crate::gws::Exporter),
            },
        );
        for (rel, why) in ingested.skipped.iter().chain(&ingested.warnings) {
            notes.push(format!("{rel}: {why}"));
        }
        if ingested.docs.is_empty() {
            return Ok(None);
        }
        let backend = run::select(&opts.backend, env)?;
        Ok(Some(SemanticPass {
            opts,
            root: root.to_path_buf(),
            out_dir: out_dir.to_path_buf(),
            docs: ingested.docs,
            notes,
            backend,
        }))
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

    /// Requests for the files that need the model, packed tightly (`run::cache_misses`), then
    /// one part per cached file, which `run::extract` serves from the cache without a request.
    fn chunks(&self, eo: &ExtractOptions) -> Vec<Chunk> {
        let misses = run::cache_misses(&self.docs, self.backend.as_ref(), eo);
        let subset: Vec<SourceDoc> = misses.iter().map(|&i| self.docs[i].clone()).collect();
        let mut chunks: Vec<Chunk> = ingest::chunk(&subset, self.opts.token_budget)
            .into_iter()
            .map(|c| Chunk {
                index: c.index,
                parts: c
                    .parts
                    .into_iter()
                    .map(|p| Part {
                        doc: misses[p.doc],
                        piece: p.piece,
                    })
                    .collect(),
            })
            .collect();
        for i in (0..self.docs.len()).filter(|i| !misses.contains(i)) {
            chunks.push(Chunk {
                index: chunks.len(),
                parts: vec![Part {
                    doc: i,
                    piece: None,
                }],
            });
        }
        chunks
    }
}

impl Documents for SemanticPass {
    fn fingerprint(&self) -> String {
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
            h.update(d.dispatched().as_bytes());
            h.update(b"\0");
            h.update(d.sha256.as_bytes());
            h.update(b"\0");
        }
        // While any file still needs the model (failed, omitted, partial or new), a build is
        // never "unchanged": the next run asks again.
        let eo = self.extract_options(&self.root, &self.out_dir);
        if !run::cache_misses(&self.docs, self.backend.as_ref(), &eo).is_empty() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            h.update(now.to_le_bytes());
        }
        hex(&h.finalize())
    }

    /// `extract` adds no markdown structure (§2.1): the model reads the files instead.
    fn accepts(&self, _rel: &str) -> bool {
        false
    }

    fn extract(&self, _root: &Path, _files: &[(SourceFile, Vec<u8>)]) -> DocumentFacts {
        DocumentFacts::default()
    }

    fn resolve(&self, _views: &[View], _index: &Index) -> Resolution {
        Resolution::default()
    }

    fn force(&self) -> bool {
        self.opts.force
    }

    fn augment(
        &self,
        root: &Path,
        out_dir: &Path,
        graph: &mut Graph,
    ) -> Result<Option<SemanticReport>> {
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
        let mut notes = self.notes.clone();
        notes.push(outcome.summary());
        notes.push(outcome.usage.summary());
        if !outcome.pending.is_empty() {
            notes.push(format!(
                "{} request(s) written for your assistant under {}; answer them, then run \
                 `dijkstral semantic import`",
                outcome.pending.len(),
                run::backend::handoff::pending_dir(out_dir).display()
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
        Ok(Some(SemanticReport {
            input_tokens: outcome.usage.input_tokens,
            output_tokens: outcome.usage.output_tokens,
            calls: outcome.usage.requests,
            nodes_added: stats.nodes_added,
            edges_added: stats.edges_added,
            notes,
        }))
    }
}

/// The files of this run as replies name them, with the text the evidence check searches. An
/// office file counts under its converted path (Graphify's rule) and under its own path, which
/// is the one `run`'s requests frame it by, so replies naming either are kept.
pub fn dispatched(docs: &[SourceDoc]) -> Vec<Dispatched> {
    let mut out = Vec::with_capacity(docs.len());
    for d in docs {
        let text =
            matches!(d.kind, DocKind::Document | DocKind::VectorImage).then(|| d.text.clone());
        if d.converted.is_some() {
            out.push(Dispatched::new(d.rel.clone(), text.clone()));
        }
        out.push(Dispatched::new(d.dispatched(), text));
    }
    out
}

/// After a no-model `update`: flag the semantic pass as stale when a non-code file it read
/// changed, or a new one appeared, since it last ran (`check-update` then reports it, §9.3).
pub fn note_pending_changes(root: &Path, out_dir: &Path) -> Result<bool> {
    let index = run::cache::Index::load(out_dir);
    if index.files.is_empty() {
        // The semantic pass never ran here.
        return Ok(false);
    }
    let new_file = || {
        discover_all(root, 1)
            .documents
            .iter()
            .filter(|rel| ingest::classify(rel).is_some())
            .any(|rel| !index.files.contains_key(rel.as_str()))
    };
    let pending = run::pending_changes(root, out_dir).is_some() || new_file();
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
    // `--missing-only`: the names saved next to the graph, by community id.
    let saved: std::collections::HashMap<String, String> = if opts.missing_only {
        let labels = graph.map_or_else(
            || dir.join(OUT_DIR).join(LABELS_FILE),
            |g| g.with_file_name(LABELS_FILE),
        );
        std::fs::read(&labels)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.as_object().cloned())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .filter(|(k, v)| !v.is_empty() && *v != format!("Community {k}"))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Default::default()
    };
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
            .filter(|community: &run::Community| !saved.contains_key(&community.id))
            .collect();
        let named = run::name_communities(&communities, backend, opts);
        let names = (0..c.members.len())
            .map(|cid| {
                let id = cid.to_string();
                saved.get(&id).or_else(|| named.names.get(&id)).cloned()
            })
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

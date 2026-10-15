//! The build: discover → stat → cache lookup → extract misses in parallel → resolve → write.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::analyze::{analyze, ANALYSIS_FILE, REPORT_FILE};
use crate::cache::{content_key, Cache, Key, Stat};
use crate::cluster::{cluster, Communities, Topology};
use crate::discover::{discover_all, SourceFile};
use crate::extract::extract_file;
use crate::facts::{Attr, FileFacts};
use crate::graph::Graph;
use crate::index;
use crate::report::{self, Corpus};
use crate::resolve::resolve;

pub const OUT_DIR: &str = "dijkstral-out";

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub root: PathBuf,
    /// Directory that receives `dijkstral-out/`; defaults to `root`.
    pub out: Option<PathBuf>,
    pub threads: usize,
    /// Cluster, analyse and write `GRAPH_REPORT.md` and `.graphify_analysis.json` (Graphify's
    /// default); without it `graph.json` has no `community` and the sidecars are removed.
    pub cluster: bool,
}

#[derive(Debug, Default)]
pub struct BuildReport {
    pub files: usize,
    pub extracted: usize,
    pub cached: usize,
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    pub out_dir: PathBuf,
    pub phases: Vec<(&'static str, Duration)>,
    /// Nothing changed since the last build; outputs were left as they are.
    pub unchanged: bool,
}

enum Probe {
    /// Stat matched the cache index; content not read.
    Known(Key, Stat),
    /// Read and hashed. The content is not kept: extraction re-reads the (rare) misses, so a
    /// cold build never holds the whole corpus in memory.
    Read(Key, Stat),
    /// Unreadable (removed mid-walk, permissions…).
    Failed,
}

/// What an [`Exporter`] sees of a finished build.
pub struct Built<'a> {
    pub graph: &'a Graph,
    /// Node indices in `graph.json` array order.
    pub node_order: &'a [u32],
    /// Edge indices in `graph.json` array order.
    pub edge_order: &'a [u32],
    /// The communities and their `GRAPH_REPORT.md` labels; `None` without clustering.
    pub communities: Option<(&'a Communities, &'a [String])>,
    pub out_dir: &'a Path,
}

/// An output written after the core ones (`graph.html` and the wiki live in `crates/export`).
pub trait Exporter: Sync {
    /// Phase name for `--timings`.
    fn name(&self) -> &'static str;
    /// Paths it writes, relative to the output directory. An unchanged build is skipped only when
    /// every one exists.
    fn outputs(&self) -> Vec<PathBuf>;
    fn write(&self, built: &Built) -> Result<()>;
}

pub fn build(opts: &BuildOptions) -> Result<BuildReport> {
    build_with(opts, &[])
}

/// [`build`], then `exporters` in order, before the build is recorded as current.
pub fn build_with(opts: &BuildOptions, exporters: &[&dyn Exporter]) -> Result<BuildReport> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads.max(1))
        .build()
        .context("building thread pool")?;
    pool.install(|| build_inner(opts, exporters))
}

fn build_inner(opts: &BuildOptions, exporters: &[&dyn Exporter]) -> Result<BuildReport> {
    let mut report = BuildReport::default();
    let mut clock = Instant::now();
    let mut phase = |report: &mut BuildReport, name: &'static str| {
        report.phases.push((name, clock.elapsed()));
        clock = Instant::now();
    };

    let root = opts
        .root
        .canonicalize()
        .with_context(|| format!("reading {}", opts.root.display()))?;
    let out_dir = opts
        .out
        .clone()
        .unwrap_or_else(|| root.clone())
        .join(OUT_DIR);
    fs::create_dir_all(&out_dir)?;
    report.out_dir = out_dir.clone();

    let found = discover_all(&root, opts.threads);
    let (files, unclassified) = (found.files, found.unclassified);
    report.files = files.len();
    phase(&mut report, "discover");

    let mut cache = Cache::open(&out_dir.join("cache.redb"))?;
    let probes: Vec<Probe> = files.par_iter().map(|f| probe(&cache, f)).collect();
    phase(&mut report, "stat");

    let file_keys: Vec<Option<Key>> = probes
        .iter()
        .map(|p| match p {
            Probe::Known(k, _) | Probe::Read(k, _) => Some(*k),
            Probe::Failed => None,
        })
        .collect();
    let commit = git_head(&root);
    let date = today();
    let digest = build_digest(
        &files,
        &file_keys,
        &unclassified,
        opts.cluster,
        commit.as_deref(),
        &date,
    );
    let extra: Vec<PathBuf> = exporters.iter().flat_map(|e| e.outputs()).collect();
    if file_keys.iter().all(Option::is_some)
        && outputs_current(&out_dir, opts.cluster, &digest, &extra)
    {
        report.unchanged = true;
        report.cached = files.len();
        // Record that the outputs were confirmed current now: the hook guard compares a file's
        // mtime with this, so a touched-but-identical file stops counting as stale.
        if let Ok(f) = fs::OpenOptions::new()
            .write(true)
            .open(out_dir.join(DIGEST_FILE))
        {
            f.set_modified(SystemTime::now()).ok();
        }
        phase(&mut report, "unchanged");
        return Ok(report);
    }
    let keys: Vec<Key> = file_keys.iter().flatten().copied().collect();
    let mut hits = cache.get_many(&keys)?;
    phase(&mut report, "cache-read");

    // Extract every file the cache could not answer.
    let fresh: Vec<(usize, Key, FileFacts)> = probes
        .par_iter()
        .enumerate()
        .filter_map(|(i, p)| {
            match p {
                Probe::Known(k, _) | Probe::Read(k, _) if !hits.contains_key(k) => {}
                _ => return None,
            }
            let f = &files[i];
            let content = fs::read(&f.abs).ok()?;
            // Key what was actually parsed, in case the file changed since the probe.
            let key = content_key(f.lang, &f.rel, &content);
            Some((i, key, extract_file(f.lang, &f.rel, &content)))
        })
        .collect();
    report.extracted = fresh.len();
    phase(&mut report, "extract");

    let mut stats = HashMap::with_capacity(files.len());
    for (f, p) in files.iter().zip(&probes) {
        if let Probe::Known(k, s) | Probe::Read(k, s) = p {
            stats.insert(f.rel.clone(), (*s, *k));
        }
    }
    let new_refs: Vec<(Key, &FileFacts)> = fresh.iter().map(|(_, k, f)| (*k, f)).collect();
    cache.commit(&new_refs, stats, now_ns())?;
    drop(probes);
    phase(&mut report, "cache-write");

    // Assemble per-file facts in path order.
    let mut by_index: Vec<Option<FileFacts>> = vec![None; files.len()];
    for (i, _, f) in fresh {
        by_index[i] = Some(f);
    }
    // Keys include the relative path, so each hit belongs to exactly one file.
    let ordered: Vec<(&SourceFile, FileFacts)> = files
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let facts = by_index[i].take().or_else(|| hits.remove(&file_keys[i]?))?;
            Some((f, facts))
        })
        .collect();
    report.cached = ordered.len() - report.extracted;

    let words: Vec<u64> = ordered.iter().map(|(_, f)| f.words).collect();
    let graph: Graph = resolve(ordered);
    phase(&mut report, "resolve");

    report.nodes = graph.nodes.len();
    report.edges = graph.edges.len();
    // A failed or interrupted build must never look current: drop the digest first.
    fs::remove_file(out_dir.join(DIGEST_FILE)).ok();

    let clustered = if opts.cluster {
        let topo = Topology::of(&graph);
        let communities = cluster(&topo);
        report.communities = communities.members.len();
        phase(&mut report, "cluster");
        Some((topo, communities))
    } else {
        None
    };

    let target = out_dir.join("graph.json");
    let tmp = target.with_extension("json.tmp");
    let (node_order, edge_order) = graph.write_graphify_json_ordered(
        fs::File::create(&tmp)?,
        commit.as_deref(),
        clustered.as_ref().map(|(_, c)| c.of_node.as_slice()),
    )?;
    fs::rename(&tmp, &target)?;
    index::write(
        &out_dir.join(index::INDEX_FILE),
        &graph,
        clustered.as_ref().map(|(_, c)| c.of_node.as_slice()),
    )?;
    phase(&mut report, "write");

    let mut labelled = None;
    if let Some((topo, communities)) = clustered {
        let analysis = analyze(&graph, &topo, &communities);
        write_atomic(
            &out_dir.join(ANALYSIS_FILE),
            analysis.to_json(&graph, &communities).as_bytes(),
        )?;
        phase(&mut report, "analyze");
        let corpus = Corpus {
            name: root
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            stats: Some((words.len(), words.iter().sum())),
            unclassified,
            commit: commit.clone(),
            date: date.clone(),
        };
        let md = report::render(&graph, &topo, &communities, &analysis, &corpus);
        write_atomic(&out_dir.join(REPORT_FILE), md.as_bytes())?;
        phase(&mut report, "report");
        labelled = Some((communities, analysis.labels));
    } else {
        // A previous clustered build's sidecars would describe a different graph.
        fs::remove_file(out_dir.join(ANALYSIS_FILE)).ok();
        fs::remove_file(out_dir.join(REPORT_FILE)).ok();
    }
    let built = Built {
        graph: &graph,
        node_order: &node_order,
        edge_order: &edge_order,
        communities: labelled.as_ref().map(|(c, l)| (c, l.as_slice())),
        out_dir: &out_dir,
    };
    for exporter in exporters {
        exporter
            .write(&built)
            .with_context(|| format!("writing {}", exporter.name()))?;
        phase(&mut report, exporter.name());
    }
    write_atomic(&out_dir.join(DIGEST_FILE), digest.as_bytes())?;
    Ok(report)
}

/// Records what the outputs in `dijkstral-out/` were built from.
pub const DIGEST_FILE: &str = ".build-digest";

/// Everything that determines the outputs: the binary, each file's content key, the unclassified
/// files the report counts, the options, the git commit written into `graph.json`, and the date
/// written into the report.
fn build_digest(
    files: &[SourceFile],
    keys: &[Option<Key>],
    unclassified: &[String],
    cluster: bool,
    commit: Option<&str>,
    date: &str,
) -> String {
    let mut h = blake3::Hasher::new();
    h.update(env!("DIJKSTRAL_BUILD_FINGERPRINT").as_bytes());
    h.update(&[cluster as u8]);
    h.update(commit.unwrap_or("").as_bytes());
    h.update(&[0]);
    h.update(date.as_bytes());
    h.update(&[0]);
    for (f, k) in files.iter().zip(keys) {
        h.update(f.rel.as_bytes());
        h.update(&[0]);
        h.update(k.as_ref().map_or(&[0u8; 32][..], |k| &k[..]));
    }
    for rel in unclassified {
        h.update(rel.as_bytes());
        h.update(&[0]);
    }
    h.finalize().to_hex().to_string()
}

/// The last build had the same digest and every output it writes is still there, the exporters'
/// (`extra`, relative to `out_dir`) included.
fn outputs_current(out_dir: &Path, cluster: bool, digest: &str, extra: &[PathBuf]) -> bool {
    let recorded = fs::read_to_string(out_dir.join(DIGEST_FILE)).unwrap_or_default();
    let mut required = vec![out_dir.join("graph.json"), out_dir.join(index::INDEX_FILE)];
    if cluster {
        required.push(out_dir.join(ANALYSIS_FILE));
        required.push(out_dir.join(REPORT_FILE));
    }
    required.extend(extra.iter().map(|p| out_dir.join(p)));
    recorded == digest && required.iter().all(|p| p.is_file())
}

fn probe(cache: &Cache, f: &SourceFile) -> Probe {
    let Some(stat) = stat(&f.abs) else {
        return Probe::Failed;
    };
    if let Some(key) = cache.key_from_stat(&f.rel, stat) {
        return Probe::Known(key, stat);
    }
    match fs::read(&f.abs) {
        Ok(content) => Probe::Read(content_key(f.lang, &f.rel, &content), stat),
        Err(_) => Probe::Failed,
    }
}

fn stat(path: &Path) -> Option<Stat> {
    let meta = fs::metadata(path).ok()?;
    let mtime_ns = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some(Stat {
        mtime_ns,
        size: meta.len(),
    })
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

/// What `cluster_only` produces, in memory.
pub struct ClusterOnly {
    pub graph_json: String,
    pub analysis_json: String,
    pub report: String,
    pub communities: usize,
}

/// Re-cluster an existing clustered `graph.json` the way `graphify cluster-only --no-label` does:
/// nodes and links in file order, `Community N` names (also written as `community_name`), no file
/// statistics, and `questions` in the analysis sidecar.
pub fn cluster_only(doc: &serde_json::Value, name: &str) -> Result<ClusterOnly> {
    let mut graph = Graph::from_graphify_json(doc).map_err(anyhow::Error::msg)?;
    let commit = doc
        .get("built_at_commit")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let topo = Topology::of(&graph);
    let communities = cluster(&topo);
    let mut analysis = analyze(&graph, &topo, &communities);
    analysis.labels = (0..communities.members.len())
        .map(|c| format!("Community {c}"))
        .collect();
    let analysis_json = report::cluster_only_analysis_json(&graph, &communities, &analysis);
    let corpus = Corpus {
        name: name.to_string(),
        stats: None,
        unclassified: Vec::new(),
        commit: commit.clone(),
        date: today(),
    };
    let md = report::render(&graph, &topo, &communities, &analysis, &corpus);
    for (i, n) in graph.nodes.iter_mut().enumerate() {
        let c = communities.of_node[i];
        n.attrs
            .push(("community_name".into(), Attr::Str(format!("Community {c}"))));
    }
    let graph_json = graph.to_graphify_json(commit.as_deref(), Some(&communities.of_node));
    Ok(ClusterOnly {
        graph_json,
        analysis_json,
        report: md,
        communities: communities.members.len(),
    })
}

/// `cluster_only` on `<dir>/dijkstral-out/graph.json` (or `graph`), rewriting that graph and its
/// sidecars in place. Returns the output directory and the community count.
pub fn cluster_only_files(dir: &Path, graph: Option<&Path>) -> Result<(PathBuf, usize)> {
    let path = graph.map_or_else(|| dir.join(OUT_DIR).join("graph.json"), Path::to_path_buf);
    let doc: serde_json::Value = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    let name = dir
        .canonicalize()
        .ok()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let out = cluster_only(&doc, &name)?;
    drop(doc);
    let out_dir = path.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    write_atomic(&path, out.graph_json.as_bytes())?;
    write_atomic(&out_dir.join(ANALYSIS_FILE), out.analysis_json.as_bytes())?;
    write_atomic(&out_dir.join(REPORT_FILE), out.report.as_bytes())?;
    Ok((out_dir, out.communities))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Today's date (UTC) as `YYYY-MM-DD`; `SOURCE_DATE_EPOCH` pins it for reproducible reports.
fn today() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
        });
    // Howard Hinnant's civil_from_days.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn git_head(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

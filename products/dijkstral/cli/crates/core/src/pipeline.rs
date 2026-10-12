//! The build: discover → stat → cache lookup → extract misses in parallel → resolve → write.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::cache::{content_key, Cache, Key, Stat};
use crate::discover::{discover, SourceFile};
use crate::extract::extract_file;
use crate::facts::FileFacts;
use crate::graph::Graph;
use crate::resolve::resolve;

pub const OUT_DIR: &str = "dijkstral-out";

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub root: PathBuf,
    /// Directory that receives `dijkstral-out/`; defaults to `root`.
    pub out: Option<PathBuf>,
    pub threads: usize,
}

#[derive(Debug, Default)]
pub struct BuildReport {
    pub files: usize,
    pub extracted: usize,
    pub cached: usize,
    pub nodes: usize,
    pub edges: usize,
    pub out_dir: PathBuf,
    pub phases: Vec<(&'static str, Duration)>,
}

enum Probe {
    /// Stat matched the cache index; content not read.
    Known(Key, Stat),
    /// Read and hashed.
    Read(Key, Stat, Vec<u8>),
    /// Unreadable (removed mid-walk, permissions…).
    Failed,
}

pub fn build(opts: &BuildOptions) -> Result<BuildReport> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads.max(1))
        .build()
        .context("building thread pool")?;
    pool.install(|| build_inner(opts))
}

fn build_inner(opts: &BuildOptions) -> Result<BuildReport> {
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

    let files = discover(&root, opts.threads);
    report.files = files.len();
    phase(&mut report, "discover");

    let mut cache = Cache::open(&out_dir.join("cache.redb"))?;
    let probes: Vec<Probe> = files.par_iter().map(|f| probe(&cache, f)).collect();
    phase(&mut report, "stat");

    let file_keys: Vec<Option<Key>> = probes
        .iter()
        .map(|p| match p {
            Probe::Known(k, _) | Probe::Read(k, _, _) => Some(*k),
            Probe::Failed => None,
        })
        .collect();
    let keys: Vec<Key> = file_keys.iter().flatten().copied().collect();
    let mut hits = cache.get_many(&keys)?;
    phase(&mut report, "cache-read");

    // Extract every file the cache could not answer.
    let fresh: Vec<(usize, Key, FileFacts)> = probes
        .par_iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let (key, content) = match p {
                Probe::Known(k, _) if !hits.contains_key(k) => (*k, fs::read(&files[i].abs).ok()?),
                Probe::Read(k, _, c) if !hits.contains_key(k) => (*k, c.clone()),
                _ => return None,
            };
            let f = &files[i];
            Some((i, key, extract_file(f.lang, &f.rel, &content)))
        })
        .collect();
    report.extracted = fresh.len();
    phase(&mut report, "extract");

    let mut stats = HashMap::with_capacity(files.len());
    for (f, p) in files.iter().zip(&probes) {
        if let Probe::Known(k, s) | Probe::Read(k, s, _) = p {
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

    let graph: Graph = resolve(ordered);
    phase(&mut report, "resolve");

    report.nodes = graph.nodes.len();
    report.edges = graph.edges.len();
    let json = graph.to_graphify_json(git_head(&root).as_deref());
    write_atomic(&out_dir.join("graph.json"), json.as_bytes())?;
    phase(&mut report, "write");
    Ok(report)
}

fn probe(cache: &Cache, f: &SourceFile) -> Probe {
    let Some(stat) = stat(&f.abs) else {
        return Probe::Failed;
    };
    if let Some(key) = cache.key_from_stat(&f.rel, stat) {
        return Probe::Known(key, stat);
    }
    match fs::read(&f.abs) {
        Ok(content) => Probe::Read(content_key(f.lang, &f.rel, &content), stat, content),
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

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

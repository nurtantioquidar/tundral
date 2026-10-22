//! Builds: one at a time per host, coalesced per repository, each in a child process with a
//! timeout, swapped in atomically so queries never see a half-built graph.
//!
//! Layout under `<data>/repos/<repo id>/`: one directory per built commit (the checkout, with
//! `dijkstral-out/` inside it) and `current`, a symlink to the one being served. The previous
//! build is kept until the next swap so queries in flight finish on it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::db::Db;
use crate::git::Source;

/// What a finished build reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Built {
    pub nodes: i64,
    pub edges: i64,
}

/// Called after a repository's graph changed or was removed (drops cached MCP servers).
pub type OnSwap = Arc<dyn Fn(i64) + Send + Sync>;

pub struct Builder {
    tx: mpsc::UnboundedSender<i64>,
    pending: Mutex<HashSet<i64>>,
}

impl Builder {
    /// Starts the worker; builds run until the returned builder is dropped.
    pub fn start(
        config: Arc<Config>,
        db: Arc<Db>,
        source: Arc<Source>,
        on_swap: OnSwap,
    ) -> Arc<Builder> {
        let (tx, mut rx) = mpsc::unbounded_channel::<i64>();
        let builder = Arc::new(Builder {
            tx,
            pending: Mutex::new(HashSet::new()),
        });
        let weak = Arc::downgrade(&builder);
        tokio::spawn(async move {
            while let Some(repo) = rx.recv().await {
                let Some(b) = weak.upgrade() else { break };
                // Taken off the pending set first: a push arriving during this build queues one
                // more build of the newest commit.
                b.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&repo);
                drop(b);
                if let Err(e) = build_repo(&config, &db, &source, &on_swap, repo).await {
                    tracing::warn!(repo, error = %e, "build failed");
                }
            }
        });
        builder
    }

    /// Queues a build of `repo` unless one is already waiting.
    pub fn enqueue(&self, repo: i64) {
        if self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(repo)
        {
            let _ = self.tx.send(repo);
        }
    }
}

pub fn repo_dir(config: &Config, repo: i64) -> PathBuf {
    config.data.join("repos").join(repo.to_string())
}

/// The graph a repository's queries use.
pub fn current_graph(config: &Config, repo: i64) -> PathBuf {
    repo_dir(config, repo).join("current/dijkstral-out/graph.dij")
}

/// Deletes everything stored for `repo`.
pub fn remove_repo_files(config: &Config, repo: i64) -> Result<()> {
    let dir = repo_dir(config, repo);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

async fn build_repo(
    config: &Config,
    db: &Db,
    source: &Source,
    on_swap: &OnSwap,
    repo: i64,
) -> Result<()> {
    let Some(r) = db.repo(repo)? else {
        return Ok(()); // removed while queued
    };
    if r.state == "over_limit" {
        return Ok(());
    }
    if db.builds_today(repo)? >= config.limits.builds_per_repo_per_day {
        db.set_state(
            repo,
            "failed",
            Some("daily build limit reached; next push after 24 h"),
        )?;
        return Ok(());
    }
    let build = db.start_build(repo)?;
    db.set_state(repo, "building", None)?;
    let dir = repo_dir(config, repo);
    let work = dir.join(format!("work-{build}"));
    let result = async {
        let branch = match r.default_branch.clone() {
            Some(b) => b,
            None => {
                let b = source
                    .default_branch(r.installation_id, &r.full_name)
                    .await?;
                db.set_default_branch(repo, &b)?;
                b
            }
        };
        if work.exists() {
            std::fs::remove_dir_all(&work)?;
        }
        std::fs::create_dir_all(&dir)?;
        let commit = source
            .checkout(r.installation_id, &r.full_name, &branch, &work)
            .await?;
        if r.built_commit.as_deref() == Some(commit.as_str()) && dir.join("current").exists() {
            return Ok::<_, anyhow::Error>((commit, None));
        }
        prepare(&work, config.limits.checkout_bytes)?;
        let built = run_child(config, &work).await?;
        if built.nodes as usize > config.limits.nodes {
            bail!(
                "graph too large: {} nodes (limit {})",
                built.nodes,
                config.limits.nodes
            );
        }
        Ok((commit, Some(built)))
    }
    .await;
    if db.repo(repo)?.is_none() {
        // Removed during the build: keep nothing.
        remove_repo_files(config, repo)?;
        return Ok(());
    }
    match result {
        Ok((commit, Some(built))) => {
            swap(&dir, &work, &commit)?;
            db.finish_build(build, repo, Some(&commit), Ok((built.nodes, built.edges)))?;
            on_swap(repo);
            tracing::info!(repo, commit, nodes = built.nodes, "built");
        }
        Ok((commit, None)) => {
            let _ = std::fs::remove_dir_all(&work);
            let (n, e) = (r.nodes.unwrap_or(0), r.edges.unwrap_or(0));
            db.finish_build(build, repo, Some(&commit), Ok((n, e)))?;
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            db.finish_build(build, repo, None, Err(&e.to_string()))?;
        }
    }
    Ok(())
}

/// Removes any build outputs the repository itself carries (a committed `dijkstral-out/` could
/// hold a crafted index or a cache that makes the build keep it) and enforces the size limit.
fn prepare(work: &Path, max_bytes: u64) -> Result<()> {
    for out in ["dijkstral-out", "graphify-out"] {
        let p = work.join(out);
        if p.symlink_metadata().is_ok() {
            if p.is_dir() && !p.symlink_metadata()?.file_type().is_symlink() {
                std::fs::remove_dir_all(&p)?;
            } else {
                std::fs::remove_file(&p)?;
            }
        }
    }
    let size = dir_size(work)?;
    if size > max_bytes {
        bail!(
            "repository too large: {} MB (limit {} MB)",
            size >> 20,
            max_bytes >> 20
        );
    }
    Ok(())
}

fn dir_size(dir: &Path) -> Result<u64> {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let m = e.metadata()?;
            if m.is_dir() {
                stack.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    Ok(total)
}

/// Runs `<exe> build <work>` with the build timeout; it prints the build's counts as JSON.
async fn run_child(config: &Config, work: &Path) -> Result<Built> {
    let mut cmd = Command::new(&config.exe);
    cmd.arg("build")
        .arg(work)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("DIJKSTRAL_BUILD_THREADS", config.build_threads.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(config.limits.build_timeout, cmd.output())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "build timed out after {} s",
                config.limits.build_timeout.as_secs()
            )
        })?
        .context("build could not start")?;
    if !out.status.success() {
        bail!("build failed");
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("build printed no report")?;
    Ok(Built {
        nodes: v["nodes"].as_i64().context("report without nodes")?,
        edges: v["edges"].as_i64().context("report without edges")?,
    })
}

/// Moves `work` to `<dir>/<commit>` and points `current` at it atomically; keeps the build it
/// replaces (queries in flight) and deletes older ones.
fn swap(dir: &Path, work: &Path, commit: &str) -> Result<()> {
    let target = dir.join(commit);
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::rename(work, &target)?;
    let previous = std::fs::read_link(dir.join("current")).ok();
    let tmp = dir.join("current.tmp");
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(commit, &tmp)?;
    std::fs::rename(&tmp, dir.join("current"))?;
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name();
        let keep = name == "current"
            || name == commit
            || previous.as_deref().is_some_and(|p| p.as_os_str() == name);
        if !keep && e.file_type()?.is_dir() && !name.to_string_lossy().starts_with("work-") {
            std::fs::remove_dir_all(e.path())?;
        }
    }
    Ok(())
}

/// The child: builds the code graph of `dir` (no model, no network) and prints its counts.
pub fn child_main(dir: &Path) -> Result<()> {
    let threads = std::env::var("DIJKSTRAL_BUILD_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(2);
    let report = dijkstral_core::pipeline::build(&dijkstral_core::pipeline::BuildOptions {
        root: dir.to_path_buf(),
        out: None,
        threads,
        cluster: true,
    })?;
    println!(
        "{}",
        serde_json::json!({"nodes": report.nodes, "edges": report.edges})
    );
    Ok(())
}

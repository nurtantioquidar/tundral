//! `dijkstral watch [PATH]` (`docs/graphify-git.md` §5), run by `dijkstral` in this binary: rebuild
//! the graph with `dijkstral update` once the files under PATH have been quiet for the debounce
//! time. Only changes to files a build
//! would read count (the corpus after ignore rules, skipped directories and the sensitive-file
//! filter), so the build's own writes under `dijkstral-out/` never trigger another one. Ctrl-C
//! stops it between rebuilds; a second Ctrl-C exits at once.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use dijkstral_core::discover::{discover_all, SKIP_DIRS};
use dijkstral_core::pipeline::OUT_DIR;
use notify::{RecursiveMode, Watcher};

const TAG: &str = "[dijkstral watch]";
/// Files whose edits change what the corpus is.
const IGNORE_FILES: &[&str] = &[".gitignore", ".graphifyignore", ".dijkstralignore"];

enum Msg {
    Changed(Vec<PathBuf>),
    Failed(String),
    Stop,
}

/// Watch `path`, running `exe update` (the `dijkstral` binary that started this one).
pub fn run(exe: &Path, path: &Path, debounce: f64) -> ExitCode {
    match watch(exe, path, debounce) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn watch(exe: &Path, path: &Path, secs: f64) -> anyhow::Result<()> {
    use anyhow::Context;
    let root = path
        .canonicalize()
        .with_context(|| format!("reading {}", path.display()))?;
    anyhow::ensure!(root.is_dir(), "{} is not a directory", root.display());
    anyhow::ensure!(
        secs.is_finite() && secs >= 0.0,
        "--debounce must be a number of seconds"
    );
    let debounce = Duration::from_secs_f64(secs);

    let (tx, rx) = mpsc::channel();
    let stopping = Arc::new(AtomicBool::new(false));
    {
        let tx = tx.clone();
        let stopping = stopping.clone();
        ctrlc::set_handler(move || {
            if stopping.swap(true, Ordering::SeqCst) {
                std::process::exit(130);
            }
            let _ = tx.send(Msg::Stop);
        })
        .context("installing the Ctrl-C handler")?;
    }
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let _ = tx.send(match res {
            Ok(ev) => Msg::Changed(ev.paths),
            Err(e) => Msg::Failed(e.to_string()),
        });
    })
    .context("starting the file watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", root.display()))?;

    println!("{TAG} watching {} (Ctrl-C stops)", root.display());
    println!(
        "{TAG} code and document changes run `dijkstral update` after {}s of quiet; no model is \
         called",
        secs
    );
    let mut known = corpus(&root);
    update(exe, &root);

    let out = root.join(OUT_DIR);
    let mut stop = false;
    while !stop {
        let mut changed = BTreeSet::new();
        // Wait for a change that is not noise, then for the quiet period.
        loop {
            match rx.recv() {
                Ok(Msg::Changed(paths)) => changed.extend(candidates(&root, &out, paths)),
                Ok(Msg::Failed(e)) => eprintln!("{TAG} watcher error: {e}"),
                Ok(Msg::Stop) | Err(_) => stop = true,
            }
            if stop || !changed.is_empty() {
                break;
            }
        }
        while !stop {
            match rx.recv_timeout(debounce) {
                Ok(Msg::Changed(paths)) => changed.extend(candidates(&root, &out, paths)),
                Ok(Msg::Failed(e)) => eprintln!("{TAG} watcher error: {e}"),
                Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => stop = true,
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        if stop {
            break;
        }
        let now = corpus(&root);
        let mut relevant: Vec<String> = changed
            .iter()
            .filter_map(|p| relevant(&root, p, &known, &now))
            .collect();
        // A new directory is reported by the files in it.
        let all = relevant.clone();
        relevant.retain(|r| !all.iter().any(|o| o.starts_with(&format!("{r}/"))));
        known = now;
        if relevant.is_empty() {
            continue;
        }
        println!();
        println!("{TAG} {}", describe(&relevant));
        update(exe, &root);
    }
    drop(watcher);
    println!("{TAG} stopped");
    Ok(())
}

/// Paths under the root, outside the output directory and the always-skipped directories.
fn candidates(root: &Path, out: &Path, paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|p| {
            let Ok(rel) = p.strip_prefix(root) else {
                return false;
            };
            !p.starts_with(out)
                && !rel.components().any(|c| {
                    let name = c.as_os_str().to_string_lossy();
                    SKIP_DIRS.contains(&name.as_ref()) || name.ends_with(".egg-info")
                })
        })
        .collect()
}

/// Every file a build walks, relative to the root.
fn corpus(root: &Path) -> BTreeSet<String> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let found = discover_all(root, threads);
    found
        .files
        .into_iter()
        .map(|f| f.rel)
        .chain(found.documents)
        .chain(found.unclassified)
        .collect()
}

/// The changed path's relative name when it touches the corpus: a file walked before or after
/// the change, an ignore file, or a directory that held or now holds such files.
fn relevant(
    root: &Path,
    p: &Path,
    before: &BTreeSet<String>,
    after: &BTreeSet<String>,
) -> Option<String> {
    let rel = p
        .strip_prefix(root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    if rel.is_empty() {
        return None;
    }
    let name = rel.rsplit('/').next().unwrap_or(&rel);
    if IGNORE_FILES.contains(&name) || before.contains(&rel) || after.contains(&rel) {
        return Some(rel);
    }
    let prefix = format!("{rel}/");
    let (was, is) = (under(before, &prefix), under(after, &prefix));
    (was != is).then_some(rel)
}

fn under<'a>(set: &'a BTreeSet<String>, prefix: &str) -> Vec<&'a str> {
    set.range(prefix.to_string()..)
        .take_while(|f| f.starts_with(prefix))
        .map(String::as_str)
        .collect()
}

fn describe(rel: &[String]) -> String {
    const SHOWN: usize = 5;
    let mut s = format!(
        "{} changed: {}",
        rel.len(),
        rel.iter()
            .take(SHOWN)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
    if rel.len() > SHOWN {
        s.push_str(&format!(" (+{} more)", rel.len() - SHOWN));
    }
    s
}

/// Run `dijkstral update <root>` with this terminal as its output.
fn update(exe: &Path, root: &Path) {
    match Command::new(exe).arg("update").arg(root).status() {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("{TAG} update failed ({s}); still watching"),
        Err(e) => eprintln!("{TAG} could not run update: {e}; still watching"),
    }
}

//! What the hook block runs (`dijkstral hook run <hook> <git's arguments>`): decide whether the
//! graph needs a rebuild, then hand it to a detached worker that rebuilds with `dijkstral update`
//! while git carries on. Workers serialise on a lock file and coalesce: a trigger that finds a
//! rebuild running leaves a note, and the running worker rebuilds once more (§2.3).
//!
//! Nothing here may fail a git command: callers ignore every error, and the hook block itself
//! discards output and exit status.

use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use dijkstral_core::pipeline::OUT_DIR;

/// Set (non-empty) to make the hooks do nothing.
pub const SKIP_ENV: &str = "DIJKSTRAL_HOOK_SKIP";
/// Set to `1` to rebuild before the hook returns instead of in the background.
pub const SYNC_ENV: &str = "DIJKSTRAL_HOOK_SYNC";
/// The worker's argument in place of a hook name.
pub const WORKER: &str = "rebuild";

pub const LOCK_FILE: &str = ".hook.lock";
pub const PENDING_FILE: &str = ".hook.pending";
/// Output of the last hook rebuild.
pub const LOG_FILE: &str = "hook.log";

/// Rebuild rounds one worker runs before leaving the rest to the next trigger.
const MAX_ROUNDS: usize = 8;

/// Whether `hook` with git's `args` should refresh the graph of the work tree at `top`.
pub fn wanted(top: &Path, hook: &str, args: &[String]) -> bool {
    if std::env::var_os(SKIP_ENV).is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if !top.join(OUT_DIR).join("graph.json").is_file() {
        // No graph yet: building one is the user's call.
        return false;
    }
    match hook {
        "post-commit" => true,
        // `<previous HEAD> <new HEAD> <1 for a branch checkout, 0 for files>`.
        "post-checkout" => args.len() >= 3 && args[2] == "1" && args[0] != args[1],
        _ => false,
    }
}

/// Run the hook: start the worker detached (or, with [`SYNC_ENV`], rebuild now).
pub fn run(top: &Path, hook: &str, args: &[String], exe: &Path) -> Result<()> {
    if hook == WORKER {
        return work(top, exe);
    }
    if !wanted(top, hook, args) {
        return Ok(());
    }
    if std::env::var_os(SYNC_ENV).is_some_and(|v| v == "1") {
        return work(top, exe);
    }
    let mut cmd = Command::new(exe);
    cmd.args(["hook", "run", WORKER])
        .current_dir(top)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    clean_git_env(&mut cmd);
    #[cfg(unix)]
    {
        // Out of git's process group: a Ctrl-C aimed at git does not reach the rebuild.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().context("starting the rebuild")?;
    Ok(())
}

/// Git exports these to hooks; the rebuild finds the repository from its directory instead.
fn clean_git_env(cmd: &mut Command) {
    for k in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_PREFIX"] {
        cmd.env_remove(k);
    }
}

/// The worker: rebuild under the lock until no trigger is pending.
fn work(top: &Path, exe: &Path) -> Result<()> {
    let out = top.join(OUT_DIR);
    let pending = out.join(PENDING_FILE);
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(out.join(LOCK_FILE))
        .context("opening the hook lock")?;
    let mut rounds = 0;
    loop {
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                // Another worker is rebuilding: ask it for one more round. If it let go in the
                // meantime, take over so the request cannot be lost.
                fs::write(&pending, b"")?;
                match lock.try_lock() {
                    Ok(()) => {}
                    Err(TryLockError::WouldBlock) => return Ok(()),
                    Err(TryLockError::Error(e)) => return Err(e.into()),
                }
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
        while rounds < MAX_ROUNDS {
            rounds += 1;
            fs::remove_file(&pending).ok();
            update(top, &out, exe)?;
            if !pending.exists() {
                break;
            }
        }
        lock.unlock()?;
        if rounds >= MAX_ROUNDS || !pending.exists() {
            return Ok(());
        }
    }
}

/// One `dijkstral update <top>`, its output in `dijkstral-out/hook.log`.
fn update(top: &Path, out: &Path, exe: &Path) -> Result<()> {
    let log = File::create(out.join(LOG_FILE)).context("creating the hook log")?;
    let mut cmd = Command::new(exe);
    cmd.arg("update")
        .arg(top)
        .current_dir(top)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    clean_git_env(&mut cmd);
    cmd.status().context("running dijkstral update")?;
    Ok(())
}

/// The work tree a hook runs in: git starts hooks at its top.
pub fn hook_top() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_rebuilds_only_on_a_branch_switch() {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path();
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!wanted(top, "post-commit", &[]), "no graph, no rebuild");
        fs::create_dir_all(top.join(OUT_DIR)).unwrap();
        fs::write(top.join(OUT_DIR).join("graph.json"), "{}").unwrap();
        assert!(wanted(top, "post-commit", &[]));
        assert!(wanted(top, "post-checkout", &args(&["a", "b", "1"])));
        assert!(!wanted(top, "post-checkout", &args(&["a", "b", "0"])));
        assert!(!wanted(top, "post-checkout", &args(&["a", "a", "1"])));
        assert!(!wanted(top, "post-merge", &args(&["0"])));
    }
}

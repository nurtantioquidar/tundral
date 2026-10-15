//! The repository `hook` commands act on, through the `git` binary: its work tree, the hooks
//! directory git actually runs (`core.hooksPath` and linked worktrees included) and its local
//! config.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

pub struct Repo {
    /// Top of the work tree.
    pub top: PathBuf,
    /// Where git looks for hooks: `core.hooksPath` when set, else the common `.git/hooks`.
    pub hooks_dir: PathBuf,
}

impl Repo {
    /// The work tree containing `dir`; `None` outside one (a bare repository has none).
    pub fn find(dir: &Path) -> Result<Option<Repo>> {
        let Some(top) = git(dir, &["rev-parse", "--show-toplevel"])? else {
            return Ok(None);
        };
        if top.is_empty() {
            return Ok(None);
        }
        let top = PathBuf::from(top);
        let hooks = git(&top, &["rev-parse", "--git-path", "hooks"])?
            .context("git rev-parse --git-path hooks failed")?;
        // Relative to the directory git ran in, which is the top.
        let hooks_dir = top.join(hooks);
        Ok(Some(Repo { top, hooks_dir }))
    }

    pub fn hook_path(&self, hook: &str) -> PathBuf {
        self.hooks_dir.join(hook)
    }

    /// A key of the repository's own config file (`--local`).
    pub fn config_get(&self, key: &str) -> Result<Option<String>> {
        git(&self.top, &["config", "--local", "--get", key])
    }

    pub fn config_set(&self, key: &str, value: &str) -> Result<()> {
        if git(&self.top, &["config", "--local", key, value])?.is_none() {
            bail!("git config --local {key} failed");
        }
        Ok(())
    }

    /// Remove a whole `[section]` from the local config; false when it was not there.
    pub fn config_remove_section(&self, section: &str) -> Result<bool> {
        Ok(git(
            &self.top,
            &["config", "--local", "--remove-section", section],
        )?
        .is_some())
    }
}

/// Run `git -C dir args…`: trimmed stdout on success, `None` when git exits non-zero.
fn git(dir: &Path, args: &[&str]) -> Result<Option<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("running git (is it installed?)")?;
    Ok(out
        .status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_string()))
}

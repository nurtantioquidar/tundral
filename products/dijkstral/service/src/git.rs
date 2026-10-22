//! Fetching a repository's default branch with `git`, hardened for untrusted repositories.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

use crate::config::SourceConfig;

/// Where a repository is fetched from. The URL may carry a credential, so it never appears in
/// errors or logs: failures name the repository instead.
pub struct Source {
    config: SourceConfig,
}

impl Source {
    pub fn new(config: SourceConfig) -> Source {
        Source { config }
    }

    fn url(&self, full_name: &str) -> Result<String> {
        match &self.config {
            SourceConfig::Local(base) => {
                let p = base.join(format!("{full_name}.git"));
                Ok(format!("file://{}", p.display()))
            }
        }
    }

    /// The default branch a repository's `HEAD` points to.
    pub async fn default_branch(&self, full_name: &str) -> Result<String> {
        let url = self.url(full_name)?;
        let out = git(None, &["ls-remote", "--symref", &url, "HEAD"], full_name).await?;
        out.lines()
            .find_map(|l| {
                l.strip_prefix("ref: refs/heads/")
                    .and_then(|r| r.split_whitespace().next())
            })
            .map(str::to_string)
            .with_context(|| format!("{full_name}: no default branch"))
    }

    /// Fetches the tip of `branch` into a fresh `dir` (depth 1, no tags, no submodules, no hooks)
    /// and checks it out; returns its commit.
    pub async fn checkout(&self, full_name: &str, branch: &str, dir: &Path) -> Result<String> {
        let url = self.url(full_name)?;
        let d = dir.to_str().context("checkout path is not UTF-8")?;
        git(None, &["init", "-q", d], full_name).await?;
        git(
            Some(dir),
            &[
                "fetch",
                "-q",
                "--depth",
                "1",
                "--no-tags",
                "--no-recurse-submodules",
                &url,
                &format!("refs/heads/{branch}"),
            ],
            full_name,
        )
        .await?;
        git(
            Some(dir),
            &["checkout", "-q", "--force", "FETCH_HEAD"],
            full_name,
        )
        .await?;
        Ok(git(Some(dir), &["rev-parse", "HEAD"], full_name)
            .await?
            .trim()
            .to_string())
    }
}

const GIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs `git` with the user's and the system's configuration shut out, hooks off, no prompts and
/// LFS content left unfetched. Errors name `what`, never the command line.
async fn git(dir: Option<&Path>, args: &[&str], what: &str) -> Result<String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd.args(["-c", "core.hooksPath=/dev/null"])
        .args(["-c", "core.fsmonitor=false"])
        .args(["-c", "protocol.allow=never"])
        .args(["-c", "protocol.https.allow=always"])
        .args(["-c", "protocol.file.allow=always"])
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(GIT_TIMEOUT, cmd.output())
        .await
        .with_context(|| format!("{what}: git {} timed out", args[0]))?
        .with_context(|| format!("{what}: git could not start"))?;
    if !out.status.success() {
        bail!("{what}: git {} failed", args[0]);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

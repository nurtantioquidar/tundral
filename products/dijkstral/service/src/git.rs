//! Fetching a repository's default branch with `git`, hardened for untrusted repositories.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

use base64::Engine;

use crate::config::SourceConfig;
use crate::github::Tokens;

/// Where a repository is fetched from. Credentials reach `git` only through its environment
/// (`GIT_CONFIG_*`), never its arguments, and failures name the repository, never the URL.
pub struct Source {
    kind: Kind,
}

enum Kind {
    Local(std::path::PathBuf),
    GitHub(Tokens),
}

impl Source {
    pub fn new(config: SourceConfig) -> Result<Source> {
        Ok(Source {
            kind: match config {
                SourceConfig::Local(base) => Kind::Local(base),
                SourceConfig::GitHub(app) => Kind::GitHub(Tokens::new(app)?),
            },
        })
    }

    fn url(&self, full_name: &str) -> String {
        match &self.kind {
            Kind::Local(base) => {
                format!("file://{}", base.join(format!("{full_name}.git")).display())
            }
            Kind::GitHub(t) => format!("{}/{full_name}.git", t.app().web),
        }
    }

    /// The environment that authenticates `git` for `installation`: an `Authorization` header
    /// for the GitHub host only, from a fresh installation token.
    pub async fn auth_env(&self, installation: i64) -> Result<Vec<(String, String)>> {
        let Kind::GitHub(t) = &self.kind else {
            return Ok(Vec::new());
        };
        let token = t.installation(installation).await?;
        let basic =
            base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
        Ok(vec![
            ("GIT_CONFIG_COUNT".into(), "1".into()),
            (
                "GIT_CONFIG_KEY_0".into(),
                format!("http.{}/.extraheader", t.app().web),
            ),
            (
                "GIT_CONFIG_VALUE_0".into(),
                format!("AUTHORIZATION: basic {basic}"),
            ),
        ])
    }

    /// Drops what is cached for an uninstalled installation.
    pub async fn forget(&self, installation: i64) {
        if let Kind::GitHub(t) = &self.kind {
            t.forget(installation).await;
        }
    }

    /// The default branch a repository's `HEAD` points to.
    pub async fn default_branch(&self, installation: i64, full_name: &str) -> Result<String> {
        let env = self.auth_env(installation).await?;
        let url = self.url(full_name);
        let out = git(
            None,
            &["ls-remote", "--symref", &url, "HEAD"],
            &env,
            full_name,
        )
        .await?;
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
    pub async fn checkout(
        &self,
        installation: i64,
        full_name: &str,
        branch: &str,
        dir: &Path,
    ) -> Result<String> {
        let env = self.auth_env(installation).await?;
        let url = self.url(full_name);
        let d = dir.to_str().context("checkout path is not UTF-8")?;
        git(None, &["init", "-q", d], &[], full_name).await?;
        let branch_ref = format!("refs/heads/{branch}");
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
                &branch_ref,
            ],
            &env,
            full_name,
        )
        .await?;
        git(
            Some(dir),
            &["checkout", "-q", "--force", "FETCH_HEAD"],
            &[],
            full_name,
        )
        .await?;
        Ok(git(Some(dir), &["rev-parse", "HEAD"], &[], full_name)
            .await?
            .trim()
            .to_string())
    }
}

const GIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs `git` with the user's and the system's configuration shut out, hooks off, no prompts and
/// LFS content left unfetched. Errors name `what`, never the command line.
async fn git(
    dir: Option<&Path>,
    args: &[&str],
    env: &[(String, String)],
    what: &str,
) -> Result<String> {
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
        .envs(env.iter().map(|(k, v)| (k, v)))
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

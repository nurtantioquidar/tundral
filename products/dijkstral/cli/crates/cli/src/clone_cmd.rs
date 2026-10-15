//! `dijkstral clone <github-url>` (`docs/graphify-multirepo.md` §6): a shallow clone under
//! `$DIJKSTRAL_HOME/repos/<owner>/<repo>` (or `--out`), updated with a fast-forward pull when it
//! is already there. Progress goes to stderr; stdout is the clone's path alone, so
//! `cd "$(dijkstral clone URL)"` works.
//!
//! `DIJKSTRAL_CLONE_BASE` (test-only) replaces the remote with `<base>/<owner>/<repo>.git`, so
//! tests clone a local `file://` repository instead of reaching GitHub.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{bail, Context, Result};
use clap::Args;
use dijkstral_core::multirepo::dijkstral_home;

#[derive(Args, Debug)]
pub struct CloneArgs {
    /// A GitHub repository: https://github.com/<owner>/<repo>[.git], git@github.com:<owner>/<repo>.git
    /// or github.com/<owner>/<repo>.
    url: String,
    /// Branch to clone (default: the repository's default branch).
    #[arg(long, value_name = "BRANCH")]
    branch: Option<String>,
    /// Directory to clone into (default: $DIJKSTRAL_HOME/repos/<owner>/<repo>).
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
}

/// A recognised GitHub repository reference.
#[derive(Debug, PartialEq, Eq)]
pub struct GitHubRepo {
    pub owner: String,
    pub repo: String,
    /// What `git clone` is given.
    pub remote: String,
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Parse the URL forms Graphify accepts (§6.2). A scheme-less `github.com/<owner>/<repo>` is
/// cloned over https; anything with more path, a query or a fragment is rejected.
pub fn parse_github(url: &str) -> Option<GitHubRepo> {
    let (prefix, rest) = if let Some(rest) = url.strip_prefix("git@github.com:") {
        ("git@github.com:".to_string(), rest)
    } else {
        let (scheme, after) = match url.split_once("://") {
            Some((s @ ("http" | "https"), after)) => (s, after),
            Some(_) => return None,
            None => ("https", url),
        };
        let (host, rest) = after.split_once('/')?;
        if host != "github.com" && host != "www.github.com" {
            return None;
        }
        (format!("{scheme}://{host}/"), rest)
    };
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    if !valid_name(owner) || !valid_name(repo) {
        return None;
    }
    let remote = match std::env::var("DIJKSTRAL_CLONE_BASE") {
        Ok(base) if !base.is_empty() => {
            format!("{}/{owner}/{repo}.git", base.trim_end_matches('/'))
        }
        _ => format!("{prefix}{owner}/{repo}.git"),
    };
    Some(GitHubRepo {
        owner: owner.into(),
        repo: repo.into(),
        remote,
    })
}

fn git() -> Command {
    let mut c = Command::new("git");
    // Never wait on a credential prompt: a private or missing repository fails at once.
    c.env("GIT_TERMINAL_PROMPT", "0").stdout(Stdio::null());
    c
}

pub fn run(a: CloneArgs) -> ExitCode {
    match clone(&a) {
        Ok(dest) => {
            println!("{}", dest.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn clone(a: &CloneArgs) -> Result<PathBuf> {
    let Some(gh) = parse_github(&a.url) else {
        bail!(
            "{} is not a GitHub repository URL (https://github.com/<owner>/<repo>)",
            a.url
        );
    };
    let dest = match &a.out {
        Some(out) => out.clone(),
        None => dijkstral_home()?
            .join("repos")
            .join(&gh.owner)
            .join(&gh.repo),
    };
    if dest.join(".git").exists() {
        eprintln!("{} is already cloned; pulling", dest.display());
        pull(&dest, a.branch.as_deref());
        return Ok(dest);
    }
    if dest.exists() && !is_empty_dir(&dest)? {
        bail!(
            "{} exists and is not a git clone; choose another --out",
            dest.display()
        );
    }
    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    eprintln!("cloning {} into {}", gh.remote, dest.display());
    let mut cmd = git();
    cmd.args(["clone", "--depth", "1"]);
    if let Some(b) = &a.branch {
        cmd.args(["--branch", b]);
    }
    cmd.arg(&gh.remote).arg(&dest);
    let status = cmd.status().context("running git")?;
    if !status.success() {
        bail!("git clone of {} failed", gh.remote);
    }
    Ok(dest)
}

fn is_empty_dir(p: &Path) -> Result<bool> {
    Ok(p.is_dir()
        && std::fs::read_dir(p)
            .with_context(|| format!("reading {}", p.display()))?
            .next()
            .is_none())
}

/// Fast-forward an existing clone (from `origin <branch>` when given). A failure is a warning:
/// the clone is still usable as it is.
fn pull(dest: &Path, branch: Option<&str>) {
    let mut cmd = git();
    cmd.arg("-C").arg(dest).args(["pull", "--ff-only"]);
    if let Some(b) = branch {
        cmd.args(["origin", b]);
    }
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(_) => eprintln!("warning: git pull failed; using the clone as it is"),
        Err(e) => eprintln!("warning: could not run git pull: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(url: &str) -> Option<(String, String, String)> {
        parse_github(url).map(|g| (g.owner, g.repo, g.remote))
    }

    #[test]
    fn url_forms() {
        let want = |remote: &str| {
            Some((
                "octocat".to_string(),
                "Hello-World".to_string(),
                remote.to_string(),
            ))
        };
        let https = "https://github.com/octocat/Hello-World.git";
        assert_eq!(
            parsed("https://github.com/octocat/Hello-World"),
            want(https)
        );
        assert_eq!(
            parsed("https://github.com/octocat/Hello-World.git"),
            want(https)
        );
        assert_eq!(
            parsed("https://github.com/octocat/Hello-World/"),
            want(https)
        );
        assert_eq!(parsed("github.com/octocat/Hello-World.git"), want(https));
        assert_eq!(
            parsed("http://github.com/octocat/Hello-World/"),
            want("http://github.com/octocat/Hello-World.git")
        );
        assert_eq!(
            parsed("https://www.github.com/octocat/Hello-World"),
            want("https://www.github.com/octocat/Hello-World.git")
        );
        assert_eq!(
            parsed("git@github.com:octocat/Hello-World.git"),
            want("git@github.com:octocat/Hello-World.git")
        );
        for bad in [
            "octocat/Hello-World",
            "https://github.com/octocat/Hello-World/tree/master",
            "https://gitlab.com/octocat/Hello-World",
            "not-a-url",
            "https://github.com/octocat/Hello-World?x=1",
            "https://github.com/octocat/Hello-World#frag",
            "https://github.com/octocat/..",
            "ftp://github.com/octocat/Hello-World",
        ] {
            assert_eq!(parsed(bad), None, "{bad}");
        }
    }
}

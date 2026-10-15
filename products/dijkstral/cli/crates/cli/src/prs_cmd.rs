//! `prs` (`docs/graphify-memory.md` §5): the open pull requests `gh` lists, as a dashboard or one
//! PR in detail. Every `gh` and `git` call is read-only.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use dijkstral_serve::prs::{self, Gh, Listing, GH_UNAVAILABLE};
use dijkstral_serve::{clock, load};

use crate::work_cmds::project_graph;

#[derive(Args, Debug)]
#[command(args_override_self = true)]
pub struct PrsArgs {
    /// Show this PR in detail (`201` or `#201`): branch, author, CI, review, worktree, and the
    /// graph nodes and communities its changed files touch.
    #[arg(value_name = "NUMBER", value_parser = parse_number)]
    number: Option<i64>,
    /// Base branch the PRs should target (default: the repository's default branch).
    #[arg(long, value_name = "BRANCH")]
    base: Option<String>,
    /// GitHub repository (OWNER/REPO) instead of the current one.
    #[arg(long, value_name = "OWNER/REPO")]
    repo: Option<String>,
    /// Graph for the detail view's impact. Default: dijkstral-out/graph.dij or graph.json, else
    /// graphify-out/graph.json.
    #[arg(long, value_name = "PATH")]
    graph: Option<PathBuf>,
}

fn parse_number(s: &str) -> Result<i64, String> {
    s.strip_prefix('#')
        .unwrap_or(s)
        .parse::<i64>()
        .map_err(|_| format!("not a PR number: {s}"))
}

pub fn run(a: PrsArgs) -> ExitCode {
    let cwd = Path::new(".");
    let gh = Gh::new(cwd);
    let repo = a.repo.as_deref();
    let base = a.base.clone().unwrap_or_else(|| gh.default_branch(repo));
    let prs = match gh.list(repo) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("  {GH_UNAVAILABLE}");
            return ExitCode::FAILURE;
        }
    };
    let now = clock::now();
    let listing = Listing::new(base, prs, now);
    let worktrees = prs::worktrees(cwd);
    let Some(number) = a.number else {
        print!("{}", prs::render_dashboard(&listing, &worktrees, now));
        return ExitCode::SUCCESS;
    };
    let Some((pr, status)) = listing.find(number) else {
        eprintln!("  PR #{number} is not among the open pull requests.");
        return ExitCode::FAILURE;
    };
    let graph = a.graph.clone().or_else(|| project_graph(cwd));
    let index = graph.and_then(|g| load(&g).ok()).map(|(ix, _)| ix);
    let files = index
        .as_ref()
        .and_then(|_| gh.changed_files(number, repo))
        .unwrap_or_default();
    let impact = index.as_ref().map(|ix| prs::impact(ix, &files));
    let detail = prs::render_detail(
        pr,
        *status,
        worktrees.get(&pr.head).map(PathBuf::as_path),
        impact.as_ref().map(|i| (files.as_slice(), i)),
        now,
    );
    print!("{detail}");
    ExitCode::SUCCESS
}

//! `prs` and the MCP PR tools (`docs/graphify-memory.md` §5): open pull requests from `gh`, their
//! CI and review state, the worktrees that have them checked out, and their footprint in the graph.
//!
//! Every `gh` and `git` call is read-only. The states, ranks, ages and line formats are Graphify's;
//! headers and messages are dijkstral's own.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::clock;
use crate::Index;

/// `gh pr list` returns at most this many open PRs.
pub const LIST_LIMIT: usize = 50;
/// The fields every listing asks `gh` for.
pub const LIST_FIELDS: &str =
    "number,title,headRefName,baseRefName,author,isDraft,reviewDecision,statusCheckRollup,updatedAt";
/// The fields `get_pr_impact` asks for one PR.
pub const VIEW_FIELDS: &str =
    "title,headRefName,baseRefName,author,isDraft,reviewDecision,statusCheckRollup,updatedAt";
/// The base used when the default branch cannot be read.
pub const FALLBACK_BASE: &str = "main";
/// A PR untouched this many days is stale.
pub const STALE_DAYS: i64 = 14;
/// Titles longer than this are cut in the dashboard.
const TITLE_CHARS: usize = 52;
/// Changed files listed by the detail view and by `get_pr_impact`.
const DETAIL_FILES: usize = 10;
const MCP_FILES: usize = 20;
/// Concurrent `gh pr diff` calls for `triage_prs`.
const DIFF_WORKERS: usize = 8;

/// The aggregate CI state of a PR (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ci {
    Failure,
    Pending,
    Success,
    None,
}

impl Ci {
    pub fn name(self) -> &'static str {
        match self {
            Ci::Failure => "FAILURE",
            Ci::Pending => "PENDING",
            Ci::Success => "SUCCESS",
            Ci::None => "NONE",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Ci::Failure => "✗",
            Ci::Pending => "…",
            Ci::Success => "✓",
            Ci::None => "–",
        }
    }

    /// From `statusCheckRollup`: only `CheckRun` items count.
    pub fn from_rollup(rollup: Option<&Value>) -> Ci {
        let runs: Vec<&Value> = rollup
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|c| c.get("__typename").and_then(Value::as_str) == Some("CheckRun"))
            .collect();
        fn conclusion(c: &Value) -> &str {
            c.get("conclusion").and_then(Value::as_str).unwrap_or("")
        }
        fn status(c: &Value) -> &str {
            c.get("status").and_then(Value::as_str).unwrap_or("")
        }
        if runs.iter().any(|c| {
            matches!(
                conclusion(c),
                "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
            )
        }) {
            Ci::Failure
        } else if runs
            .iter()
            .any(|c| conclusion(c).is_empty() && matches!(status(c), "IN_PROGRESS" | "QUEUED"))
        {
            Ci::Pending
        } else if runs.iter().any(|c| conclusion(c) == "SUCCESS") {
            Ci::Success
        } else {
            Ci::None
        }
    }
}

/// The dashboard status of a PR (§5.3), in rank order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    CiFail,
    ChangesReq,
    Draft,
    Stale,
    Pending,
    Approved,
    Ready,
    WrongBase,
}

impl Status {
    pub const RANKED: [Status; 7] = [
        Status::CiFail,
        Status::ChangesReq,
        Status::Draft,
        Status::Stale,
        Status::Pending,
        Status::Approved,
        Status::Ready,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Status::CiFail => "CI-FAIL",
            Status::ChangesReq => "CHANGES-REQ",
            Status::Draft => "DRAFT",
            Status::Stale => "STALE",
            Status::Pending => "PENDING",
            Status::Approved => "APPROVED",
            Status::Ready => "READY",
            Status::WrongBase => "WRONG-BASE",
        }
    }

    /// The summary label (dashboard footer), in [`Status::RANKED`] order of display.
    fn label(self) -> &'static str {
        match self {
            Status::Ready => "ready",
            Status::Approved => "approved",
            Status::Pending => "CI pending",
            Status::CiFail => "CI failing",
            Status::ChangesReq => "changes requested",
            Status::Draft => "draft",
            Status::Stale => "stale",
            Status::WrongBase => "on another base",
        }
    }
}

/// One open PR as `gh` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Pr {
    pub number: i64,
    pub title: String,
    pub head: String,
    pub base: String,
    /// `None` when `gh` reports no author (a deleted account).
    pub author: Option<String>,
    pub draft: bool,
    /// `reviewDecision`, `""` when none.
    pub review: String,
    pub ci: Ci,
    /// `updatedAt` as Unix seconds; `None` when missing or unreadable.
    pub updated: Option<f64>,
}

impl Pr {
    pub fn from_json(v: &Value) -> Option<Pr> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Some(Pr {
            number: v.get("number")?.as_i64()?,
            title: s("title"),
            head: s("headRefName"),
            base: s("baseRefName"),
            author: match v.get("author") {
                Some(Value::Object(a)) => Some(
                    a.get("login")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ),
                _ => None,
            },
            draft: v.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
            review: s("reviewDecision"),
            ci: Ci::from_rollup(v.get("statusCheckRollup")),
            updated: v
                .get("updatedAt")
                .and_then(Value::as_str)
                .and_then(clock::parse_iso),
        })
    }

    /// Whole days since the last update, floored; negative in the future.
    pub fn age_days(&self, now: f64) -> Option<i64> {
        self.updated.map(|u| ((now - u) / 86_400.0).floor() as i64)
    }

    pub fn status(&self, base: &str, now: f64) -> Status {
        if self.base != base {
            Status::WrongBase
        } else if self.ci == Ci::Failure {
            Status::CiFail
        } else if self.review == "CHANGES_REQUESTED" {
            Status::ChangesReq
        } else if self.draft {
            Status::Draft
        } else if self.age_days(now).is_some_and(|d| d >= STALE_DAYS) {
            Status::Stale
        } else if self.review == "APPROVED" {
            Status::Approved
        } else if self.ci == Ci::Pending {
            Status::Pending
        } else {
            Status::Ready
        }
    }

    fn author_text(&self) -> &str {
        self.author.as_deref().unwrap_or("?")
    }

    fn review_text(&self) -> &str {
        if self.review.is_empty() {
            "none"
        } else {
            &self.review
        }
    }
}

/// Why `gh` gave no listing.
#[derive(Debug, Clone, PartialEq)]
pub enum GhError {
    /// `gh` missing, not logged in, failing or answering with something that is not a list.
    Unavailable,
}

/// Runs `gh` (the first `gh` on `PATH`, or `program`) in a directory.
#[derive(Debug, Clone)]
pub struct Gh {
    pub program: OsString,
    pub cwd: PathBuf,
}

impl Gh {
    pub fn new(cwd: &Path) -> Gh {
        Gh {
            program: "gh".into(),
            cwd: cwd.to_path_buf(),
        }
    }

    /// stdout of a successful call.
    pub fn run(&self, args: &[&str]) -> Option<String> {
        let out = Command::new(&self.program)
            .args(args)
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The repository's default branch, or [`FALLBACK_BASE`].
    pub fn default_branch(&self, repo: Option<&str>) -> String {
        let mut args = vec!["repo", "view"];
        args.extend(repo);
        args.extend(["--json", "defaultBranchRef"]);
        self.run(&args)
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| {
                v.get("defaultBranchRef")?
                    .get("name")?
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| FALLBACK_BASE.to_string())
    }

    /// Open PRs in `gh`'s order.
    pub fn list(&self, repo: Option<&str>) -> Result<Vec<Pr>, GhError> {
        let limit = LIST_LIMIT.to_string();
        let mut args = vec![
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            &limit,
            "--json",
            LIST_FIELDS,
        ];
        if let Some(r) = repo {
            args.extend(["--repo", r]);
        }
        let out = self.run(&args).ok_or(GhError::Unavailable)?;
        let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&out) else {
            return Err(GhError::Unavailable);
        };
        Ok(items.iter().filter_map(Pr::from_json).collect())
    }

    /// One PR by number (`gh pr view`); `number` is not part of the reply.
    pub fn view(&self, number: i64, repo: Option<&str>) -> Option<Pr> {
        let n = number.to_string();
        let mut args = vec!["pr", "view", n.as_str(), "--json", VIEW_FIELDS];
        if let Some(r) = repo {
            args.extend(["--repo", r]);
        }
        let mut v: Value = serde_json::from_str(&self.run(&args)?).ok()?;
        v.as_object_mut()?
            .insert("number".into(), Value::from(number));
        Pr::from_json(&v)
    }

    /// The files a PR changes; `None` when `gh` fails.
    pub fn changed_files(&self, number: i64, repo: Option<&str>) -> Option<Vec<String>> {
        let n = number.to_string();
        let mut args = vec!["pr", "diff", n.as_str(), "--name-only"];
        if let Some(r) = repo {
            args.extend(["--repo", r]);
        }
        let out = self.run(&args)?;
        Some(
            out.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }
}

/// A listing for one base.
#[derive(Debug, Clone)]
pub struct Listing {
    pub base: String,
    /// Every open PR, wrong base included, in dashboard order.
    pub prs: Vec<(Pr, Status)>,
}

impl Listing {
    /// Sorted by rank, then most recently updated (missing dates last), then `gh`'s order.
    pub fn new(base: String, prs: Vec<Pr>, now: f64) -> Listing {
        let mut prs: Vec<(Pr, Status)> = prs
            .into_iter()
            .map(|p| {
                let s = p.status(&base, now);
                (p, s)
            })
            .collect();
        prs.sort_by(|(a, sa), (b, sb)| {
            sa.cmp(sb).then_with(|| {
                let (ua, ub) = (
                    a.updated.unwrap_or(f64::NEG_INFINITY),
                    b.updated.unwrap_or(f64::NEG_INFINITY),
                );
                ub.partial_cmp(&ua).unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        Listing { base, prs }
    }

    /// The PRs on the base, in order.
    pub fn on_base(&self) -> impl Iterator<Item = &(Pr, Status)> {
        self.prs.iter().filter(|(_, s)| *s != Status::WrongBase)
    }

    pub fn wrong_base(&self) -> usize {
        self.prs.len() - self.on_base().count()
    }

    pub fn find(&self, number: i64) -> Option<&(Pr, Status)> {
        self.prs.iter().find(|(p, _)| p.number == number)
    }
}

/// Branch → worktree path, from `git worktree list --porcelain` in `cwd` (empty outside a
/// repository).
pub fn worktrees(cwd: &Path) -> HashMap<String, PathBuf> {
    let Ok(out) = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return HashMap::new();
    };
    if !out.status.success() {
        return HashMap::new();
    }
    let mut map = HashMap::new();
    let mut path: Option<PathBuf> = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            let p = PathBuf::from(p);
            path = Some(std::fs::canonicalize(&p).unwrap_or(p));
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            if let Some(p) = &path {
                map.entry(b.to_string()).or_insert_with(|| p.clone());
            }
        }
    }
    map
}

/// The graph nodes a set of changed files touches, and their communities (sorted).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Impact {
    pub nodes: usize,
    pub communities: Vec<u32>,
}

pub fn impact(ix: &Index, files: &[String]) -> Impact {
    let files: HashSet<&str> = files.iter().map(String::as_str).collect();
    let mut nodes = 0;
    let mut communities = BTreeSet::new();
    for i in 0..ix.node_count() as u32 {
        let n = ix.node(i);
        if files.contains(n.source_file) {
            nodes += 1;
            if let dijkstral_core::index::Community::Id(c) = n.community {
                communities.insert(c);
            }
        }
    }
    Impact {
        nodes,
        communities: communities.into_iter().collect(),
    }
}

fn int_list(v: &[u32]) -> String {
    let parts: Vec<String> = v.iter().map(u32::to_string).collect();
    format!("[{}]", parts.join(", "))
}

fn communities_word(n: usize) -> &'static str {
    if n == 1 {
        "community"
    } else {
        "communities"
    }
}

/// One line for display: line breaks become spaces.
fn one_line(s: &str) -> String {
    s.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

fn dashboard_title(p: &Pr) -> String {
    let title = one_line(&p.title);
    let mut t = if title.chars().count() > TITLE_CHARS {
        let cut: String = title.chars().take(TITLE_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        title
    };
    if p.draft {
        t.push_str(" [draft]");
    }
    t
}

fn age_cell(p: &Pr, now: f64) -> String {
    match p.age_days(now) {
        None => "?".into(),
        Some(d) if d <= 0 => "today".into(),
        Some(d) => format!("{d}d"),
    }
}

/// The dashboard (§5.4), ending with a newline.
pub fn render_dashboard(l: &Listing, worktrees: &HashMap<String, PathBuf>, now: f64) -> String {
    let rows: Vec<&(Pr, Status)> = l.on_base().collect();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n  dijkstral prs  ·  base: {}  ·  {} open\n",
        l.base,
        rows.len()
    );
    if rows.is_empty() {
        let _ = writeln!(out, "  No open pull requests target {}.\n", l.base);
    } else {
        let num_w = rows
            .iter()
            .map(|(p, _)| p.number.to_string().len() + 1)
            .max()
            .unwrap_or(1)
            .max(2);
        let _ = writeln!(
            out,
            "  {:<num_w$}  WT  CI  {:<11}  {:>7}  IMPACT  TITLE",
            "#", "STATUS", "UPDATED"
        );
        let _ = writeln!(
            out,
            "  {}  ──  ──  {}  {}  ──────  {}",
            "─".repeat(num_w),
            "─".repeat(11),
            "─".repeat(7),
            "─".repeat(40)
        );
        for (p, s) in rows {
            let wt = if worktrees.contains_key(&p.head) {
                "⬡"
            } else {
                " "
            };
            let _ = writeln!(
                out,
                "  {:<num_w$}  {wt}   {}   {:<11}  {:>7}  {:<6}  {}",
                format!("#{}", p.number),
                p.ci.glyph(),
                s.name(),
                age_cell(p, now),
                "–",
                dashboard_title(p)
            );
        }
        out.push('\n');
    }
    let mut counts: HashMap<Status, usize> = HashMap::new();
    for (_, s) in &l.prs {
        *counts.entry(*s).or_default() += 1;
    }
    let order = [
        Status::Ready,
        Status::Approved,
        Status::Pending,
        Status::CiFail,
        Status::ChangesReq,
        Status::Draft,
        Status::Stale,
        Status::WrongBase,
    ];
    let parts: Vec<String> = order
        .iter()
        .filter_map(|s| counts.get(s).map(|n| format!("{n} {}", s.label())))
        .collect();
    if !parts.is_empty() {
        let _ = writeln!(out, "  {}\n", parts.join(" · "));
    }
    out
}

/// The detail view of one PR (§5.5), ending with a newline. `files` are its changed files when a
/// graph is present.
pub fn render_detail(
    pr: &Pr,
    status: Status,
    worktree: Option<&Path>,
    files_and_impact: Option<(&[String], &Impact)>,
    now: f64,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\n  #{}  ·  {}", pr.number, status.name());
    let _ = writeln!(out, "  {}\n", one_line(&pr.title));
    let _ = writeln!(out, "  branch    {} -> {}", pr.head, pr.base);
    let _ = writeln!(out, "  author    {}", pr.author_text());
    let updated = match pr.age_days(now) {
        Some(d) => format!("{d}d ago"),
        None => "unknown".into(),
    };
    let _ = writeln!(out, "  updated   {updated}");
    let _ = writeln!(out, "  CI        {} {}", pr.ci.glyph(), pr.ci.name());
    if !pr.review.is_empty() {
        let _ = writeln!(out, "  review    {}", pr.review);
    }
    if let Some(w) = worktree {
        let _ = writeln!(out, "  worktree  {}", w.display());
    }
    if let Some((files, imp)) = files_and_impact.filter(|(_, i)| i.nodes > 0) {
        let _ = writeln!(
            out,
            "\n  impact    {} nodes in {} {} {}",
            imp.nodes,
            imp.communities.len(),
            communities_word(imp.communities.len()),
            int_list(&imp.communities)
        );
        let _ = writeln!(out, "  files     {} changed", files.len());
        for f in files.iter().take(DETAIL_FILES) {
            let _ = writeln!(out, "    {f}");
        }
        if files.len() > DETAIL_FILES {
            let _ = writeln!(out, "    … and {} more", files.len() - DETAIL_FILES);
        }
    }
    out.push('\n');
    out
}

/// `#<n> [<STATUS>] CI=<ci> review=<decision> age=<d>d author=<login>`.
fn mcp_line(p: &Pr, s: Status, now: f64) -> String {
    let age = p
        .age_days(now)
        .map_or_else(|| "?".to_string(), |d| d.to_string());
    format!(
        "#{} [{}] CI={} review={} age={age}d author={}",
        p.number,
        s.name(),
        p.ci.name(),
        p.review_text(),
        p.author_text()
    )
}

/// `list_prs` (§5.6).
pub fn mcp_list(l: &Listing, now: f64) -> String {
    let rows: Vec<&(Pr, Status)> = l.on_base().collect();
    let mut lines = vec![
        format!(
            "Open pull requests on {}: {}  ({} on other bases, not listed)",
            l.base,
            rows.len(),
            l.wrong_base()
        ),
        String::new(),
    ];
    for (p, s) in rows {
        lines.push(format!(
            "\n{}\n  {}",
            mcp_line(p, *s, now),
            one_line(&p.title)
        ));
    }
    lines.join("\n")
}

/// The diff of every PR in `numbers`, a few at a time.
pub fn diffs(gh: &Gh, numbers: &[i64], repo: Option<&str>) -> HashMap<i64, Option<Vec<String>>> {
    let mut out = HashMap::new();
    for chunk in numbers.chunks(DIFF_WORKERS) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|&n| scope.spawn(move || (n, gh.changed_files(n, repo))))
                .collect();
            for h in handles {
                if let Ok((n, files)) = h.join() {
                    out.insert(n, files);
                }
            }
        });
    }
    out
}

/// `triage_prs` (§5.6): the PRs that are not stale, by rank then number, with their footprint.
pub fn mcp_triage(
    l: &Listing,
    gh: &Gh,
    repo: Option<&str>,
    ix: Option<&Index>,
    worktrees: &HashMap<String, PathBuf>,
    now: f64,
) -> String {
    let mut rows: Vec<&(Pr, Status)> = l.on_base().filter(|(_, s)| *s != Status::Stale).collect();
    if rows.is_empty() {
        return format!("No pull requests to triage on {}.", l.base);
    }
    rows.sort_by_key(|(p, s)| (*s, p.number));
    let numbers: Vec<i64> = rows.iter().map(|(p, _)| p.number).collect();
    let files = if ix.is_some() {
        diffs(gh, &numbers, repo)
    } else {
        HashMap::new()
    };
    let mut lines = vec![
        format!("Pull requests to triage on {}: {}", l.base, rows.len()),
        "Order them for review; a larger blast_radius (more graph communities touched) means a riskier merge.".to_string(),
        String::new(),
    ];
    for (p, s) in rows {
        let mut head = format!("PR {}", mcp_line(p, *s, now));
        if let (Some(ix), Some(Some(f))) = (ix, files.get(&p.number)) {
            let imp = impact(ix, f);
            if imp.nodes > 0 {
                let c = imp.communities.len();
                let _ = write!(
                    head,
                    "  blast_radius={} nodes / {c} {}",
                    imp.nodes,
                    communities_word(c)
                );
            }
        }
        if let Some(w) = worktrees.get(&p.head) {
            let _ = write!(head, "  worktree={}", w.display());
        }
        lines.push(format!("\n{head}\n  title: {}", one_line(&p.title)));
    }
    lines.join("\n")
}

/// `get_pr_impact` (§5.6): `Err` text is a failure.
pub fn mcp_impact(
    gh: &Gh,
    number: i64,
    repo: Option<&str>,
    ix: Option<&Index>,
) -> Result<String, String> {
    let Some(pr) = gh.view(number, repo) else {
        return Err(format!(
            "PR #{number} could not be read: it does not exist here, or gh is missing or not logged in (gh auth login)."
        ));
    };
    let files = gh.changed_files(number, repo).unwrap_or_default();
    if files.is_empty() {
        return Ok(format!(
            "PR #{number}: gh listed no changed files (it may need gh auth login)."
        ));
    }
    let imp = ix.map(|ix| impact(ix, &files)).unwrap_or_default();
    let mut s = format!(
        "PR #{number}: {}\nCI: {}  Review: {}\nBase: {}  Author: {}\n\nGraph impact: {} nodes across {} communities\nCommunities touched: {}\nFiles changed ({}):",
        one_line(&pr.title),
        pr.ci.name(),
        pr.review_text(),
        pr.base,
        pr.author_text(),
        imp.nodes,
        imp.communities.len(),
        int_list(&imp.communities),
        files.len()
    );
    for f in files.iter().take(MCP_FILES) {
        let _ = write!(s, "\n  {f}");
    }
    if files.len() > MCP_FILES {
        let _ = write!(s, "\n  … and {} more", files.len() - MCP_FILES);
    }
    Ok(s)
}

/// The text of [`GhError::Unavailable`].
pub const GH_UNAVAILABLE: &str =
    "Error: gh is missing, not logged in, or could not list pull requests here. Run: gh auth login";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(c: Option<&str>, s: &str) -> Value {
        json!({"__typename": "CheckRun", "conclusion": c, "status": s})
    }

    #[test]
    fn ci_states() {
        let ci = |v: Value| Ci::from_rollup(Some(&v));
        assert_eq!(
            ci(json!([
                run(Some("SUCCESS"), "COMPLETED"),
                run(Some("FAILURE"), "COMPLETED")
            ])),
            Ci::Failure
        );
        assert_eq!(
            ci(json!([
                run(Some("SUCCESS"), "COMPLETED"),
                run(None, "IN_PROGRESS")
            ])),
            Ci::Pending
        );
        assert_eq!(ci(json!([run(Some(""), "QUEUED")])), Ci::Pending);
        assert_eq!(
            ci(json!([
                run(None, "PENDING"),
                run(Some("NEUTRAL"), "COMPLETED")
            ])),
            Ci::None
        );
        assert_eq!(ci(json!([run(Some("failure"), "COMPLETED")])), Ci::None);
        assert_eq!(
            ci(json!([{"__typename": "StatusContext", "state": "FAILURE"}])),
            Ci::None
        );
        assert_eq!(Ci::from_rollup(None), Ci::None);
    }

    fn pr(n: i64, days: f64, review: &str, draft: bool, ci: Ci) -> Pr {
        Pr {
            number: n,
            title: format!("t{n}"),
            head: format!("h{n}"),
            base: "main".into(),
            author: Some("a".into()),
            draft,
            review: review.into(),
            ci,
            updated: Some(1_000_000_000.0 - days * 86_400.0),
        }
    }

    #[test]
    fn status_precedence_and_rank() {
        let now = 1_000_000_000.0;
        assert_eq!(
            pr(1, 1.0, "CHANGES_REQUESTED", true, Ci::Failure).status("main", now),
            Status::CiFail
        );
        assert_eq!(
            pr(1, 1.0, "CHANGES_REQUESTED", true, Ci::Success).status("main", now),
            Status::ChangesReq
        );
        assert_eq!(
            pr(1, 30.0, "", true, Ci::Pending).status("main", now),
            Status::Draft
        );
        assert_eq!(
            pr(1, 14.0, "APPROVED", false, Ci::Pending).status("main", now),
            Status::Stale
        );
        assert_eq!(
            pr(1, 13.9, "APPROVED", false, Ci::Pending).status("main", now),
            Status::Approved
        );
        assert_eq!(
            pr(1, 1.0, "REVIEW_REQUIRED", false, Ci::Pending).status("main", now),
            Status::Pending
        );
        assert_eq!(
            pr(1, -3.0, "", false, Ci::None).status("dev", now),
            Status::WrongBase
        );
        let l = Listing::new(
            "main".into(),
            vec![
                pr(1, 2.0, "APPROVED", false, Ci::Success),
                pr(2, 1.0, "", false, Ci::Pending),
                pr(3, 0.5, "", false, Ci::Success),
                pr(4, 3.0, "", false, Ci::Success),
            ],
            now,
        );
        let order: Vec<i64> = l.prs.iter().map(|(p, _)| p.number).collect();
        assert_eq!(order, [2, 1, 3, 4]);
    }

    #[test]
    fn titles_are_cut_at_52_characters() {
        let mut p = pr(1, 1.0, "", false, Ci::None);
        p.title = "x".repeat(52);
        assert_eq!(dashboard_title(&p), "x".repeat(52));
        p.title = "y".repeat(53);
        p.draft = true;
        assert_eq!(dashboard_title(&p), format!("{}… [draft]", "y".repeat(51)));
    }
}

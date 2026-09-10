//! Deterministic verification. Butterfly does not trust the model saying it is
//! finished: in autonomous modes the loop runs acceptance commands, checks it
//! detects from the repository, and a workspace-diff test after the model
//! yields, and classifies whatever fails so the next attempt gets evidence
//! instead of a repeated prompt.
//!
//! Checks are argv vectors executed exactly like `run_command` (no shell,
//! credential-stripped environment, bounded output) and are never produced by
//! the model — only the caller (`--verify-cmd`), repository detection, or an
//! explicit `update_plan.verification` declaration the loop then runs itself.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::failure::{
    classify_command_output, evidence_tail, first_line, Failure, FailureKind, FailureSource,
    EVIDENCE_CAP_BYTES,
};
use crate::tools::command::{run_command, RunCommandArgs};
use crate::tools::policy::PolicySettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckSource {
    /// Supplied by the operator or benchmark harness.
    Explicit,
    /// Derived from manifests in the workspace.
    Detected,
    /// Declared by the model via `update_plan`; run, never trusted.
    ModelDeclared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub argv: Vec<String>,
    pub timeout_seconds: u64,
    pub source: CheckSource,
}

impl Check {
    pub fn label(&self) -> String {
        self.argv.join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub check: Check,
    pub ok: bool,
    pub duration_ms: u64,
    /// Bounded tail of the command output (empty on success).
    pub output_tail: String,
    pub failure: Option<FailureKind>,
}

/// What the verification stage should do. Empty plan = nothing runs and the
/// turn's own completion stands (interactive default).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerificationPlan {
    pub explicit: Vec<Vec<String>>,
    /// Run checks detected from repository manifests.
    pub detect: bool,
    /// Fail when the workspace shows no change (git-tracked repos only).
    pub require_diff: bool,
    pub check_timeout_seconds: u64,
}

impl VerificationPlan {
    pub fn is_empty(&self) -> bool {
        self.explicit.is_empty() && !self.detect && !self.require_diff
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationOutcome {
    pub checks: Vec<CheckResult>,
    /// Number of changed/untracked paths reported by git; None when the
    /// workspace is not a git repository or the check was not requested.
    pub changed_paths: Option<usize>,
    pub require_diff: bool,
}

impl VerificationOutcome {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.ok) && !(self.require_diff && self.changed_paths == Some(0))
    }

    /// The first failure, classified, with evidence for the next attempt.
    pub fn failure(&self) -> Option<Failure> {
        if let Some(result) = self.checks.iter().find(|c| !c.ok) {
            return Some(Failure {
                kind: result.failure.unwrap_or(FailureKind::UncertainResult),
                source: FailureSource::Verification {
                    check: result.check.label(),
                },
                summary: first_line(&result.output_tail),
                evidence: result.output_tail.clone(),
            });
        }
        if self.require_diff && self.changed_paths == Some(0) {
            return Some(Failure {
                kind: FailureKind::UncertainResult,
                source: FailureSource::Task,
                summary: "no files changed in the workspace".into(),
                evidence: String::new(),
            });
        }
        None
    }

    /// Evidence block appended as the next attempt's user message.
    pub fn render_evidence(&self, attempt: u32, max_attempts: u32) -> String {
        let mut out = String::new();
        let kind = self
            .failure()
            .map(|f| f.kind)
            .unwrap_or(FailureKind::UncertainResult);
        out.push_str(&format!(
            "VERIFICATION FAILED (attempt {attempt} of {max_attempts}): {kind}.\n"
        ));
        for result in self.checks.iter().filter(|c| !c.ok) {
            out.push_str(&format!(
                "\n$ {}\n{}\n",
                result.check.label(),
                result.output_tail
            ));
        }
        if self.require_diff && self.changed_paths == Some(0) {
            out.push_str("\nNo files changed in the workspace, but the task requires a change.\n");
        }
        out.push_str(&format!(
            "\n{}\nRe-run the failing check yourself before finishing. Do not repeat the previous patch verbatim.",
            kind.guidance()
        ));
        out
    }

    pub fn summary_line(&self) -> String {
        let passed = self.checks.iter().filter(|c| c.ok).count();
        let mut line = format!("{passed}/{} checks passed", self.checks.len());
        if let Some(changed) = self.changed_paths {
            line.push_str(&format!(", {changed} path(s) changed"));
        }
        if self.passed() {
            line.push_str(" → verified");
        } else {
            line.push_str(" → FAILED");
        }
        line
    }
}

fn ecosystem_touched(files_changed: &BTreeSet<String>, extensions: &[&str]) -> bool {
    files_changed.is_empty()
        || files_changed.iter().any(|path| {
            Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| extensions.contains(&ext))
        })
}

/// Checks implied by the workspace's manifests, filtered to ecosystems the
/// session actually touched (all of them when nothing was changed yet).
pub fn detect_checks(
    root: &Path,
    files_changed: &BTreeSet<String>,
    timeout_seconds: u64,
) -> Vec<Check> {
    let mut checks = Vec::new();
    let check = |argv: &[&str]| Check {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        timeout_seconds,
        source: CheckSource::Detected,
    };
    if root.join("Cargo.toml").exists() && ecosystem_touched(files_changed, &["rs", "toml"]) {
        checks.push(check(&["cargo", "test", "--quiet"]));
    }
    if root.join("package.json").exists()
        && ecosystem_touched(
            files_changed,
            &["ts", "tsx", "js", "jsx", "mjs", "cjs", "json"],
        )
    {
        if root.join("bun.lock").exists() || root.join("bun.lockb").exists() {
            checks.push(check(&["bun", "test"]));
        } else if package_has_test_script(root) {
            checks.push(check(&["npm", "test", "--silent"]));
        }
    }
    if (root.join("pyproject.toml").exists()
        || root.join("pytest.ini").exists()
        || root.join("setup.py").exists())
        && ecosystem_touched(files_changed, &["py"])
    {
        if root.join("uv.lock").exists() {
            checks.push(check(&["uv", "run", "pytest", "-q"]));
        } else {
            checks.push(check(&["pytest", "-q"]));
        }
    }
    if root.join("go.mod").exists() && ecosystem_touched(files_changed, &["go"]) {
        checks.push(check(&["go", "test", "./..."]));
    }
    checks
}

fn package_has_test_script(root: &Path) -> bool {
    std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("scripts")?.get("test").cloned())
        .is_some()
}

/// Lightweight workspace fingerprint (path → size, mtime) for workspaces that
/// are not git repositories, such as benchmark containers: `--require-diff`
/// compares against it instead of `git status`. Ignore-aware, skips `.git`,
/// `node_modules`, and `target`, capped so a huge tree cannot stall a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    entries: BTreeMap<PathBuf, (u64, Option<SystemTime>)>,
    root: PathBuf,
    captured_at: SystemTime,
}

const SNAPSHOT_CAP: usize = 20_000;

impl WorkspaceSnapshot {
    pub fn capture(root: &Path) -> Self {
        let mut entries = BTreeMap::new();
        let walker = ignore::WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .require_git(false)
            .filter_entry(|entry| {
                let name = entry.file_name();
                name != ".git" && name != "node_modules" && name != "target"
            })
            .build();
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                entries.insert(
                    entry.path().to_path_buf(),
                    (meta.len(), meta.modified().ok()),
                );
            }
            if entries.len() >= SNAPSHOT_CAP {
                break;
            }
        }
        Self {
            entries,
            root: root.to_path_buf(),
            captured_at: SystemTime::now(),
        }
    }

    /// Whether one file is new or modified since the snapshot (a missing
    /// file is "not changed": there is nothing to run). One `stat`, no walk.
    /// Files outside the workspace (a script in /tmp, a system binary) are
    /// judged by modification time against the capture instant: written
    /// since → changed; older → pre-existing.
    pub fn path_changed(&self, path: &Path) -> bool {
        let Ok(meta) = std::fs::metadata(path) else {
            return false;
        };
        if !meta.is_file() {
            return false;
        }
        let modified = meta.modified().ok();
        if !path.starts_with(&self.root) {
            return modified.is_some_and(|m| m > self.captured_at);
        }
        let stamp = (meta.len(), modified);
        match self.entries.get(path) {
            Some(previous) => *previous != stamp,
            None => true,
        }
    }

    /// Paths added, removed, or modified since the snapshot.
    pub fn changed_since(&self, root: &Path) -> usize {
        let now = Self::capture(root);
        let mut changed = 0usize;
        for (path, stamp) in &now.entries {
            match self.entries.get(path) {
                Some(previous) if previous == stamp => {}
                _ => changed += 1,
            }
        }
        changed += self
            .entries
            .keys()
            .filter(|path| !now.entries.contains_key(*path))
            .count();
        changed
    }
}

/// Changed-path count for `--require-diff`: git when the root is a work tree,
/// else the snapshot taken before the task started, else unknown.
pub fn changed_paths(root: &Path, snapshot: Option<&WorkspaceSnapshot>) -> Option<usize> {
    git_changed_paths(root).or_else(|| snapshot.map(|s| s.changed_since(root)))
}

/// Count changed or untracked paths via `git status --porcelain`. None when
/// the root is not inside a git work tree (or git is unavailable).
pub fn git_changed_paths(root: &Path) -> Option<usize> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(root)
        .env_clear()
        .envs(crate::tools::policy::sanitized_environment())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
    )
}

/// Run the plan. `model_declared` are argv vectors the model recorded via
/// `update_plan`; they run last and count like any other check.
pub async fn run_verification(
    root: &Path,
    plan: &VerificationPlan,
    files_changed: &BTreeSet<String>,
    model_declared: &[Vec<String>],
    policy: &PolicySettings,
    snapshot: Option<&WorkspaceSnapshot>,
    cancel: &CancellationToken,
) -> VerificationOutcome {
    let timeout = plan.check_timeout_seconds.max(1);
    let mut checks: Vec<Check> = plan
        .explicit
        .iter()
        .filter(|argv| !argv.is_empty())
        .map(|argv| Check {
            argv: argv.clone(),
            timeout_seconds: timeout,
            source: CheckSource::Explicit,
        })
        .collect();
    if plan.detect {
        checks.extend(detect_checks(root, files_changed, timeout));
    }
    checks.extend(
        model_declared
            .iter()
            .filter(|argv| !argv.is_empty())
            .map(|argv| Check {
                argv: argv.clone(),
                timeout_seconds: timeout,
                source: CheckSource::ModelDeclared,
            }),
    );
    let mut results = Vec::with_capacity(checks.len());
    for check in checks {
        let started = std::time::Instant::now();
        let outcome = run_command(
            root,
            RunCommandArgs {
                command: check.argv.clone(),
                timeout_seconds: Some(check.timeout_seconds),
            },
            policy,
            cancel,
        )
        .await;
        let duration_ms = started.elapsed().as_millis() as u64;
        let (ok, output) = match outcome {
            Ok(output) => (!output.is_error, output.content),
            Err(error) => (false, error.to_string()),
        };
        results.push(CheckResult {
            failure: (!ok).then(|| classify_command_output(&output)),
            output_tail: if ok {
                String::new()
            } else {
                evidence_tail(&output, EVIDENCE_CAP_BYTES)
            },
            check,
            ok,
            duration_ms,
        });
        if cancel.is_cancelled() {
            break;
        }
    }
    VerificationOutcome {
        checks: results,
        changed_paths: if plan.require_diff {
            changed_paths(root, snapshot)
        } else {
            None
        },
        require_diff: plan.require_diff,
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn path_changed_judges_outside_files_by_modification_time() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let old = outside.path().join("old.sh");
        std::fs::write(&old, "exit 0\n").unwrap();
        let ancient = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        filetime_set(&old, ancient);
        std::fs::write(root.path().join("kept.txt"), "k").unwrap();
        let snapshot = WorkspaceSnapshot::capture(root.path());
        // Outside, older than the task: pre-existing (a system tool, say).
        assert!(!snapshot.path_changed(&old));
        // Outside, written after capture: the model's.
        let fresh = outside.path().join("check.sh");
        std::fs::write(&fresh, "exit 0\n").unwrap();
        filetime_set(
            &fresh,
            SystemTime::now() + std::time::Duration::from_secs(5),
        );
        assert!(snapshot.path_changed(&fresh));
        // Inside: membership and stamp decide.
        assert!(!snapshot.path_changed(&root.path().join("kept.txt")));
        std::fs::write(root.path().join("new.txt"), "n").unwrap();
        assert!(snapshot.path_changed(&root.path().join("new.txt")));
        assert!(!snapshot.path_changed(&root.path().join("missing.txt")));
    }

    fn filetime_set(path: &Path, time: SystemTime) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(time).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_follows_manifests_and_touched_ecosystems() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'").unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"x"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("go.mod"), "module x").unwrap();
        let none = BTreeSet::new();
        let all = detect_checks(dir.path(), &none, 60);
        let labels: Vec<String> = all.iter().map(Check::label).collect();
        assert_eq!(
            labels,
            vec!["cargo test --quiet", "npm test --silent", "go test ./..."]
        );
        // Only Rust touched → only cargo.
        let rust: BTreeSet<String> = ["src/lib.rs".to_string()].into_iter().collect();
        let only_rust = detect_checks(dir.path(), &rust, 60);
        assert_eq!(only_rust.len(), 1);
        assert_eq!(only_rust[0].argv[0], "cargo");
        // Bun lockfile prefers bun test.
        std::fs::write(dir.path().join("bun.lock"), "").unwrap();
        let ts: BTreeSet<String> = ["a.ts".to_string()].into_iter().collect();
        assert_eq!(detect_checks(dir.path(), &ts, 60)[0].argv[0], "bun");
    }

    #[tokio::test]
    async fn explicit_checks_run_and_failures_carry_evidence() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ok.txt"), "fine\n").unwrap();
        let plan = VerificationPlan {
            explicit: vec![
                vec!["cat".into(), "ok.txt".into()],
                vec!["cat".into(), "missing.txt".into()],
            ],
            detect: false,
            require_diff: false,
            check_timeout_seconds: 10,
        };
        let outcome = run_verification(
            dir.path(),
            &plan,
            &BTreeSet::new(),
            &[],
            &PolicySettings::default(),
            None,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(outcome.checks.len(), 2);
        assert!(outcome.checks[0].ok);
        assert!(!outcome.checks[1].ok);
        assert!(!outcome.passed());
        let failure = outcome.failure().unwrap();
        assert!(matches!(failure.source, FailureSource::Verification { .. }));
        assert!(failure.evidence.contains("missing.txt"));
        let evidence = outcome.render_evidence(1, 3);
        assert!(evidence.contains("VERIFICATION FAILED (attempt 1 of 3)"));
        assert!(evidence.contains("$ cat missing.txt"));
        assert!(evidence.contains("Do not repeat the previous patch verbatim"));
        assert!(outcome.summary_line().contains("1/2 checks passed"));
    }

    #[tokio::test]
    async fn model_declared_checks_run_but_are_labelled() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = run_verification(
            dir.path(),
            &VerificationPlan {
                check_timeout_seconds: 10,
                ..VerificationPlan::default()
            },
            &BTreeSet::new(),
            &[vec!["pwd".into()]],
            &PolicySettings::default(),
            None,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(outcome.checks.len(), 1);
        assert_eq!(outcome.checks[0].check.source, CheckSource::ModelDeclared);
        assert!(outcome.passed());
    }

    #[test]
    fn snapshot_detects_changes_where_git_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "1").unwrap();
        let before = WorkspaceSnapshot::capture(dir.path());
        assert_eq!(before.changed_since(dir.path()), 0);
        assert_eq!(changed_paths(dir.path(), Some(&before)), Some(0));
        std::fs::write(dir.path().join("a.txt"), "22").unwrap();
        std::fs::write(dir.path().join("b.txt"), "new").unwrap();
        assert_eq!(before.changed_since(dir.path()), 2);
        std::fs::remove_file(dir.path().join("a.txt")).unwrap();
        assert_eq!(before.changed_since(dir.path()), 2, "removed + added");
        assert_eq!(changed_paths(dir.path(), None), None);
    }

    #[test]
    fn diff_requirement_fails_clean_repos_and_skips_non_repos() {
        let dir = tempfile::tempdir().unwrap();
        // Not a git repo: the check is unknown, never a failure.
        assert_eq!(git_changed_paths(dir.path()), None);
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        assert_eq!(git_changed_paths(dir.path()), Some(0));
        std::fs::write(dir.path().join("new.txt"), "x").unwrap();
        assert_eq!(git_changed_paths(dir.path()), Some(1));
        let clean = VerificationOutcome {
            checks: vec![],
            changed_paths: Some(0),
            require_diff: true,
        };
        assert!(!clean.passed());
        assert_eq!(clean.failure().unwrap().source, FailureSource::Task);
        let unknown = VerificationOutcome {
            checks: vec![],
            changed_paths: None,
            require_diff: true,
        };
        assert!(unknown.passed());
    }
}

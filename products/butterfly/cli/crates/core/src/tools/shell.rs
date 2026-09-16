//! The `shell` tool: a real shell command line, available ONLY in sandboxed
//! sessions (disposable containers such as benchmark environments). Outside a
//! sandbox the tool is not even advertised, so the default prompt prefix is
//! unchanged. Inside one, pipelines, redirects, globbing, and package
//! installation become possible without a human approving every line.
//!
//! Still applied: credential-stripped environment, wall-clock timeout, bounded
//! output, and the privilege-escalation ban (a sandbox root does not need
//! sudo; a leaked one must not get it).

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use super::policy::sanitized_environment;
use super::{ToolError, ToolOutput};

const OUTPUT_CAP_BYTES: usize = 40_000;
const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_TIMEOUT_SECONDS: u64 = 1_800;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellArgs {
    /// The command line, interpreted by `bash -lc` (falls back to `sh -c`).
    pub command: String,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

fn starts_with_escalation(command: &str) -> bool {
    command
        .split(|c: char| c.is_whitespace() || c == ';' || c == '&' || c == '|')
        .filter(|token| !token.is_empty())
        .any(|token| matches!(token, "sudo" | "su" | "doas"))
}

pub async fn run_shell(
    root: &Path,
    args: ShellArgs,
    cancel: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    if args.command.trim().is_empty() {
        return Err(ToolError::Failed("empty command".into()));
    }
    if starts_with_escalation(&args.command) {
        return Err(ToolError::Policy(super::policy::PolicyError::Command(
            "privilege escalation is never run (sudo/su/doas)".into(),
        )));
    }
    let timeout = Duration::from_secs(
        args.timeout_seconds
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
            .clamp(1, MAX_TIMEOUT_SECONDS),
    );
    let (program, flag) = if Path::new("/bin/bash").exists() {
        ("/bin/bash", "-lc")
    } else {
        ("/bin/sh", "-c")
    };
    let mut command = tokio::process::Command::new(program);
    command
        .arg(flag)
        .arg(&args.command)
        .current_dir(root)
        .env_clear()
        .envs(sanitized_environment())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|error| ToolError::Failed(format!("spawn failed: {error}")))?;
    let waited = tokio::select! {
        _ = cancel.cancelled() => return Err(ToolError::Cancelled),
        waited = tokio::time::timeout(timeout, child.wait_with_output()) => waited,
    };
    let output = match waited {
        Ok(result) => result.map_err(|error| ToolError::Failed(error.to_string()))?,
        Err(_) => {
            return Ok(ToolOutput::error(format!(
                "command timed out after {}s",
                timeout.as_secs()
            )));
        }
    };
    let mut rendered = String::new();
    let exit = output
        .status
        .code()
        .map_or_else(|| "signal".to_string(), |code| code.to_string());
    rendered.push_str(&format!("exit: {exit}\n"));
    push_stream(&mut rendered, "stdout", &output.stdout);
    push_stream(&mut rendered, "stderr", &output.stderr);
    if output.status.success() {
        Ok(ToolOutput::text(rendered))
    } else {
        rendered.push_str(EXIT_RULE);
        Ok(ToolOutput::error(rendered))
    }
}

/// Appended to every non-zero exit: the DeepSeek Harness rule that a failed
/// command is evidence to read, not noise to step past.
const EXIT_RULE: &str =
    "[non-zero exit: read the error above and fix its cause before moving on; do not repeat the \
     command unchanged or treat the task as done while it fails]\n";

fn push_stream(rendered: &mut String, label: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    let budget = OUTPUT_CAP_BYTES / 2;
    let kept = if text.len() > budget {
        let mut start = text.len() - budget;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        format!("[head truncated]\n{}", &text[start..])
    } else {
        text.into_owned()
    };
    rendered.push_str(&format!("{label}:\n{kept}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shell_runs_pipelines_with_bounded_output() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "b\na\n").unwrap();
        let output = run_shell(
            dir.path(),
            ShellArgs {
                command: "cat f.txt | sort | head -1".into(),
                timeout_seconds: Some(10),
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains("exit: 0"));
        assert!(output.content.contains("a\n"));
    }

    #[tokio::test]
    async fn shell_rejects_privilege_escalation_and_reports_failures() {
        let dir = tempfile::tempdir().unwrap();
        let denied = run_shell(
            dir.path(),
            ShellArgs {
                command: "echo hi && sudo ls".into(),
                timeout_seconds: None,
            },
            &CancellationToken::new(),
        )
        .await;
        assert!(denied.is_err());
        let failed = run_shell(
            dir.path(),
            ShellArgs {
                command: "exit 3".into(),
                timeout_seconds: Some(5),
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(failed.is_error);
        assert!(failed.content.contains("exit: 3"));
        assert!(
            failed.content.contains("fix its cause before moving on"),
            "{}",
            failed.content
        );
    }
}

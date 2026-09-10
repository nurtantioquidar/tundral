//! Model-issued command execution: argv vector only (never a shell), policy
//! validated, credential-stripped environment, wall-clock timeout, bounded
//! output.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use super::policy::{
    classify_command_with, sanitized_environment, CommandClass, PolicyError, PolicySettings,
};
use super::{ToolError, ToolOutput};

const OUTPUT_CAP_BYTES: usize = 40_000;
const DEFAULT_TIMEOUT_SECONDS: u64 = 60;
const MAX_TIMEOUT_SECONDS: u64 = 300;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCommandArgs {
    /// Command as an argv vector, e.g. ["cargo", "test"].
    #[serde(deserialize_with = "lenient_argv")]
    pub command: Vec<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

/// Deserialize argv leniently via [`super::extract_argv`]: models sometimes
/// double-encode the array as a JSON string, and rejecting that shape strands
/// them in a retry loop (dogfood 2026-09-03).
fn lenient_argv<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    super::extract_argv(&value).ok_or_else(|| {
        serde::de::Error::custom(
            "command must be a JSON array of strings (argv), e.g. [\"git\", \"status\"]",
        )
    })
}

pub async fn run_command(
    root: &Path,
    args: RunCommandArgs,
    settings: &PolicySettings,
    cancel: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    // Approval decisions happen upstream in the session loop; the executor is
    // the last line of defense only for the never-runnable class.
    if let CommandClass::Forbidden(reason) = classify_command_with(&args.command, settings) {
        return Err(ToolError::Policy(PolicyError::Command(reason)));
    }
    let timeout = Duration::from_secs(
        args.timeout_seconds
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
            .clamp(1, MAX_TIMEOUT_SECONDS),
    );
    let mut command = tokio::process::Command::new(&args.command[0]);
    command
        .args(&args.command[1..])
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
            // kill_on_drop reaps the child when `command` is dropped here.
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
        Ok(ToolOutput::error(rendered))
    }
}

fn push_stream(rendered: &mut String, label: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    // Keep the tail: failures usually report at the end.
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
    async fn runs_allowlisted_commands_with_bounded_output() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello\n").unwrap();
        let output = run_command(
            dir.path(),
            RunCommandArgs {
                command: vec!["cat".into(), "f.txt".into()],
                timeout_seconds: Some(10),
            },
            &PolicySettings::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!output.is_error);
        assert!(output.content.contains("exit: 0"));
        assert!(output.content.contains("hello"));
    }

    #[tokio::test]
    async fn rejects_forbidden_commands_before_spawning() {
        let dir = tempfile::tempdir().unwrap();
        let result = run_command(
            dir.path(),
            RunCommandArgs {
                command: vec!["sudo".into(), "true".into()],
                timeout_seconds: None,
            },
            &PolicySettings::default(),
            &CancellationToken::new(),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn nonzero_exit_is_an_error_result_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let output = run_command(
            dir.path(),
            RunCommandArgs {
                command: vec!["cat".into(), "missing.txt".into()],
                timeout_seconds: Some(10),
            },
            &PolicySettings::default(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("stderr"));
    }
}

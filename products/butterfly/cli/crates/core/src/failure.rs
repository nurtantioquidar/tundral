//! Structured failure classification. Tool errors, command output, provider
//! errors, and verification results are mapped onto one closed vocabulary so
//! the recovery loop can choose a response, telemetry can aggregate, and a
//! router (Nimble) can learn from `previous_failure` without parsing prose.
//!
//! Classification is rule-based and deterministic: exit codes, well-known
//! compiler/test-runner signatures, tool error shapes, and repetition counts.
//! It is deliberately conservative — when nothing matches, the answer is
//! `UncertainResult`, never a guess.

use serde::{Deserialize, Serialize};

use crate::provider::ProviderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    SyntaxFailure,
    CompileFailure,
    TestFailure,
    MissingContext,
    IncorrectAssumption,
    ToolFailure,
    EnvironmentFailure,
    Timeout,
    ModelFormatFailure,
    RepeatedPatchFailure,
    /// The attempt wrote nothing: no file was created or edited, so there is
    /// no work to verify. Distinct from `UncertainResult` because nothing
    /// about it is uncertain — it is the clearest signal the loop gets, and
    /// telling the model the outcome is "unclear" sent it to verify work that
    /// does not exist (10 of 14 unclassified bench failures, 2026-09-19).
    WorkspaceUnchanged,
    /// The attempt stopped without calling `complete_task` under the
    /// completion gate.
    CompletionNotClaimed,
    /// The attempt used its per-attempt provider-call budget before finishing.
    CallBudgetExhausted,
    /// The response hit the output-token limit with no visible answer.
    OutputTruncated,
    /// A completion claim whose acceptance checks cannot fail on wrong
    /// behaviour (compile-only, existence-only).
    UnprovenClaim,
    UncertainResult,
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureKind::SyntaxFailure => "syntax_failure",
            FailureKind::CompileFailure => "compile_failure",
            FailureKind::TestFailure => "test_failure",
            FailureKind::MissingContext => "missing_context",
            FailureKind::IncorrectAssumption => "incorrect_assumption",
            FailureKind::ToolFailure => "tool_failure",
            FailureKind::EnvironmentFailure => "environment_failure",
            FailureKind::Timeout => "timeout",
            FailureKind::ModelFormatFailure => "model_format_failure",
            FailureKind::RepeatedPatchFailure => "repeated_patch_failure",
            FailureKind::WorkspaceUnchanged => "workspace_unchanged",
            FailureKind::CompletionNotClaimed => "completion_not_claimed",
            FailureKind::CallBudgetExhausted => "call_budget_exhausted",
            FailureKind::OutputTruncated => "output_truncated",
            FailureKind::UnprovenClaim => "unproven_claim",
            FailureKind::UncertainResult => "uncertain_result",
        }
    }

    /// One line of recovery guidance appended to evidence so the next attempt
    /// changes approach instead of repeating the last one.
    pub fn guidance(self) -> &'static str {
        match self {
            FailureKind::SyntaxFailure => {
                "Fix the syntax error at the reported location, then re-run the check."
            }
            FailureKind::CompileFailure => {
                "Resolve the compiler errors in order; read the failing file around each reported line before editing."
            }
            FailureKind::TestFailure => {
                "Read the failing test and the code under test before changing either; do not weaken or delete tests."
            }
            FailureKind::MissingContext => {
                "Something you referenced does not exist; list or search the workspace to find the real path or symbol."
            }
            FailureKind::IncorrectAssumption => {
                "The file did not contain what you expected; re-read it and base the edit on its actual contents."
            }
            FailureKind::ToolFailure => {
                "The tool call itself was rejected; check the arguments and the policy message, then use a permitted approach."
            }
            FailureKind::EnvironmentFailure => {
                "The environment lacks something (binary, dependency, network); check what is installed before retrying."
            }
            FailureKind::Timeout => {
                "The command exceeded its time limit; narrow its scope or raise timeout_seconds deliberately."
            }
            FailureKind::ModelFormatFailure => {
                "The tool arguments were malformed; follow the schema exactly (run_command takes a JSON array of strings)."
            }
            FailureKind::RepeatedPatchFailure => {
                "The same edit has failed repeatedly; re-read the file and choose a different anchor or approach."
            }
            FailureKind::WorkspaceUnchanged => {
                "No file was created or edited, so there is nothing to verify; make the change itself before running any check."
            }
            FailureKind::CompletionNotClaimed => {
                "The attempt stopped without declaring completion; finish the remaining deliverables and call complete_task with the evidence for each."
            }
            FailureKind::CallBudgetExhausted => {
                "The attempt ran out of provider calls before finishing; continue from the current state instead of re-exploring, and write the remaining deliverables first."
            }
            FailureKind::OutputTruncated => {
                "The previous response was cut off by the output limit before any answer; keep the response short and make the changes with tools instead of narrating them."
            }
            FailureKind::UnprovenClaim => {
                "The claimed checks cannot fail on wrong behaviour; run one that actually exercises the requirement before claiming completion again."
            }
            FailureKind::UncertainResult => {
                "The outcome is unclear; run the most targeted check that would prove the change works."
            }
        }
    }
}

impl std::fmt::Display for FailureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum FailureSource {
    Tool { name: String },
    Verification { check: String },
    Provider,
    Task,
}

/// One classified failure with enough evidence to act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub kind: FailureKind,
    #[serde(flatten)]
    pub source: FailureSource,
    /// First meaningful line, for lists and telemetry.
    pub summary: String,
    /// Bounded tail of the raw output, for the next attempt's evidence.
    pub evidence: String,
}

pub const EVIDENCE_CAP_BYTES: usize = 3_000;

/// Keep the tail of an output (failures report at the end), bounded.
pub fn evidence_tail(output: &str, cap: usize) -> String {
    let trimmed = output.trim_end();
    if trimmed.len() <= cap {
        return trimmed.to_string();
    }
    let mut start = trimmed.len() - cap;
    while !trimmed.is_char_boundary(start) {
        start += 1;
    }
    format!("[…]\n{}", &trimmed[start..])
}

pub fn first_line(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

fn contains_any(lower: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| lower.contains(needle))
}

/// Classify a failed tool result. `output` is the tool's error text (without
/// the transcript's `ERROR: ` prefix).
pub fn classify_tool_output(tool: &str, output: &str) -> FailureKind {
    let lower = output.to_ascii_lowercase();
    if lower.contains("timed out") {
        return FailureKind::Timeout;
    }
    if contains_any(
        &lower,
        &[
            "invalid arguments",
            "must be a json array",
            "unknown tool",
            "unknown field",
            "missing field",
            "missing required",
            "expected value",
        ],
    ) {
        return FailureKind::ModelFormatFailure;
    }
    if contains_any(
        &lower,
        &[
            "not allowed",
            "denied",
            "escapes the workspace",
            "privilege escalation",
            "is disabled",
        ],
    ) {
        return FailureKind::ToolFailure;
    }
    match tool {
        "edit" => {
            if contains_any(&lower, &["not found", "no such file", "does not exist"]) {
                if lower.contains("old_string") || lower.contains("match") {
                    FailureKind::IncorrectAssumption
                } else {
                    FailureKind::MissingContext
                }
            } else if contains_any(
                &lower,
                &["occurrence", "matches", "ambiguous", "replace_all"],
            ) {
                FailureKind::IncorrectAssumption
            } else {
                FailureKind::ToolFailure
            }
        }
        "read_file" | "list_files" | "search_text" | "symbol_search" | "repo_map" => {
            if contains_any(
                &lower,
                &["no such file", "not found", "does not exist", "os error 2"],
            ) {
                FailureKind::MissingContext
            } else if lower.contains("invalid pattern") {
                FailureKind::ModelFormatFailure
            } else {
                FailureKind::ToolFailure
            }
        }
        "run_command" | "shell" => classify_command_output(output),
        _ => FailureKind::ToolFailure,
    }
}

/// Classify the combined stdout/stderr of a failed command (non-zero exit).
/// Signatures cover the toolchains Butterfly's verification detects: Rust,
/// Bun/Node/TypeScript, Python, Go, plus generic shell failures.
pub fn classify_command_output(output: &str) -> FailureKind {
    let lower = output.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") && lower.contains("exceeded") {
        return FailureKind::Timeout;
    }
    if contains_any(
        &lower,
        &[
            "command not found",
            "spawn failed",
            "no such file or directory (os error 2)",
            "not recognized as an internal",
            "permission denied",
            "could not resolve host",
            "connection refused",
            "network is unreachable",
            "temporary failure in name resolution",
            "modulenotfounderror",
            "cannot find module",
            "could not find a version",
            "no space left",
            "killed",
            "out of memory",
        ],
    ) {
        return FailureKind::EnvironmentFailure;
    }
    // A missing file or directory named by the command (not the binary
    // itself): the expected artifact was never produced.
    if contains_any(&lower, &["no such file or directory", "cannot access"]) {
        return FailureKind::MissingContext;
    }
    if contains_any(
        &lower,
        &[
            "syntaxerror",
            "syntax error",
            "unexpected token",
            "indentationerror",
            "unexpected eof",
            "unterminated",
            "expected `;`",
            "expected one of",
            "unclosed delimiter",
            "mismatched closing delimiter",
            "parse error",
        ],
    ) {
        return FailureKind::SyntaxFailure;
    }
    if contains_any(
        &lower,
        &[
            "error[e",
            "could not compile",
            "cannot find value",
            "cannot find type",
            "cannot find function",
            "mismatched types",
            "cannot borrow",
            "does not live long enough",
            "error ts",
            "type error",
            "typeerror:",
            "undefined:",
            "declared and not used",
            "cannot use",
            "compilation failed",
            "build failed",
            "unresolved import",
            "no method named",
            "nameerror",
            "attributeerror",
            "importerror",
        ],
    ) {
        return FailureKind::CompileFailure;
    }
    if contains_any(
        &lower,
        &[
            "test result: failed",
            "tests failed",
            "test failed",
            "failed:",
            " failed",
            "assertionerror",
            "assertion failed",
            "expected",
            "✗",
            "(fail)",
            "fail ",
            "--- fail",
            "panicked at",
        ],
    ) {
        return FailureKind::TestFailure;
    }
    FailureKind::UncertainResult
}

/// `None` for a cancellation: the operator stopping a turn is an outcome, not
/// a failure, and recording it as one puts an imaginary problem in front of
/// both the model (the working-state checkpoint) and the router
/// (`previous_failure`). Callers must skip recording when this is `None`.
pub fn classify_provider_error(error: &ProviderError) -> Option<FailureKind> {
    Some(match error {
        ProviderError::Cancelled => return None,
        ProviderError::Status { status, .. } if *status >= 500 => FailureKind::EnvironmentFailure,
        ProviderError::Status { .. } => FailureKind::ToolFailure,
        ProviderError::RateLimited { .. } => FailureKind::EnvironmentFailure,
        ProviderError::ContextLengthExceeded => FailureKind::MissingContext,
        ProviderError::Transport(text) if text.to_ascii_lowercase().contains("timed out") => {
            FailureKind::Timeout
        }
        ProviderError::Transport(_) => FailureKind::EnvironmentFailure,
        ProviderError::Malformed(_) => FailureKind::ModelFormatFailure,
    })
}

impl Failure {
    pub fn from_tool(name: &str, output: &str) -> Self {
        Self {
            kind: classify_tool_output(name, output),
            source: FailureSource::Tool {
                name: name.to_string(),
            },
            summary: first_line(output),
            evidence: evidence_tail(output, EVIDENCE_CAP_BYTES),
        }
    }

    pub fn with_kind(mut self, kind: FailureKind) -> Self {
        self.kind = kind;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_output_signatures_map_to_kinds() {
        assert_eq!(
            classify_command_output("error[E0308]: mismatched types\n --> src/a.rs"),
            FailureKind::CompileFailure
        );
        assert_eq!(
            classify_command_output("error: expected `;`, found `}`"),
            FailureKind::SyntaxFailure
        );
        assert_eq!(
            classify_command_output("test result: FAILED. 1 passed; 2 failed"),
            FailureKind::TestFailure
        );
        assert_eq!(
            classify_command_output("  File \"a.py\", line 3\nIndentationError: unexpected indent"),
            FailureKind::SyntaxFailure
        );
        assert_eq!(
            classify_command_output("bash: cargo: command not found"),
            FailureKind::EnvironmentFailure
        );
        assert_eq!(
            classify_command_output("cat: made.txt: No such file or directory"),
            FailureKind::MissingContext
        );
        assert_eq!(
            classify_command_output("command timed out after 60s"),
            FailureKind::Timeout
        );
        assert_eq!(
            classify_command_output("exit: 3\nstdout:\nsomething odd\n"),
            FailureKind::UncertainResult
        );
        assert_eq!(
            classify_command_output("./a.go:12:2: undefined: Foo"),
            FailureKind::CompileFailure
        );
        assert_eq!(
            classify_command_output("AssertionError: expected 1 to equal 2"),
            FailureKind::TestFailure
        );
    }

    #[test]
    fn tool_outputs_map_by_tool_shape() {
        assert_eq!(
            classify_tool_output("edit", "old_string not found in src/a.rs"),
            FailureKind::IncorrectAssumption
        );
        assert_eq!(
            classify_tool_output("edit", "old_string matches 3 times; set replace_all"),
            FailureKind::IncorrectAssumption
        );
        assert_eq!(
            classify_tool_output(
                "read_file",
                "src/missing.rs: No such file or directory (os error 2)"
            ),
            FailureKind::MissingContext
        );
        assert_eq!(
            classify_tool_output(
                "run_command",
                "invalid arguments: command must be a JSON array of strings (argv)"
            ),
            FailureKind::ModelFormatFailure
        );
        assert_eq!(
            classify_tool_output(
                "run_command",
                "command is not allowed: privilege escalation is never run: sudo"
            ),
            FailureKind::ToolFailure
        );
        assert_eq!(
            classify_tool_output("write_file", "denied: session is in read-only mode"),
            FailureKind::ToolFailure
        );
        assert_eq!(
            classify_tool_output("run_command", "exit: 101\nstderr:\ntest result: FAILED"),
            FailureKind::TestFailure
        );
    }

    #[test]
    fn provider_errors_classify() {
        assert_eq!(
            classify_provider_error(&ProviderError::RateLimited {
                status: 429,
                body: String::new()
            }),
            Some(FailureKind::EnvironmentFailure)
        );
        assert_eq!(
            classify_provider_error(&ProviderError::Malformed("x".into())),
            Some(FailureKind::ModelFormatFailure)
        );
        assert_eq!(
            classify_provider_error(&ProviderError::Transport("operation timed out".into())),
            Some(FailureKind::Timeout)
        );
        assert_eq!(
            classify_provider_error(&ProviderError::ContextLengthExceeded),
            Some(FailureKind::MissingContext)
        );
    }

    /// Stopping a turn is the operator's decision, not a fault. It used to be
    /// filed as `uncertain_result`, which put an imaginary problem in front of
    /// the model (the working-state checkpoint) and the router
    /// (`previous_failure`). `None` means "record nothing".
    #[test]
    fn a_cancelled_turn_is_not_a_failure() {
        assert_eq!(classify_provider_error(&ProviderError::Cancelled), None);
    }

    #[test]
    fn evidence_is_tail_bounded_and_summaries_are_first_lines() {
        let long = format!("{}\nlast line", "x".repeat(5_000));
        let tail = evidence_tail(&long, 100);
        assert!(tail.starts_with("[…]"));
        assert!(tail.ends_with("last line"));
        assert!(tail.len() <= 110);
        assert_eq!(first_line("\n\n  boom: bad\nmore"), "boom: bad");
        let failure = Failure::from_tool("edit", "old_string not found in a.rs\ndetails");
        assert_eq!(failure.kind, FailureKind::IncorrectAssumption);
        assert_eq!(failure.summary, "old_string not found in a.rs");
        assert!(serde_json::to_string(&failure)
            .unwrap()
            .contains("\"source\":\"tool\""));
    }
}

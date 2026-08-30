//! Approval gating. Deterministic code decides WHICH tool calls need review
//! (by tool kind and permission mode); the gate only answers the prompts it
//! is asked. UI implementations live in the CLI crate.

use async_trait::async_trait;

use crate::tools::ToolKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Mutating and command tools are denied outright.
    ReadOnly,
    /// Mutating and command tools require gate approval.
    Ask,
    /// Everything runs without prompting.
    Auto,
}

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub tool_name: String,
    /// One-line preview, e.g. `run_command(cargo test)`.
    pub preview: String,
    /// Session-scope approval key: approving "for the session" allows future
    /// calls with the same key without prompting.
    pub scope_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approve,
    ApproveForSession,
    Deny,
}

#[async_trait]
pub trait ApprovalGate: Send + Sync {
    async fn review(&self, request: &ApprovalRequest) -> Decision;
}

/// Gate used by tests and --auto mode plumbing.
pub struct AutoApprove;

#[async_trait]
impl ApprovalGate for AutoApprove {
    async fn review(&self, _request: &ApprovalRequest) -> Decision {
        Decision::Approve
    }
}

/// Session-scope approval key for a call: run_command scopes to its binary,
/// everything else to the tool name.
pub fn scope_key(tool_name: &str, arguments: &str) -> String {
    if tool_name == "run_command" {
        let binary = serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|value| {
                value
                    .get("command")
                    .and_then(|c| c.get(0))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        format!("run_command:{binary}")
    } else {
        tool_name.to_string()
    }
}

/// What deterministic policy says about a call before any gate is consulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyOutcome {
    AutoApprove,
    NeedsReview,
    Deny,
}

pub fn policy_outcome(mode: PermissionMode, kind: ToolKind) -> PolicyOutcome {
    match (mode, kind) {
        (_, ToolKind::ReadOnly) => PolicyOutcome::AutoApprove,
        (PermissionMode::ReadOnly, _) => PolicyOutcome::Deny,
        (PermissionMode::Auto, _) => PolicyOutcome::AutoApprove,
        (PermissionMode::Ask, _) => PolicyOutcome::NeedsReview,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_tools_never_prompt_and_read_only_mode_denies_mutations() {
        assert_eq!(
            policy_outcome(PermissionMode::Ask, ToolKind::ReadOnly),
            PolicyOutcome::AutoApprove
        );
        assert_eq!(
            policy_outcome(PermissionMode::ReadOnly, ToolKind::Mutating),
            PolicyOutcome::Deny
        );
        assert_eq!(
            policy_outcome(PermissionMode::Ask, ToolKind::Command),
            PolicyOutcome::NeedsReview
        );
        assert_eq!(
            policy_outcome(PermissionMode::Auto, ToolKind::Command),
            PolicyOutcome::AutoApprove
        );
    }

    #[test]
    fn command_scope_keys_include_the_binary() {
        assert_eq!(
            scope_key("run_command", r#"{"command":["cargo","test"]}"#),
            "run_command:cargo"
        );
        assert_eq!(scope_key("edit", "{}"), "edit");
    }
}

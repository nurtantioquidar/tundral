//! Tool registry: schemas (frozen order, deterministic serialization) and
//! dispatch. Argument validation happens locally regardless of what the
//! provider claims to enforce.

mod command;
mod fs_tools;
pub mod policy;
mod search;

use std::path::{Path, PathBuf};

use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::provider::ToolSchema;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error(transparent)]
    Policy(#[from] policy::PolicyError),
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("{0}")]
    Failed(String),
    #[error("cancelled by user")]
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn text(content: String) -> Self {
        Self {
            content,
            is_error: false,
        }
    }

    pub fn error(content: String) -> Self {
        Self {
            content,
            is_error: true,
        }
    }
}

/// How a tool interacts with the world; deterministic code — not the model,
/// not the UI — maps kinds to approval requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    ReadOnly,
    Mutating,
    Command,
}

pub const TOOL_NAMES: &[(&str, ToolKind)] = &[
    ("read_file", ToolKind::ReadOnly),
    ("list_files", ToolKind::ReadOnly),
    ("search_text", ToolKind::ReadOnly),
    ("edit", ToolKind::Mutating),
    ("write_file", ToolKind::Mutating),
    ("run_command", ToolKind::Command),
];

pub fn tool_kind(name: &str) -> Option<ToolKind> {
    TOOL_NAMES
        .iter()
        .find(|(tool, _)| *tool == name)
        .map(|(_, kind)| *kind)
}

/// Tool schemas in frozen order. The serialized bytes of this list are part of
/// the cached prompt prefix, so ordering and content must never vary within a
/// session (and should not vary across versions without accepting a cache
/// reset).
pub fn tool_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "read_file".into(),
            description: "Read a file in the workspace. Returns numbered lines. Use offset/limit for large files.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path relative to the workspace root"},
                    "offset": {"type": "integer", "description": "1-based first line to read"},
                    "limit": {"type": "integer", "description": "Maximum number of lines"}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSchema {
            name: "list_files".into(),
            description: "Recursively list files under a directory, respecting .gitignore.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Directory relative to the workspace root; omit for the root"}
                },
                "additionalProperties": false
            }),
        },
        ToolSchema {
            name: "search_text".into(),
            description: "Search file contents with a regular expression. Returns path:line: text matches.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Rust regex pattern"},
                    "path": {"type": "string", "description": "Directory or file to search; omit for the whole workspace"},
                    "case_insensitive": {"type": "boolean"}
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
        ToolSchema {
            name: "edit".into(),
            description: "Replace an exact string in a file. old_string must match exactly once unless replace_all is set.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "old_string": {"type": "string"},
                    "new_string": {"type": "string"},
                    "replace_all": {"type": "boolean"}
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            }),
        },
        ToolSchema {
            name: "write_file".into(),
            description: "Create or overwrite a file with the given content.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        },
        ToolSchema {
            name: "run_command".into(),
            description: "Run an allowlisted command as an argv vector (no shell). Example: {\"command\": [\"cargo\", \"test\"]}.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "array", "items": {"type": "string"}, "description": "argv vector"},
                    "timeout_seconds": {"type": "integer", "description": "1-300, default 60"}
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
    ]
}

pub struct ToolExecutor {
    root: PathBuf,
}

impl ToolExecutor {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Execute one tool call. Argument or policy failures become error outputs
    /// (fed back to the model), never crashes.
    pub async fn execute(
        &self,
        name: &str,
        arguments: &str,
        cancel: &CancellationToken,
    ) -> ToolOutput {
        match self.dispatch(name, arguments, cancel).await {
            Ok(output) => output,
            Err(ToolError::Cancelled) => ToolOutput::error("cancelled by user".into()),
            Err(error) => ToolOutput::error(error.to_string()),
        }
    }

    async fn dispatch(
        &self,
        name: &str,
        arguments: &str,
        cancel: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        fn parse<T: serde::de::DeserializeOwned>(arguments: &str) -> Result<T, ToolError> {
            serde_json::from_str(arguments)
                .map_err(|error| ToolError::Failed(format!("invalid arguments: {error}")))
        }
        match name {
            "read_file" => fs_tools::read_file(&self.root, parse(arguments)?),
            "list_files" => fs_tools::list_files(&self.root, parse(arguments)?),
            "search_text" => search::search_text(&self.root, parse(arguments)?),
            "edit" => fs_tools::edit(&self.root, parse(arguments)?),
            "write_file" => fs_tools::write_file(&self.root, parse(arguments)?),
            "run_command" => command::run_command(&self.root, parse(arguments)?, cancel).await,
            _ => Err(ToolError::Failed(format!("unknown tool: {name}"))),
        }
    }
}

/// Human-readable one-line preview of a tool call for approval prompts and
/// transcript rendering.
pub fn describe_call(name: &str, arguments: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(arguments).ok();
    let detail = parsed
        .as_ref()
        .map(|value| match name {
            "run_command" => value
                .get("command")
                .and_then(serde_json::Value::as_array)
                .map(|argv| {
                    argv.iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default(),
            _ => value
                .get("path")
                .or_else(|| value.get("pattern"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .unwrap_or_default();
    if detail.is_empty() {
        name.to_string()
    } else {
        format!("{name}({detail})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_serialize_deterministically_and_match_registry() {
        let a: Vec<String> = tool_schemas()
            .iter()
            .map(|schema| serde_json::to_string(&schema.parameters).unwrap())
            .collect();
        let b: Vec<String> = tool_schemas()
            .iter()
            .map(|schema| serde_json::to_string(&schema.parameters).unwrap())
            .collect();
        assert_eq!(a, b);
        let names: Vec<String> = tool_schemas().iter().map(|s| s.name.clone()).collect();
        let expected: Vec<&str> = TOOL_NAMES.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names.iter().map(String::as_str).collect::<Vec<_>>(),
            expected
        );
    }

    #[tokio::test]
    async fn unknown_tools_and_bad_arguments_become_error_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let executor = ToolExecutor::new(dir.path().to_path_buf());
        let cancel = CancellationToken::new();
        let unknown = executor.execute("nope", "{}", &cancel).await;
        assert!(unknown.is_error);
        let bad_args = executor.execute("read_file", "{not json", &cancel).await;
        assert!(bad_args.is_error);
        let unknown_field = executor
            .execute("read_file", r#"{"path":"a","bogus":1}"#, &cancel)
            .await;
        assert!(unknown_field.is_error);
    }

    #[test]
    fn call_previews_are_compact() {
        assert_eq!(
            describe_call("run_command", r#"{"command":["cargo","test"]}"#),
            "run_command(cargo test)"
        );
        assert_eq!(
            describe_call("read_file", r#"{"path":"src/main.rs"}"#),
            "read_file(src/main.rs)"
        );
    }
}

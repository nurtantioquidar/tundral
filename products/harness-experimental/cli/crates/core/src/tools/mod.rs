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

const DETAIL_CAP_BYTES: usize = 4_000;
const DIFF_CONTEXT_LINES: usize = 3;

/// Rich preview for approval prompts: what will actually change if the user
/// says yes. Returns None for calls where the one-line preview is enough or
/// when the arguments cannot be interpreted (the executor will surface the
/// real error after approval anyway).
pub fn approval_detail(root: &Path, name: &str, arguments: &str) -> Option<String> {
    let args: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let text = |key: &str| args.get(key).and_then(serde_json::Value::as_str);
    let detail = match name {
        "edit" => {
            let path = text("path")?;
            let old_string = text("old_string")?;
            let new_string = text("new_string")?;
            let replace_all = args
                .get("replace_all")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            edit_diff(root, path, old_string, new_string, replace_all)?
        }
        "write_file" => {
            let path = text("path")?;
            let content = text("content")?;
            let existing = fs_tools::resolve(root, path)
                .ok()
                .and_then(|resolved| std::fs::metadata(resolved).ok());
            let mut rendered = match existing {
                Some(meta) => format!(
                    "overwrites {path} ({} bytes) with {} bytes:\n",
                    meta.len(),
                    content.len()
                ),
                None => format!("creates {path} ({} bytes):\n", content.len()),
            };
            for line in content.lines().take(40) {
                rendered.push_str("  + ");
                rendered.push_str(line);
                rendered.push('\n');
            }
            if content.lines().count() > 40 {
                rendered.push_str("  … (truncated)\n");
            }
            rendered
        }
        "run_command" => {
            let argv = args.get("command")?.as_array()?;
            let line = argv
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            let timeout = args
                .get("timeout_seconds")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(60);
            format!("  $ {line}\n  (workspace root, {timeout}s timeout, credential-stripped env)\n")
        }
        _ => return None,
    };
    Some(truncate_detail(detail))
}

/// Unified-style diff for an exact-string edit: context lines, `-` for the
/// removed block, `+` for the replacement, anchored at the first match.
fn edit_diff(
    root: &Path,
    path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Option<String> {
    let resolved = fs_tools::resolve(root, path).ok()?;
    let original = std::fs::read_to_string(resolved).ok()?;
    let occurrences = original.matches(old_string).count();
    if occurrences == 0 {
        return Some(format!("{path}: old_string not found (edit will fail)\n"));
    }
    let match_start = original.find(old_string)?;
    let match_end = match_start + old_string.len();
    // Expand the match to whole lines so substring replacements render as the
    // resulting line, not the bare replacement text.
    let line_start = original[..match_start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = original[match_end..]
        .find('\n')
        .map_or(original.len(), |i| match_end + i);
    let old_block = &original[line_start..line_end];
    let new_block = format!(
        "{}{}{}",
        &original[line_start..match_start],
        new_string,
        &original[match_end..line_end]
    );
    let lines: Vec<&str> = original.lines().collect();
    let block_first = original[..match_start].matches('\n').count();
    let block_last = (block_first + old_block.lines().count().max(1)).min(lines.len());
    let match_line = block_first + 1;

    let mut rendered = format!(
        "{path}:{match_line}{}\n",
        if replace_all && occurrences > 1 {
            format!(" ({occurrences} occurrences; first shown, replace_all)")
        } else if occurrences > 1 {
            format!(" ({occurrences} occurrences — edit will fail without replace_all)")
        } else {
            String::new()
        }
    );
    for line in lines
        .iter()
        .take(block_first)
        .skip(block_first.saturating_sub(DIFF_CONTEXT_LINES))
    {
        rendered.push_str(&format!("    {line}\n"));
    }
    for line in old_block.lines() {
        rendered.push_str(&format!("  - {line}\n"));
    }
    if new_block.is_empty() {
        rendered.push_str("  + (deleted)\n");
    } else {
        for line in new_block.lines() {
            rendered.push_str(&format!("  + {line}\n"));
        }
    }
    for line in lines.iter().skip(block_last).take(DIFF_CONTEXT_LINES) {
        rendered.push_str(&format!("    {line}\n"));
    }
    Some(rendered)
}

fn truncate_detail(mut detail: String) -> String {
    if detail.len() > DETAIL_CAP_BYTES {
        let mut end = DETAIL_CAP_BYTES;
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
        detail.push_str("\n  … (truncated)\n");
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_detail_renders_edit_diffs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("f.rs"),
            "fn a() {}\nfn target() {\n    old_body();\n}\nfn z() {}\n",
        )
        .unwrap();
        let detail = approval_detail(
            dir.path(),
            "edit",
            r#"{"path":"f.rs","old_string":"    old_body();","new_string":"    new_body();\n    extra();"}"#,
        )
        .unwrap();
        assert!(detail.contains("f.rs:3"), "{detail}");
        assert!(detail.contains("  -     old_body();"), "{detail}");
        assert!(detail.contains("  +     new_body();"), "{detail}");
        assert!(detail.contains("  +     extra();"), "{detail}");
        assert!(detail.contains("    fn target() {"), "{detail}");
        assert!(detail.contains("    fn z() {}"), "{detail}");
    }

    #[test]
    fn approval_detail_flags_ambiguous_and_missing_edits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dup.txt"), "x\nx\n").unwrap();
        let ambiguous = approval_detail(
            dir.path(),
            "edit",
            r#"{"path":"dup.txt","old_string":"x","new_string":"y"}"#,
        )
        .unwrap();
        assert!(ambiguous.contains("edit will fail without replace_all"));
        let missing = approval_detail(
            dir.path(),
            "edit",
            r#"{"path":"dup.txt","old_string":"absent","new_string":"y"}"#,
        )
        .unwrap();
        assert!(missing.contains("not found"));
    }

    #[test]
    fn approval_detail_splices_substring_replacements_into_the_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("s.txt"), "hello world\nbye\n").unwrap();
        let detail = approval_detail(
            dir.path(),
            "edit",
            r#"{"path":"s.txt","old_string":"world","new_string":"rust"}"#,
        )
        .unwrap();
        assert!(detail.contains("  - hello world"), "{detail}");
        assert!(detail.contains("  + hello rust"), "{detail}");
    }

    #[test]
    fn approval_detail_previews_writes_and_commands() {
        let dir = tempfile::tempdir().unwrap();
        let write = approval_detail(
            dir.path(),
            "write_file",
            r#"{"path":"new.txt","content":"hello\nworld"}"#,
        )
        .unwrap();
        assert!(write.contains("creates new.txt"));
        assert!(write.contains("  + hello"));
        let command = approval_detail(
            dir.path(),
            "run_command",
            r#"{"command":["cargo","test"],"timeout_seconds":120}"#,
        )
        .unwrap();
        assert!(command.contains("$ cargo test"));
        assert!(command.contains("120s timeout"));
        let huge = approval_detail(
            dir.path(),
            "write_file",
            &format!(r#"{{"path":"big.txt","content":"{}"}}"#, "a".repeat(10_000)),
        )
        .unwrap();
        assert!(huge.len() < DETAIL_CAP_BYTES + 100);
    }

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

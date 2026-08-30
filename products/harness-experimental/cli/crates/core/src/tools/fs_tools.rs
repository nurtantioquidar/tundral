//! Filesystem tools: read, list, edit, write. All paths pass the policy
//! layer; writes are atomic (temp file + rename in the same directory).

use std::path::Path;

use serde::Deserialize;

use super::policy::{confine_to_root, validate_relative_path, PolicyError};
use super::{ToolError, ToolOutput};

const READ_CAP_BYTES: usize = 24_000;
const LIST_CAP_ENTRIES: usize = 500;
const WRITE_CAP_BYTES: usize = 500_000;

pub(crate) fn resolve(root: &Path, path: &str) -> Result<std::path::PathBuf, PolicyError> {
    let relative = validate_relative_path(path)?;
    confine_to_root(root, &relative)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileArgs {
    pub path: String,
    /// 1-based first line to include.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum number of lines to include.
    #[serde(default)]
    pub limit: Option<usize>,
}

pub fn read_file(root: &Path, args: ReadFileArgs) -> Result<ToolOutput, ToolError> {
    let path = resolve(root, &args.path)?;
    let bytes = std::fs::read(&path).map_err(|error| ToolError::Io(args.path.clone(), error))?;
    let text = String::from_utf8_lossy(&bytes);
    let start = args.offset.unwrap_or(1).max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut total_lines = 0usize;
    for (index, line) in text.lines().enumerate() {
        total_lines = index + 1;
        if index + 1 < start {
            continue;
        }
        if let Some(limit) = args.limit {
            if lines.len() >= limit {
                continue;
            }
        }
        lines.push(format!("{:>6}\t{line}", index + 1));
    }
    let mut body = lines.join("\n");
    let mut truncated = false;
    if body.len() > READ_CAP_BYTES {
        let mut end = READ_CAP_BYTES;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
        truncated = true;
    }
    if truncated {
        body.push_str("\n[output truncated: use offset/limit to read further]");
    }
    if body.is_empty() {
        body = format!("[empty file: {} lines total]", total_lines);
    }
    Ok(ToolOutput::text(body))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFilesArgs {
    /// Directory to list, relative to the workspace root. Defaults to the root.
    #[serde(default)]
    pub path: Option<String>,
}

pub fn list_files(root: &Path, args: ListFilesArgs) -> Result<ToolOutput, ToolError> {
    let base = match args.path.as_deref().filter(|p| !p.is_empty()) {
        Some(path) => resolve(root, path)?,
        None => root.to_path_buf(),
    };
    let mut entries = Vec::new();
    let walker = ignore::WalkBuilder::new(&base)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .sort_by_file_path(|a, b| a.cmp(b))
        .build();
    for entry in walker {
        let entry = entry.map_err(|error| ToolError::Failed(error.to_string()))?;
        if entry.depth() == 0 {
            continue;
        }
        let suffix = if entry.file_type().is_some_and(|t| t.is_dir()) {
            "/"
        } else {
            ""
        };
        let display = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .display();
        entries.push(format!("{display}{suffix}"));
        if entries.len() >= LIST_CAP_ENTRIES {
            entries.push(format!(
                "[listing truncated at {LIST_CAP_ENTRIES} entries: pass a narrower path]"
            ));
            break;
        }
    }
    if entries.is_empty() {
        entries.push("[empty directory]".to_string());
    }
    Ok(ToolOutput::text(entries.join("\n")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

pub fn edit(root: &Path, args: EditArgs) -> Result<ToolOutput, ToolError> {
    if args.old_string == args.new_string {
        return Err(ToolError::Failed(
            "old_string and new_string are identical".into(),
        ));
    }
    if args.old_string.is_empty() {
        return Err(ToolError::Failed(
            "old_string must not be empty; use write_file to create files".into(),
        ));
    }
    let path = resolve(root, &args.path)?;
    let original =
        std::fs::read_to_string(&path).map_err(|error| ToolError::Io(args.path.clone(), error))?;
    let occurrences = original.matches(&args.old_string).count();
    match occurrences {
        0 => {
            return Err(ToolError::Failed(format!(
                "old_string not found in {}",
                args.path
            )))
        }
        1 => {}
        n if !args.replace_all => {
            return Err(ToolError::Failed(format!(
                "old_string matches {n} times in {}; make it unique or set replace_all",
                args.path
            )))
        }
        _ => {}
    }
    let updated = if args.replace_all {
        original.replace(&args.old_string, &args.new_string)
    } else {
        original.replacen(&args.old_string, &args.new_string, 1)
    };
    atomic_write(&path, updated.as_bytes())?;
    let replaced = if args.replace_all { occurrences } else { 1 };
    Ok(ToolOutput::text(format!(
        "replaced {replaced} occurrence(s) in {}",
        args.path
    )))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
}

pub fn write_file(root: &Path, args: WriteFileArgs) -> Result<ToolOutput, ToolError> {
    if args.content.len() > WRITE_CAP_BYTES {
        return Err(ToolError::Failed(format!(
            "content exceeds {WRITE_CAP_BYTES} byte cap"
        )));
    }
    let path = resolve(root, &args.path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| ToolError::Io(args.path.clone(), error))?;
    }
    atomic_write(&path, args.content.as_bytes())?;
    Ok(ToolOutput::text(format!(
        "wrote {} bytes to {}",
        args.content.len(),
        args.path
    )))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let temp = directory.join(format!(
        ".{}.hx-tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&temp, bytes)
        .map_err(|error| ToolError::Io(temp.display().to_string(), error))?;
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        ToolError::Io(path.display().to_string(), error)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha\nbeta\ngamma\n").unwrap();
        dir
    }

    #[test]
    fn read_respects_offset_and_limit() {
        let dir = workspace();
        let output = read_file(
            dir.path(),
            ReadFileArgs {
                path: "a.txt".into(),
                offset: Some(2),
                limit: Some(1),
            },
        )
        .unwrap();
        assert_eq!(output.content, "     2\tbeta");
    }

    #[test]
    fn edit_requires_unique_match_and_is_atomic() {
        let dir = workspace();
        std::fs::write(dir.path().join("dup.txt"), "x x").unwrap();
        let ambiguous = edit(
            dir.path(),
            EditArgs {
                path: "dup.txt".into(),
                old_string: "x".into(),
                new_string: "y".into(),
                replace_all: false,
            },
        );
        assert!(ambiguous.is_err());
        edit(
            dir.path(),
            EditArgs {
                path: "dup.txt".into(),
                old_string: "x".into(),
                new_string: "y".into(),
                replace_all: true,
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dup.txt")).unwrap(),
            "y y"
        );
        // No temp files left behind.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("hx-tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn write_creates_nested_paths_inside_root_only() {
        let dir = workspace();
        write_file(
            dir.path(),
            WriteFileArgs {
                path: "nested/dir/new.txt".into(),
                content: "hello".into(),
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("nested/dir/new.txt")).unwrap(),
            "hello"
        );
        assert!(write_file(
            dir.path(),
            WriteFileArgs {
                path: "../escape.txt".into(),
                content: "x".into(),
            },
        )
        .is_err());
    }
}

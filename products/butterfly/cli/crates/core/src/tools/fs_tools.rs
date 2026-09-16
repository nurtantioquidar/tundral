//! Filesystem tools: read, list, edit, write. All paths pass the policy
//! layer; writes are atomic (temp file + rename in the same directory).

use std::path::Path;

use serde::Deserialize;

use super::policy::{resolve_path, PolicyError, PolicySettings};
use super::{ToolError, ToolOutput};

const READ_CAP_BYTES: usize = 24_000;
const LIST_CAP_ENTRIES: usize = 500;
const WRITE_CAP_BYTES: usize = 500_000;

pub(crate) fn resolve(
    root: &Path,
    path: &str,
    settings: &PolicySettings,
) -> Result<std::path::PathBuf, PolicyError> {
    resolve_path(root, path, settings)
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

/// Test convenience: the default (confined) policy, outline reads off.
#[cfg(test)]
pub fn read_file(root: &Path, args: ReadFileArgs) -> Result<ToolOutput, ToolError> {
    read_file_with(root, args, &PolicySettings::default(), 0)
}

/// `outline_min_lines`: a whole-file read (no offset, no limit) of a file with
/// at least this many lines returns an Atlas outline — definitions with exact
/// line ranges, imports — instead of the text, so a large file costs a few
/// hundred tokens to orient in rather than the 24 KB cap. 0 disables. The
/// model opts out per call with `offset: 1`. Files Atlas cannot parse fall
/// back to the plain read.
pub fn read_file_with(
    root: &Path,
    args: ReadFileArgs,
    settings: &PolicySettings,
    outline_min_lines: usize,
) -> Result<ToolOutput, ToolError> {
    let path = resolve(root, &args.path, settings)?;
    let bytes = std::fs::read(&path).map_err(|error| ToolError::Io(args.path.clone(), error))?;
    let text = String::from_utf8_lossy(&bytes);
    if outline_min_lines > 0
        && args.offset.is_none()
        && args.limit.is_none()
        && text.lines().count() >= outline_min_lines
    {
        if let Some(outline) = butterfly_atlas::outline(&args.path, &text) {
            let plain_bytes = plain_render_bytes(&text);
            let mut output = ToolOutput::text(cap_body(outline));
            output.avoided_bytes = plain_bytes.saturating_sub(output.content.len()) as u64;
            return Ok(output);
        }
    }
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
    let mut body = cap_body(lines.join("\n"));
    if body.is_empty() {
        body = format!("[empty file: {} lines total]", total_lines);
    }
    Ok(ToolOutput::text(body))
}

/// Size the plain numbered rendering would have had after the cap: what an
/// outline read keeps out of the transcript.
fn plain_render_bytes(text: &str) -> usize {
    let mut total = 0usize;
    for line in text.lines() {
        // "{:>6}\t{line}\n"
        total += 7 + line.len() + 1;
        if total > READ_CAP_BYTES {
            return READ_CAP_BYTES + "\n[output truncated: use offset/limit to read further]".len();
        }
    }
    total.saturating_sub(1)
}

fn cap_body(mut body: String) -> String {
    if body.len() > READ_CAP_BYTES {
        let mut end = READ_CAP_BYTES;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
        body.push_str("\n[output truncated: use offset/limit to read further]");
    }
    body
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFilesArgs {
    /// Directory to list, relative to the workspace root. Defaults to the root.
    #[serde(default)]
    pub path: Option<String>,
}

pub fn list_files_with(
    root: &Path,
    args: ListFilesArgs,
    settings: &PolicySettings,
) -> Result<ToolOutput, ToolError> {
    let base = match args.path.as_deref().filter(|p| !p.is_empty()) {
        Some(path) => resolve(root, path, settings)?,
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

/// Test convenience: the default (confined) policy.
#[cfg(test)]
pub fn edit(root: &Path, args: EditArgs) -> Result<ToolOutput, ToolError> {
    edit_with(root, args, &PolicySettings::default())
}

pub fn edit_with(
    root: &Path,
    args: EditArgs,
    settings: &PolicySettings,
) -> Result<ToolOutput, ToolError> {
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
    let path = resolve(root, &args.path, settings)?;
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

/// Test convenience: the default (confined) policy.
#[cfg(test)]
pub fn write_file(root: &Path, args: WriteFileArgs) -> Result<ToolOutput, ToolError> {
    write_file_with(root, args, &PolicySettings::default())
}

pub fn write_file_with(
    root: &Path,
    args: WriteFileArgs,
    settings: &PolicySettings,
) -> Result<ToolOutput, ToolError> {
    if args.content.len() > WRITE_CAP_BYTES {
        return Err(ToolError::Failed(format!(
            "content exceeds {WRITE_CAP_BYTES} byte cap"
        )));
    }
    let path = resolve(root, &args.path, settings)?;
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
        ".{}.butterfly-tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&temp, bytes)
        .map_err(|error| ToolError::Io(temp.display().to_string(), error))?;
    // Replacing a file must not silently change what it is: keep the original
    // mode (an edited script stays executable) and, when running as root in a
    // container, the original owner (a privilege-dropped process must still
    // read it). Observed on Terminal-Bench 2026-09-04.
    if let Ok(existing) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&temp, existing.permissions());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let _ = std::os::unix::fs::chown(&temp, Some(existing.uid()), Some(existing.gid()));
        }
    }
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

    #[cfg(unix)]
    #[test]
    fn rewriting_a_file_preserves_its_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = workspace();
        let script = dir.path().join("run.sh");
        std::fs::write(&script, "#!/bin/sh\necho old\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        edit(
            dir.path(),
            EditArgs {
                path: "run.sh".into(),
                old_string: "old".into(),
                new_string: "new".into(),
                replace_all: false,
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(&script).unwrap().permissions().mode() & 0o777,
            0o755,
            "edit must keep the executable bit"
        );
        write_file(
            dir.path(),
            WriteFileArgs {
                path: "run.sh".into(),
                content: "#!/bin/sh\necho again\n".into(),
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(&script).unwrap().permissions().mode() & 0o777,
            0o755,
            "write_file over an existing file must keep its mode"
        );
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
            .filter(|e| e.file_name().to_string_lossy().contains("butterfly-tmp"))
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

    #[test]
    fn a_whole_file_read_of_a_large_java_file_returns_an_outline_with_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let mut source =
            String::from("package a;\n\nimport java.util.List;\n\npublic class Job {\n");
        for i in 0..200 {
            source.push_str(&format!(
                "    public void step{i}(List<String> in) {{\n        go(in);\n    }}\n"
            ));
        }
        source.push_str("}\n");
        std::fs::write(dir.path().join("Job.java"), &source).unwrap();
        let args = || ReadFileArgs {
            path: "Job.java".into(),
            offset: None,
            limit: None,
        };
        let outline = read_file_with(dir.path(), args(), &PolicySettings::default(), 400).unwrap();
        assert!(
            outline.content.starts_with("[outline] Job.java: 606 lines"),
            "{}",
            outline.content
        );
        assert!(outline
            .content
            .contains("[fn] public void step199(List<String> in) {"));
        assert!(
            !outline.content.contains("go(in);"),
            "bodies stay out of the outline"
        );
        assert!(
            outline.avoided_bytes > 3_000,
            "the plain read is ~17 KB, the outline ~11 KB; avoided {}",
            outline.avoided_bytes
        );
        // Below the threshold, or with an explicit range, the text comes back.
        let plain = read_file_with(dir.path(), args(), &PolicySettings::default(), 1_000).unwrap();
        assert!(plain.content.starts_with("     1\tpackage a;"));
        let ranged = read_file_with(
            dir.path(),
            ReadFileArgs {
                path: "Job.java".into(),
                offset: Some(1),
                limit: None,
            },
            &PolicySettings::default(),
            400,
        )
        .unwrap();
        assert!(ranged.content.starts_with("     1\tpackage a;"));
        // 0 disables outlines entirely.
        let off = read_file(dir.path(), args()).unwrap();
        assert!(off.content.starts_with("     1\tpackage a;"));
    }

    #[test]
    fn a_large_file_atlas_cannot_parse_falls_back_to_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let source: String = (0..500).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.path().join("notes.txt"), &source).unwrap();
        let out = read_file_with(
            dir.path(),
            ReadFileArgs {
                path: "notes.txt".into(),
                offset: None,
                limit: None,
            },
            &PolicySettings::default(),
            400,
        )
        .unwrap();
        assert!(out.content.starts_with("     1\tline 0"));
    }
}

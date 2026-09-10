//! Text search built on the ripgrep library crates (`ignore` + `grep-*`), so
//! there is no external binary dependency and results respect .gitignore.

use std::path::Path;

use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use serde::Deserialize;

use super::policy::{resolve_path, PolicySettings};
use super::{ToolError, ToolOutput};

const MATCH_CAP: usize = 200;
const OUTPUT_CAP_BYTES: usize = 30_000;
/// A single matching line is clipped to this many bytes: one hit inside a
/// minified bundle or a one-line data file must not inject a megabyte into
/// the context (observed on Terminal-Bench 2026-09-04: a 375k-token request).
const LINE_CAP_BYTES: usize = 400;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchTextArgs {
    /// Regular expression (Rust regex syntax).
    pub pattern: String,
    /// Directory or file to search, relative to the workspace root.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub case_insensitive: bool,
}

pub fn search_text(root: &Path, args: SearchTextArgs) -> Result<ToolOutput, ToolError> {
    search_text_with(root, args, &PolicySettings::default())
}

pub fn search_text_with(
    root: &Path,
    args: SearchTextArgs,
    settings: &PolicySettings,
) -> Result<ToolOutput, ToolError> {
    let base = match args.path.as_deref().filter(|p| !p.is_empty()) {
        Some(path) => resolve_path(root, path, settings)?,
        None => root.to_path_buf(),
    };
    let matcher = RegexMatcherBuilder::new()
        .case_insensitive(args.case_insensitive)
        .line_terminator(Some(b'\n'))
        .build(&args.pattern)
        .map_err(|error| ToolError::Failed(format!("invalid pattern: {error}")))?;
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .line_number(true)
        .build();

    let mut results: Vec<String> = Vec::new();
    let mut total_bytes = 0usize;
    let mut hit_cap = false;
    let walker = ignore::WalkBuilder::new(&base)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .sort_by_file_path(|a, b| a.cmp(b))
        .build();
    for entry in walker {
        if hit_cap {
            break;
        }
        let entry = entry.map_err(|error| ToolError::Failed(error.to_string()))?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let display = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .display()
            .to_string();
        let sink = UTF8(|line_number, line| {
            let rendered = format!("{display}:{line_number}: {}", clip_line(line.trim_end()));
            total_bytes += rendered.len();
            results.push(rendered);
            if results.len() >= MATCH_CAP || total_bytes >= OUTPUT_CAP_BYTES {
                hit_cap = true;
                return Ok(false);
            }
            Ok(true)
        });
        // Per-file search errors (permissions, encoding) are skipped, not fatal.
        let _ = searcher.search_path(&matcher, entry.path(), sink);
    }
    if hit_cap {
        results.push("[results truncated: narrow the pattern or path]".to_string());
    }
    if results.is_empty() {
        results.push("[no matches]".to_string());
    }
    Ok(ToolOutput::text(results.join("\n")))
}

/// Clip an over-long match line on a char boundary, keeping the head (where
/// the path/identifier context usually is) and noting the cut.
fn clip_line(line: &str) -> std::borrow::Cow<'_, str> {
    if line.len() <= LINE_CAP_BYTES {
        return std::borrow::Cow::Borrowed(line);
    }
    let mut end = LINE_CAP_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    std::borrow::Cow::Owned(format!(
        "{}… [line truncated: {} bytes]",
        &line[..end],
        line.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn giant_single_lines_are_clipped_not_dumped() {
        let dir = tempfile::tempdir().unwrap();
        let blob = format!("needle{}", "x".repeat(1_500_000));
        std::fs::write(dir.path().join("bundle.min.js"), &blob).unwrap();
        let output = search_text(
            dir.path(),
            SearchTextArgs {
                pattern: "needle".into(),
                path: None,
                case_insensitive: false,
            },
        )
        .unwrap();
        assert!(output.content.len() < 1_000, "{}", output.content.len());
        assert!(output.content.contains("[line truncated: 1500006 bytes]"));
        assert!(output.content.starts_with("bundle.min.js:1: needle"));
    }

    #[test]
    fn searches_with_line_numbers_and_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hit.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
        std::fs::create_dir(dir.path().join("skipped")).unwrap();
        std::fs::write(dir.path().join("skipped/hidden.rs"), "fn alpha() {}\n").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "skipped/\n").unwrap();
        let output = search_text(
            dir.path(),
            SearchTextArgs {
                pattern: "fn alpha".into(),
                path: None,
                case_insensitive: false,
            },
        )
        .unwrap();
        assert_eq!(output.content, "hit.rs:1: fn alpha() {}");
    }

    #[test]
    fn rejects_invalid_patterns_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let result = search_text(
            dir.path(),
            SearchTextArgs {
                pattern: "(unclosed".into(),
                path: None,
                case_insensitive: false,
            },
        );
        assert!(result.is_err());
    }
}

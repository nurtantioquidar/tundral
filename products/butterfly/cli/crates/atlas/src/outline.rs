//! Outline rendering for one file: the deterministic, addressable alternative
//! to pasting a large file into the context. Every entry carries the exact
//! 1-based line range of its node, so the model can follow up with a ranged
//! read and edit against real line numbers — which is what a model-written
//! summary (Spotify Portal's shunt, 2026-09) cannot offer.

use std::path::Path;

use crate::extract::{extract, DefKind, Definition, Lang};

/// Imports listed verbatim before the list collapses to a count.
const MAX_IMPORTS_SHOWN: usize = 24;
/// Definitions listed before the outline truncates (generated files).
const MAX_DEFS_SHOWN: usize = 400;

/// Render `source` (at `display_path`) as an outline, or `None` when the
/// language is unknown or the file yields no definitions — the caller then
/// falls back to the plain read.
pub fn outline(display_path: &str, source: &str) -> Option<String> {
    let lang = Lang::from_path(Path::new(display_path))?;
    let extraction = extract(lang, source);
    if extraction.definitions.is_empty() {
        return None;
    }
    let total_lines = source.lines().count();
    let mut out = format!(
        "[outline] {display_path}: {total_lines} lines, {} bytes. The file is large, so read_file \
         returned its structure instead of the text. Line ranges are exact: read the parts you \
         need with offset/limit (offset=1 with no limit returns the raw text).\n",
        source.len()
    );
    if !extraction.imports.is_empty() {
        let shown: Vec<&str> = extraction
            .imports
            .iter()
            .take(MAX_IMPORTS_SHOWN)
            .map(String::as_str)
            .collect();
        let more = extraction.imports.len().saturating_sub(shown.len());
        out.push_str(&format!(
            "imports ({}): {}",
            extraction.imports.len(),
            shown.join(", ")
        ));
        if more > 0 {
            out.push_str(&format!(", +{more} more"));
        }
        out.push('\n');
    }
    let mut defs: Vec<&Definition> = extraction.definitions.iter().collect();
    defs.sort_by_key(|def| (def.line, def.end_line));
    let width = total_lines.to_string().len();
    for def in defs.iter().take(MAX_DEFS_SHOWN) {
        let range = if def.end_line > def.line {
            format!("{:>width$}-{:<width$}", def.line, def.end_line)
        } else {
            format!("{:>width$} {:<width$}", def.line, "")
        };
        let indent = if def.container.is_some() { "  " } else { "" };
        let signature = match def.kind {
            // A field's own line is its signature; a container's first line
            // already names it, so keep both short.
            DefKind::Field => def.signature.clone(),
            _ => def.signature.clone(),
        };
        out.push_str(&format!(
            "{range}  {indent}[{}] {signature}\n",
            def.kind.label()
        ));
    }
    if defs.len() > MAX_DEFS_SHOWN {
        out.push_str(&format!(
            "[outline truncated: {} more definitions; read ranges directly]\n",
            defs.len() - MAX_DEFS_SHOWN
        ));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_outline_lists_ranges_imports_and_nesting() {
        let source = "package a;\n\nimport java.util.List;\nimport java.util.Map;\n\npublic class Job {\n    private static final int CAP = 1;\n\n    public void run(List<String> in) {\n        go(in);\n    }\n}\n";
        let text = outline("src/Job.java", source).expect("outline");
        assert!(
            text.starts_with("[outline] src/Job.java: 12 lines"),
            "{text}"
        );
        assert!(text.contains("imports (2): java.util.List, java.util.Map"));
        assert!(text.contains(" 6-12  [class] public class Job {"), "{text}");
        assert!(
            text.contains("  [field] private static final int CAP = 1;"),
            "{text}"
        );
        assert!(
            text.contains(" 9-11    [fn] public void run(List<String> in) {"),
            "{text}"
        );
    }

    #[test]
    fn unknown_language_or_no_definitions_yields_none() {
        assert!(outline("notes.md", "# heading\n").is_none());
        assert!(outline("empty.py", "x = 1\n").is_none());
    }

    #[test]
    fn imports_collapse_past_the_cap() {
        let mut source = String::new();
        for i in 0..30 {
            source.push_str(&format!("import m{i}\n"));
        }
        source.push_str("def f():\n    pass\n");
        let text = outline("big.py", &source).unwrap();
        assert!(text.contains("imports (30):"), "{text}");
        assert!(text.contains(", +6 more"), "{text}");
    }
}

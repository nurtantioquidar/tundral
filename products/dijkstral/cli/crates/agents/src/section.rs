//! The dijkstral section of a Markdown instruction file (`CLAUDE.md`, `AGENTS.md`).
//!
//! A section is a heading line `#…# <title>` (at its level, surrounding whitespace tolerated) up to
//! the next heading of the same or a higher level, or the end of the file. Unlike Graphify
//! (§12.3), a higher-level heading also ends it (so a following `# H1` section is never swallowed)
//! and headings inside fenced code blocks are ignored.

/// Byte ranges of every section headed `title` at `level`.
fn find(text: &str, level: usize, title: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut open: Option<usize> = None;
    let mut fence: Option<&str> = None;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let start = pos;
        pos += line.len();
        let trimmed = line.trim();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            fence = Some("```");
            continue;
        }
        if trimmed.starts_with("~~~") {
            fence = Some("~~~");
            continue;
        }
        let Some(heading_level) = heading_level(line) else {
            continue;
        };
        if heading_level <= level {
            if let Some(s) = open.take() {
                found.push((s, start));
            }
        }
        if heading_level == level && trimmed[level..].trim() == title {
            open = Some(start);
        }
    }
    if let Some(s) = open {
        found.push((s, text.len()));
    }
    found
}

/// The ATX heading level of `line`, if it is one (at most three leading spaces).
fn heading_level(line: &str) -> Option<usize> {
    let stripped = line.trim_start_matches(' ');
    if line.len() - stripped.len() > 3 {
        return None;
    }
    let hashes = stripped.bytes().take_while(|&b| b == b'#').count();
    let after = stripped[hashes..].chars().next();
    ((1..=6).contains(&hashes) && after.is_none_or(char::is_whitespace)).then_some(hashes)
}

/// `text` with the section written: the first existing one replaced in place (later duplicates
/// removed), or `section` appended after one blank line. `None` when nothing changes.
pub fn upsert(text: &str, level: usize, title: &str, section: &str) -> Option<String> {
    let spans = find(text, level, title);
    let out = match spans.first() {
        None if text.trim().is_empty() => section.to_string(),
        None => {
            let mut out = text.to_string();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
            out.push_str(section);
            out
        }
        Some(&(_, first_end)) => {
            let mut out = String::with_capacity(text.len() + section.len());
            let mut cursor = 0;
            for (i, &(s, e)) in spans.iter().enumerate() {
                out.push_str(&text[cursor..s]);
                if i == 0 {
                    out.push_str(section);
                    if first_end < text.len() {
                        out.push('\n');
                    }
                }
                cursor = e;
            }
            out.push_str(&text[cursor..]);
            out
        }
    };
    (out != text).then_some(out)
}

/// `text` with every such section removed; `None` when there is none. A section at the end of the
/// file also takes the one blank line `upsert` put before it, so install then uninstall restores
/// a file that ended with a newline byte for byte.
pub fn remove(text: &str, level: usize, title: &str) -> Option<String> {
    let spans = find(text, level, title);
    if spans.is_empty() {
        return None;
    }
    let mut out = text.to_string();
    for &(s, e) in spans.iter().rev() {
        if e == out.len() {
            out.truncate(s);
            if out.ends_with("\n\n") {
                out.pop();
            }
        } else {
            out.replace_range(s..e, "");
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECTION: &str = "## dijkstral\n\nUse the graph.\n";

    fn up(text: &str) -> String {
        upsert(text, 2, "dijkstral", SECTION).unwrap_or_else(|| text.to_string())
    }

    #[test]
    fn append_and_remove_round_trip() {
        for original in [
            "# Project\n\nRules.\n",
            "# Project\n\nRules.\n\n\n",
            "## Other\n- a\n",
        ] {
            let installed = up(original);
            assert_eq!(installed, format!("{original}\n{SECTION}"));
            assert_eq!(upsert(&installed, 2, "dijkstral", SECTION), None);
            assert_eq!(remove(&installed, 2, "dijkstral").unwrap(), original);
        }
        // A missing final newline is added, and stays after removal.
        assert_eq!(up("text"), format!("text\n\n{SECTION}"));
        assert_eq!(remove(&up("text"), 2, "dijkstral").unwrap(), "text\n");
        // An empty or whitespace-only file becomes the section.
        assert_eq!(up(""), SECTION);
        assert_eq!(up("\n\n"), SECTION);
        assert_eq!(remove(SECTION, 2, "dijkstral").unwrap(), "");
    }

    #[test]
    fn replaces_in_place_and_keeps_neighbours() {
        let text =
            "# P\n\nIntro.\n\n## dijkstral\n\nold text\n### detail\nmore\n\n## After\n\nKeep.\n";
        let installed = up(text);
        assert_eq!(
            installed,
            format!("# P\n\nIntro.\n\n{SECTION}\n## After\n\nKeep.\n")
        );
        assert_eq!(upsert(&installed, 2, "dijkstral", SECTION), None);
        assert_eq!(
            remove(&installed, 2, "dijkstral").unwrap(),
            "# P\n\nIntro.\n\n## After\n\nKeep.\n"
        );
    }

    #[test]
    fn a_higher_level_heading_ends_the_section() {
        let text = format!("{SECTION}\n# Next chapter\n\nKeep me.\n");
        assert_eq!(
            remove(&text, 2, "dijkstral").unwrap(),
            "# Next chapter\n\nKeep me.\n"
        );
    }

    #[test]
    fn fenced_headings_are_ignored() {
        let text = "# P\n\n```md\n## dijkstral\n```\n\nRest.\n";
        assert_eq!(remove(text, 2, "dijkstral"), None);
        let installed = up(text);
        assert!(installed.starts_with(text));
        let fenced_inside = format!("{SECTION}\n```\n## not a heading\n```\n## After\n");
        assert_eq!(
            remove(&fenced_inside, 2, "dijkstral").unwrap(),
            "## After\n"
        );
    }

    #[test]
    fn other_headings_are_not_ours() {
        for text in [
            "## Dijkstral\n",
            "### dijkstral\n",
            "# dijkstral\n",
            "## dijkstral notes\n",
        ] {
            assert_eq!(remove(text, 2, "dijkstral"), None, "{text}");
        }
        assert!(remove("  ## dijkstral  \nx\n", 2, "dijkstral").is_some());
    }

    #[test]
    fn duplicates_collapse_to_one() {
        let text = format!("{SECTION}\n## Mid\n\n{SECTION}");
        let installed = up(&text);
        assert_eq!(installed, format!("{SECTION}\n## Mid\n\n"));
        assert_eq!(remove(&text, 2, "dijkstral").unwrap(), "## Mid\n");
    }

    #[test]
    fn level_one_sections() {
        let section = "# dijkstral\n\nPointer.\n";
        let original = "# Mine\n\n## Sub\n";
        let installed = upsert(original, 1, "dijkstral", section).unwrap();
        assert_eq!(installed, format!("{original}\n{section}"));
        assert_eq!(remove(&installed, 1, "dijkstral").unwrap(), original);
    }
}

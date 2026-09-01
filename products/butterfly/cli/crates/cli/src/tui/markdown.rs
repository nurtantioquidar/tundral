//! Line-buffered terminal markdown: assistant output is styled one completed
//! line at a time (streaming never rewrites scrollback). Deliberately small:
//! headings, bold, inline code, bullets, and dim fence delimiters — no
//! heavyweight syntax-highlighting dependency.

const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

#[derive(Default)]
pub struct MarkdownRenderer {
    in_fence: bool,
}

impl MarkdownRenderer {
    /// Style one complete output line, hiding markdown syntax markers
    /// (Claude Code-style rendering: styled text, not annotated text).
    /// Stateful across lines for code fences.
    pub fn render_line(&mut self, line: &str) -> String {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            self.in_fence = !self.in_fence;
            // The delimiter itself renders as a blank spacer line.
            return String::new();
        }
        if self.in_fence {
            // Code stays byte-identical for copy-paste; only tinted.
            return format!("{CYAN}{line}{RESET}");
        }
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count();
            if level <= 6 && trimmed[level..].starts_with(' ') {
                let text = trimmed[level..].trim_start();
                return format!("{BOLD}{CYAN}{text}{RESET}");
            }
        }
        let styled = style_inline(line);
        for bullet in ["- ", "* "] {
            if let Some(rest) = trimmed.strip_prefix(bullet) {
                let indent = &line[..line.len() - trimmed.len()];
                return format!("{indent}{CYAN}•{RESET} {}", style_inline(rest));
            }
        }
        styled
    }

    #[cfg(test)]
    pub fn in_fence(&self) -> bool {
        self.in_fence
    }
}

/// `**bold**` and `` `code` `` spans; unmatched markers pass through as-is.
fn style_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let mut rest = line;
    loop {
        // (offset, is_bold) of the earliest span opener, if any.
        let next = match (rest.find("**"), rest.find('`')) {
            (None, None) => None,
            (Some(b), None) => Some((b, true)),
            (None, Some(c)) => Some((c, false)),
            (Some(b), Some(c)) => Some(if b < c { (b, true) } else { (c, false) }),
        };
        let Some((at, is_bold)) = next else {
            out.push_str(rest);
            return out;
        };
        let (open_len, close, style) = if is_bold {
            (2, "**", BOLD)
        } else {
            (1, "`", CYAN)
        };
        match rest[at + open_len..].find(close) {
            Some(end) => {
                out.push_str(&rest[..at]);
                out.push_str(style);
                out.push_str(&rest[at + open_len..at + open_len + end]);
                out.push_str(RESET);
                rest = &rest[at + open_len + end + close.len()..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_and_bullets_are_styled_with_markers_hidden() {
        let mut md = MarkdownRenderer::default();
        let heading = md.render_line("## Plan");
        assert!(heading.contains(BOLD));
        assert!(!heading.contains('#'), "heading markers must be hidden");
        assert!(heading.contains("Plan"));
        assert!(md.render_line("- do the thing").contains('•'));
        assert!(md.render_line("  * nested").contains('•'));
        // A bare #hashtag is not a heading.
        let hashtag = md.render_line("#hashtag");
        assert!(!hashtag.contains(BOLD));
        assert!(hashtag.contains("#hashtag"));
    }

    #[test]
    fn fences_toggle_state_hide_delimiters_and_tint_content() {
        let mut md = MarkdownRenderer::default();
        let open = md.render_line("```rust");
        assert!(open.is_empty(), "fence delimiters render as spacer lines");
        assert!(md.in_fence());
        let code = md.render_line("let **x** = 1;");
        // Inside a fence, inline markers must NOT be interpreted.
        assert!(code.contains("**x**"));
        assert!(md.render_line("```").is_empty());
        assert!(!md.in_fence());
    }

    #[test]
    fn inline_spans_are_styled_and_unmatched_markers_pass_through() {
        let mut md = MarkdownRenderer::default();
        let line = md.render_line("use **edit** on `src/main.rs` now");
        assert!(line.contains(&format!("{BOLD}edit{RESET}")));
        assert!(line.contains(&format!("{CYAN}src/main.rs{RESET}")));
        let odd = md.render_line("2 ** 3 is not bold");
        assert!(odd.contains("2 ** 3"));
    }
}

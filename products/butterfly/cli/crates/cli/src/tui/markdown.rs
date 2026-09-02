//! Line-buffered markdown → styled spans (streaming never rewrites
//! scrollback). Markers are hidden, Claude Code-style; prose comes back as
//! spans plus a hanging indent so the caller can word-wrap it to the
//! terminal — mid-word terminal wrapping at column 0 is what makes raw
//! transcripts unreadable. Deliberately small: headings, bold, inline code,
//! bullet/numbered lists, tinted fences.

pub const BOLD: &str = "\x1b[1m";
pub const CYAN: &str = "\x1b[36m";
pub const RESET: &str = "\x1b[0m";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanStyle {
    Plain,
    Bold,
    Code,
    Heading,
    Marker,
    /// Secondary text (reasoning, status notes): dim, re-wrappable.
    Dim,
}

impl SpanStyle {
    pub fn escape(self) -> &'static str {
        match self {
            SpanStyle::Plain => "",
            SpanStyle::Bold => BOLD,
            SpanStyle::Code => CYAN,
            SpanStyle::Heading => "\x1b[1m\x1b[36m",
            SpanStyle::Marker => CYAN,
            SpanStyle::Dim => "\x1b[2m",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub style: SpanStyle,
    pub text: String,
}

fn span(style: SpanStyle, text: impl Into<String>) -> Span {
    Span {
        style,
        text: text.into(),
    }
}

/// One rendered logical line.
#[derive(Debug, PartialEq, Eq)]
pub enum Rendered {
    /// Pre-styled verbatim content (code); never word-wrapped so it stays
    /// byte-identical for copy-paste. Empty for hidden fence delimiters.
    Raw(String),
    /// Prose: styled spans; continuation rows indent by `cont_indent` so
    /// wrapped list items hang under their own text.
    Prose {
        spans: Vec<Span>,
        cont_indent: usize,
    },
}

#[derive(Default)]
pub struct MarkdownRenderer {
    in_fence: bool,
}

impl MarkdownRenderer {
    /// Render one complete output line. Stateful across lines for fences.
    pub fn render(&mut self, line: &str) -> Rendered {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.starts_with("```") {
            self.in_fence = !self.in_fence;
            // The delimiter itself renders as a blank spacer line.
            return Rendered::Raw(String::new());
        }
        if self.in_fence {
            return Rendered::Raw(format!("{CYAN}{line}{RESET}"));
        }
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count();
            if level <= 6 && trimmed[level..].starts_with(' ') {
                let text = trimmed[level..].trim_start();
                return Rendered::Prose {
                    spans: vec![span(SpanStyle::Heading, text)],
                    cont_indent: 0,
                };
            }
        }
        for bullet in ["- ", "* "] {
            if let Some(rest) = trimmed.strip_prefix(bullet) {
                let mut spans = Vec::new();
                if indent > 0 {
                    spans.push(span(SpanStyle::Plain, " ".repeat(indent)));
                }
                spans.push(span(SpanStyle::Marker, "• "));
                spans.extend(inline_spans(rest));
                return Rendered::Prose {
                    spans,
                    cont_indent: indent + 2,
                };
            }
        }
        if let Some((number, rest)) = split_numbered_item(trimmed) {
            let mut spans = Vec::new();
            if indent > 0 {
                spans.push(span(SpanStyle::Plain, " ".repeat(indent)));
            }
            let hang = indent + number.len() + 1;
            spans.push(span(SpanStyle::Marker, format!("{number} ")));
            spans.extend(inline_spans(rest));
            return Rendered::Prose {
                spans,
                cont_indent: hang,
            };
        }
        Rendered::Prose {
            spans: inline_spans(line),
            cont_indent: 0,
        }
    }

    #[cfg(test)]
    pub fn in_fence(&self) -> bool {
        self.in_fence
    }
}

/// `1. text` / `12) text` list openers.
fn split_numbered_item(text: &str) -> Option<(&str, &str)> {
    let digits = text.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let rest = &text[digits..];
    let punct = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    let body = punct.strip_prefix(' ')?;
    Some((&text[..digits + 1], body))
}

/// `**bold**` and `` `code` `` spans; unmatched markers pass through as-is.
fn inline_spans(line: &str) -> Vec<Span> {
    let mut spans = Vec::new();
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
            if !rest.is_empty() {
                spans.push(span(SpanStyle::Plain, rest));
            }
            return spans;
        };
        let (open_len, close, style) = if is_bold {
            (2, "**", SpanStyle::Bold)
        } else {
            (1, "`", SpanStyle::Code)
        };
        match rest[at + open_len..].find(close) {
            Some(end) => {
                if at > 0 {
                    spans.push(span(SpanStyle::Plain, &rest[..at]));
                }
                spans.push(span(style, &rest[at + open_len..at + open_len + end]));
                rest = &rest[at + open_len + end + close.len()..];
            }
            None => {
                if !rest.is_empty() {
                    spans.push(span(SpanStyle::Plain, rest));
                }
                return spans;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_text(rendered: &Rendered) -> String {
        match rendered {
            Rendered::Raw(text) => text.clone(),
            Rendered::Prose { spans, .. } => spans.iter().map(|s| s.text.as_str()).collect(),
        }
    }

    #[test]
    fn headings_hide_markers_and_style() {
        let mut md = MarkdownRenderer::default();
        let rendered = md.render("## Plan");
        match &rendered {
            Rendered::Prose { spans, .. } => {
                assert_eq!(spans.len(), 1);
                assert_eq!(spans[0].style, SpanStyle::Heading);
                assert_eq!(spans[0].text, "Plan");
            }
            other => panic!("expected prose, got {other:?}"),
        }
        // A bare #hashtag is not a heading.
        assert_eq!(plain_text(&md.render("#hashtag")), "#hashtag");
    }

    #[test]
    fn bullets_and_numbered_items_hang_their_continuations() {
        let mut md = MarkdownRenderer::default();
        match md.render("- do the thing") {
            Rendered::Prose { spans, cont_indent } => {
                assert_eq!(spans[0].text, "• ");
                assert_eq!(cont_indent, 2);
            }
            other => panic!("{other:?}"),
        }
        match md.render("  * nested") {
            Rendered::Prose { cont_indent, .. } => assert_eq!(cont_indent, 4),
            other => panic!("{other:?}"),
        }
        match md.render("12. numbered item") {
            Rendered::Prose { spans, cont_indent } => {
                assert_eq!(spans[0].text, "12. ");
                assert_eq!(cont_indent, 4);
            }
            other => panic!("{other:?}"),
        }
        // A year is not a list item.
        assert_eq!(plain_text(&md.render("2026 was a year")), "2026 was a year");
    }

    #[test]
    fn fences_toggle_state_hide_delimiters_and_never_wrap() {
        let mut md = MarkdownRenderer::default();
        assert_eq!(md.render("```rust"), Rendered::Raw(String::new()));
        assert!(md.in_fence());
        match md.render("let **x** = 1;") {
            // Inside a fence, inline markers must NOT be interpreted.
            Rendered::Raw(styled) => assert!(styled.contains("**x**")),
            other => panic!("{other:?}"),
        }
        assert_eq!(md.render("```"), Rendered::Raw(String::new()));
        assert!(!md.in_fence());
    }

    #[test]
    fn inline_spans_strip_markers_and_pass_unmatched_through() {
        let spans = inline_spans("use **edit** on `src/main.rs` now");
        let styles: Vec<SpanStyle> = spans.iter().map(|s| s.style).collect();
        assert_eq!(
            styles,
            vec![
                SpanStyle::Plain,
                SpanStyle::Bold,
                SpanStyle::Plain,
                SpanStyle::Code,
                SpanStyle::Plain,
            ]
        );
        assert_eq!(spans[1].text, "edit");
        assert_eq!(spans[3].text, "src/main.rs");
        let odd = inline_spans("2 ** 3 is not bold");
        assert_eq!(odd.len(), 1);
        assert_eq!(odd[0].text, "2 ** 3 is not bold");
    }
}

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
    Italic,
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
            SpanStyle::Italic => "\x1b[3m",
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

/// One level of nesting: extract `` `code` `` spans inside emphasized text
/// so identifiers keep the code color even when the model bolds them
/// (`**\`ContextConfig\`**` must not leak raw backticks).
fn nested_spans(content: &str, outer: SpanStyle) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut rest = content;
    while let Some(open) = rest.find('`') {
        let Some(close) = rest[open + 1..].find('`') else {
            break;
        };
        if open > 0 {
            spans.push(span(outer, &rest[..open]));
        }
        spans.push(span(SpanStyle::Code, &rest[open + 1..open + 1 + close]));
        rest = &rest[open + 1 + close + 1..];
    }
    if !rest.is_empty() {
        spans.push(span(outer, rest));
    }
    spans
}

/// A single-`*` emphasis closing at the FIRST `*` in `content` (the text
/// after the opener). Valid only when the content neither starts nor ends
/// with whitespace — so `2 * 3` stays arithmetic, `*effective*` italicizes.
fn emphasis_end(content: &str) -> Option<usize> {
    let first = content.chars().next()?;
    if first.is_whitespace() || first == '*' {
        return None;
    }
    let close = content.find('*')?;
    let before = content[..close].chars().next_back()?;
    if before.is_whitespace() {
        return None;
    }
    Some(close)
}

/// `**bold**`, `*italic*`, and `` `code` `` spans (code also recognized one
/// level inside emphasis); unmatched markers pass through as-is.
fn inline_spans(line: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    fn flush(plain: &mut String, spans: &mut Vec<Span>) {
        if !plain.is_empty() {
            spans.push(span(SpanStyle::Plain, std::mem::take(plain)));
        }
    }
    let mut i = 0usize;
    while i < line.len() {
        let rest = &line[i..];
        if let Some(after) = rest.strip_prefix("**") {
            if let Some(end) = after.find("**") {
                flush(&mut plain, &mut spans);
                spans.extend(nested_spans(&after[..end], SpanStyle::Bold));
                i += 2 + end + 2;
                continue;
            }
        } else if let Some(after) = rest.strip_prefix('`') {
            if let Some(end) = after.find('`') {
                flush(&mut plain, &mut spans);
                spans.push(span(SpanStyle::Code, &after[..end]));
                i += 1 + end + 1;
                continue;
            }
        } else if let Some(after) = rest.strip_prefix('*') {
            if let Some(end) = emphasis_end(after) {
                flush(&mut plain, &mut spans);
                spans.extend(nested_spans(&after[..end], SpanStyle::Italic));
                i += 1 + end + 1;
                continue;
            }
        }
        let ch = rest.chars().next().expect("in-bounds char");
        plain.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut plain, &mut spans);
    spans
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

    #[test]
    fn single_asterisk_emphasis_renders_italic_but_math_stays_literal() {
        let spans = inline_spans("the *effective* window");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].style, SpanStyle::Italic);
        assert_eq!(spans[1].text, "effective");
        // Multi-word emphasis works too.
        let spans = inline_spans("*current history plus one instruction*");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].style, SpanStyle::Italic);
        // Arithmetic never italicizes.
        for literal in ["2 * 3", "2 * 3 * 4", "a * b"] {
            let spans = inline_spans(literal);
            assert_eq!(spans.len(), 1, "{literal:?}");
            assert_eq!(spans[0].style, SpanStyle::Plain);
            assert_eq!(spans[0].text, literal);
        }
    }

    #[test]
    fn code_nested_in_emphasis_keeps_the_code_style_and_drops_backticks() {
        // The exact shape from the dogfood report: **`ContextConfig`** — ...
        let spans = inline_spans("**`ContextConfig`** — tunable knobs");
        assert_eq!(spans[0].style, SpanStyle::Code);
        assert_eq!(spans[0].text, "ContextConfig");
        assert!(spans.iter().all(|s| !s.text.contains('`')));
        // Mixed content inside bold keeps both styles.
        let spans = inline_spans("**bold `code` mix**");
        let styles: Vec<SpanStyle> = spans.iter().map(|s| s.style).collect();
        assert_eq!(
            styles,
            vec![SpanStyle::Bold, SpanStyle::Code, SpanStyle::Bold]
        );
        // And inside italics.
        let spans = inline_spans("*see `rebase()` here*");
        assert!(spans
            .iter()
            .any(|s| s.style == SpanStyle::Code && s.text == "rebase()"));
    }
}

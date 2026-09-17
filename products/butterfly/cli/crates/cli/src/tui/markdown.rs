//! Line-buffered markdown ⇢ styled spans (streaming never rewrites
//! scrollback). Markers are hidden, Claude Code-style; prose comes back as
//! spans plus a hanging indent so the caller can word-wrap it to the
//! terminal ‒ mid-word terminal wrapping at column 0 is what makes raw
//! transcripts unreadable. Deliberately small: headings, bold, inline code,
//! bullet/numbered lists, tinted fences, pipe tables.
//!
//! Tables are the one construct that cannot be rendered a line at a time:
//! column widths depend on every row. A table therefore buffers until it
//! ends (a non-table line, or `flush` at the end of the stream) and comes
//! back as one `Rendered::Table` the caller lays out at its own width.

use ratatui::style::Style;
use ratatui::text::Line;

use super::theme as th;

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
    /// The one mapping from a parsed markdown role to a drawn style; every
    /// color the renderer uses comes from `theme` through here.
    pub fn style(self) -> Style {
        match self {
            SpanStyle::Plain => th::plain(),
            SpanStyle::Bold => th::bold(),
            SpanStyle::Italic => th::italic(),
            SpanStyle::Code => th::accent(),
            SpanStyle::Heading => th::accent_bold(),
            SpanStyle::Marker => th::accent(),
            SpanStyle::Dim => th::dim(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A parsed pipe table. Cells keep their inline styling, so the layout can
/// re-wrap them at any width without re-parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub headers: Vec<Vec<Span>>,
    pub aligns: Vec<Align>,
    pub rows: Vec<Vec<Vec<Span>>>,
}

impl Table {
    pub fn columns(&self) -> usize {
        self.headers.len()
    }
}

/// One rendered logical line.
#[derive(Debug, PartialEq, Eq)]
pub enum Rendered {
    /// Pre-styled verbatim content (code); never word-wrapped so it stays
    /// byte-identical for copy-paste. Empty for hidden fence delimiters.
    Raw(Line<'static>),
    /// Prose: styled spans; continuation rows indent by `cont_indent` so
    /// wrapped list items hang under their own text.
    Prose {
        spans: Vec<Span>,
        cont_indent: usize,
    },
    /// A complete table, to be laid out by the caller at its width.
    Table(Table),
}

/// Where a pipe table is in its two-line opening handshake. A pipe row is
/// only a table once the delimiter row underneath confirms it ‒ otherwise
/// prose that happens to contain a `|` would vanish into a table.
#[derive(Default)]
enum Pending {
    #[default]
    None,
    Candidate(String),
    Active(Table),
}

#[derive(Default)]
pub struct MarkdownRenderer {
    in_fence: bool,
    pending: Pending,
}

impl MarkdownRenderer {
    /// Render one complete output line. Stateful across lines for fences and
    /// tables, so a line may produce nothing (buffered into a table) or more
    /// than one thing (the line that ends a table).
    pub fn render(&mut self, line: &str) -> Vec<Rendered> {
        if self.in_fence || line.trim_start().starts_with("```") {
            // A fence always wins: a `|` inside code is code.
            let mut out = self.flush();
            out.push(self.render_line(line));
            return out;
        }
        match std::mem::take(&mut self.pending) {
            Pending::None => {
                if is_pipe_row(line) {
                    self.pending = Pending::Candidate(line.to_string());
                    return Vec::new();
                }
            }
            Pending::Candidate(header) => {
                if let Some(aligns) = delimiter_aligns(line) {
                    let mut headers = split_cells(&header);
                    headers.resize(aligns.len(), Vec::new());
                    self.pending = Pending::Active(Table {
                        headers,
                        aligns,
                        rows: Vec::new(),
                    });
                    return Vec::new();
                }
                // Not a table after all: the buffered line was just prose.
                let mut out = vec![self.render_line(&header)];
                out.extend(self.render(line));
                return out;
            }
            Pending::Active(mut table) => {
                if is_pipe_row(line) {
                    let mut cells = split_cells(line);
                    cells.resize(table.columns(), Vec::new());
                    table.rows.push(cells);
                    self.pending = Pending::Active(table);
                    return Vec::new();
                }
                let mut out = vec![Rendered::Table(table)];
                out.extend(self.render(line));
                return out;
            }
        }
        vec![self.render_line(line)]
    }

    /// Emit whatever is still buffered. The caller MUST call this when the
    /// text run ends (turn over, or a tool call interrupts it) or a table
    /// that closes the response never reaches the screen.
    pub fn flush(&mut self) -> Vec<Rendered> {
        match std::mem::take(&mut self.pending) {
            Pending::None => Vec::new(),
            Pending::Candidate(line) => vec![self.render_line(&line)],
            Pending::Active(table) => vec![Rendered::Table(table)],
        }
    }

    /// The table currently accumulating, if any. Lets the caller show a
    /// live preview instead of a blank gap: a table cannot be drawn a row
    /// at a time, but it can be re-drawn in the block as it grows.
    pub fn pending_table(&self) -> Option<&Table> {
        match &self.pending {
            Pending::Active(table) => Some(table),
            _ => None,
        }
    }

    fn render_line(&mut self, line: &str) -> Rendered {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.starts_with("```") {
            self.in_fence = !self.in_fence;
            // The delimiter itself renders as a blank spacer line.
            return Rendered::Raw(th::blank());
        }
        if self.in_fence {
            return Rendered::Raw(Line::from(vec![th::span(line, th::accent())]));
        }
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count();
            if level <= 6 && trimmed[level..].starts_with(' ') {
                let text = trimmed[level..].trim_start();
                // Headings run the inline parser too (`## \`mod.rs\``
                // must not leak backticks), then restyle uniformly ‒ a
                // heading is one visual unit.
                let spans = inline_spans(text)
                    .into_iter()
                    .map(|mut s| {
                        s.style = SpanStyle::Heading;
                        s
                    })
                    .collect();
                return Rendered::Prose {
                    spans,
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
                spans.push(span(SpanStyle::Marker, "‣ "));
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

    /// Test helper: render a line expected to produce exactly one thing.
    #[cfg(test)]
    fn one(&mut self, line: &str) -> Rendered {
        let mut rendered = self.render(line);
        assert_eq!(rendered.len(), 1, "expected one item for {line:?}");
        rendered.pop().expect("one item")
    }
}

/// A row of a pipe table: contains a `|` outside inline code, and is not a
/// bullet or a fence. Leading/trailing pipes are optional (GitHub style).
fn is_pipe_row(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("```") {
        return false;
    }
    let mut in_code = false;
    for ch in trimmed.chars() {
        match ch {
            '`' => in_code = !in_code,
            '|' if !in_code => return true,
            _ => {}
        }
    }
    false
}

/// The `|---|:--:|---:|` row under a header, which both confirms the table
/// and fixes its column count and alignments.
fn delimiter_aligns(line: &str) -> Option<Vec<Align>> {
    let trimmed = line.trim();
    if !trimmed.contains('-') {
        return None;
    }
    let cells = raw_cells(trimmed);
    if cells.is_empty() {
        return None;
    }
    let mut aligns = Vec::with_capacity(cells.len());
    for cell in cells {
        let cell = cell.trim();
        let body = cell.trim_start_matches(':').trim_end_matches(':');
        if body.is_empty() || !body.chars().all(|c| c == '-') {
            return None;
        }
        aligns.push(match (cell.starts_with(':'), cell.ends_with(':')) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        });
    }
    Some(aligns)
}

/// Split a row on unescaped pipes that are not inside inline code, dropping
/// the optional leading and trailing pipe.
fn raw_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut in_code = false;
    let mut escaped = false;
    for ch in trimmed.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '`' => {
                in_code = !in_code;
                current.push(ch);
            }
            '|' if !in_code => cells.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    cells.push(current);
    // A leading `|` produces an empty first cell and a trailing one an empty
    // last cell; neither is a column.
    if cells.first().is_some_and(|c| c.trim().is_empty()) {
        cells.remove(0);
    }
    if cells.len() > 1 && cells.last().is_some_and(|c| c.trim().is_empty()) {
        cells.pop();
    }
    cells
}

fn split_cells(line: &str) -> Vec<Vec<Span>> {
    raw_cells(line)
        .iter()
        .map(|cell| inline_spans(cell.trim()))
        .collect()
}

/// Plain text of a span run, for measurement.
pub fn spans_text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// Word-wrap styled spans into lines of at most `first_budget` /
/// `cont_budget` columns, breaking at spaces (hard-cutting single words
/// longer than a row). Spans carry their own style, so a style simply
/// survives a row boundary instead of having to be re-opened.
///
/// Widths are measured with `theme::cell_width`, not ratatui's own
/// `Line::width`: an ambiguous-width glyph must be budgeted at two columns
/// or the row shears on terminals that render it wide.
pub fn styled_lines(spans: &[Span], first_budget: usize, cont_budget: usize) -> Vec<Line<'static>> {
    let chars: Vec<(char, SpanStyle)> = spans
        .iter()
        .flat_map(|s| s.text.chars().map(move |c| (c, s.style)))
        .collect();
    let mut rows: Vec<Vec<(char, SpanStyle)>> = Vec::new();
    let mut row: Vec<(char, SpanStyle)> = Vec::new();
    let mut cols = 0usize;
    let mut budget = first_budget.max(4);
    let mut last_space: Option<usize> = None;
    for &(ch, style) in &chars {
        let w = th::cell_width(ch);
        // A `while`, not an `if`. Breaking a row also switches the budget from
        // the first row's to the (narrower) continuation's, and the text
        // carried past the break can already fill that narrower budget — so
        // one break is not always enough. With a single check the carry plus
        // the next character overflowed by a column, the terminal wrapped the
        // row itself, and the continuation landed in column 0 with the content
        // grid lost.
        while cols + w > budget && !row.is_empty() {
            if let Some(at) = last_space.filter(|at| *at + 1 < row.len()) {
                let carry = row.split_off(at + 1);
                while row.last().is_some_and(|(c, _)| *c == ' ') {
                    row.pop();
                }
                rows.push(std::mem::take(&mut row));
                row = carry;
            } else {
                rows.push(std::mem::take(&mut row));
            }
            budget = cont_budget.max(4);
            cols = row.iter().map(|(c, _)| th::cell_width(*c)).sum();
            last_space = row.iter().rposition(|(c, _)| *c == ' ');
        }
        if ch == ' ' && row.is_empty() && !rows.is_empty() {
            continue;
        }
        if ch == ' ' {
            last_space = Some(row.len());
        }
        row.push((ch, style));
        cols += w;
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows.into_iter().map(paint_row).collect()
}

/// Collapse a run of `(char, style)` into one line of styled spans.
fn paint_row(row: Vec<(char, SpanStyle)>) -> Line<'static> {
    let mut out: Vec<ratatui::text::Span<'static>> = Vec::new();
    let mut current: Option<(SpanStyle, String)> = None;
    for (ch, style) in row {
        match &mut current {
            Some((active, buffer)) if *active == style => buffer.push(ch),
            Some(_) => {
                let (active, buffer) = current.take().expect("active run");
                out.push(th::span(buffer, active.style()));
                current = Some((style, String::from(ch)));
            }
            None => current = Some((style, String::from(ch))),
        }
    }
    if let Some((active, buffer)) = current {
        out.push(th::span(buffer, active.style()));
    }
    Line::from(out)
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
/// with whitespace ‒ so `2 * 3` stays arithmetic, `*effective*` italicizes.
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
            Rendered::Raw(line) => line.to_string(),
            Rendered::Prose { spans, .. } => spans.iter().map(|s| s.text.as_str()).collect(),
            Rendered::Table(table) => table
                .rows
                .iter()
                .flat_map(|row| row.iter().map(|cell| spans_text(cell)))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    #[test]
    fn headings_hide_markers_and_style() {
        let mut md = MarkdownRenderer::default();
        let rendered = md.one("## Plan");
        match &rendered {
            Rendered::Prose { spans, .. } => {
                assert_eq!(spans.len(), 1);
                assert_eq!(spans[0].style, SpanStyle::Heading);
                assert_eq!(spans[0].text, "Plan");
            }
            other => panic!("expected prose, got {other:?}"),
        }
        // A bare #hashtag is not a heading.
        assert_eq!(plain_text(&md.one("#hashtag")), "#hashtag");
        // Inline markers inside headings are parsed away too (dogfood:
        // "## `mod.rs` contents" leaked its backticks), and the heading
        // stays one uniform visual unit.
        let rendered = md.one("## `mod.rs` contents");
        match &rendered {
            Rendered::Prose { spans, .. } => {
                assert_eq!(plain_text(&rendered), "mod.rs contents");
                assert!(spans.iter().all(|s| s.style == SpanStyle::Heading));
                assert!(spans.iter().all(|s| !s.text.contains('`')));
            }
            other => panic!("expected prose, got {other:?}"),
        }
        assert_eq!(plain_text(&md.one("### **Key pieces:**")), "Key pieces:");
    }

    #[test]
    fn bullets_and_numbered_items_hang_their_continuations() {
        let mut md = MarkdownRenderer::default();
        match md.one("- do the thing") {
            Rendered::Prose { spans, cont_indent } => {
                assert_eq!(spans[0].text, "‣ ");
                assert_eq!(cont_indent, 2);
            }
            other => panic!("{other:?}"),
        }
        match md.one("  * nested") {
            Rendered::Prose { cont_indent, .. } => assert_eq!(cont_indent, 4),
            other => panic!("{other:?}"),
        }
        match md.one("12. numbered item") {
            Rendered::Prose { spans, cont_indent } => {
                assert_eq!(spans[0].text, "12. ");
                assert_eq!(cont_indent, 4);
            }
            other => panic!("{other:?}"),
        }
        // A year is not a list item.
        assert_eq!(plain_text(&md.one("2026 was a year")), "2026 was a year");
    }

    #[test]
    fn fences_toggle_state_hide_delimiters_and_never_wrap() {
        let mut md = MarkdownRenderer::default();
        assert_eq!(md.one("```rust"), Rendered::Raw(th::blank()));
        assert!(md.in_fence());
        match md.one("let **x** = 1;") {
            // Inside a fence, inline markers must NOT be interpreted.
            Rendered::Raw(styled) => assert!(styled.to_string().contains("**x**")),
            other => panic!("{other:?}"),
        }
        assert_eq!(md.one("```"), Rendered::Raw(th::blank()));
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
        // The exact shape from the dogfood report: **`ContextConfig`** ‒ ...
        let spans = inline_spans("**`ContextConfig`** ‒ tunable knobs");
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

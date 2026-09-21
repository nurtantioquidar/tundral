//! The one place terminal styling is defined, plus the width-aware text
//! primitives every surface needs. Butterfly's palette is deliberately
//! small — one accent, three status colors, one elevated band — because the
//! inline TUI shares the user's scrollback with everything else they run:
//! loud color reads as noise, not as product.
//!
//! Style is expressed as ratatui [`Style`] and text as ratatui [`Line`]:
//! a span carries its own style, so wrapping, truncating and justifying a
//! line never has to re-open an escape sequence by hand. The one place raw
//! escapes still exist is [`ansi`], for the non-TTY paths (`-p` runs, piped
//! stdin) where no ratatui terminal is running at all.
//!
//! Terminal styling stays local to this crate (`packages/ui` is the web
//! token layer and never covers the terminal).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// The single accent: markers, selection, the prompt caret, code spans.
pub const ACCENT_COLOR: Color = Color::Cyan;

pub fn plain() -> Style {
    Style::new()
}

pub fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

pub fn italic() -> Style {
    Style::new().add_modifier(Modifier::ITALIC)
}

pub fn accent() -> Style {
    Style::new().fg(ACCENT_COLOR)
}

pub fn accent_bold() -> Style {
    accent().add_modifier(Modifier::BOLD)
}

pub fn ok() -> Style {
    Style::new().fg(Color::Green)
}

pub fn warn() -> Style {
    Style::new().fg(Color::Yellow)
}

pub fn danger() -> Style {
    Style::new().fg(Color::Red)
}

/// Subtle elevated band: dark-gray background with soft light-gray text,
/// both forced so the contrast is gentle on any theme — deliberately NOT
/// inverse video, which reads as harsh black-on-white.
pub fn band() -> Style {
    Style::new().bg(Color::Indexed(236)).fg(Color::Indexed(252))
}

/// Markers. Every one of these sits in column 0 of the two-column grid the
/// whole surface shares (marker, space, content) so the eye can run down a
/// single edge and tell turns, answers, tools and summaries apart.
/// The composer caret and the marker on a user turn, Codex-style.
pub const MARK_USER: &str = "›";
/// Everything the agent emits — answers, tool rows, notices — opens with this.
pub const MARK_ANSWER: &str = "•";
pub const MARK_TOOL: &str = "•";
pub const MARK_SUMMARY: &str = "•";
pub const MARK_SELECT: &str = "›";
pub const MARK_BRAND: &str = ">_";

/// Rounded box, for the opening card.
pub const BOX_TL: &str = "╭";
pub const BOX_TR: &str = "╮";
pub const BOX_BL: &str = "╰";
pub const BOX_BR: &str = "╯";
pub const BOX_H: &str = "─";
pub const BOX_V: &str = "│";
pub const MARK_WARN: &str = "⚠";
/// A failed tool call and nothing else: success is the default and draws no mark.
pub const MARK_FAIL: &str = "✗";

/// Cut marker for truncated text. `…` (U+2026) is ambiguous-width; this is
/// not, so a clamped row is exactly as wide as the clamp asked for.
pub const ELLIPSIS: &str = "⋯";
/// The separator between fields in footers, hints and receipts. `·` (U+00B7)
/// is ambiguous-width and appeared ~29 times, so a single hint row could
/// overflow by five columns on a terminal that renders it wide.
pub const SEP: &str = "·";
/// Horizontal rule glyph. Box-drawing `─` is ambiguous-width, which made a
/// full-width rule render at DOUBLE the terminal width and split in two.
pub const RULE: &str = "⎯";
/// List bullet (`•` is ambiguous-width).
pub const BULLET: &str = "‣";

/// Braille spinner: one cell wide everywhere, and it animates without the
/// jitter a multi-glyph frame set causes at different widths.
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ width ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// Display columns one character occupies, counted CONSERVATIVELY.
///
/// Unicode marks a large set of characters "East Asian Ambiguous" — `·`, `—`,
/// `…`, `●`, `─`, the arrows — and a terminal is free to render them one
/// column wide or two. `unicode_width::width()` assumes one, and so does
/// ratatui's own `Line::width` and its `Buffer` cell placement; Ghostty,
/// iTerm2 and others render two. Guessing low is the dangerous direction:
/// every row containing one is then wider on screen than the layout
/// budgeted, the terminal soft-wraps it mid-word, and the whole column grid
/// shears. Guessing high costs a column of unused space on terminals that
/// render narrow, which nobody can see. So `width_cjk` it is — ambiguous
/// counts as two, and **layout must always measure with [`line_cols`] rather
/// than `Line::width`.**
///
/// Butterfly's OWN glyphs avoid the ambiguous set entirely (see the marker
/// constants above and `glyphs_have_one_unambiguous_width`), so this only ever
/// applies to text the model wrote.
pub fn cell_width(ch: char) -> usize {
    if CHROME_GLYPHS.contains(ch) {
        // Butterfly's own chrome, measured in the reference terminal (herdr
        // 0.8.0): a 3/4-width run of each of these occupies exactly as many
        // columns as the same run of ASCII, i.e. ONE column apiece, which is
        // also what ratatui and Codex assume. The box card only lines up if
        // its border and its content agree, so these are measured at their
        // real width rather than budgeted at two.
        return 1;
    }
    ch.width_cjk().unwrap_or(0)
}

/// The glyphs the chrome draws itself. Text the model wrote is never in here,
/// so unknown content keeps the conservative budget.
const CHROME_GLYPHS: &str = "›•─│╭╮╰╯·⋅⋯▮▯⇡⇣⇢✗⚠‣⎯…";

/// Display columns a plain string occupies.
pub fn width_of(text: &str) -> usize {
    text.chars().map(cell_width).sum()
}

/// Display columns a styled line occupies, measured conservatively.
///
/// Never use `Line::width()` for layout: it counts ambiguous-width glyphs as
/// one column, which is exactly the assumption that shears the grid.
pub fn line_cols(line: &Line<'_>) -> usize {
    line.spans.iter().map(|span| width_of(&span.content)).sum()
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ building ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// A styled span that owns its text, so every composed row is `'static` and
/// can be stored in the transcript and re-rendered at any later width.
pub fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

/// An unstyled span.
pub fn text(text: impl Into<String>) -> Span<'static> {
    Span::raw(text.into())
}

/// `n` spaces, unstyled — the shared content grid's indent.
pub fn indent(cols: usize) -> Span<'static> {
    Span::raw(" ".repeat(cols))
}

/// An empty row.
pub fn blank() -> Line<'static> {
    Line::default()
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ line ops ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// Truncate plain text to `max` columns, appending `⋯` when cut.
pub fn truncate_cols(text: &str, max: usize) -> String {
    if width_of(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut cols = 0usize;
    for ch in text.chars() {
        let w = cell_width(ch);
        if cols + w > max.saturating_sub(1) {
            break;
        }
        cols += w;
        out.push(ch);
    }
    out.push_str(ELLIPSIS);
    out
}

/// Keep the TAIL of a streaming line within `max` columns (a live line grows
/// rightward; the head is what scrolls away).
pub fn tail_cols(text: &str, max: usize) -> String {
    if width_of(text) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut cols = 0usize;
    let mut start = chars.len();
    while start > 0 {
        let w = cell_width(chars[start - 1]);
        if cols + w > max.saturating_sub(1) {
            break;
        }
        cols += w;
        start -= 1;
    }
    format!("{ELLIPSIS}{}", chars[start..].iter().collect::<String>())
}

/// Truncate a styled line to `max` visible columns, keeping each surviving
/// span's style and marking the cut with `⋯`.
pub fn truncate_line(line: Line<'static>, max: usize) -> Line<'static> {
    if line_cols(&line) <= max {
        return line;
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cols = 0usize;
    let budget = max.saturating_sub(1);
    for source in line.spans {
        if cols >= budget {
            break;
        }
        let mut kept = String::new();
        for ch in source.content.chars() {
            let w = cell_width(ch);
            if cols + w > budget {
                break;
            }
            cols += w;
            kept.push(ch);
        }
        let full = kept.len() == source.content.len();
        if !kept.is_empty() {
            spans.push(Span::styled(kept, source.style));
        }
        if !full {
            break;
        }
    }
    spans.push(Span::raw(ELLIPSIS));
    Line::from(spans)
}

/// Hard-wrap a styled line into rows of at most `width` columns. Used for
/// verbatim content (fenced code) where clamping would silently destroy what
/// the user asked to see — every row still obeys the width, which is what
/// keeps the frame's row arithmetic honest.
pub fn wrap_line(line: &Line<'_>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    if line_cols(line) <= width {
        return vec![owned(line)];
    }
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut cols = 0usize;
    for source in &line.spans {
        let mut chunk = String::new();
        for ch in source.content.chars() {
            let w = cell_width(ch);
            if cols + w > width {
                if !chunk.is_empty() {
                    current.push(Span::styled(std::mem::take(&mut chunk), source.style));
                }
                rows.push(Line::from(std::mem::take(&mut current)));
                cols = 0;
            }
            chunk.push(ch);
            cols += w;
        }
        if !chunk.is_empty() {
            current.push(Span::styled(chunk, source.style));
        }
    }
    rows.push(Line::from(current));
    rows
}

/// Pad plain text out to `cols` columns (no-op when it already overflows).
pub fn pad_to(text: &str, cols: usize) -> String {
    let width = width_of(text);
    if width >= cols {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(cols - width))
    }
}

/// Pad a styled line out to `cols` columns with `style`d spaces — used by
/// the user bar, whose background must reach the right edge.
pub fn pad_line(mut line: Line<'static>, cols: usize, style: Style) -> Line<'static> {
    let width = line_cols(&line);
    if width < cols {
        line.spans
            .push(Span::styled(" ".repeat(cols - width), style));
    }
    line
}

/// Lay a styled `left` and a styled `right` out on one row of `width`
/// columns, right-aligning `right`. When they cannot both fit, `right` is
/// dropped — the left side is always the more important half.
pub fn justify(left: Line<'static>, right: Line<'static>, width: usize) -> Line<'static> {
    let (lw, rw) = (line_cols(&left), line_cols(&right));
    if rw > 0 && lw + rw + 2 <= width {
        let mut spans = left.spans;
        spans.push(Span::raw(" ".repeat(width - lw - rw)));
        spans.extend(right.spans);
        Line::from(spans)
    } else {
        truncate_line(left, width)
    }
}

/// Clone a borrowed line into an owned one.
pub fn owned(line: &Line<'_>) -> Line<'static> {
    Line::from(
        line.spans
            .iter()
            .map(|span| Span::styled(span.content.to_string(), span.style))
            .collect::<Vec<_>>(),
    )
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ formatting ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// A compact fill meter, e.g. `▮▮▮▯▯▯▯` at 3/7. Plain text: the caller
/// colors it, because the same meter means different things in different
/// places.
pub fn meter(fraction: f64, cells: usize) -> String {
    let filled = ((fraction.clamp(0.0, 1.0) * cells as f64).round() as usize).min(cells);
    format!("{}{}", "▮".repeat(filled), "▯".repeat(cells - filled))
}

/// `12s`, `1m04s`, `2h11m` — always short, never scientific.
pub fn duration(elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// Milliseconds for the things that finish fast (tool calls): `8ms`,
/// `1.2s`, `45s`.
pub fn short_elapsed(elapsed: std::time::Duration) -> String {
    let ms = elapsed.as_millis();
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", elapsed.as_secs_f64())
    } else {
        duration(elapsed)
    }
}

/// `840`, `24.0k`, `1.2M` — token counts are magnitudes, not exact figures.
pub fn tokens(count: u64) -> String {
    match count {
        0..=999 => count.to_string(),
        1_000..=999_999 => format!("{:.1}k", count as f64 / 1_000.0),
        _ => format!("{:.1}M", count as f64 / 1_000_000.0),
    }
}

/// Money at a precision a human can read. Integer micro-USD stays the unit
/// of record (invariant 7); this is presentation only, and sub-cent amounts
/// keep four decimals so a cheap turn is not rendered as `$0.00`.
pub fn money(microusd: u64) -> String {
    let usd = microusd as f64 / 1_000_000.0;
    if microusd == 0 {
        "$0".to_string()
    } else if usd < 0.01 {
        format!("${usd:.4}")
    } else if usd < 10.0 {
        format!("${usd:.3}")
    } else {
        format!("${usd:.2}")
    }
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ raw escapes ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// Escape sequences for the paths where no ratatui terminal exists: `-p`
/// runs, piped stdin, and the pre-session key prompt. Everything drawn by
/// the inline TUI goes through ratatui instead, so these are the ONLY
/// escapes the CLI writes by hand.
pub mod ansi {
    use ratatui::style::{Color, Modifier};
    use ratatui::text::Line;

    pub const RESET: &str = "\x1b[0m";
    pub const DIM: &str = "\x1b[2m";
    pub const BOLD: &str = "\x1b[1m";
    pub const ACCENT: &str = "\x1b[36m";
    pub const ACCENT_BOLD: &str = "\x1b[1m\x1b[36m";
    pub const DANGER: &str = "\x1b[31m";

    /// Serialize one composed line to an escape-coded string, for the plain
    /// loops that print with `println!` instead of drawing a frame.
    pub fn render(line: &Line<'_>) -> String {
        let mut out = String::new();
        for span in &line.spans {
            let mut prefix = String::new();
            if span.style.add_modifier.contains(Modifier::BOLD) {
                prefix.push_str(BOLD);
            }
            if span.style.add_modifier.contains(Modifier::DIM) {
                prefix.push_str(DIM);
            }
            if span.style.add_modifier.contains(Modifier::ITALIC) {
                prefix.push_str("\x1b[3m");
            }
            if let Some(color) = span.style.fg {
                prefix.push_str(&fg(color));
            }
            if let Some(color) = span.style.bg {
                prefix.push_str(&bg(color));
            }
            if prefix.is_empty() {
                out.push_str(&span.content);
            } else {
                out.push_str(&prefix);
                out.push_str(&span.content);
                out.push_str(RESET);
            }
        }
        out
    }

    /// Serialize composed lines to one printable block.
    pub fn render_lines(lines: &[Line<'_>]) -> String {
        lines.iter().map(render).collect::<Vec<_>>().join("\n")
    }

    fn fg(color: Color) -> String {
        match color {
            Color::Reset => String::new(),
            Color::Red => "\x1b[31m".to_string(),
            Color::Green => "\x1b[32m".to_string(),
            Color::Yellow => "\x1b[33m".to_string(),
            Color::Cyan => "\x1b[36m".to_string(),
            Color::Indexed(i) => format!("\x1b[38;5;{i}m"),
            _ => String::new(),
        }
    }

    fn bg(color: Color) -> String {
        match color {
            Color::Indexed(i) => format!("\x1b[48;5;{i}m"),
            _ => String::new(),
        }
    }
}

/// True when a terminal is free to render `ch` either one or two columns
/// wide (Unicode "East Asian Ambiguous"). Layout cannot be exact for these,
/// so none of Butterfly's own glyphs may be one.
#[cfg(test)]
pub fn is_ambiguous_width(ch: char) -> bool {
    use unicode_width::UnicodeWidthChar as _;
    ch.width() != ch.width_cjk()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_have_one_unambiguous_width() {
        // A glyph whose width the terminal gets to choose makes every row
        // containing it a guess. `·` (29 uses) and `─` both shipped as real
        // corruption: a hint row overflowed by five columns and a full-width
        // rule rendered at double the terminal and split in two.
        let glyphs: &[(&str, &str)] = &[
            ("MARK_USER", MARK_USER),
            ("MARK_ANSWER", MARK_ANSWER),
            ("MARK_TOOL", MARK_TOOL),
            ("MARK_SUMMARY", MARK_SUMMARY),
            ("MARK_SELECT", MARK_SELECT),
            ("MARK_BRAND", MARK_BRAND),
            ("MARK_WARN", MARK_WARN),
            ("MARK_FAIL", MARK_FAIL),
            ("ELLIPSIS", ELLIPSIS),
            ("SEP", SEP),
            ("RULE", RULE),
            ("BULLET", BULLET),
        ];
        for (name, glyph) in glyphs {
            for ch in glyph.chars() {
                // Chrome glyphs are measured, not assumed — see CHROME_GLYPHS.
                if CHROME_GLYPHS.contains(ch) {
                    assert_eq!(cell_width(ch), 1, "{name} must measure one column");
                    continue;
                }
                assert!(
                    !is_ambiguous_width(ch),
                    "{name} is ambiguous-width: {ch:?} (U+{:04X}) renders 1 or 2 \
                     columns depending on the terminal",
                    ch as u32
                );
            }
        }
        for frame in SPINNER {
            for ch in frame.chars() {
                assert!(
                    !is_ambiguous_width(ch),
                    "spinner frame {frame:?} is ambiguous"
                );
            }
        }
        for ch in meter(0.5, 8).chars() {
            assert!(!is_ambiguous_width(ch), "meter glyph {ch:?} is ambiguous");
        }
    }

    #[test]
    fn ambiguous_text_is_budgeted_at_its_widest() {
        // Model output we do not control still has to be safe: an em dash or
        // a middle dot is counted as two so a row can only ever come out
        // SHORTER than the terminal, never wider.
        // `—` is model text: nothing measured it, so it is budgeted wide.
        assert_eq!(width_of("—"), 2);
        assert_eq!(width_of("plain"), 5);
        // `·` is chrome: measured at one column in the reference terminal, so
        // the box card and the status line line up instead of doubling.
        assert_eq!(width_of("·"), 1);
        let cut = truncate_cols("a — b — c — d — e", 10);
        assert!(width_of(&cut) <= 10, "{cut:?} is {} cols", width_of(&cut));
    }

    /// The whole reason `line_cols` exists: ratatui measures narrow.
    #[test]
    fn line_cols_budgets_wider_than_ratatui_does() {
        let line = Line::from(vec![span("a — b — c", accent())]);
        assert_eq!(line.width(), 9, "ratatui counts ambiguous as one");
        assert_eq!(
            line_cols(&line),
            11,
            "unmeasured ambiguous glyphs must count as two"
        );
    }

    #[test]
    fn meters_fill_proportionally_and_never_overflow() {
        assert_eq!(meter(0.0, 6), "▯▯▯▯▯▯");
        assert_eq!(meter(1.0, 6), "▮▮▮▮▮▮");
        assert_eq!(meter(0.5, 6), "▮▮▮▯▯▯");
        assert_eq!(width_of(&meter(9.0, 6)), 6);
        assert_eq!(width_of(&meter(-3.0, 6)), 6);
    }

    #[test]
    fn justify_drops_the_right_half_rather_than_overflowing() {
        let row = justify(
            Line::from(vec![text("left")]),
            Line::from(vec![text("right")]),
            20,
        );
        assert_eq!(line_cols(&row), 20);
        assert_eq!(row.to_string(), "left           right");
        // No room for both: the left half survives, clamped to the width.
        let row = justify(
            Line::from(vec![text("a rather long left side")]),
            Line::from(vec![text("right")]),
            12,
        );
        assert!(line_cols(&row) <= 12);
        assert!(!row.to_string().contains("right"));
    }

    #[test]
    fn compact_numbers_read_as_magnitudes() {
        assert_eq!(tokens(840), "840");
        assert_eq!(tokens(24_000), "24.0k");
        assert_eq!(tokens(1_500_000), "1.5M");
        assert_eq!(duration(std::time::Duration::from_secs(9)), "9s");
        assert_eq!(duration(std::time::Duration::from_secs(64)), "1m04s");
        assert_eq!(duration(std::time::Duration::from_secs(7_860)), "2h11m");
        assert_eq!(short_elapsed(std::time::Duration::from_millis(8)), "8ms");
        assert_eq!(
            short_elapsed(std::time::Duration::from_millis(1_240)),
            "1.2s"
        );
    }

    #[test]
    fn money_keeps_sub_cent_turns_visible() {
        assert_eq!(money(0), "$0");
        assert_eq!(money(2_916), "$0.0029");
        assert_eq!(money(1_250_000), "$1.250");
        assert_eq!(money(42_500_000), "$42.50");
    }

    #[test]
    fn wrap_line_keeps_every_row_inside_the_width_and_loses_nothing() {
        let line = Line::from(vec![span("abcdefghij".repeat(5), accent())]);
        let rows = wrap_line(&line, 12);
        assert!(rows.len() >= 5);
        assert!(rows.iter().all(|row| line_cols(row) <= 12), "{rows:?}");
        let joined: String = rows.iter().map(|row| row.to_string()).collect();
        assert_eq!(joined, "abcdefghij".repeat(5));
        // The style survives every break — spans carry it, so there is no
        // escape state to re-open.
        assert!(rows[1].spans.iter().all(|s| s.style == accent()));
        let short = Line::from(vec![text("short")]);
        assert_eq!(wrap_line(&short, 20).len(), 1);
    }

    #[test]
    fn truncate_line_counts_visible_columns_only() {
        let line = Line::from(vec![span("0123456789", accent()), text("0123456789")]);
        let cut = truncate_line(line, 12);
        assert!(line_cols(&cut) <= 12);
        assert!(cut.to_string().contains(ELLIPSIS));
        assert_eq!(cut.spans[0].style, accent());
    }

    #[test]
    fn ansi_render_reopens_nothing_it_did_not_open() {
        let line = Line::from(vec![span("hot", danger()), text(" cold")]);
        let rendered = ansi::render(&line);
        assert_eq!(rendered, "\x1b[31mhot\x1b[0m cold");
    }
}

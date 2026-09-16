//! The one place terminal styling is defined, plus the width-aware text
//! primitives every surface needs. Butterfly's palette is deliberately
//! small — one accent, three status colors, one elevated band — because the
//! inline TUI shares the user's scrollback with everything else they run:
//! loud color reads as noise, not as product.
//!
//! Terminal styling stays local to this crate (`packages/ui` is the web
//! token layer and never covers the terminal).

use unicode_width::UnicodeWidthChar;

pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
/// The single accent: markers, selection, the prompt caret, code spans.
pub const ACCENT: &str = "\x1b[36m";
pub const ACCENT_BOLD: &str = "\x1b[1m\x1b[36m";
pub const OK: &str = "\x1b[32m";
pub const WARN: &str = "\x1b[33m";
pub const DANGER: &str = "\x1b[31m";
/// Subtle elevated band: dark-gray background with soft light-gray text,
/// both forced so the contrast is gentle on any theme — deliberately NOT
/// inverse video, which reads as harsh black-on-white.
pub const BAND: &str = "\x1b[48;5;236m\x1b[38;5;252m";

/// Markers. Every one of these sits in column 0 of the two-column grid the
/// whole surface shares (marker, space, content) so the eye can run down a
/// single edge and tell turns, answers, tools and summaries apart.
pub const MARK_USER: &str = "❯";
pub const MARK_ANSWER: &str = "◉";
pub const MARK_TOOL: &str = "⚒";
pub const MARK_SUMMARY: &str = "▪";
pub const MARK_SELECT: &str = "▸";
pub const MARK_BRAND: &str = "✦";
pub const MARK_WARN: &str = "⚠";
pub const MARK_OK: &str = "✓";
pub const MARK_FAIL: &str = "✗";

/// Cut marker for truncated text. `…` (U+2026) is ambiguous-width; this is
/// not, so a clamped row is exactly as wide as the clamp asked for.
pub const ELLIPSIS: &str = "⋯";
/// The separator between fields in footers, hints and receipts. `·` (U+00B7)
/// is ambiguous-width and appeared ~29 times, so a single hint row could
/// overflow by five columns on a terminal that renders it wide.
pub const SEP: &str = "⋅";
/// Horizontal rule glyph. Box-drawing `─` is ambiguous-width, which made a
/// full-width rule render at DOUBLE the terminal width and split in two.
pub const RULE: &str = "⎯";
/// List bullet (`•` is ambiguous-width).
pub const BULLET: &str = "‣";

/// Braille spinner: one cell wide everywhere, and it animates without the
/// jitter a multi-glyph frame set causes at different widths.
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Display columns one character occupies, counted CONSERVATIVELY.
///
/// Unicode marks a large set of characters "East Asian Ambiguous" — `·`, `—`,
/// `…`, `●`, `─`, the arrows — and a terminal is free to render them one
/// column wide or two. `unicode_width::width()` assumes one; Ghostty, iTerm2
/// and others render two. Guessing low is the dangerous direction: every row
/// containing one is then wider on screen than the layout budgeted, the
/// terminal soft-wraps it mid-word, and the whole column grid shears. Guessing
/// high costs a column of unused space on terminals that render narrow, which
/// nobody can see. So `width_cjk` it is — ambiguous counts as two.
///
/// Butterfly's OWN glyphs avoid the ambiguous set entirely (see the marker
/// constants above and `glyphs_have_one_unambiguous_width`), so this only ever
/// applies to text the model wrote.
pub fn cell_width(ch: char) -> usize {
    ch.width_cjk().unwrap_or(0)
}

/// Display columns a plain string occupies.
pub fn width_of(text: &str) -> usize {
    text.chars().map(cell_width).sum()
}

/// Display columns with ANSI escape sequences stripped.
pub fn visible_cols(line: &str) -> usize {
    let mut cols = 0usize;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for esc in chars.by_ref() {
                    if esc.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        cols += cell_width(ch);
    }
    cols
}

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

/// Truncate a STYLED line to `max` visible columns (escapes pass through,
/// visible chars are counted); always ends with a reset.
pub fn truncate_styled(line: &str, max: usize) -> String {
    if visible_cols(line) <= max {
        return format!("{line}{RESET}");
    }
    let mut out = String::new();
    let mut cols = 0usize;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            out.push(ch);
            if chars.peek() == Some(&'[') {
                out.push(chars.next().expect("peeked"));
                for esc in chars.by_ref() {
                    out.push(esc);
                    if esc.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        let w = cell_width(ch);
        if cols + w > max.saturating_sub(1) {
            out.push_str(ELLIPSIS);
            break;
        }
        cols += w;
        out.push(ch);
    }
    out.push_str(RESET);
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

/// Hard-wrap a STYLED line into rows of at most `width` visible columns,
/// re-opening the active SGR state on each continuation row. Used for
/// verbatim content (fenced code) where clamping would silently destroy
/// what the user asked to see — every row still obeys the width, which is
/// what keeps the repaint's row arithmetic honest.
pub fn wrap_styled(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if visible_cols(line) <= width {
        return vec![line.to_string()];
    }
    let mut rows = Vec::new();
    // SGR codes seen since the last reset, replayed after each break.
    let mut active = String::new();
    let mut current = String::new();
    let mut cols = 0usize;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            let mut escape = String::from(ch);
            if chars.peek() == Some(&'[') {
                escape.push(chars.next().expect("peeked"));
                for esc in chars.by_ref() {
                    escape.push(esc);
                    if esc.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            if escape == RESET {
                active.clear();
            } else {
                active.push_str(&escape);
            }
            current.push_str(&escape);
            continue;
        }
        let w = cell_width(ch);
        if cols + w > width {
            if !active.is_empty() {
                current.push_str(RESET);
            }
            rows.push(std::mem::take(&mut current));
            current.push_str(&active);
            cols = 0;
        }
        current.push(ch);
        cols += w;
    }
    if !active.is_empty() {
        current.push_str(RESET);
    }
    rows.push(current);
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

/// Lay a styled `left` and a styled `right` out on one row of `width`
/// columns, right-aligning `right`. When they cannot both fit, `right` is
/// dropped — the left side is always the more important half.
pub fn justify(left: &str, right: &str, width: usize) -> String {
    let (lw, rw) = (visible_cols(left), visible_cols(right));
    if rw > 0 && lw + rw + 2 <= width {
        format!("{left}{}{right}", " ".repeat(width - lw - rw))
    } else {
        truncate_styled(left, width)
    }
}

/// A compact fill meter, e.g. `███░░░░` at 3/7. Plain text: the caller
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

/// True when a terminal is free to render `ch` either one or two columns
/// wide (Unicode "East Asian Ambiguous"). Layout cannot be exact for these,
/// so none of Butterfly's own glyphs may be one.
#[cfg(test)]
fn is_ambiguous_width(ch: char) -> bool {
    ch.width() != ch.width_cjk()
}

#[cfg(test)]
mod tests {

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
            ("MARK_OK", MARK_OK),
            ("MARK_FAIL", MARK_FAIL),
            ("ELLIPSIS", ELLIPSIS),
            ("SEP", SEP),
            ("RULE", RULE),
            ("BULLET", BULLET),
        ];
        for (name, glyph) in glyphs {
            for ch in glyph.chars() {
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
        assert_eq!(width_of("—"), 2);
        assert_eq!(width_of("·"), 2);
        assert_eq!(width_of("plain"), 5);
        // And the clamp honours that budget.
        let cut = truncate_cols("a — b — c — d — e", 10);
        assert!(width_of(&cut) <= 10, "{cut:?} is {} cols", width_of(&cut));
    }
    use super::*;

    #[test]
    fn meters_fill_proportionally_and_never_overflow() {
        assert_eq!(meter(0.0, 6), "▯▯▯▯▯▯");
        assert_eq!(meter(1.0, 6), "▮▮▮▮▮▮");
        assert_eq!(meter(0.5, 6), "▮▮▮▯▯▯");
        // Out-of-range fractions clamp instead of panicking on repeat().
        assert_eq!(width_of(&meter(9.0, 6)), 6);
        assert_eq!(width_of(&meter(-3.0, 6)), 6);
    }

    #[test]
    fn justify_drops_the_right_half_rather_than_overflowing() {
        let row = justify("left", "right", 20);
        assert_eq!(visible_cols(&row), 20);
        assert!(row.starts_with("left") && row.ends_with("right"));
        // No room for both: the left half survives, clamped to the width.
        let row = justify("a rather long left side", "right", 12);
        assert!(visible_cols(&row) <= 12);
        assert!(!row.contains("right"));
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
    fn wrap_styled_keeps_every_row_inside_the_width_and_loses_nothing() {
        let line = format!("{ACCENT}{}{RESET}", "abcdefghij".repeat(5));
        let rows = wrap_styled(&line, 12);
        assert!(rows.len() >= 5);
        assert!(rows.iter().all(|row| visible_cols(row) <= 12), "{rows:?}");
        // Nothing is dropped, and the style survives every break.
        let joined: String = rows
            .iter()
            .map(|row| row.replace(ACCENT, "").replace(RESET, ""))
            .collect();
        assert_eq!(joined, "abcdefghij".repeat(5));
        assert!(rows[1].starts_with(ACCENT), "style lost: {:?}", rows[1]);
        // A line that already fits is returned untouched.
        assert_eq!(wrap_styled("short", 20), vec!["short".to_string()]);
    }

    #[test]
    fn truncate_styled_counts_visible_columns_only() {
        let styled = format!("{ACCENT}0123456789{RESET}0123456789");
        let cut = truncate_styled(&styled, 12);
        assert!(visible_cols(&cut) <= 12);
        assert!(cut.contains(ELLIPSIS));
        assert!(cut.starts_with(ACCENT));
    }
}

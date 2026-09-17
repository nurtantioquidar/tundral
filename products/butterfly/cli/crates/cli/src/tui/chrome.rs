//! Composed surfaces: the welcome card, footer, activity line, tool lines,
//! approval card, turn summary, help card and model rows.
//!
//! Everything here is a pure function from data to ratatui [`Line`]s at a
//! given width, so each surface can be unit-tested for exact column budgets
//! without a terminal. Style lives on the spans, so no surface ever writes
//! an escape sequence, and a composed line can be stored, re-measured and
//! re-drawn at a different width without being re-parsed.

use std::time::Duration;

use butterfly_core::session::approval::ApprovalRequest;
use ratatui::text::{Line, Span};

use super::markdown::{self as md, Align};

use super::theme::{
    self as th, blank, indent, justify, line_cols, meter, money, pad_to, short_elapsed, span,
    text as raw, tokens, truncate_cols, truncate_line, width_of,
};

/// Content column of the shared grid: every marker sits in column 0 and
/// every piece of content starts in column 2, so `❯`, `◉`, `⚒`, `▪` and `✦`
/// all line up down the left edge.
pub const GRID: usize = 2;

/// Shorthand for composing one row out of spans.
fn row(spans: Vec<Span<'static>>) -> Line<'static> {
    Line::from(spans)
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ tool lines ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// A short verb for each tool, padded into one column so a run of tool
/// calls reads as a list of actions rather than a list of function calls.
/// Unknown tools (config-defined, future) fall back to their own name.
fn verb_of(tool: &str) -> &str {
    match tool {
        "read_file" => "Read",
        "list_files" => "List",
        "search_text" => "Search",
        "edit" => "Edit",
        "write_file" => "Write",
        "run_command" => "Running",
        "shell" => "Shell",
        "update_plan" => "Plan",
        "repo_map" => "Map",
        "symbol_search" => "Symbol",
        "complete_task" => "Done",
        "report_verdict" => "Verdict",
        "ask_user" => "Ask",
        other => other,
    }
}

const VERB_COL: usize = 7;

/// Tools that change the workspace. Their verb takes the accent so a write
/// is distinguishable from a read at a glance, down the left edge.
fn is_mutating(tool: &str) -> bool {
    matches!(tool, "edit" | "write_file" | "run_command" | "shell")
}

/// Split a core `describe_call` preview (`read_file(src/main.rs)`, or a bare
/// tool name) into `(tool, target)`.
pub fn split_preview(preview: &str) -> (&str, &str) {
    match preview.find('(') {
        Some(open) if preview.ends_with(')') => {
            (&preview[..open], &preview[open + 1..preview.len() - 1])
        }
        _ => (preview, ""),
    }
}

/// One committed tool row: `⚒ edit   README.md            ✓ 12ms`.
pub fn tool_line(preview: &str, outcome: Option<ToolOutcome>, width: usize) -> Line<'static> {
    let (tool, target) = split_preview(preview);
    let verb_style = if is_mutating(tool) {
        th::accent()
    } else {
        th::dim()
    };
    let head = vec![
        span(format!("{} ", th::MARK_TOOL), th::dim()),
        span(pad_to(verb_of(tool), VERB_COL), verb_style),
        raw(" "),
    ];
    let right: Vec<Span<'static>> = match &outcome {
        Some(ToolOutcome::Ok { elapsed }) => vec![
            span(th::MARK_OK, th::ok()),
            raw(" "),
            span(short_elapsed(*elapsed), th::dim()),
        ],
        Some(ToolOutcome::Failed { reason, elapsed }) => {
            let timing = short_elapsed(*elapsed);
            // Tool errors often lead with the path the row already shows.
            let reason = reason
                .strip_prefix(&format!("{target}: "))
                .unwrap_or(reason)
                .trim();
            if reason.is_empty() {
                vec![
                    span(th::MARK_FAIL, th::danger()),
                    raw(" "),
                    span(timing, th::dim()),
                ]
            } else {
                vec![
                    span(
                        format!("{} {}", th::MARK_FAIL, truncate_cols(reason, width / 3)),
                        th::danger(),
                    ),
                    raw(" "),
                    span(timing, th::dim()),
                ]
            }
        }
        None => Vec::new(),
    };
    let head_cols: usize = head.iter().map(|s| width_of(&s.content)).sum();
    let right_cols: usize = right.iter().map(|s| width_of(&s.content)).sum();
    // The target gets whatever the markers and the outcome leave behind.
    let budget = width
        .saturating_sub(head_cols)
        .saturating_sub(right_cols)
        .saturating_sub(if right.is_empty() { 0 } else { 2 });
    let mut body = head;
    body.push(raw(truncate_cols(target, budget.max(8))));
    justify(row(body), row(right), width)
}

/// How a tool call ended, for the right-hand side of its row.
pub enum ToolOutcome {
    Ok { elapsed: Duration },
    Failed { reason: String, elapsed: Duration },
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ activity ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// What the agent is doing right now. The verb is the point: `thinking`
/// tells the user the silence is reasoning, not a hang.
#[derive(Clone, PartialEq)]
pub enum Phase {
    Starting,
    Thinking,
    Writing,
    Running(String),
    /// An approval prompt is open: nothing is running and Esc means "no",
    /// so the interrupt hint must not be shown.
    Deciding,
}

impl Phase {
    fn label(&self) -> String {
        match self {
            Phase::Starting => "working".to_string(),
            Phase::Thinking => "thinking".to_string(),
            Phase::Writing => "writing".to_string(),
            Phase::Running(tool) => {
                let (name, target) = split_preview(tool);
                if target.is_empty() {
                    verb_of(name).to_string()
                } else {
                    format!("{} {target}", verb_of(name))
                }
            }
            Phase::Deciding => "waiting for you".to_string(),
        }
    }
}

/// The single live status row shown under the stream while a turn runs.
pub fn activity_line(
    phase: &Phase,
    spinner_frame: usize,
    elapsed: Duration,
    out_tokens: u64,
    width: usize,
) -> Line<'static> {
    let deciding = *phase == Phase::Deciding;
    let mark = if deciding {
        span(th::MARK_WARN, th::warn())
    } else {
        span(th::SPINNER[spinner_frame % th::SPINNER.len()], th::accent())
    };
    let mut body = format!(" {} {} {}", phase.label(), th::SEP, th::duration(elapsed));
    if out_tokens > 0 && !deciding {
        body.push_str(&format!(" {} ~{} out", th::SEP, tokens(out_tokens)));
    }
    if !deciding {
        body.push_str(&format!(" {} esc to interrupt", th::SEP));
    }
    // Left-aligned, never padded out to the full width. Right-aligning the
    // hint would pad this row to exactly the terminal width, and a
    // full-width row above the cursor is what a reflowing terminal turns
    // into two rows on resize.
    truncate_line(row(vec![mark, span(body, th::dim())]), width)
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ footer ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// Everything the footer needs. Deliberately small: the footer answers
/// "where am I, what am I talking to, how full is the window, what has this
/// cost" and nothing else.
pub struct Footer<'a> {
    pub model_id: &'a str,
    pub mode: &'a str,
    /// Fraction of the context window in use.
    pub ctx_used: f64,
    pub session_microusd: u64,
    pub price_verified: bool,
}

/// Color the context meter by how close compaction is: quiet until it
/// matters, then increasingly loud. This is the only ambient warning the
/// user gets before the window starts rewriting itself.
fn ctx_style(used: f64) -> ratatui::style::Style {
    if used >= 0.85 {
        th::danger()
    } else if used >= 0.6 {
        th::warn()
    } else {
        th::dim()
    }
}

pub fn mode_style(mode: &str) -> ratatui::style::Style {
    match mode {
        "auto" => th::warn(),
        "read-only" => th::ok(),
        _ => th::dim(),
    }
}

/// The status line under the composer: model, how much context is left, the
/// permission mode, and what the spend has been. Dim, one row, no meter and
/// no right-aligned column — Codex's shape, with the mode kept because it is
/// the one setting here with real consequences.
pub fn footer(state: &Footer<'_>, width: usize) -> Line<'static> {
    let left = (1.0 - state.ctx_used.clamp(0.0, 1.0)) * 100.0;
    let mut spans = vec![
        indent(GRID),
        span(state.model_id.to_string(), th::dim()),
        span(format!(" {} ", th::SEP), th::dim()),
        span(
            format!("Context {left:.0}% left"),
            ctx_style(state.ctx_used),
        ),
        span(format!(" {} ", th::SEP), th::dim()),
        span(format!("{} mode", state.mode), mode_style(state.mode)),
    ];
    if state.session_microusd > 0 {
        let cost = if state.price_verified {
            money(state.session_microusd)
        } else {
            // Invariant 7: an unverified price card is never shown as money.
            format!("~{} (unverified)", money(state.session_microusd))
        };
        spans.push(span(format!(" {} {cost}", th::SEP), th::dim()));
    }
    truncate_line(row(spans), width)
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ welcome ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// The opening card's CONTENT, kept as data so the card re-composes at
/// whatever width the screen is now. It used to be committed as rows composed
/// once at the startup width, which meant a narrowing could only clamp them —
/// and clamping a box eats its right border and replaces it with an ellipsis.
#[derive(Clone, Debug, PartialEq)]
pub struct Welcome {
    pub workspace: String,
    pub branch: Option<String>,
    pub model_name: String,
    pub context_tokens: u64,
    pub mode: String,
}

/// One-line explanation of what a permission mode actually does. Shown on
/// the welcome card and every time Shift+Tab changes it ‒ the names alone
/// ("ask", "auto") do not say what is gated, and that is exactly the thing
/// a user must not guess about.
pub fn mode_blurb(mode: &str) -> &'static str {
    match mode {
        "auto" => "edits run unprompted; commands off the allowlist still ask",
        "read-only" => "nothing is written and no command runs",
        _ => "every edit and command asks you first",
    }
}

/// The opening card: a rounded box sized to its content, holding the name,
/// the model and the directory — Codex's shape.
pub fn welcome(info: &Welcome, width: usize) -> Vec<Line<'static>> {
    let title = format!("{} butterfly", th::MARK_BRAND);
    let place = match &info.branch {
        Some(branch) => format!("{} on {branch}", info.workspace),
        None => info.workspace.to_string(),
    };
    let rows: Vec<(&str, String)> = vec![
        (
            "model:",
            format!(
                "{}  ({} context)   /model to change",
                info.model_name,
                tokens(info.context_tokens)
            ),
        ),
        ("directory:", place),
        (
            "mode:",
            format!("{}   {}", info.mode, mode_blurb(&info.mode)),
        ),
    ];
    let label_col = rows
        .iter()
        .map(|(label, _)| width_of(label))
        .max()
        .unwrap_or(0);
    // The card spans the terminal. It used to size itself to its content and
    // stop there, which on a wide window left it marooned in the corner with
    // most of the row empty beside it. A resize replays the transcript, so the
    // card is re-rendered at the new width and stretches with the window.
    // Content still truncates on a narrow one.
    let outer = width;
    let inner = outer.saturating_sub(4);
    let border = |left: &str, right: &str| {
        row(vec![span(
            format!("{left}{}{right}", th::BOX_H.repeat(inner + 2)),
            th::dim(),
        )])
    };
    let boxed = |content: Vec<Span<'static>>| {
        let mut spans = vec![span(format!("{} ", th::BOX_V), th::dim())];
        let used: usize = content.iter().map(|s| width_of(&s.content)).sum();
        spans.extend(content);
        if used < inner {
            spans.push(raw(" ".repeat(inner - used)));
        }
        spans.push(span(format!(" {}", th::BOX_V), th::dim()));
        row(spans)
    };
    let mut out = vec![border(th::BOX_TL, th::BOX_TR)];
    out.push(boxed(vec![span(
        truncate_cols(&title, inner),
        th::accent_bold(),
    )]));
    out.push(boxed(Vec::new()));
    for (label, value) in rows {
        let budget = inner.saturating_sub(label_col + 1);
        out.push(boxed(vec![
            span(pad_to(label, label_col + 1), th::dim()),
            raw(truncate_cols(&value, budget)),
        ]));
    }
    out.push(border(th::BOX_BL, th::BOX_BR));
    out
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ tables ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// Below this a column cannot carry a readable word, so the table stops
/// being a table and becomes stacked records instead.
const MIN_COL: usize = 8;
const COL_GAP: usize = 2;

/// Lay a parsed table out as aligned columns with a rule under the header
/// and no vertical lines ‒ box drawing around every cell costs a third of a
/// narrow terminal's width and buys nothing the alignment does not already
/// give. Cells wrap inside their own column, so a long value grows the row
/// downward instead of pushing the table off-screen.
pub fn table_rows(table: &md::Table, width: usize) -> Vec<Line<'static>> {
    let columns = table.columns();
    if columns == 0 {
        return Vec::new();
    }
    let avail = width.saturating_sub(GRID);
    let Some(widths) = column_widths(table, avail) else {
        return record_rows(table, width);
    };
    let mut out = Vec::new();
    out.extend(row_of(&table.headers, &widths, &table.aligns, true));
    out.push(row(vec![
        indent(GRID),
        span(
            widths
                .iter()
                .map(|w| th::RULE.repeat(*w))
                .collect::<Vec<_>>()
                .join(&" ".repeat(COL_GAP)),
            th::dim(),
        ),
    ]));
    for line in &table.rows {
        out.extend(row_of(line, &widths, &table.aligns, false));
    }
    out
}

/// A single token longer than this may be broken; below it, a column that
/// cannot hold its longest word is a reason to abandon columns entirely.
const WORD_CAP: usize = 24;

/// Column widths, or `None` when columns cannot be made to work and the
/// caller should stack the rows as records instead.
///
/// Natural widths win when they fit. Otherwise columns shrink widest-first,
/// but never below the longest word they must hold: a table whose cells are
/// chopped mid-word (`ledger-worke` / `r`) is harder to read than the same
/// data stacked, so that case gives up on columns rather than shredding.
fn column_widths(table: &md::Table, avail: usize) -> Option<Vec<usize>> {
    let columns = table.columns();
    let gaps = COL_GAP * columns.saturating_sub(1);
    if avail < gaps + columns * MIN_COL {
        return None;
    }
    let budget = avail - gaps;
    let cells = |i: usize| {
        std::iter::once(&table.headers)
            .flat_map(move |h| h.get(i))
            .chain(table.rows.iter().filter_map(move |row| row.get(i)))
            .map(|cell| md::spans_text(cell))
    };
    let mut widths: Vec<usize> = (0..columns)
        .map(|i| {
            cells(i)
                .map(|text| width_of(&text))
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect();
    if widths.iter().sum::<usize>() <= budget {
        return Some(widths);
    }
    let floors: Vec<usize> = (0..columns)
        .map(|i| {
            let longest_word = cells(i)
                .flat_map(|text| text.split_whitespace().map(width_of).collect::<Vec<_>>())
                .max()
                .unwrap_or(0);
            // Never wider than the column actually needs, never wider than
            // one over-long token justifies ‒ and a naturally narrow column
            // floors at its own width rather than at MIN_COL, which would
            // otherwise invert the bounds.
            let natural = widths[i];
            longest_word
                .min(WORD_CAP)
                .min(natural)
                .max(MIN_COL.min(natural))
        })
        .collect();
    if floors.iter().sum::<usize>() > budget {
        return None;
    }
    while widths.iter().sum::<usize>() > budget {
        // Take from the widest column that still has slack above its floor.
        let (victim, _) = widths
            .iter()
            .enumerate()
            .filter(|(i, w)| **w > floors[*i])
            .max_by_key(|(i, w)| (**w, std::cmp::Reverse(*i)))?;
        widths[victim] -= 1;
    }
    Some(widths)
}

/// One table row as its PHYSICAL rows ‒ a tall row (a cell that wrapped)
/// is several screen lines, and every caller counts screen lines.
fn row_of(
    cells: &[Vec<md::Span>],
    widths: &[usize],
    aligns: &[Align],
    header: bool,
) -> Vec<Line<'static>> {
    let wrapped: Vec<Vec<Line<'static>>> = widths
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let empty = Vec::new();
            let spans = cells.get(i).unwrap_or(&empty);
            let spans: Vec<md::Span> = if header {
                // A header is one visual unit; inline styling inside it
                // would fight the weight that marks it as the header.
                spans
                    .iter()
                    .map(|s| md::Span {
                        style: md::SpanStyle::Bold,
                        text: s.text.clone(),
                    })
                    .collect()
            } else {
                spans.clone()
            };
            md::styled_lines(&spans, *w, *w)
        })
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    let mut lines = Vec::with_capacity(height);
    for index in 0..height {
        let mut spans: Vec<Span<'static>> = vec![indent(GRID)];
        for (i, cell) in wrapped.iter().enumerate() {
            if i > 0 {
                spans.push(indent(COL_GAP));
            }
            let content = cell.get(index).cloned().unwrap_or_default();
            let align = aligns.get(i).copied().unwrap_or(Align::Left);
            // The last column needs no trailing pad; trailing spaces make a
            // copied row ragged.
            let last = i + 1 == wrapped.len();
            spans.extend(align_cell(content, widths[i], align, last).spans);
        }
        lines.push(trim_end(row(spans)));
    }
    lines
}

/// Drop trailing whitespace-only spans (and trailing spaces in the last
/// span) so a copied row is not ragged.
fn trim_end(mut line: Line<'static>) -> Line<'static> {
    while let Some(last) = line.spans.last() {
        if last.content.trim_end().is_empty() {
            line.spans.pop();
        } else {
            break;
        }
    }
    if let Some(last) = line.spans.last_mut() {
        let trimmed = last.content.trim_end().to_string();
        last.content = trimmed.into();
    }
    line
}

fn align_cell(content: Line<'static>, width: usize, align: Align, last: bool) -> Line<'static> {
    let pad = width.saturating_sub(line_cols(&content));
    match align {
        Align::Left if last => content,
        Align::Left => th::pad_line(content, width, th::plain()),
        Align::Right => {
            let mut spans = vec![indent(pad)];
            spans.extend(content.spans);
            row(spans)
        }
        Align::Center => {
            let left = pad / 2;
            let mut spans = vec![indent(left)];
            spans.extend(content.spans);
            let line = row(spans);
            if last {
                line
            } else {
                th::pad_line(line, width, th::plain())
            }
        }
    }
}

/// Too narrow for columns: stack each row as a record ‒ its first cell as
/// the title, the rest indented under it. The responsive answer, and far
/// more readable than three-character-wide columns.
fn record_rows(table: &md::Table, width: usize) -> Vec<Line<'static>> {
    let label_col = table
        .headers
        .iter()
        .skip(1)
        .map(|h| width_of(&md::spans_text(h)))
        .max()
        .unwrap_or(0);
    let named = table.columns() > 2 && label_col > 0;
    let mut out = Vec::new();
    for (index, record) in table.rows.iter().enumerate() {
        if index > 0 {
            out.push(blank());
        }
        let title = md::spans_text(record.first().unwrap_or(&Vec::new()));
        for line in md::styled_lines(
            &[md::Span {
                style: md::SpanStyle::Bold,
                text: title,
            }],
            width.saturating_sub(GRID),
            width.saturating_sub(GRID),
        ) {
            let mut spans = vec![indent(GRID)];
            spans.extend(line.spans);
            out.push(row(spans));
        }
        for (i, cell) in record.iter().enumerate().skip(1) {
            let gutter = GRID + 2;
            let label = if named {
                pad_to(&md::spans_text(&table.headers[i]), label_col + COL_GAP)
            } else {
                String::new()
            };
            let label_cols = width_of(&label);
            let budget = width.saturating_sub(gutter + label_cols);
            for (n, line) in md::styled_lines(cell, budget.max(MIN_COL), budget.max(MIN_COL))
                .into_iter()
                .enumerate()
            {
                let mut spans = vec![indent(gutter)];
                if n == 0 && !label.is_empty() {
                    spans.push(span(label.clone(), th::dim()));
                } else if label_cols > 0 {
                    spans.push(indent(label_cols));
                }
                spans.extend(line.spans);
                out.push(row(spans));
            }
        }
    }
    out
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ list rows ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// One row of any arrow-key list ‒ approval options, the model picker, the
/// provider picker, completion choices. They share a shape on purpose: the
/// selection marker sits in column 0 with the rest of the grid, the label
/// starts in column 2, and a dim hint right-aligns. Selection is carried by
/// the marker AND the color, so it survives a terminal with no color.
pub fn option_row(label: Line<'static>, hint: &str, selected: bool, width: usize) -> Line<'static> {
    let marker = if selected { th::MARK_SELECT } else { " " };
    let style = if selected { th::accent() } else { th::dim() };
    let mut spans = vec![span(format!("{marker} "), style)];
    // A picker row may carry its own styling (the model rows do); a plain
    // label takes the selection color so the whole row reads as one unit.
    spans.extend(label.spans.into_iter().map(|mut s| {
        if s.style == th::plain() {
            s.style = style;
        }
        s
    }));
    let left = row(spans);
    if hint.is_empty() {
        truncate_line(left, width)
    } else {
        justify(left, row(vec![span(hint.to_string(), th::dim())]), width)
    }
}

/// A plain-text picker row, for the lists whose labels carry no styling.
pub fn option_text(label: &str, hint: &str, selected: bool, width: usize) -> Line<'static> {
    option_row(row(vec![raw(label.to_string())]), hint, selected, width)
}

/// A numbered choice, the shape Codex uses for a decision: `› 1. Yes, run it`.
/// The number is an access key, so it is part of the label, not decoration.
pub fn numbered_row(
    index: usize,
    label: &str,
    key: &str,
    selected: bool,
    width: usize,
) -> Line<'static> {
    let marker = if selected { th::MARK_SELECT } else { " " };
    let style = if selected { th::accent() } else { th::dim() };
    truncate_line(
        row(vec![span(
            format!("{marker} {}. {label} ({key})", index + 1),
            style,
        )]),
        width,
    )
}

/// A completion row. Unlike a picker option, its hint belongs immediately
/// after the label column, not at the far edge: the label is a short file
/// name and the hint is the directory that disambiguates it, so the two
/// have to read as one phrase.
pub fn completion_row(label: &str, hint: &str, selected: bool, width: usize) -> Line<'static> {
    let marker = if selected { th::MARK_SELECT } else { " " };
    let style = if selected { th::accent() } else { th::dim() };
    let mut spans = vec![span(format!("{marker} {label}"), style)];
    if !hint.is_empty() {
        spans.push(span(format!("  {hint}"), th::dim()));
    }
    truncate_line(row(spans), width)
}

/// The dim instruction row under a selector: what this list is on the left,
/// which keys drive it on the right.
pub fn hint_row(left: &str, right: &str, width: usize) -> Line<'static> {
    justify(
        row(vec![indent(GRID), span(left.to_string(), th::dim())]),
        row(vec![span(right.to_string(), th::dim())]),
        width,
    )
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ approvals ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// What the decision is about, said plainly, plus the change itself.
/// Presentation only: this never alters which calls are gated, the options
/// offered, their order, or what a keypress consents to.
pub fn approval_card(
    request: &ApprovalRequest,
    detail: Option<&str>,
    width: usize,
) -> Vec<Line<'static>> {
    let (tool, target) = split_preview(&request.preview);
    let mut rows: Vec<Line<'static>> = vec![
        blank(),
        row(vec![
            indent(GRID),
            span(approval_question(tool), th::bold()),
        ]),
        blank(),
    ];
    if !target.is_empty() {
        rows.push(row(vec![
            indent(GRID),
            span(format!("{} ", approval_sigil(tool)), th::dim()),
            raw(truncate_cols(target, width.saturating_sub(GRID + 2))),
        ]));
        rows.push(blank());
    }
    if let Some(warning) = &request.policy_warning {
        rows.push(truncate_line(
            row(vec![
                indent(GRID),
                span(
                    truncate_cols(warning, width.saturating_sub(GRID)),
                    th::danger(),
                ),
            ]),
            width,
        ));
        rows.push(blank());
    }
    if let Some(detail) = detail {
        let stat = diff_stat(detail);
        if !stat.is_empty() {
            let mut spans = vec![indent(GRID)];
            spans.extend(stat);
            rows.push(row(spans));
        }
        rows.extend(detail_rows(detail, width));
        rows.push(blank());
    }
    rows
}

/// The sentence Codex asks, in Butterfly's vocabulary.
fn approval_question(tool: &str) -> &'static str {
    match tool {
        "run_command" | "shell" => "Would you like to run the following command?",
        "edit" | "write_file" => "Would you like to apply the following change?",
        _ => "Would you like to allow the following?",
    }
}

/// `$` for a command, the bullet for anything else.
fn approval_sigil(tool: &str) -> &'static str {
    match tool {
        "run_command" | "shell" => "$",
        _ => th::MARK_TOOL,
    }
}

/// `edit README.md`, `run cargo test`, `write src/new.rs` ‒ the sentence a
/// user is actually being asked to approve.
pub fn approval_subject(tool: &str, target: &str) -> String {
    if target.is_empty() {
        verb_of(tool).to_string()
    } else {
        format!("{} {target}", verb_of(tool))
    }
}

/// `+3 −1` for a detail that carries a diff; empty otherwise.
fn diff_stat(detail: &str) -> Vec<Span<'static>> {
    let (mut added, mut removed) = (0usize, 0usize);
    for line in detail.lines() {
        match classify(line) {
            Row::Added(_) => added += 1,
            Row::Removed(_) => removed += 1,
            _ => {}
        }
    }
    if added == 0 && removed == 0 {
        Vec::new()
    } else {
        vec![
            span(format!("+{added}"), th::ok()),
            raw(" "),
            span(format!("−{removed}"), th::danger()),
        ]
    }
}

enum Row<'a> {
    Added(&'a str),
    Removed(&'a str),
    Context(&'a str),
    Command(&'a str),
    Note(&'a str),
}

/// Core emits a fixed shape (`  - `, `  + `, `  $ `, four-space context);
/// classify it so the CLI can own the colors, the gutter and ‒ the reason
/// this exists ‒ the width clamp. Unclamped detail lines used to wrap at
/// column 0 and shred the layout of the one prompt that must be readable.
fn classify(line: &str) -> Row<'_> {
    if let Some(rest) = line.strip_prefix("  - ") {
        Row::Removed(rest)
    } else if let Some(rest) = line.strip_prefix("  + ") {
        Row::Added(rest)
    } else if let Some(rest) = line.strip_prefix("  $ ") {
        Row::Command(rest)
    } else if let Some(rest) = line.strip_prefix("    ") {
        Row::Context(rest)
    } else {
        Row::Note(line.trim_start())
    }
}

/// Render an approval detail as clamped, gutter-marked rows.
pub fn detail_rows(detail: &str, width: usize) -> Vec<Line<'static>> {
    let budget = width.saturating_sub(GRID + 2).max(12);
    detail
        .lines()
        .map(|line| {
            let (body, style) = match classify(line) {
                Row::Removed(text) => (
                    format!("- {}", truncate_cols(&text.replace('\t', "    "), budget)),
                    th::danger(),
                ),
                Row::Added(text) => (
                    format!("+ {}", truncate_cols(&text.replace('\t', "    "), budget)),
                    th::ok(),
                ),
                Row::Command(text) => (format!("$ {}", truncate_cols(text, budget)), th::accent()),
                Row::Context(text) => (
                    format!("  {}", truncate_cols(&text.replace('\t', "    "), budget)),
                    th::dim(),
                ),
                Row::Note(text) => (truncate_cols(text, budget), th::dim()),
            };
            row(vec![indent(GRID), span(body, style)])
        })
        .collect()
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ turn summary ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

pub struct TurnFacts {
    pub elapsed: Duration,
    pub calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_rate: f64,
    pub microusd: u64,
    pub price_verified: bool,
    pub elisions: u32,
    pub compactions: u32,
    pub cancelled: bool,
}

/// The one-line receipt after a turn. Running totals (session cost, context
/// pressure) live in the footer now, so this says only what THIS turn did.
fn plural(count: u64, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Plain text: the caller re-wraps it as dim prose, so it carries no
/// styling of its own.
pub fn turn_summary(facts: &TurnFacts) -> String {
    let cost = if facts.price_verified {
        money(facts.microusd)
    } else {
        format!("~{} unverified", money(facts.microusd))
    };
    let sep = th::SEP;
    let mut line = format!(
        "{} {sep} {} {sep} {} in ({:.0}% cached) {sep} {} out {sep} {cost}",
        short_elapsed(facts.elapsed),
        plural(facts.calls as u64, "call"),
        tokens(facts.input_tokens),
        facts.cache_rate * 100.0,
        tokens(facts.output_tokens),
    );
    if facts.elisions > 0 || facts.compactions > 0 {
        line.push_str(&format!(
            " {sep} compacted ({} elision, {} summarize)",
            facts.elisions, facts.compactions
        ));
    }
    if facts.cancelled {
        line.push_str(&format!(" {sep} cancelled"));
    }
    line
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ help ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

const HELP_SECTIONS: [(&str, &[(&str, &str)]); 3] = [
    (
        "commands",
        &[
            ("/task <statement>", "run an autonomous, verified task"),
            ("/plan", "show the tracked working state"),
            ("/compact [focus]", "force a compaction now"),
            ("/usage", "tokens, cost, cache-hit rate, session id"),
            ("/model [name]", "pick a model, or switch directly"),
            ("/connect", "save a provider API key"),
            ("/help", "this card"),
            ("/quit", "exit"),
        ],
    ),
    (
        "writing a message",
        &[
            ("@path", "attach a file; tab or enter accepts"),
            ("enter", "send · ctrl+j inserts a newline"),
            ("⇡ ⇣", "history, or move between lines"),
            ("ctrl+w / ctrl+u", "delete a word / clear the line"),
            ("ctrl+a / ctrl+e", "jump to line start / end"),
            ("paste", "large pastes collapse to a placeholder"),
        ],
    ),
    (
        "during a session",
        &[
            ("shift+tab", "cycle mode: ask ⇢ auto ⇢ read-only"),
            ("ctrl+t", "full transcript, hidden reasoning included"),
            ("ctrl+r", "toggle the live reasoning stream"),
            ("esc", "close the dropdown, or interrupt the turn"),
            ("ctrl+c", "clear the line, or cancel the running turn"),
            ("ctrl+d", "exit"),
        ],
    ),
];

/// The help card: grouped, aligned, and styled so the eye lands on the key
/// column. The old help was one flat block of twenty lines in which every
/// line looked equally important.
pub fn help_card(width: usize) -> Vec<Line<'static>> {
    let key_col = HELP_SECTIONS
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .map(|(key, _)| width_of(key))
        .max()
        .unwrap_or(16);
    let mut out: Vec<Line<'static>> = Vec::new();
    for (title, rows) in HELP_SECTIONS {
        if !out.is_empty() {
            out.push(blank());
        }
        out.push(row(vec![span(title, th::bold())]));
        for (key, description) in rows {
            out.push(truncate_line(
                row(vec![
                    indent(GRID),
                    span(pad_to(key, key_col), th::accent()),
                    indent(2),
                    span(*description, th::dim()),
                ]),
                width,
            ));
        }
    }
    out.push(blank());
    out.push(truncate_line(
        row(vec![span(
            "Type while a turn runs ‒ your message lands at the next turn boundary.",
            th::dim(),
        )]),
        width,
    ));
    out
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ usage ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

pub struct UsageFacts<'a> {
    pub session_id: &'a str,
    pub model_id: &'a str,
    pub input_tokens: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_rate: f64,
    pub output_tokens: u64,
    pub microusd: u64,
    pub price_verified: bool,
    pub ctx_used: f64,
}

/// `/usage`, laid out as a label column with the numbers right-aligned on
/// one edge so magnitudes compare at a glance instead of having to be read.
pub fn usage_card(facts: &UsageFacts<'_>, width: usize) -> Vec<Line<'static>> {
    let cost = if facts.price_verified {
        money(facts.microusd)
    } else {
        format!("~{} (unverified price card)", money(facts.microusd))
    };
    let rows: Vec<(&str, String)> = vec![
        ("model", facts.model_id.to_string()),
        ("input tokens", tokens(facts.input_tokens)),
        ("cache hits", tokens(facts.cache_hits)),
        ("cache misses", tokens(facts.cache_misses)),
        ("hit rate", format!("{:.1}%", facts.cache_rate * 100.0)),
        ("output tokens", tokens(facts.output_tokens)),
        ("cost so far", cost),
        (
            "context used",
            format!(
                "{} {:.0}%",
                meter(facts.ctx_used, 8),
                facts.ctx_used.clamp(0.0, 1.0) * 100.0
            ),
        ),
    ];
    let label_col = rows
        .iter()
        .map(|(label, _)| width_of(label))
        .max()
        .unwrap_or(0);
    let mut out = vec![truncate_line(
        row(vec![
            span("session", th::bold()),
            raw(" "),
            span(facts.session_id.to_string(), th::dim()),
        ]),
        width,
    )];
    for (label, value) in rows {
        out.push(truncate_line(
            row(vec![
                indent(GRID),
                span(pad_to(label, label_col + 2), th::dim()),
                raw(value),
            ]),
            width,
        ));
    }
    out
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ model picker ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

pub struct ModelRow<'a> {
    pub display_name: &'a str,
    pub id: &'a str,
    pub context_tokens: u64,
    pub current: bool,
    pub has_key: bool,
}

/// Aligned picker rows. Which model you can actually reach (`no key`) and
/// how much window it has are the two facts that decide the choice, and
/// neither was visible before.
pub fn model_rows(models: &[ModelRow<'_>], width: usize) -> Vec<Line<'static>> {
    let name_col = models
        .iter()
        .map(|m| width_of(m.display_name))
        .max()
        .unwrap_or(0)
        .min(width / 3);
    let id_col = models.iter().map(|m| width_of(m.id)).max().unwrap_or(0);
    models
        .iter()
        .map(|model| {
            let mut spans = vec![
                raw(pad_to(
                    &truncate_cols(model.display_name, name_col),
                    name_col,
                )),
                indent(2),
                span(pad_to(model.id, id_col), th::dim()),
            ];
            if width >= 58 {
                spans.push(indent(2));
                spans.push(span(
                    format!("{:>6}", tokens(model.context_tokens)),
                    th::dim(),
                ));
            }
            if model.current {
                spans.push(indent(2));
                spans.push(span("current", th::accent()));
            } else if !model.has_key {
                spans.push(indent(2));
                spans.push(span("no key", th::dim()));
            }
            row(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {

    use super::*;

    const WIDTHS: [usize; 5] = [24, 40, 62, 80, 120];

    /// Both halves of the width contract, checked on the same composed row:
    /// it must fit, and its width must not be something the terminal gets to
    /// choose. A glyph marked East Asian Ambiguous (`·`, `─`, `…`, `●`)
    /// renders one column on xterm and two on Ghostty, so a row built from
    /// them is only accidentally the right size. The footer carried two and
    /// the turn receipt five before this was enforced.
    fn check(line: &Line<'_>, width: usize, label: &str) {
        assert!(
            line_cols(line) <= width,
            "{label} @{width}: {:?} is {} cols",
            line.to_string(),
            line_cols(line)
        );
        for ch in line.to_string().chars() {
            // Chrome glyphs are measured in the reference terminal rather than
            // assumed (see `theme::CHROME_GLYPHS`); everything else must be
            // unambiguous, because its width would otherwise be the terminal's
            // choice and the row only accidentally the right size.
            if th::cell_width(ch) == 1 && th::is_ambiguous_width(ch) {
                continue;
            }
            assert!(
                !th::is_ambiguous_width(ch),
                "{label} @{width} draws ambiguous-width {ch:?} (U+{:04X}): {:?}",
                ch as u32,
                line.to_string(),
            );
        }
    }

    #[test]
    fn every_surface_fits_its_width() {
        for width in WIDTHS {
            let info = Welcome {
                workspace: "butterfly-cli".into(),
                branch: Some("feature/a-fairly-long-branch-name".into()),
                model_name: "DeepSeek V4 Flash (served as V4.1 Flash)".into(),
                context_tokens: 164_000,
                mode: "ask".into(),
            };
            for line in welcome(&info, width) {
                check(&line, width, "welcome");
            }
            let state = Footer {
                model_id: "deepseek-v4-flash",
                mode: "read-only",
                ctx_used: 0.93,
                session_microusd: 2_916,
                price_verified: true,
            };
            check(&footer(&state, width), width, "footer");
            let line = tool_line(
                "read_file(a/very/deeply/nested/path/that/keeps/going/mod.rs)",
                Some(ToolOutcome::Failed {
                    reason: "no such file or directory in this workspace".into(),
                    elapsed: Duration::from_millis(12),
                }),
                width,
            );
            check(&line, width, "tool");
            let line = activity_line(
                &Phase::Running("run_command(cargo test --workspace --locked)".into()),
                3,
                Duration::from_secs(97),
                12_400,
                width,
            );
            check(&line, width, "activity");
            for line in detail_rows(
                "README.md:1\n    context line that is really quite long indeed and then some\n  - # butterfly ‒ Harness\n  + # butterfly\n  $ cargo test --workspace\n",
                width,
            ) {
                check(&line, width, "detail");
            }
            for line in help_card(width) {
                check(&line, width, "help");
            }
            let facts = UsageFacts {
                session_id: "0199c0de-dead-beef-cafe-000000000000",
                model_id: "deepseek-v4-flash",
                input_tokens: 240_000,
                cache_hits: 180_000,
                cache_misses: 60_000,
                cache_rate: 0.75,
                output_tokens: 8_400,
                microusd: 2_916,
                price_verified: false,
                ctx_used: 0.42,
            };
            for line in usage_card(&facts, width) {
                check(&line, width, "usage");
            }
        }
    }

    #[test]
    fn failed_rows_do_not_repeat_the_path_they_already_show() {
        let line = tool_line(
            "read_file(src/nope.rs)",
            Some(ToolOutcome::Failed {
                reason: "src/nope.rs: no such file".into(),
                elapsed: Duration::from_millis(1),
            }),
            80,
        )
        .to_string();
        assert!(line.contains("no such file"));
        assert_eq!(line.matches("src/nope.rs").count(), 1, "{line:?}");
    }

    #[test]
    fn tool_rows_align_verbs_and_carry_their_outcome() {
        let line = tool_line(
            "read_file(Cargo.toml)",
            Some(ToolOutcome::Ok {
                elapsed: Duration::from_millis(8),
            }),
            60,
        );
        let plain = line.to_string();
        assert!(plain.contains("Read   "), "verb column lost: {plain:?}");
        assert!(plain.contains("Cargo.toml") && plain.contains("8ms"));
        assert_eq!(line_cols(&line), 60, "outcome must right-align");
        // A mutating verb takes the accent; a read stays dim.
        let mutating = tool_line("edit(README.md)", None, 60);
        assert!(mutating
            .spans
            .iter()
            .any(|s| s.content.starts_with("Edit") && s.style == th::accent()));
        let reading = tool_line("read_file(README.md)", None, 60);
        assert!(reading
            .spans
            .iter()
            .any(|s| s.content.starts_with("Read") && s.style == th::dim()));
        // A bare tool name (no arguments) still renders.
        assert!(tool_line("update_plan", None, 40)
            .to_string()
            .contains("Plan"));
    }

    #[test]
    fn approval_cards_state_the_decision_and_count_the_diff() {
        let request = ApprovalRequest {
            tool_name: "edit".into(),
            preview: "edit(README.md)".into(),
            detail: None,
            policy_warning: Some("command is not on the auto-run allowlist".into()),
            scope_key: "edit".into(),
        };
        let detail = "README.md:1\n  - old line\n  + new line\n  + another new line\n";
        let rows = approval_card(&request, Some(detail), 70);
        let text: String = rows
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("Would you like to apply the following change?"),
            "question missing: {text}"
        );
        assert!(text.contains("README.md"), "target missing: {text}");
        assert!(
            text.contains("+2") && text.contains("−1"),
            "stat missing: {text}"
        );
        assert!(text.contains("auto-run allowlist"), "warning dropped");
        assert!(rows.iter().all(|line| line_cols(line) <= 70));
    }

    #[test]
    fn deciding_never_offers_the_interrupt_hint() {
        // Esc answers the approval menu; telling the user it interrupts the
        // turn would misdescribe what the key does.
        let line = activity_line(&Phase::Deciding, 0, Duration::from_secs(4), 900, 80).to_string();
        assert!(!line.contains("interrupt"), "{line:?}");
        assert!(line.contains("waiting for you"));
        let line = activity_line(&Phase::Thinking, 0, Duration::from_secs(4), 900, 80).to_string();
        assert!(line.contains("interrupt") && line.contains("thinking"));
    }

    #[test]
    fn unverified_prices_are_never_shown_as_real_money() {
        let facts = TurnFacts {
            elapsed: Duration::from_secs(2),
            calls: 3,
            input_tokens: 24_000,
            output_tokens: 840,
            cache_rate: 0.75,
            microusd: 2_916,
            price_verified: false,
            elisions: 0,
            compactions: 0,
            cancelled: false,
        };
        assert!(turn_summary(&facts).contains("unverified"));
        let state = Footer {
            model_id: "m",
            mode: "ask",
            ctx_used: 0.1,
            session_microusd: 2_916,
            price_verified: false,
        };
        assert!(footer(&state, 120).to_string().contains("unverified"));
    }

    fn cell(text: &str) -> Vec<md::Span> {
        vec![md::Span {
            style: md::SpanStyle::Plain,
            text: text.into(),
        }]
    }

    fn services_table() -> md::Table {
        md::Table {
            headers: vec![
                cell("Service"),
                cell("Language"),
                cell("Owner"),
                cell("Last deploy"),
            ],
            aligns: vec![Align::Left, Align::Right, Align::Left, Align::Left],
            rows: vec![
                vec![
                    cell("checkout-api"),
                    cell("Go"),
                    cell("payments-platform"),
                    cell("2026-09-02"),
                ],
                vec![
                    cell("notification-fanout"),
                    cell("Python"),
                    cell("growth"),
                    cell("2026-09-07"),
                ],
            ],
        }
    }

    #[test]
    fn tables_align_columns_and_respect_alignment() {
        let rows = table_rows(&services_table(), 90);
        // Header, rule, two body rows.
        assert_eq!(rows.len(), 4, "{rows:#?}");
        assert!(
            rows[1].to_string().contains('⎯'),
            "rule missing: {:?}",
            rows[1].to_string()
        );
        // Ids line up under one another.
        let rows: Vec<String> = rows.iter().map(|line| line.to_string()).collect();
        assert_eq!(
            rows[2].find("payments-platform"),
            rows[0].find("Owner"),
            "columns drifted: {rows:#?}"
        );
        // The right-aligned column ends on a shared edge.
        let end = |row: &String, text: &str| row.find(text).map(|at| at + text.len());
        assert_eq!(end(&rows[2], "Go"), end(&rows[3], "Python"));
    }

    #[test]
    fn narrow_tables_stack_rather_than_cut_words_in_half() {
        // Wide enough to squeeze four columns, too narrow to do it without
        // chopping `notification-fanout` and `payments-platform` mid-word.
        let rows: Vec<String> = table_rows(&services_table(), 56)
            .iter()
            .map(|line| line.to_string())
            .collect();
        let text = rows.join("\n");
        assert!(
            text.contains("notification-fanout") && text.contains("payments-platform"),
            "words were split: {text}"
        );
        // Stacked records: the row title on its own line, fields under it.
        assert!(rows.iter().any(|row| row.trim() == "notification-fanout"));
        assert!(rows.iter().any(|row| row.contains("Last deploy")));
    }

    #[test]
    fn table_cells_wrap_inside_their_column_instead_of_overflowing() {
        let table = md::Table {
            headers: vec![cell("Metric"), cell("Value")],
            aligns: vec![Align::Left, Align::Left],
            rows: vec![vec![
                cell("Code"),
                cell("~46.9k lines Python / 393 files, ~2.8k lines Go / 33 files, plus YAML"),
            ]],
        };
        let raw_rows = table_rows(&table, 60);
        assert!(raw_rows.iter().all(|line| line_cols(line) <= 60));
        let rows: Vec<String> = raw_rows.iter().map(|line| line.to_string()).collect();
        assert!(rows.len() > 3, "long cell did not wrap: {rows:#?}");
        // Continuation rows hang under the value column, not at column 0.
        let value_col = rows[2].find('~').expect("value column");
        for row in &rows[3..] {
            assert!(
                row.starts_with(&" ".repeat(value_col)),
                "continuation left the value column: {row:?}"
            );
        }
        // And the wrap loses nothing.
        let value: String = rows[2..]
            .iter()
            .map(|row| row.trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(value.ends_with("plus YAML"), "{value:?}");
    }

    #[test]
    fn model_rows_flag_the_ones_you_cannot_reach() {
        let rows = model_rows(
            &[
                ModelRow {
                    display_name: "DeepSeek V4 Flash",
                    id: "deepseek-v4-flash",
                    context_tokens: 164_000,
                    current: true,
                    has_key: true,
                },
                ModelRow {
                    display_name: "GLM-5.3 Flash",
                    id: "glm-5.3-flash",
                    context_tokens: 200_000,
                    current: false,
                    has_key: false,
                },
            ],
            80,
        );
        let plain: Vec<String> = rows.iter().map(|line| line.to_string()).collect();
        assert!(plain[0].contains("current") && plain[0].contains("164.0k"));
        assert!(plain[1].contains("no key"));
        // Ids align into one column.
        assert_eq!(
            plain[0].find("deepseek-v4-flash"),
            plain[1].find("glm-5.3-flash"),
        );
    }
}

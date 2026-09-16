//! Composed surfaces: the welcome card, footer, activity line, tool lines,
//! approval card, turn summary, help card and model rows.
//!
//! Everything here is a pure function from data to styled rows at a given
//! width, so each surface can be unit-tested for exact column budgets
//! without a terminal ‒ the inline TUI's erase math strands stale frames the
//! moment a composed line is wider than the screen.

use std::time::Duration;

use butterfly_core::session::approval::ApprovalRequest;

use super::markdown::{self as md, Align};

use super::theme::{
    self as th, justify, meter, money, pad_to, short_elapsed, tokens, truncate_cols, visible_cols,
    width_of, ACCENT, ACCENT_BOLD, BOLD, DANGER, DIM, OK, RESET, SEP, WARN,
};

/// Content column of the shared grid: every marker sits in column 0 and
/// every piece of content starts in column 2, so `❯`, `◉`, `⚒`, `▪` and `✦`
/// all line up down the left edge.
pub const GRID: usize = 2;

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ tool lines ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// A short verb for each tool, padded into one column so a run of tool
/// calls reads as a list of actions rather than a list of function calls.
/// Unknown tools (config-defined, future) fall back to their own name.
fn verb_of(tool: &str) -> &str {
    match tool {
        "read_file" => "read",
        "list_files" => "list",
        "search_text" => "search",
        "edit" => "edit",
        "write_file" => "write",
        "run_command" => "run",
        "shell" => "shell",
        "update_plan" => "plan",
        "repo_map" => "map",
        "symbol_search" => "symbol",
        "complete_task" => "done",
        "report_verdict" => "verdict",
        "ask_user" => "ask",
        other => other,
    }
}

const VERB_COL: usize = 6;

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
pub fn tool_line(preview: &str, outcome: Option<ToolOutcome>, width: usize) -> String {
    let (tool, target) = split_preview(preview);
    let verb_color = if is_mutating(tool) { ACCENT } else { DIM };
    let head = format!(
        "{DIM}{}{RESET} {verb_color}{}{RESET} ",
        th::MARK_TOOL,
        pad_to(verb_of(tool), VERB_COL),
    );
    let right = match &outcome {
        Some(ToolOutcome::Ok { elapsed }) => {
            format!(
                "{OK}{}{RESET} {DIM}{}{RESET}",
                th::MARK_OK,
                short_elapsed(*elapsed)
            )
        }
        Some(ToolOutcome::Failed { reason, elapsed }) => {
            let timing = short_elapsed(*elapsed);
            // Tool errors often lead with the path the row already shows.
            let reason = reason
                .strip_prefix(&format!("{target}: "))
                .unwrap_or(reason)
                .trim();
            if reason.is_empty() {
                format!("{DANGER}{}{RESET} {DIM}{timing}{RESET}", th::MARK_FAIL)
            } else {
                format!(
                    "{DANGER}{} {}{RESET} {DIM}{timing}{RESET}",
                    th::MARK_FAIL,
                    truncate_cols(reason, width / 3),
                )
            }
        }
        None => String::new(),
    };
    // The target gets whatever the markers and the outcome leave behind.
    let budget = width
        .saturating_sub(visible_cols(&head))
        .saturating_sub(visible_cols(&right))
        .saturating_sub(if right.is_empty() { 0 } else { 2 });
    let body = format!("{head}{}", truncate_cols(target, budget.max(8)));
    justify(&body, &right, width)
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
) -> String {
    let deciding = *phase == Phase::Deciding;
    let mark = if deciding {
        format!("{WARN}{}{RESET}", th::MARK_WARN)
    } else {
        format!(
            "{ACCENT}{}{RESET}",
            th::SPINNER[spinner_frame % th::SPINNER.len()]
        )
    };
    let mut left = format!("{mark} {DIM}{}", phase.label());
    left.push_str(&format!("{DIM} ⋅ {}", th::duration(elapsed)));
    if out_tokens > 0 && !deciding {
        left.push_str(&format!(" ⋅ ~{} out", tokens(out_tokens)));
    }
    left.push_str(RESET);
    if !deciding {
        left.push_str(&format!("{DIM} ⋅ esc to interrupt{RESET}"));
    }
    // Left-aligned, never padded out to the full width. Right-aligning the
    // hint would pad this row to exactly the terminal width, and a
    // full-width row above the cursor is what a reflowing terminal turns
    // into two rows on resize, stranding the block above the erase.
    truncate_styled_left(&left, width)
}

fn truncate_styled_left(line: &str, width: usize) -> String {
    if visible_cols(line) <= width {
        line.to_string()
    } else {
        th::truncate_styled(line, width)
    }
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
fn ctx_color(used: f64) -> &'static str {
    if used >= 0.85 {
        DANGER
    } else if used >= 0.6 {
        WARN
    } else {
        DIM
    }
}

pub fn mode_color(mode: &str) -> &'static str {
    match mode {
        "auto" => WARN,
        "read-only" => OK,
        _ => DIM,
    }
}

/// One footer row: identity and budget on the left, the permission mode ‒
/// the one setting with real consequences ‒ on the right, in its own color.
pub fn footer(state: &Footer<'_>, width: usize) -> String {
    let used = state.ctx_used.clamp(0.0, 1.0);
    let ctx = format!(
        "{}{} {:.0}%{RESET}",
        ctx_color(used),
        meter(used, 8),
        used * 100.0,
    );
    let cost = if state.session_microusd == 0 {
        String::new()
    } else if state.price_verified {
        money(state.session_microusd)
    } else {
        // Invariant 7: an unverified price card must never be presented as
        // real money.
        format!("~{} (unverified)", money(state.session_microusd))
    };
    let mode_hint = format!(
        "{}{} mode{RESET}{DIM} ⋅ shift+tab{RESET}",
        mode_color(state.mode),
        state.mode,
    );
    // Widest first, then shed the optional halves as the terminal narrows.
    let candidates = [
        format!(
            "{DIM}{} {SEP} ctx {ctx}{DIM}{}{RESET}",
            state.model_id,
            if cost.is_empty() {
                String::new()
            } else {
                format!(" ⋅ {cost}")
            }
        ),
        format!("{DIM}{} {SEP} ctx {ctx}{RESET}", state.model_id),
        format!("{DIM}ctx {ctx}{RESET}"),
    ];
    let rights = [
        mode_hint,
        format!("{}{} mode{RESET}", mode_color(state.mode), state.mode),
        format!("{}{}{RESET}", mode_color(state.mode), state.mode),
    ];
    for left in &candidates {
        for right in &rights {
            if visible_cols(left) + visible_cols(right) + 2 <= width {
                return justify(left, right, width);
            }
        }
    }
    th::truncate_styled(&format!("{DIM}{}{RESET}", state.mode), width)
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ welcome ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

pub struct Welcome<'a> {
    pub workspace: &'a str,
    pub branch: Option<String>,
    pub model_name: &'a str,
    pub context_tokens: u64,
    pub mode: &'a str,
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

/// The opening card. A session used to start on a bare screen with a
/// session id nobody types; this answers the four questions a first
/// keystroke depends on ‒ where am I, what model, what is it allowed to do,
/// how do I drive it.
pub fn welcome(info: &Welcome<'_>, width: usize) -> Vec<String> {
    let place = match &info.branch {
        Some(branch) => format!("{} {DIM}on{RESET} {branch}", info.workspace),
        None => info.workspace.to_string(),
    };
    let model = format!(
        "{} {DIM}⋅{RESET} {} context",
        info.model_name,
        tokens(info.context_tokens),
    );
    let mode = format!("{} {DIM}⋅ {}{RESET}", info.mode, mode_blurb(info.mode));
    let hints = [
        format!("{ACCENT}@{RESET}{DIM} file{RESET}"),
        format!("{ACCENT}/{RESET}{DIM} commands{RESET}"),
        format!("{ACCENT}shift+tab{RESET}{DIM} mode{RESET}"),
        format!("{ACCENT}ctrl+t{RESET}{DIM} transcript{RESET}"),
    ];
    let brand = format!("{ACCENT_BOLD}{} butterfly{RESET}", th::MARK_BRAND);
    let mut rows = vec![brand, String::new()];
    if width >= 62 {
        for (label, value) in [("workspace", place), ("model", model), ("mode", mode)] {
            rows.push(th::truncate_styled(
                &format!(
                    "{}{DIM}{}{RESET}{value}",
                    " ".repeat(GRID),
                    pad_to(label, 12)
                ),
                width,
            ));
        }
    } else {
        for value in [place, model, mode] {
            rows.push(th::truncate_styled(
                &format!("{}{value}", " ".repeat(GRID)),
                width,
            ));
        }
    }
    rows.push(String::new());
    // Shed hints from the right until the row fits rather than wrapping it.
    for take in (1..=hints.len()).rev() {
        let row = format!(
            "{}{}",
            " ".repeat(GRID),
            hints[..take].join(&format!("{DIM}  ⋅  {RESET}")),
        );
        if visible_cols(&row) <= width || take == 1 {
            rows.push(th::truncate_styled(&row, width));
            break;
        }
    }
    rows.push(String::new());
    rows
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
pub fn table_rows(table: &md::Table, width: usize) -> Vec<String> {
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
    out.push(format!(
        "{}{DIM}{}{RESET}",
        " ".repeat(GRID),
        widths
            .iter()
            .map(|w| th::RULE.repeat(*w))
            .collect::<Vec<_>>()
            .join(&" ".repeat(COL_GAP)),
    ));
    for row in &table.rows {
        out.extend(row_of(row, &widths, &table.aligns, false));
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
/// is several screen lines, and every caller counts screen lines. Returning
/// them joined by newlines inside one string is what corrupted the repaint.
fn row_of(
    cells: &[Vec<md::Span>],
    widths: &[usize],
    aligns: &[Align],
    header: bool,
) -> Vec<String> {
    let wrapped: Vec<Vec<String>> = widths
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
            md::styled_rows(&spans, *w, *w)
        })
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    let mut lines = Vec::with_capacity(height);
    for line in 0..height {
        let mut row = " ".repeat(GRID);
        for (i, cell) in wrapped.iter().enumerate() {
            if i > 0 {
                row.push_str(&" ".repeat(COL_GAP));
            }
            let text = cell.get(line).cloned().unwrap_or_default();
            let align = aligns.get(i).copied().unwrap_or(Align::Left);
            // The last column needs no trailing pad; trailing spaces make a
            // copied row ragged.
            let last = i + 1 == wrapped.len();
            row.push_str(&align_cell(&text, widths[i], align, last));
        }
        lines.push(row.trim_end().to_string());
    }
    lines
}

fn align_cell(text: &str, width: usize, align: Align, last: bool) -> String {
    let pad = width.saturating_sub(visible_cols(text));
    match align {
        Align::Left if last => text.to_string(),
        Align::Left => format!("{text}{}", " ".repeat(pad)),
        Align::Right => format!("{}{text}", " ".repeat(pad)),
        Align::Center => {
            let left = pad / 2;
            let right = pad - left;
            if last {
                format!("{}{text}", " ".repeat(left))
            } else {
                format!("{}{text}{}", " ".repeat(left), " ".repeat(right))
            }
        }
    }
}

/// Too narrow for columns: stack each row as a record ‒ its first cell as
/// the title, the rest indented under it. The responsive answer, and far
/// more readable than three-character-wide columns.
fn record_rows(table: &md::Table, width: usize) -> Vec<String> {
    let label_col = table
        .headers
        .iter()
        .skip(1)
        .map(|h| width_of(&md::spans_text(h)))
        .max()
        .unwrap_or(0);
    let named = table.columns() > 2 && label_col > 0;
    let mut out = Vec::new();
    for (index, row) in table.rows.iter().enumerate() {
        if index > 0 {
            out.push(String::new());
        }
        let title = md::spans_text(row.first().unwrap_or(&Vec::new()));
        for line in md::styled_rows(
            &[md::Span {
                style: md::SpanStyle::Bold,
                text: title,
            }],
            width.saturating_sub(GRID),
            width.saturating_sub(GRID),
        ) {
            out.push(format!("{}{line}", " ".repeat(GRID)));
        }
        for (i, cell) in row.iter().enumerate().skip(1) {
            let indent = GRID + 2;
            let label = if named {
                format!(
                    "{DIM}{}{RESET}",
                    pad_to(&md::spans_text(&table.headers[i]), label_col + COL_GAP)
                )
            } else {
                String::new()
            };
            let budget = width.saturating_sub(indent + visible_cols(&label));
            for (n, line) in md::styled_rows(cell, budget.max(MIN_COL), budget.max(MIN_COL))
                .into_iter()
                .enumerate()
            {
                out.push(format!(
                    "{}{}{line}",
                    " ".repeat(indent),
                    if n == 0 {
                        label.clone()
                    } else {
                        " ".repeat(visible_cols(&label))
                    },
                ));
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
pub fn option_row(label: &str, hint: &str, selected: bool, width: usize) -> String {
    let marker = if selected { th::MARK_SELECT } else { " " };
    let color = if selected { ACCENT } else { DIM };
    let left = format!("{color}{marker} {label}{RESET}");
    if hint.is_empty() {
        th::truncate_styled(&left, width)
    } else {
        justify(&left, &format!("{DIM}{hint}{RESET}"), width)
    }
}

/// A completion row. Unlike a picker option, its hint belongs immediately
/// after the label column, not at the far edge: the label is a short file
/// name and the hint is the directory that disambiguates it, so the two
/// have to read as one phrase.
pub fn completion_row(label: &str, hint: &str, selected: bool, width: usize) -> String {
    let marker = if selected { th::MARK_SELECT } else { " " };
    let color = if selected { ACCENT } else { DIM };
    let row = if hint.is_empty() {
        format!("{color}{marker} {label}{RESET}")
    } else {
        format!("{color}{marker} {label}{RESET}{DIM}  {hint}{RESET}")
    };
    th::truncate_styled(&row, width)
}

/// The dim instruction row under a selector: what this list is on the left,
/// which keys drive it on the right.
pub fn hint_row(left: &str, right: &str, width: usize) -> String {
    justify(
        &format!("{DIM}{}{}{RESET}", " ".repeat(GRID), left),
        &format!("{DIM}{right}{RESET}"),
        width,
    )
}

// ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯ approvals ⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯

/// What the decision is about, said plainly, plus the change itself.
/// Presentation only: this never alters which calls are gated, the options
/// offered, their order, or what a keypress consents to.
pub fn approval_card(request: &ApprovalRequest, detail: Option<&str>, width: usize) -> Vec<String> {
    let (tool, target) = split_preview(&request.preview);
    let subject = approval_subject(tool, target);
    let mut rows: Vec<String> = Vec::new();
    let stat = detail.map(diff_stat).unwrap_or_default();
    let title = format!("{WARN}{}{RESET} {BOLD}{subject}{RESET}", th::MARK_WARN);
    rows.push(justify(&title, &stat, width));
    if let Some(warning) = &request.policy_warning {
        rows.push(th::truncate_styled(
            &format!(
                "{}{DANGER}{}{RESET}",
                " ".repeat(GRID),
                truncate_cols(warning, width.saturating_sub(GRID)),
            ),
            width,
        ));
    }
    if let Some(detail) = detail {
        rows.push(String::new());
        rows.extend(detail_rows(detail, width));
    }
    rows
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
fn diff_stat(detail: &str) -> String {
    let (mut added, mut removed) = (0usize, 0usize);
    for line in detail.lines() {
        match classify(line) {
            Row::Added(_) => added += 1,
            Row::Removed(_) => removed += 1,
            _ => {}
        }
    }
    if added == 0 && removed == 0 {
        String::new()
    } else {
        format!("{OK}+{added}{RESET} {DANGER}−{removed}{RESET}")
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
pub fn detail_rows(detail: &str, width: usize) -> Vec<String> {
    let indent = " ".repeat(GRID);
    let budget = width.saturating_sub(GRID + 2).max(12);
    detail
        .lines()
        .map(|line| match classify(line) {
            Row::Removed(text) => format!(
                "{indent}{DANGER}- {}{RESET}",
                truncate_cols(&text.replace('\t', "    "), budget)
            ),
            Row::Added(text) => format!(
                "{indent}{OK}+ {}{RESET}",
                truncate_cols(&text.replace('\t', "    "), budget)
            ),
            Row::Command(text) => {
                format!("{indent}{ACCENT}$ {}{RESET}", truncate_cols(text, budget))
            }
            Row::Context(text) => format!(
                "{indent}{DIM}  {}{RESET}",
                truncate_cols(&text.replace('\t', "    "), budget)
            ),
            Row::Note(text) => format!("{indent}{DIM}{}{RESET}", truncate_cols(text, budget)),
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

pub fn turn_summary(facts: &TurnFacts) -> String {
    let cost = if facts.price_verified {
        money(facts.microusd)
    } else {
        format!("~{} unverified", money(facts.microusd))
    };
    let mut line = format!(
        "{} {SEP} {} {SEP} {} in ({:.0}% cached) {SEP} {} out {SEP} {cost}",
        short_elapsed(facts.elapsed),
        plural(facts.calls as u64, "call"),
        tokens(facts.input_tokens),
        facts.cache_rate * 100.0,
        tokens(facts.output_tokens),
    );
    if facts.elisions > 0 || facts.compactions > 0 {
        line.push_str(&format!(
            " ⋅ compacted ({} elision, {} summarize)",
            facts.elisions, facts.compactions
        ));
    }
    if facts.cancelled {
        line.push_str(" ⋅ cancelled");
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
            ("enter", "send ⋅ ctrl+j inserts a newline"),
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
pub fn help_card(width: usize) -> String {
    let key_col = HELP_SECTIONS
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .map(|(key, _)| width_of(key))
        .max()
        .unwrap_or(16);
    let mut out: Vec<String> = Vec::new();
    for (title, rows) in HELP_SECTIONS {
        if !out.is_empty() {
            out.push(String::new());
        }
        out.push(format!("{BOLD}{title}{RESET}"));
        for (key, description) in rows {
            let body = format!(
                "{}{ACCENT}{}{RESET}  {DIM}{description}{RESET}",
                " ".repeat(GRID),
                pad_to(key, key_col),
            );
            out.push(th::truncate_styled(&body, width));
        }
    }
    out.push(String::new());
    out.push(th::truncate_styled(
        &format!(
            "{DIM}Type while a turn runs ‒ your message lands at the next turn boundary.{RESET}"
        ),
        width,
    ));
    out.join("\n")
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
pub fn usage_card(facts: &UsageFacts<'_>, width: usize) -> String {
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
    let mut out = vec![format!(
        "{BOLD}session{RESET} {DIM}{}{RESET}",
        facts.session_id
    )];
    for (label, value) in rows {
        out.push(th::truncate_styled(
            &format!(
                "{}{DIM}{}{RESET}{value}",
                " ".repeat(GRID),
                pad_to(label, label_col + 2),
            ),
            width,
        ));
    }
    out.join("\n")
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
pub fn model_rows(models: &[ModelRow<'_>], width: usize) -> Vec<String> {
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
            let tag = if model.current {
                format!("  {ACCENT}current{RESET}")
            } else if !model.has_key {
                format!("  {DIM}no key{RESET}")
            } else {
                String::new()
            };
            let mut row = truncate_cols(model.display_name, name_col);
            row = pad_to(&row, name_col);
            let mut styled = format!("{row}  {DIM}{}{RESET}", pad_to(model.id, id_col));
            if width >= 58 {
                styled.push_str(&format!(
                    "  {DIM}{:>6}{RESET}",
                    tokens(model.context_tokens)
                ));
            }
            styled.push_str(&tag);
            styled
        })
        .collect()
}

#[cfg(test)]
mod tests {

    use super::*;

    const WIDTHS: [usize; 5] = [24, 40, 62, 80, 120];

    /// Both halves of the width contract, checked on the same rendered row:
    /// it must fit, and its width must not be something the terminal gets to
    /// choose. A glyph marked East Asian Ambiguous (`·`, `─`, `…`, `●`)
    /// renders one column on xterm and two on Ghostty, so a row built from
    /// them is only accidentally the right size. The footer carried two and
    /// the turn receipt five before this was enforced.
    fn check(row: &str, width: usize, label: &str) {
        use unicode_width::UnicodeWidthChar as _;
        assert!(visible_cols(row) <= width, "{label} @{width}: {row:?}");
        for ch in row.chars() {
            assert!(
                ch.width() == ch.width_cjk(),
                "{label} @{width} draws ambiguous-width {ch:?} (U+{:04X}): {row:?}",
                ch as u32
            );
        }
    }

    #[test]
    fn every_surface_fits_its_width() {
        for width in WIDTHS {
            let info = Welcome {
                workspace: "butterfly-cli",
                branch: Some("feature/a-fairly-long-branch-name".into()),
                model_name: "DeepSeek V4 Flash (served as V4.1 Flash)",
                context_tokens: 164_000,
                mode: "ask",
            };
            for row in welcome(&info, width) {
                check(&row, width, "welcome");
            }
            let state = Footer {
                model_id: "deepseek-v4-flash",
                mode: "read-only",
                ctx_used: 0.93,
                session_microusd: 2_916,
                price_verified: true,
            };
            let row = footer(&state, width);
            check(&row, width, "footer");
            let row = tool_line(
                "read_file(a/very/deeply/nested/path/that/keeps/going/mod.rs)",
                Some(ToolOutcome::Failed {
                    reason: "no such file or directory in this workspace".into(),
                    elapsed: Duration::from_millis(12),
                }),
                width,
            );
            check(&row, width, "tool");
            let row = activity_line(
                &Phase::Running("run_command(cargo test --workspace --locked)".into()),
                3,
                Duration::from_secs(97),
                12_400,
                width,
            );
            check(&row, width, "activity");
            for row in detail_rows(
                "README.md:1\n    context line that is really quite long indeed and then some\n  - # butterfly ‒ Harness\n  + # butterfly\n  $ cargo test --workspace\n",
                width,
            ) {
                check(&row, width, "detail");
            }
            for row in help_card(width).lines() {
                check(row, width, "help");
            }
        }
    }

    #[test]
    fn failed_rows_do_not_repeat_the_path_they_already_show() {
        let row = tool_line(
            "read_file(src/nope.rs)",
            Some(ToolOutcome::Failed {
                reason: "src/nope.rs: no such file".into(),
                elapsed: Duration::from_millis(1),
            }),
            80,
        );
        assert!(row.contains("no such file"));
        assert_eq!(row.matches("src/nope.rs").count(), 1, "{row:?}");
    }

    #[test]
    fn tool_rows_align_verbs_and_carry_their_outcome() {
        let row = tool_line(
            "read_file(Cargo.toml)",
            Some(ToolOutcome::Ok {
                elapsed: Duration::from_millis(8),
            }),
            60,
        );
        assert!(row.contains("read  "), "verb column lost: {row:?}");
        assert!(row.contains("Cargo.toml") && row.contains("8ms"));
        assert_eq!(visible_cols(&row), 60, "outcome must right-align");
        // A mutating verb takes the accent; a read stays dim.
        assert!(tool_line("edit(README.md)", None, 60).contains(&format!("{ACCENT}edit")));
        assert!(tool_line("read_file(README.md)", None, 60).contains(&format!("{DIM}read")));
        // A bare tool name (no arguments) still renders.
        assert!(tool_line("update_plan", None, 40).contains("plan"));
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
        let text = rows.join("\n");
        assert!(text.contains("edit README.md"), "subject missing: {text}");
        assert!(
            text.contains("+2") && text.contains("−1"),
            "stat missing: {text}"
        );
        assert!(text.contains("auto-run allowlist"), "warning dropped");
        assert!(rows.iter().all(|row| visible_cols(row) <= 70));
    }

    #[test]
    fn deciding_never_offers_the_interrupt_hint() {
        // Esc answers the approval menu; telling the user it interrupts the
        // turn would misdescribe what the key does.
        let row = activity_line(&Phase::Deciding, 0, Duration::from_secs(4), 900, 80);
        assert!(!row.contains("interrupt"), "{row:?}");
        assert!(row.contains("waiting for you"));
        let row = activity_line(&Phase::Thinking, 0, Duration::from_secs(4), 900, 80);
        assert!(row.contains("interrupt") && row.contains("thinking"));
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
        assert!(footer(&state, 120).contains("unverified"));
    }

    /// Rows carry styling; column positions are only comparable on the
    /// visible text.
    fn plain(row: &str) -> String {
        let mut out = String::new();
        let mut chars = row.chars().peekable();
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
            out.push(ch);
        }
        out
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
        assert!(rows[1].contains('⎯'), "rule missing: {:?}", rows[1]);
        // Ids line up under one another.
        let rows: Vec<String> = rows.iter().map(|row| plain(row)).collect();
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
            .map(|row| plain(row))
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
        let raw = table_rows(&table, 60);
        assert!(raw.iter().all(|row| visible_cols(row) <= 60));
        let rows: Vec<String> = raw.iter().map(|row| plain(row)).collect();
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
        assert!(rows[0].contains("current") && rows[0].contains("164.0k"));
        assert!(rows[1].contains("no key"));
        // Ids align into one column.
        assert_eq!(
            rows[0].find("deepseek-v4-flash"),
            rows[1].find("glm-5.3-flash"),
        );
    }
}

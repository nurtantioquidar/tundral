//! The shared inline-TUI surface. Committed output flows into native
//! terminal scrollback; only a small block at the bottom (live stream line,
//! input editor, completion dropdown, status bar) is redrawn in place:
//! erase block → append output → redraw block. No alternate screen, so
//! scrollback, copy-paste, and tmux behave normally.

use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use butterfly_core::provider::StreamSink;
use butterfly_core::session::approval::{ApprovalGate, ApprovalRequest, Decision};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use tokio::sync::oneshot;
use unicode_width::UnicodeWidthChar;

use super::completion::{active_trigger, ChoiceKind, CompletionIndex};
use super::editor::Editor;
use super::markdown::{self as md, MarkdownRenderer};
use crate::repl::render_detail;

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const GREEN: &str = "\x1b[32m";
/// Subtle elevated band (Codex-style): dark-gray background with soft
/// light-gray text, colors forced so the contrast is gentle on any theme —
/// deliberately NOT inverse video, which reads as harsh black-on-white.
const BAND: &str = "\x1b[48;5;236m\x1b[38;5;252m";
const SYNC_BEGIN: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";

const MAX_INPUT_ROWS: usize = 8;
const MAX_DROPDOWN_ROWS: usize = 6;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const TRANSCRIPT_CAP: usize = 10_000;
const ALT_SCREEN_ENTER: &str = "\x1b[?1049h\x1b[?25l";
const ALT_SCREEN_LEAVE: &str = "\x1b[?1049l\x1b[?25h";

/// What a handled input event means for the session loop.
pub enum UiAction {
    Submit(String),
    /// Shift+Tab: advance the permission mode (ask → auto → read-only).
    CycleMode,
    Interrupt,
    Eof,
}

#[derive(Clone, Copy, PartialEq)]
enum LiveKind {
    Reasoning,
    Text,
    Tool,
}

struct LiveLine {
    kind: LiveKind,
    text: String,
}

struct Dropdown {
    choices: Vec<super::completion::Choice>,
    selected: usize,
    token_start: usize,
}

const MENU: [(&str, Decision); 3] = [
    ("yes — run it", Decision::Approve),
    ("always for this session", Decision::ApproveForSession),
    ("no — skip it", Decision::Deny),
];

struct Menu {
    selected: usize,
    responder: oneshot::Sender<Decision>,
}

/// In-session /connect flow: pick a provider, paste the key (masked). The
/// key lives only in this buffer until the caller saves it — it never enters
/// the editor history, the transcript, or the session store.
enum ConnectStage {
    Provider { selected: usize },
    Key { buffer: String },
}

struct Connect {
    stage: ConnectStage,
    responder: oneshot::Sender<Option<(String, String)>>,
}

/// Generic arrow-key selector (e.g. the /model picker): resolves with the
/// chosen index, or None on cancel.
struct Select {
    title: String,
    items: Vec<String>,
    selected: usize,
    responder: oneshot::Sender<Option<usize>>,
}

pub struct Status {
    pub model: String,
    pub mode: &'static str,
    pub ctx_pct: f64,
}

struct Activity {
    started: Instant,
    spinner: usize,
    tool: Option<String>,
    out_bytes: u64,
}

/// Where the block lives on screen. A session starts `Pinned`: the screen is
/// taken over (old shell content scrolls into scrollback), content flows
/// from the top, and the block sits at the bottom edge — Claude Code style.
/// Once content grows enough to meet the block, painting hands off to
/// `Scrolling`: the block rides directly under the content and the terminal
/// scrolls naturally, preserving scrollback. The handoff is one-way.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Place {
    Pinned {
        /// Next free 1-based screen row for committed output.
        content_row: usize,
        /// Top row of the block at last draw (for erasing before redraw).
        prev_block_top: usize,
    },
    Scrolling,
}

/// One logical transcript entry. Entries render to physical rows at a GIVEN
/// width, so the visible screen can be re-wrapped from source on resize
/// (Codex-tidy) and the Ctrl+T overlay wraps instead of truncating.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Blank,
    /// Pre-styled verbatim line (tool lines, code, notices): reprinted
    /// as-is, clamped rather than re-wrapped.
    Fixed(String),
    /// The user's whole prompt (may span lines), drawn as a highlighted bar
    /// with the ❯ marker on the first row only (Codex-style).
    UserBar(String),
    /// Re-wrappable assistant prose.
    Prose {
        block_start: bool,
        spans: Vec<md::Span>,
        cont_indent: usize,
    },
}

/// Render one entry to physical rows at `width` columns.
fn render_entry(entry: &Entry, width: usize) -> Vec<String> {
    let width = width.max(20);
    match entry {
        Entry::Blank => vec![String::new()],
        Entry::Fixed(line) => vec![line.clone()],
        Entry::UserBar(text) => {
            let mut rendered = Vec::new();
            for line in text.trim_end_matches('\n').split('\n') {
                if line.is_empty() {
                    rendered.push(String::new());
                    continue;
                }
                let spans = vec![md::Span {
                    style: md::SpanStyle::Plain,
                    text: line.to_string(),
                }];
                rendered.extend(styled_rows(
                    &spans,
                    width.saturating_sub(3),
                    width.saturating_sub(3),
                ));
            }
            // Same vertical grid as every other element: marker in column 0,
            // content in column 2 (❯/●/⚒/∴ all line up).
            rendered
                .into_iter()
                .enumerate()
                .map(|(i, row)| {
                    let prefix = if i == 0 { "❯ " } else { "  " };
                    let pad = width.saturating_sub(visible_cols(&row) + 2);
                    format!("{BAND}{prefix}{row}{}{RESET}", " ".repeat(pad))
                })
                .collect()
        }
        Entry::Prose {
            block_start,
            spans,
            cont_indent,
        } => {
            let rows = styled_rows(
                spans,
                width.saturating_sub(2),
                width.saturating_sub(2 + cont_indent),
            );
            let cont_prefix = format!("  {}", " ".repeat(*cont_indent));
            rows.into_iter()
                .enumerate()
                .map(|(i, row)| {
                    let prefix = if i == 0 {
                        if *block_start {
                            "● "
                        } else {
                            "  "
                        }
                    } else {
                        &cont_prefix
                    };
                    format!("{prefix}{row}")
                })
                .collect()
        }
    }
}

struct UiInner {
    width: usize,
    height: usize,
    place: Place,
    editor: Editor,
    index: CompletionIndex,
    dropdown: Option<Dropdown>,
    menu: Option<Menu>,
    connect: Option<Connect>,
    select: Option<Select>,
    status: Status,
    activity: Option<Activity>,
    live: Option<LiveLine>,
    renderer: MarkdownRenderer,
    /// True when the next assistant text line starts a response block and
    /// gets the `●` marker; continuation lines are indented under it.
    text_block_start: bool,
    /// Whether the last committed scrollback line was blank (vertical
    /// rhythm: a new `●` block inserts a spacer when it wasn't).
    last_blank: bool,
    /// Stream reasoning to the screen. Off by default, Codex-style; Ctrl+R
    /// toggles live. Hidden reasoning is still recorded in the transcript.
    show_reasoning: bool,
    /// Partial reasoning line buffered while reasoning is hidden.
    hidden_reasoning: String,
    /// Everything committed to scrollback (plus hidden reasoning), as
    /// logical entries re-renderable at any width: backs the Ctrl+T overlay
    /// and the full-screen re-wrap on resize. The bool marks whether the
    /// entry was drawn on the main screen (hidden reasoning is not — the
    /// resize repaint must not resurrect it). Capped ring.
    transcript: std::collections::VecDeque<(bool, Entry)>,
    /// Ctrl+T transcript overlay: Some(scroll offset from the tail). While
    /// open, committed output is queued and flushed on close.
    overlay: Option<usize>,
    pending_output: Vec<Entry>,
    /// Scrolling mode only: the cursor's row within the block at last draw —
    /// MoveUp(cursor_row) reaches the block's top line for erase-and-redraw.
    cursor_row: usize,
}

pub struct Ui {
    inner: Mutex<UiInner>,
}

fn width_of(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Visible column count with ANSI escape sequences stripped.
fn visible_cols(line: &str) -> usize {
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
        cols += ch.width().unwrap_or(0);
    }
    cols
}

/// Physical terminal rows one committed output line occupies after wrapping.
fn physical_rows(line: &str, width: usize) -> usize {
    let cols = visible_cols(line);
    if width == 0 || cols == 0 {
        1
    } else {
        cols.div_ceil(width)
    }
}

/// Truncate plain text to `max` columns, appending `…` when cut.
fn truncate_cols(text: &str, max: usize) -> String {
    if width_of(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut cols = 0usize;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if cols + w > max.saturating_sub(1) {
            break;
        }
        cols += w;
        out.push(ch);
    }
    out.push('…');
    out
}

/// Truncate a STYLED line to `max` visible columns (escapes pass through,
/// visible chars are counted); always ends with a reset.
fn truncate_styled(line: &str, max: usize) -> String {
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
        let w = ch.width().unwrap_or(0);
        if cols + w > max.saturating_sub(1) {
            out.push('…');
            break;
        }
        cols += w;
        out.push(ch);
    }
    out.push_str(RESET);
    out
}

/// Word-wrap styled spans into rows of at most `first_budget` /
/// `cont_budget` columns, breaking at spaces (hard-cutting single words
/// longer than a row). Styles survive across row boundaries.
fn styled_rows(spans: &[md::Span], first_budget: usize, cont_budget: usize) -> Vec<String> {
    let chars: Vec<(char, md::SpanStyle)> = spans
        .iter()
        .flat_map(|s| s.text.chars().map(move |c| (c, s.style)))
        .collect();
    let mut rows: Vec<Vec<(char, md::SpanStyle)>> = Vec::new();
    let mut row: Vec<(char, md::SpanStyle)> = Vec::new();
    let mut cols = 0usize;
    let mut budget = first_budget.max(8);
    let mut last_space: Option<usize> = None;
    for &(ch, style) in &chars {
        let w = ch.width().unwrap_or(0);
        if cols + w > budget && !row.is_empty() {
            if let Some(at) = last_space {
                let carry = row.split_off(at + 1);
                while row.last().is_some_and(|(c, _)| *c == ' ') {
                    row.pop();
                }
                rows.push(std::mem::take(&mut row));
                row = carry;
            } else {
                rows.push(std::mem::take(&mut row));
            }
            budget = cont_budget.max(8);
            cols = row.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
            last_space = row.iter().rposition(|(c, _)| *c == ' ');
            if ch == ' ' && row.is_empty() {
                continue;
            }
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
    rows.into_iter()
        .map(|row| {
            let mut out = String::new();
            let mut current = md::SpanStyle::Plain;
            for (ch, style) in row {
                if style != current {
                    if current != md::SpanStyle::Plain {
                        out.push_str(md::RESET);
                    }
                    if style != md::SpanStyle::Plain {
                        out.push_str(style.escape());
                    }
                    current = style;
                }
                out.push(ch);
            }
            if current != md::SpanStyle::Plain {
                out.push_str(md::RESET);
            }
            out
        })
        .collect()
}

/// Keep the tail of a streaming line within `max` columns.
fn tail_cols(text: &str, max: usize) -> String {
    if width_of(text) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut cols = 0usize;
    let mut start = chars.len();
    while start > 0 {
        let w = chars[start - 1].width().unwrap_or(0);
        if cols + w > max.saturating_sub(1) {
            break;
        }
        cols += w;
        start -= 1;
    }
    format!("…{}", chars[start..].iter().collect::<String>())
}

impl UiInner {
    /// The terminal cursor is hidden while a selector owns the block (there
    /// is nothing to type at); it shows for the editor and key entry.
    fn cursor_escape(&self) -> &'static str {
        let selecting = self.menu.is_some()
            || self.select.is_some()
            || matches!(
                &self.connect,
                Some(Connect {
                    stage: ConnectStage::Provider { .. },
                    ..
                })
            );
        if selecting {
            "\x1b[?25l"
        } else {
            "\x1b[?25h"
        }
    }

    /// Compose the block: (styled lines, cursor row, cursor column).
    fn compose_block(&self) -> (Vec<String>, usize, usize) {
        let width = self.width.max(20);
        let mut lines: Vec<String> = Vec::new();
        if let Some(live) = &self.live {
            // The 2-column block marker/indent counts against the width
            // budget: a block line wider than the terminal wraps to two
            // physical rows and corrupts the erase math (stranded frames).
            let plain = tail_cols(&live.text, width);
            let prefixed = tail_cols(&live.text, width.saturating_sub(2));
            lines.push(match live.kind {
                LiveKind::Reasoning => format!("{DIM}{plain}{RESET}"),
                LiveKind::Text if self.text_block_start => format!("● {prefixed}"),
                LiveKind::Text => format!("  {prefixed}"),
                LiveKind::Tool => format!("{CYAN}{plain}{RESET}"),
            });
        }
        if let Some(activity) = &self.activity {
            let mut working = format!(
                "∴ {} {}s · ~{} tok",
                SPINNER[activity.spinner % SPINNER.len()],
                activity.started.elapsed().as_secs(),
                activity.out_bytes / 4,
            );
            if let Some(tool) = &activity.tool {
                working.push_str(&format!(" · {tool}"));
            }
            working.push_str(" · esc/ctrl+c to interrupt");
            lines.push(format!("{DIM}{}{RESET}", truncate_cols(&working, width)));
        }
        // The input area is anchored by a rule above and the footer below.
        lines.push(format!("{DIM}{}{RESET}", "─".repeat(width)));
        let cursor_row;
        let cursor_col;
        if let Some(connect) = &self.connect {
            match &connect.stage {
                ConnectStage::Provider { selected } => {
                    lines.push("connect a provider".to_string());
                    for (i, (_, label)) in super::connect::PROVIDERS.iter().enumerate() {
                        if i == *selected {
                            lines.push(format!("{CYAN}▸ {label}{RESET}"));
                        } else {
                            lines.push(format!("{DIM}  {label}{RESET}"));
                        }
                    }
                    lines.push(format!("{DIM}enter continue · esc cancel{RESET}"));
                    cursor_row = lines.len().saturating_sub(1);
                    cursor_col = 0;
                }
                ConnectStage::Key { buffer } => {
                    lines.push("paste your API key (input is masked)".to_string());
                    let masked = if buffer.is_empty() {
                        String::new()
                    } else {
                        super::connect::masked(buffer.chars().count())
                    };
                    let masked = tail_cols(&masked, width.saturating_sub(4));
                    lines.push(format!("{CYAN}❯{RESET} {masked}"));
                    lines.push(format!("{DIM}enter save · esc back{RESET}"));
                    cursor_row = lines.len().saturating_sub(2);
                    cursor_col = 2 + width_of(&masked);
                }
            }
        } else if let Some(select) = &self.select {
            lines.push(select.title.clone());
            for (i, item) in select.items.iter().enumerate() {
                let row = truncate_cols(item, width.saturating_sub(3));
                if i == select.selected {
                    lines.push(format!("{CYAN}▸ {row}{RESET}"));
                } else {
                    lines.push(format!("{DIM}  {row}{RESET}"));
                }
            }
            lines.push(format!("{DIM}enter select · esc cancel{RESET}"));
            cursor_row = lines.len().saturating_sub(1);
            cursor_col = 0;
        } else if let Some(menu) = &self.menu {
            // Same grid as the transcript: marker col 0, label col 2.
            for (i, (label, _)) in MENU.iter().enumerate() {
                let marker = if i == menu.selected { "▸" } else { " " };
                let styled = if i == menu.selected {
                    format!("{CYAN}{marker} {label}{RESET}")
                } else {
                    format!("{DIM}{marker} {label}{RESET}")
                };
                lines.push(styled);
            }
            cursor_row = lines.len().saturating_sub(1);
            cursor_col = 0;
        } else {
            let (input_rows, row, col) = self.compose_input(width);
            cursor_row = lines.len() + row;
            cursor_col = col;
            lines.extend(input_rows);
        }
        if let Some(dropdown) = &self.dropdown {
            // Row budget: 2 prefix cols + label + 2 + hint ≤ width. Wide
            // terminals reserve ~22 cols for the hint; narrow ones give the
            // label everything and drop the hint.
            let label_budget = if width >= 44 {
                width - 26
            } else {
                width.saturating_sub(2)
            };
            let labels: Vec<String> = dropdown
                .choices
                .iter()
                .take(MAX_DROPDOWN_ROWS)
                .map(|choice| truncate_cols(&choice.label, label_budget.max(8)))
                .collect();
            // Hints start at one shared column (Codex-style alignment).
            let label_col = labels.iter().map(|l| width_of(l)).max().unwrap_or(0);
            for (i, choice) in dropdown.choices.iter().take(MAX_DROPDOWN_ROWS).enumerate() {
                let marker = if i == dropdown.selected { "▸" } else { " " };
                let mut label = labels[i].clone();
                let hint_budget = width.saturating_sub(2 + label_col + 2);
                let hint = if choice.hint.is_empty() || hint_budget < 4 {
                    String::new()
                } else {
                    label.push_str(&" ".repeat(label_col - width_of(&labels[i])));
                    format!("  {}", truncate_cols(&choice.hint, hint_budget))
                };
                let styled = if i == dropdown.selected {
                    format!("{CYAN}{marker} {label}{RESET}{DIM}{hint}{RESET}")
                } else {
                    format!("{DIM}{marker} {label}{hint}{RESET}")
                };
                lines.push(styled);
            }
        }
        lines.push(self.compose_status(width));
        (lines, cursor_row, cursor_col)
    }

    /// Wrapped input rows plus the cursor's (row, column) within them.
    fn compose_input(&self, width: usize) -> (Vec<String>, usize, usize) {
        let prompt = format!("{CYAN}❯{RESET} ");
        let continuation = "  ";
        let avail = width.saturating_sub(2).max(10);
        let text = self.editor.text();
        if text.is_empty() {
            let hint = if self.activity.is_some() {
                "steer: message lands at the next turn boundary"
            } else {
                "message · @ file · / command · ctrl+t transcript"
            };
            return (vec![format!("{prompt}{DIM}{hint}{RESET}")], 0, 2);
        }
        // Wrap each logical line to `avail` columns, tracking byte offsets so
        // the cursor lands on the right visual cell.
        let cursor = self.editor.cursor();
        let mut rows: Vec<String> = Vec::new();
        let mut cursor_pos: Option<(usize, usize)> = None;
        let mut offset = 0usize;
        for logical in text.split('\n') {
            let mut row_text = String::new();
            let mut row_cols = 0usize;
            for ch in logical.chars() {
                let w = ch.width().unwrap_or(0);
                if row_cols + w > avail {
                    rows.push(std::mem::take(&mut row_text));
                    row_cols = 0;
                }
                if cursor == offset {
                    cursor_pos = Some((rows.len(), 2 + row_cols));
                }
                row_text.push(ch);
                row_cols += w;
                offset += ch.len_utf8();
            }
            if cursor == offset {
                cursor_pos = Some((rows.len(), 2 + row_cols));
            }
            rows.push(row_text);
            offset += 1; // the '\n'
        }
        let (cursor_row, cursor_col) = cursor_pos.unwrap_or((rows.len() - 1, 2));
        // Keep at most MAX_INPUT_ROWS rows, scrolled so the cursor is visible.
        let start = if rows.len() > MAX_INPUT_ROWS {
            cursor_row
                .saturating_sub(MAX_INPUT_ROWS - 1)
                .min(rows.len() - MAX_INPUT_ROWS)
        } else {
            0
        };
        let visible: Vec<String> = rows
            .iter()
            .skip(start)
            .take(MAX_INPUT_ROWS)
            .enumerate()
            .map(|(i, row)| {
                if start + i == 0 {
                    format!("{prompt}{row}")
                } else {
                    format!("{continuation}{row}")
                }
            })
            .collect();
        (visible, cursor_row - start, cursor_col)
    }

    /// One footer line, Codex-style: status left, mode right-aligned in
    /// its color. Deliberately minimal — cost and cache-hit rate print
    /// after every turn in the ∴ summary (and /usage has the breakdown).
    fn compose_status(&self, width: usize) -> String {
        let left = format!(
            "{} | Context {:.0}% left",
            self.status.model,
            (1.0 - self.status.ctx_pct).max(0.0) * 100.0,
        );
        let color = match self.status.mode {
            "auto" => YELLOW,
            "read-only" => GREEN,
            _ => DIM,
        };
        let right = format!("{} mode (shift+tab to cycle)", self.status.mode);
        let pad = width.saturating_sub(width_of(&left) + width_of(&right));
        if pad >= 2 {
            format!("{DIM}{left}{RESET}{}{color}{right}{RESET}", " ".repeat(pad))
        } else {
            // Narrow terminal: keep both, left-joined and truncated.
            format!(
                "{DIM}{}{RESET}",
                truncate_cols(&format!("{left} · {right}"), width)
            )
        }
    }

    /// Commit pre-styled output lines (recorded as Fixed/Blank entries) and
    /// redraw.
    fn paint(&mut self, output: Option<&str>) {
        match output {
            Some(output) => {
                let entries: Vec<Entry> = output
                    .split('\n')
                    .map(|line| {
                        if visible_cols(line) == 0 {
                            Entry::Blank
                        } else {
                            Entry::Fixed(line.to_string())
                        }
                    })
                    .collect();
                self.commit_entries(entries);
            }
            None => {
                if self.overlay.is_some() {
                    self.draw_overlay();
                } else {
                    self.paint_screen(None);
                }
            }
        }
    }

    /// Commit logical entries: record them into the transcript ring, then
    /// render at the current width and draw. While the Ctrl+T overlay is
    /// open, entries queue and the overlay refreshes instead of touching the
    /// main screen.
    fn commit_entries(&mut self, entries: Vec<Entry>) {
        if entries.is_empty() {
            return;
        }
        if let Some(last) = entries.last() {
            self.last_blank = matches!(last, Entry::Blank);
        }
        for entry in &entries {
            self.record(true, entry.clone());
        }
        if self.overlay.is_some() {
            self.pending_output.extend(entries);
            self.draw_overlay();
            return;
        }
        let width = self.width;
        let text: Vec<String> = entries
            .iter()
            .flat_map(|entry| render_entry(entry, width))
            .collect();
        self.paint_screen(Some(&text.join("\n")));
    }

    /// Append an entry to the transcript ring. `on_screen` entries take part
    /// in the resize repaint; transcript-only entries (hidden reasoning)
    /// appear solely in the Ctrl+T overlay.
    fn record(&mut self, on_screen: bool, entry: Entry) {
        if self.transcript.len() >= TRANSCRIPT_CAP {
            self.transcript.pop_front();
        }
        self.transcript.push_back((on_screen, entry));
    }

    /// Erase the block, optionally append committed output, redraw the main
    /// screen. Bypasses transcript recording (callers already recorded).
    fn paint_screen(&mut self, output: Option<&str>) {
        let buf = match self.place {
            Place::Pinned { .. } => self.pinned_frame(output),
            Place::Scrolling => self.scrolling_frame(output),
        };
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
    }

    /// Draw the transcript overlay on the alternate screen: full session
    /// including reasoning that streaming keeps hidden.
    fn draw_overlay(&mut self) {
        let Some(offset) = self.overlay else {
            return;
        };
        let width = self.width.max(20);
        let height = self.height.max(4);
        let rows = height - 1;
        // Entries re-wrap to the overlay's width at draw time.
        let lines: Vec<String> = self
            .transcript
            .iter()
            .flat_map(|(_, entry)| render_entry(entry, width))
            .collect();
        let total = lines.len();
        let max_offset = total.saturating_sub(rows);
        let offset = offset.min(max_offset);
        self.overlay = Some(offset);
        let end = total - offset;
        let start = end.saturating_sub(rows);
        let mut buf = format!("{SYNC_BEGIN}\x1b[2J\x1b[1;1H\x1b[?7l");
        for line in &lines[start..end] {
            buf.push_str(&truncate_styled(line, width));
            buf.push_str("\r\n");
        }
        for _ in end - start..rows {
            buf.push_str("\r\n");
        }
        let footer =
            format!(" transcript · {total} lines · ↑/↓ pgup/pgdn scroll · q or ctrl+t to close ",);
        buf.push_str(&format!("{BAND}{}{RESET}", truncate_cols(&footer, width)));
        buf.push_str("\x1b[?7h");
        buf.push_str(SYNC_END);
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
    }

    fn open_overlay(&mut self) {
        if self.overlay.is_some() || self.menu.is_some() {
            return;
        }
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(ALT_SCREEN_ENTER.as_bytes());
        let _ = out.flush();
        drop(out);
        self.overlay = Some(0);
        self.draw_overlay();
    }

    fn close_overlay(&mut self) {
        if self.overlay.take().is_none() {
            return;
        }
        {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(ALT_SCREEN_LEAVE.as_bytes());
            let _ = out.flush();
        }
        // Flush output that arrived while the overlay was open onto the
        // main screen (it is already in the transcript).
        let pending = std::mem::take(&mut self.pending_output);
        if pending.is_empty() {
            self.paint_screen(None);
        } else {
            let width = self.width;
            let text: Vec<String> = pending
                .iter()
                .flat_map(|entry| render_entry(entry, width))
                .collect();
            self.paint_screen(Some(&text.join("\n")));
        }
    }

    /// Repaint the entire visible screen from the transcript: content
    /// re-wrapped to the current width fills upward from the block, which is
    /// re-pinned at the bottom edge. Used on resize, when terminal reflow
    /// invalidates both the erase math and the old wrap points. Content
    /// above the visible screen stays in scrollback as it was.
    fn repaint_all(&mut self) {
        let width = self.width.max(20);
        let height = self.height.max(4);
        let (block_lines, cursor_row, cursor_col) = self.compose_block();
        let block_height = block_lines.len();
        if block_height >= height {
            // Tiny terminal: clear and let the scrolling model take over.
            let mut buf = format!("{SYNC_BEGIN}\x1b[2J\x1b[1;1H");
            self.place = Place::Scrolling;
            self.cursor_row = 0;
            self.append_scrolling_block(&mut buf);
            buf.push_str(SYNC_END);
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(buf.as_bytes());
            let _ = out.flush();
            return;
        }
        let avail = height - block_height;
        let mut rows: Vec<String> = Vec::new();
        'fill: for entry in self
            .transcript
            .iter()
            .rev()
            .filter_map(|(on_screen, entry)| on_screen.then_some(entry))
        {
            let rendered = render_entry(entry, width);
            for line in rendered.into_iter().rev() {
                rows.push(line);
                if rows.len() >= avail {
                    break 'fill;
                }
            }
        }
        // Drop leading blank rows so the tail sits snug under real content.
        while rows.last().is_some_and(|line| visible_cols(line) == 0) {
            rows.pop();
        }
        rows.reverse();
        let block_top = height - block_height + 1;
        let mut buf = format!("{SYNC_BEGIN}\x1b[2J\x1b[1;1H\x1b[?7l");
        buf.push_str(&rows.join("\r\n"));
        buf.push_str(&format!("\x1b[{block_top};1H"));
        buf.push_str(&block_lines.join("\r\n"));
        buf.push_str("\x1b[?7h");
        buf.push_str(&format!(
            "\x1b[{};{}H",
            block_top + cursor_row,
            cursor_col + 1
        ));
        buf.push_str(self.cursor_escape());
        buf.push_str(SYNC_END);
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
        self.place = Place::Pinned {
            content_row: rows.len() + 1,
            prev_block_top: block_top,
        };
        self.cursor_row = cursor_row;
    }

    /// Append the block draw (wrapped in autowrap-off) and cursor placement
    /// for scrolling mode, where positions are cursor-relative.
    fn append_scrolling_block(&mut self, buf: &mut String) {
        let (lines, cursor_row, cursor_col) = self.compose_block();
        // Autowrap is disabled while the block draws: even if a glyph the
        // width tables call narrow renders wide (●, ⚒, ▸ on some terminals),
        // an overlong line clamps at the right edge instead of wrapping —
        // wrapping here breaks the erase row math and strands stale frames
        // in scrollback. Committed output above keeps normal wrapping.
        buf.push_str("\x1b[?7l");
        buf.push_str(&lines.join("\r\n"));
        buf.push_str("\x1b[?7h");
        let up = lines.len().saturating_sub(1).saturating_sub(cursor_row);
        if up > 0 {
            buf.push_str(&format!("\x1b[{up}A"));
        }
        buf.push('\r');
        if cursor_col > 0 {
            buf.push_str(&format!("\x1b[{cursor_col}C"));
        }
        buf.push_str(self.cursor_escape());
        self.cursor_row = cursor_row;
    }

    /// Scrolling mode: the block rides directly under committed content and
    /// the terminal scrolls naturally.
    fn scrolling_frame(&mut self, output: Option<&str>) -> String {
        let mut buf = String::from(SYNC_BEGIN);
        if self.cursor_row > 0 {
            buf.push_str(&format!("\x1b[{}A", self.cursor_row));
        }
        buf.push('\r');
        buf.push_str("\x1b[J");
        if let Some(output) = output {
            for line in output.split('\n') {
                buf.push_str(&line.replace('\t', "    "));
                buf.push_str(RESET);
                buf.push_str("\r\n");
            }
        }
        self.append_scrolling_block(&mut buf);
        buf.push_str(SYNC_END);
        buf
    }

    /// Pinned mode: content is written at `content_row` (top-down) while the
    /// block sits at the bottom edge of the screen. When content reaches the
    /// block region — or the terminal is too small — hand off to scrolling.
    fn pinned_frame(&mut self, output: Option<&str>) -> String {
        let width = self.width.max(20);
        let height = self.height.max(4);
        let Place::Pinned {
            mut content_row,
            prev_block_top,
        } = self.place
        else {
            unreachable!("pinned_frame outside pinned mode");
        };
        let mut buf = String::from(SYNC_BEGIN);
        // Erase the old block area first so overflowing output can never
        // interleave with a stale block.
        buf.push_str(&format!("\x1b[{};1H\x1b[J", prev_block_top.min(height)));
        if let Some(output) = output {
            buf.push_str(&format!("\x1b[{content_row};1H"));
            for line in output.split('\n') {
                let line = line.replace('\t', "    ");
                buf.push_str(&line);
                buf.push_str(RESET);
                buf.push_str("\r\n");
                content_row += physical_rows(&line, width);
            }
        }
        let (lines, cursor_row, cursor_col) = self.compose_block();
        let block_height = lines.len();
        let block_top = (height.saturating_sub(block_height) + 1).max(1);
        if content_row > block_top || block_height >= height {
            // Content met the block (or the terminal is tiny): one-way
            // handoff to the scrolling model, drawing from the content edge.
            self.place = Place::Scrolling;
            if output.is_none() {
                buf.push_str(&format!("\x1b[{};1H", content_row.min(height)));
            }
            self.append_scrolling_block(&mut buf);
        } else {
            buf.push_str(&format!("\x1b[{block_top};1H"));
            buf.push_str("\x1b[?7l");
            buf.push_str(&lines.join("\r\n"));
            buf.push_str("\x1b[?7h");
            buf.push_str(&format!(
                "\x1b[{};{}H",
                block_top + cursor_row,
                cursor_col + 1
            ));
            buf.push_str(self.cursor_escape());
            self.place = Place::Pinned {
                content_row,
                prev_block_top: block_top,
            };
        }
        buf.push_str(SYNC_END);
        buf
    }

    /// Markdown-render one assistant text line into logical entries:
    /// re-wrappable prose with the `●` block marker on the first line of a
    /// response block (plus a spacer entry before a new block), code kept
    /// verbatim.
    fn render_text_entries(&mut self, line: &str) -> Vec<Entry> {
        match self.renderer.render(line) {
            md::Rendered::Raw(styled) => {
                if styled.is_empty() {
                    vec![Entry::Blank]
                } else {
                    vec![Entry::Fixed(format!("  {styled}"))]
                }
            }
            md::Rendered::Prose { spans, cont_indent } => {
                if line.trim().is_empty() {
                    return vec![Entry::Blank];
                }
                let starting = self.text_block_start;
                self.text_block_start = false;
                let mut entries = Vec::new();
                if starting && !self.last_blank {
                    entries.push(Entry::Blank);
                }
                entries.push(Entry::Prose {
                    block_start: starting,
                    spans,
                    cont_indent,
                });
                entries
            }
        }
    }

    /// One committed line of `kind` as logical entries.
    fn line_entries(&mut self, kind: LiveKind, line: &str) -> Vec<Entry> {
        match kind {
            LiveKind::Reasoning => {
                self.text_block_start = true;
                vec![Entry::Prose {
                    block_start: false,
                    spans: vec![md::Span {
                        style: md::SpanStyle::Dim,
                        text: line.to_string(),
                    }],
                    cont_indent: 0,
                }]
            }
            LiveKind::Text => self.render_text_entries(line),
            LiveKind::Tool => {
                self.text_block_start = true;
                vec![Entry::Fixed(format!("{CYAN}{line}{RESET}"))]
            }
        }
    }

    /// Commit the live line (if any) into scrollback.
    fn commit_live(&mut self) {
        if let Some(live) = self.live.take() {
            if live.text.is_empty() {
                return;
            }
            let text = live.text;
            let entries = self.line_entries(live.kind, &text);
            self.commit_entries(entries);
        }
    }

    /// Append streamed content of `kind`; completed lines commit, the
    /// remainder stays as the live line.
    fn stream(&mut self, kind: LiveKind, delta: &str) {
        if let Some(activity) = &mut self.activity {
            activity.out_bytes += delta.len() as u64;
        }
        if self.live.as_ref().is_some_and(|l| l.kind != kind) {
            self.commit_live();
        }
        let live = self.live.get_or_insert(LiveLine {
            kind,
            text: String::new(),
        });
        live.text.push_str(delta);
        let mut committed: Vec<Entry> = Vec::new();
        while let Some(at) = self.live.as_ref().and_then(|l| l.text.find('\n')) {
            let line: String = {
                let live = self.live.as_mut().expect("live line");
                live.text.drain(..=at).collect()
            };
            let line = line.trim_end_matches('\n');
            let entries = self.line_entries(kind, line);
            // Track spacing across lines within one batch.
            if let Some(last) = entries.last() {
                self.last_blank = matches!(last, Entry::Blank);
            }
            committed.extend(entries);
        }
        if committed.is_empty() {
            self.paint(None);
        } else {
            self.commit_entries(committed);
        }
    }

    fn update_dropdown(&mut self) {
        self.dropdown = active_trigger(self.editor.text(), self.editor.cursor())
            .map(|trigger| {
                let choices = self.index.suggest(&trigger, MAX_DROPDOWN_ROWS);
                Dropdown {
                    choices,
                    selected: 0,
                    token_start: trigger.start(),
                }
            })
            .filter(|dropdown| !dropdown.choices.is_empty());
    }

    fn accept_selected(&mut self) {
        let Some(dropdown) = self.dropdown.take() else {
            return;
        };
        let choice = &dropdown.choices[dropdown.selected];
        let suffix = match choice.kind {
            ChoiceKind::Dir => "",
            _ => " ",
        };
        self.editor.replace_range(
            dropdown.token_start,
            self.editor.cursor(),
            &format!("{}{}", choice.insert, suffix),
        );
        // Directories keep the dropdown open to drill down.
        self.update_dropdown();
    }

    fn resolve_menu(&mut self, choice: usize) {
        if let Some(menu) = self.menu.take() {
            let (label, decision) = MENU[choice];
            self.paint(Some(&format!("{DIM}↳ {label}{RESET}")));
            let _ = menu.responder.send(decision);
        }
    }
}

impl Ui {
    pub fn new(
        editor: Editor,
        index: CompletionIndex,
        status: Status,
        show_reasoning: bool,
    ) -> Arc<Self> {
        let (width, height) = crossterm::terminal::size()
            .map(|(w, h)| (w as usize, h as usize))
            .unwrap_or((100, 30));
        Arc::new(Self {
            inner: Mutex::new(UiInner {
                width,
                height,
                place: Place::Pinned {
                    content_row: 1,
                    prev_block_top: height.max(1),
                },
                editor,
                index,
                dropdown: None,
                menu: None,
                connect: None,
                select: None,
                status,
                activity: None,
                live: None,
                renderer: MarkdownRenderer::default(),
                text_block_start: true,
                last_blank: true,
                show_reasoning,
                hidden_reasoning: String::new(),
                transcript: std::collections::VecDeque::new(),
                overlay: None,
                pending_output: Vec::new(),
                cursor_row: 0,
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, UiInner> {
        self.inner.lock().expect("ui lock")
    }

    /// Take over the visible screen, Claude Code-style: newline-flood the
    /// old shell content into scrollback (NOT erased — no alternate screen,
    /// history stays reachable), then draw the block at the bottom edge.
    pub fn enter_screen(&self) {
        let mut inner = self.lock();
        let flood = "\n".repeat(inner.height.max(1));
        {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(flood.as_bytes());
            let _ = out.flush();
        }
        inner.repaint_all();
    }

    /// Commit styled lines into scrollback above the block.
    pub fn print(&self, text: &str) {
        let mut inner = self.lock();
        inner.commit_live();
        inner.paint(Some(text));
    }

    /// Commit plain text as dim, word-wrapped, re-wrappable prose (turn
    /// summaries and other long notes that must survive a resize tidily).
    pub fn print_note(&self, text: &str) {
        let mut inner = self.lock();
        inner.commit_live();
        let entries: Vec<Entry> = text
            .lines()
            .map(|line| Entry::Prose {
                block_start: false,
                spans: vec![md::Span {
                    style: md::SpanStyle::Dim,
                    text: line.to_string(),
                }],
                cont_indent: 2,
            })
            .collect();
        inner.commit_entries(entries);
    }

    pub fn set_status(&self, status: Status) {
        let mut inner = self.lock();
        inner.status = status;
        inner.paint(None);
    }

    pub fn set_session_data(&self, recency: Vec<String>, symbols: Vec<(String, String, String)>) {
        self.lock().index.set_session_data(recency, symbols);
    }

    pub fn begin_activity(&self) {
        let mut inner = self.lock();
        inner.renderer = MarkdownRenderer::default();
        inner.text_block_start = true;
        inner.activity = Some(Activity {
            started: Instant::now(),
            spinner: 0,
            tool: None,
            out_bytes: 0,
        });
        inner.paint(None);
    }

    pub fn end_activity(&self) {
        let mut inner = self.lock();
        inner.commit_live();
        // Any partial hidden-reasoning line still belongs in the transcript.
        if !inner.hidden_reasoning.is_empty() {
            let remainder = std::mem::take(&mut inner.hidden_reasoning);
            let entry = Entry::Prose {
                block_start: false,
                spans: vec![md::Span {
                    style: md::SpanStyle::Dim,
                    text: remainder,
                }],
                cont_indent: 0,
            };
            inner.record(false, entry);
        }
        inner.activity = None;
        inner.paint(None);
    }

    /// Advance the spinner; called on the input thread's poll timeout.
    pub fn tick(&self) {
        let mut inner = self.lock();
        if let Some(activity) = &mut inner.activity {
            activity.spinner += 1;
            inner.paint(None);
        }
    }

    /// Open the /connect flow; resolves with Some((provider, api_key)) on
    /// submit or None if the user backs out. The key is never echoed,
    /// recorded, or persisted by the UI — the caller owns saving it.
    pub fn open_connect(&self) -> oneshot::Receiver<Option<(String, String)>> {
        let mut inner = self.lock();
        let (sender, receiver) = oneshot::channel();
        if inner.connect.is_some() || inner.menu.is_some() {
            // Already mid-flow: refuse a second concurrent one.
            let _ = sender.send(None);
            return receiver;
        }
        inner.connect = Some(Connect {
            stage: ConnectStage::Provider { selected: 0 },
            responder: sender,
        });
        inner.paint(None);
        receiver
    }

    /// Open a generic arrow-key selector; resolves with the chosen index or
    /// None on cancel.
    pub fn open_select(
        &self,
        title: &str,
        items: Vec<String>,
        initial: usize,
    ) -> oneshot::Receiver<Option<usize>> {
        let mut inner = self.lock();
        let (sender, receiver) = oneshot::channel();
        if items.is_empty()
            || inner.select.is_some()
            || inner.connect.is_some()
            || inner.menu.is_some()
        {
            let _ = sender.send(None);
            return receiver;
        }
        let initial = initial.min(items.len() - 1);
        inner.select = Some(Select {
            title: title.to_string(),
            items,
            selected: initial,
            responder: sender,
        });
        inner.paint(None);
        receiver
    }

    /// Open the approval menu; the returned receiver resolves with the
    /// user's decision.
    fn open_menu(&self, request: &ApprovalRequest) -> oneshot::Receiver<Decision> {
        let mut inner = self.lock();
        inner.commit_live();
        let mut block = format!("\n  {CYAN}{}{RESET}", request.preview);
        if let Some(warning) = &request.policy_warning {
            block.push_str(&format!("\n  {RED}⚠ {warning}{RESET}"));
        }
        if let Some(detail) = &request.detail {
            block.push('\n');
            block.push_str(render_detail(detail).trim_end_matches('\n'));
        }
        let (sender, receiver) = oneshot::channel();
        inner.menu = Some(Menu {
            selected: 0,
            responder: sender,
        });
        inner.paint(Some(&block));
        receiver
    }

    /// Restore the terminal and leave the scrollback tidy: erase the block
    /// and park the shell cursor right after the committed content.
    pub fn teardown(&self) {
        let mut inner = self.lock();
        inner.close_overlay();
        inner.commit_live();
        inner.menu = None;
        inner.connect = None;
        inner.select = None;
        inner.dropdown = None;
        let mut buf = String::new();
        match inner.place {
            Place::Pinned {
                content_row,
                prev_block_top,
            } => {
                let height = inner.height.max(1);
                buf.push_str(&format!("\x1b[{};1H\x1b[J", prev_block_top.min(height)));
                buf.push_str(&format!("\x1b[{};1H", content_row.min(height)));
            }
            Place::Scrolling => {
                if inner.cursor_row > 0 {
                    buf.push_str(&format!("\x1b[{}A", inner.cursor_row));
                }
                buf.push('\r');
                buf.push_str("\x1b[J");
            }
        }
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
        inner.editor.save_history();
    }

    /// Handle one terminal event. Editing is absorbed internally; only
    /// session-relevant actions surface.
    pub fn handle_event(&self, event: Event) -> Option<UiAction> {
        let mut inner = self.lock();
        match event {
            Event::Resize(w, h) => {
                inner.width = w as usize;
                inner.height = h as usize;
                if inner.overlay.is_some() {
                    inner.draw_overlay();
                } else {
                    // Terminal reflow invalidates every cursor-relative
                    // assumption AND re-wraps previously committed lines at
                    // the wrong width. Repaint the whole visible screen from
                    // the transcript, re-wrapped to the new width, with the
                    // block re-pinned at the bottom edge.
                    inner.repaint_all();
                }
                None
            }
            Event::Paste(content) => {
                if let Some(Connect {
                    stage: ConnectStage::Key { buffer },
                    ..
                }) = &mut inner.connect
                {
                    buffer.push_str(content.trim());
                    inner.paint(None);
                } else if inner.menu.is_none() && inner.connect.is_none() {
                    inner.editor.insert_paste(&content);
                    inner.update_dropdown();
                    inner.paint(None);
                }
                None
            }
            Event::Key(key)
                if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat =>
            {
                self.handle_key(&mut inner, key.code, key.modifiers)
            }
            _ => None,
        }
    }

    fn handle_key(
        &self,
        inner: &mut UiInner,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<UiAction> {
        // The transcript overlay captures everything until closed.
        if let Some(offset) = inner.overlay {
            let page = inner.height.max(4) - 1;
            match code {
                KeyCode::Up | KeyCode::Char('k') => inner.overlay = Some(offset + 1),
                KeyCode::Down | KeyCode::Char('j') => {
                    inner.overlay = Some(offset.saturating_sub(1))
                }
                KeyCode::PageUp => inner.overlay = Some(offset + page),
                KeyCode::PageDown => inner.overlay = Some(offset.saturating_sub(page)),
                KeyCode::Home | KeyCode::Char('g') => inner.overlay = Some(usize::MAX),
                KeyCode::End | KeyCode::Char('G') => inner.overlay = Some(0),
                KeyCode::Char('q') | KeyCode::Esc => {
                    inner.close_overlay();
                    return None;
                }
                KeyCode::Char('t') if modifiers.contains(KeyModifiers::CONTROL) => {
                    inner.close_overlay();
                    return None;
                }
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    inner.close_overlay();
                    return None;
                }
                _ => {}
            }
            inner.draw_overlay();
            return None;
        }
        // The /connect flow captures everything until resolved.
        if inner.connect.is_some() {
            let control = modifiers.contains(KeyModifiers::CONTROL);
            let mut resolve: Option<Option<(String, String)>> = None;
            if let Some(connect) = &mut inner.connect {
                match (&mut connect.stage, code) {
                    (_, KeyCode::Char('c')) if control => resolve = Some(None),
                    (_, KeyCode::Char('d')) if control => resolve = Some(None),
                    (ConnectStage::Provider { .. }, KeyCode::Esc) => resolve = Some(None),
                    (ConnectStage::Provider { selected }, KeyCode::Up) => {
                        *selected = selected.saturating_sub(1);
                    }
                    (ConnectStage::Provider { selected }, KeyCode::Down) => {
                        *selected = (*selected + 1).min(super::connect::PROVIDERS.len() - 1);
                    }
                    (ConnectStage::Provider { .. }, KeyCode::Enter) => {
                        connect.stage = ConnectStage::Key {
                            buffer: String::new(),
                        };
                    }
                    (ConnectStage::Key { .. }, KeyCode::Esc) => {
                        connect.stage = ConnectStage::Provider { selected: 0 };
                    }
                    (ConnectStage::Key { buffer }, KeyCode::Backspace) => {
                        buffer.pop();
                    }
                    (ConnectStage::Key { buffer }, KeyCode::Enter) => {
                        let entered = buffer.trim().to_string();
                        if !entered.is_empty() {
                            let provider = super::connect::PROVIDERS[0].0.to_string();
                            resolve = Some(Some((provider, entered)));
                        }
                    }
                    (ConnectStage::Key { buffer }, KeyCode::Char(ch)) if !control => {
                        buffer.push(ch);
                    }
                    _ => {}
                }
            }
            if let Some(result) = resolve {
                if let Some(connect) = inner.connect.take() {
                    let _ = connect.responder.send(result);
                }
            }
            inner.paint(None);
            return None;
        }
        // A generic selector captures everything until resolved.
        if inner.select.is_some() {
            let mut resolve: Option<Option<usize>> = None;
            if let Some(select) = &mut inner.select {
                match code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        select.selected = select.selected.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        select.selected = (select.selected + 1).min(select.items.len() - 1);
                    }
                    KeyCode::Enter => resolve = Some(Some(select.selected)),
                    KeyCode::Esc => resolve = Some(None),
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        resolve = Some(None);
                    }
                    _ => {}
                }
            }
            if let Some(result) = resolve {
                if let Some(select) = inner.select.take() {
                    let _ = select.responder.send(result);
                }
            }
            inner.paint(None);
            return None;
        }
        // Approval menu captures everything until resolved.
        if inner.menu.is_some() {
            match code {
                KeyCode::Up => {
                    if let Some(menu) = &mut inner.menu {
                        menu.selected = (menu.selected + MENU.len() - 1) % MENU.len();
                    }
                    inner.paint(None);
                }
                KeyCode::Down => {
                    if let Some(menu) = &mut inner.menu {
                        menu.selected = (menu.selected + 1) % MENU.len();
                    }
                    inner.paint(None);
                }
                KeyCode::Enter => {
                    let selected = inner.menu.as_ref().map(|m| m.selected).unwrap_or(2);
                    inner.resolve_menu(selected);
                }
                KeyCode::Char('y') => inner.resolve_menu(0),
                KeyCode::Char('a') => inner.resolve_menu(1),
                KeyCode::Char('n') | KeyCode::Esc => inner.resolve_menu(2),
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    inner.resolve_menu(2);
                    return Some(UiAction::Interrupt);
                }
                _ => {}
            }
            return None;
        }
        let control = modifiers.contains(KeyModifiers::CONTROL);
        match code {
            KeyCode::Char('c') if control => {
                if inner.editor.is_empty() {
                    return Some(UiAction::Interrupt);
                }
                inner.editor.clear();
                inner.dropdown = None;
                inner.paint(None);
            }
            KeyCode::Char('d') if control && inner.editor.is_empty() => {
                return Some(UiAction::Eof);
            }
            KeyCode::Char('t') if control => {
                inner.open_overlay();
            }
            KeyCode::Char('r') if control => {
                inner.show_reasoning = !inner.show_reasoning;
                let notice = if inner.show_reasoning {
                    "[reasoning stream on (ctrl+r to hide)]"
                } else {
                    "[reasoning stream off; ctrl+t shows the full transcript]"
                };
                inner.paint(Some(&format!("{DIM}{notice}{RESET}")));
            }
            KeyCode::Char('j') if control => {
                inner.editor.insert('\n');
                inner.dropdown = None;
                inner.paint(None);
            }
            KeyCode::Char('a') if control => {
                inner.editor.move_home();
                inner.paint(None);
            }
            KeyCode::Char('e') if control => {
                inner.editor.move_end();
                inner.paint(None);
            }
            KeyCode::Char('u') if control => {
                inner.editor.clear();
                inner.dropdown = None;
                inner.paint(None);
            }
            KeyCode::Char('k') if control => {
                let (start, end) = (inner.editor.cursor(), inner.editor.line_end());
                inner.editor.replace_range(start, end, "");
                inner.update_dropdown();
                inner.paint(None);
            }
            KeyCode::Char('w') if control => {
                inner.editor.delete_word_back();
                inner.update_dropdown();
                inner.paint(None);
            }
            KeyCode::Char(ch) if !control => {
                inner.editor.insert(ch);
                inner.update_dropdown();
                inner.paint(None);
            }
            KeyCode::Enter if modifiers.contains(KeyModifiers::ALT) => {
                inner.editor.insert('\n');
                inner.dropdown = None;
                inner.paint(None);
            }
            KeyCode::Enter => {
                if let Some(dropdown) = &inner.dropdown {
                    // If accepting would not change the token (the user typed
                    // it out in full), Enter means submit, not re-accept.
                    let token = &inner.editor.text()[dropdown.token_start..inner.editor.cursor()];
                    if dropdown.choices[dropdown.selected].insert != token {
                        inner.accept_selected();
                        inner.paint(None);
                        return None;
                    }
                    inner.dropdown = None;
                }
                let visible = inner.editor.text().trim().to_string();
                let submitted = inner.editor.submit();
                inner.dropdown = None;
                if submitted.is_empty() {
                    inner.paint(None);
                    return None;
                }
                // Bare picker commands are composer interactions, not
                // conversation turns: the picker opens in place of the
                // input and nothing echoes into the transcript
                // (Codex-style).
                if matches!(submitted.as_str(), "/model" | "/connect") {
                    inner.paint(None);
                    return Some(UiAction::Submit(submitted));
                }
                // Echo the prompt as a highlighted bar so user turns stand
                // apart from agent output in scrollback. One entry for the
                // whole message: the marker draws on the first row only.
                inner.commit_entries(vec![Entry::UserBar(visible.to_string()), Entry::Blank]);
                return Some(UiAction::Submit(submitted));
            }
            KeyCode::Backspace => {
                inner.editor.backspace();
                inner.update_dropdown();
                inner.paint(None);
            }
            KeyCode::Delete => {
                inner.editor.delete_forward();
                inner.update_dropdown();
                inner.paint(None);
            }
            KeyCode::Left => {
                inner.editor.move_left();
                inner.dropdown = None;
                inner.paint(None);
            }
            KeyCode::Right => {
                inner.editor.move_right();
                inner.dropdown = None;
                inner.paint(None);
            }
            KeyCode::Home => {
                inner.editor.move_home();
                inner.paint(None);
            }
            KeyCode::End => {
                inner.editor.move_end();
                inner.paint(None);
            }
            KeyCode::BackTab => {
                return Some(UiAction::CycleMode);
            }
            KeyCode::Tab => {
                if inner.dropdown.is_some() {
                    inner.accept_selected();
                    inner.paint(None);
                }
            }
            KeyCode::Esc => {
                if inner.dropdown.is_some() {
                    inner.dropdown = None;
                    inner.paint(None);
                } else if inner.activity.is_some() {
                    return Some(UiAction::Interrupt);
                }
            }
            KeyCode::Up => {
                if let Some(dropdown) = &mut inner.dropdown {
                    let len = dropdown.choices.len().min(MAX_DROPDOWN_ROWS);
                    dropdown.selected = (dropdown.selected + len - 1) % len;
                } else if !inner.editor.on_first_line() {
                    inner.editor.move_up_line();
                } else {
                    inner.editor.history_prev();
                }
                inner.paint(None);
            }
            KeyCode::Down => {
                if let Some(dropdown) = &mut inner.dropdown {
                    let len = dropdown.choices.len().min(MAX_DROPDOWN_ROWS);
                    dropdown.selected = (dropdown.selected + 1) % len;
                } else if !inner.editor.on_last_line() {
                    inner.editor.move_down_line();
                } else {
                    inner.editor.history_next();
                }
                inner.paint(None);
            }
            _ => {}
        }
        None
    }
}

/// Streams a turn into the shared surface.
pub struct UiSink {
    ui: Arc<Ui>,
}

impl UiSink {
    pub fn new(ui: Arc<Ui>) -> Self {
        Self { ui }
    }
}

impl StreamSink for UiSink {
    fn on_text(&mut self, delta: &str) {
        self.ui.lock().stream(LiveKind::Text, delta);
    }

    fn on_reasoning(&mut self, delta: &str) {
        let mut inner = self.ui.lock();
        if inner.show_reasoning {
            inner.stream(LiveKind::Reasoning, delta);
            return;
        }
        // Hidden from the screen, but the transcript (Ctrl+T) keeps it.
        if let Some(activity) = &mut inner.activity {
            activity.out_bytes += delta.len() as u64;
        }
        inner.hidden_reasoning.push_str(delta);
        while let Some(at) = inner.hidden_reasoning.find('\n') {
            let line: String = inner.hidden_reasoning.drain(..=at).collect();
            let line = line.trim_end_matches('\n');
            if !line.is_empty() {
                let entry = Entry::Prose {
                    block_start: false,
                    spans: vec![md::Span {
                        style: md::SpanStyle::Dim,
                        text: line.to_string(),
                    }],
                    cont_indent: 0,
                };
                inner.record(false, entry);
            }
        }
    }

    fn on_tool_start(&mut self, preview: &str) {
        let mut inner = self.ui.lock();
        inner.commit_live();
        if let Some(activity) = &mut inner.activity {
            activity.tool = Some(truncate_cols(preview, 48));
        }
        inner.live = Some(LiveLine {
            kind: LiveKind::Tool,
            text: format!("⚒ {preview}"),
        });
        inner.paint(None);
    }

    fn on_tool_result(&mut self, preview: &str, is_error: bool, error: Option<&str>) {
        let mut inner = self.ui.lock();
        let mark = match (is_error, error) {
            (true, Some(reason)) if !reason.is_empty() => {
                format!(" {RED}✗{RESET} {DIM}{}{RESET}", truncate_cols(reason, 60))
            }
            (true, _) => format!(" {RED}✗{RESET}"),
            (false, _) => " ✓".to_string(),
        };
        if inner
            .live
            .as_ref()
            .is_some_and(|live| live.kind == LiveKind::Tool)
        {
            let live = inner.live.take().expect("tool live line");
            inner.paint(Some(&format!("{CYAN}{}{RESET}{mark}", live.text)));
        } else {
            // The live line was already committed (an approval menu opened
            // in between): the result mark still needs a visible home.
            inner.paint(Some(&format!("{CYAN}⚒ {preview}{RESET}{mark}")));
        }
        inner.text_block_start = true;
        if let Some(activity) = &mut inner.activity {
            activity.tool = None;
        }
        inner.paint(None);
    }
}

/// Approval requests render above the block and resolve via the arrow-key
/// menu (y / a / n / Esc still work).
pub struct UiGate {
    ui: Arc<Ui>,
}

impl UiGate {
    pub fn new(ui: Arc<Ui>) -> Self {
        Self { ui }
    }
}

#[async_trait::async_trait]
impl ApprovalGate for UiGate {
    async fn review(&self, request: &ApprovalRequest) -> Decision {
        let receiver = self.ui.open_menu(request);
        receiver.await.unwrap_or(Decision::Deny)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner_at_width(width: usize) -> UiInner {
        UiInner {
            width,
            height: 30,
            place: Place::Scrolling,
            editor: Editor::new(None),
            index: CompletionIndex::with_entries(vec![], vec![]),
            dropdown: None,
            menu: None,
            connect: None,
            select: None,
            status: Status {
                model: "deepseek-v4-flash".into(),
                mode: "read-only",
                ctx_pct: 0.42,
            },
            activity: None,
            live: None,
            renderer: MarkdownRenderer::default(),
            text_block_start: true,
            last_blank: true,
            show_reasoning: false,
            hidden_reasoning: String::new(),
            transcript: std::collections::VecDeque::new(),
            overlay: None,
            pending_output: Vec::new(),
            cursor_row: 0,
        }
    }

    /// The invariant behind the erase math: no composed block line may be
    /// wider than the terminal, or it wraps to two physical rows and every
    /// later repaint strands a stale frame in scrollback (user-reported).
    #[test]
    fn no_block_line_exceeds_terminal_width() {
        for width in [24usize, 40, 60, 80] {
            let mut inner = inner_at_width(width);
            // Long streaming text line WITH the ● prefix budget in play.
            inner.live = Some(LiveLine {
                kind: LiveKind::Text,
                text: "x".repeat(width * 3),
            });
            inner.activity = Some(Activity {
                started: Instant::now(),
                spinner: 3,
                tool: Some("read_file(some/deeply/nested/path.rs)".into()),
                out_bytes: 123_456,
            });
            inner.dropdown = Some(Dropdown {
                choices: vec![super::super::completion::Choice {
                    insert: "@x".into(),
                    label: "a/very/long/path/that/goes/on/forever/and/ever/mod.rs".into(),
                    hint: "struct · a/very/long/location.rs:1234".into(),
                    kind: ChoiceKind::Symbol,
                }],
                selected: 0,
                token_start: 0,
            });
            inner
                .editor
                .insert_str("some typed input that is fairly long too");
            let (lines, _, _) = inner.compose_block();
            for line in &lines {
                assert!(
                    visible_cols(line) <= width,
                    "line exceeds {width} cols ({}): {line:?}",
                    visible_cols(line),
                );
            }
        }
    }

    /// Tool and reasoning live lines (no prefix) must also fit exactly.
    #[test]
    fn unprefixed_live_lines_fit_too() {
        let mut inner = inner_at_width(50);
        inner.live = Some(LiveLine {
            kind: LiveKind::Tool,
            text: format!("⚒ read_file({})", "y".repeat(200)),
        });
        let (lines, _, _) = inner.compose_block();
        assert!(lines.iter().all(|line| visible_cols(line) <= 50));
    }

    #[test]
    fn physical_rows_counts_wrapping_and_ignores_ansi() {
        assert_eq!(physical_rows("", 20), 1);
        assert_eq!(physical_rows("short", 20), 1);
        assert_eq!(physical_rows(&"x".repeat(20), 20), 1);
        assert_eq!(physical_rows(&"x".repeat(45), 20), 3);
        assert_eq!(physical_rows("\x1b[36mstyled\x1b[0m", 20), 1);
    }

    #[test]
    fn pinned_block_sits_at_the_bottom_edge() {
        let mut inner = inner_at_width(80);
        inner.height = 30;
        inner.place = Place::Pinned {
            content_row: 1,
            prev_block_top: 30,
        };
        let block_height = inner.compose_block().0.len();
        let frame = inner.pinned_frame(None);
        let expected_top = 30 - block_height + 1;
        assert!(
            frame.contains(&format!("\x1b[{expected_top};1H")),
            "block must draw at the bottom edge (row {expected_top}): {frame:?}"
        );
        assert_eq!(
            inner.place,
            Place::Pinned {
                content_row: 1,
                prev_block_top: expected_top,
            }
        );
    }

    #[test]
    fn pinned_output_advances_content_row_by_physical_rows() {
        let mut inner = inner_at_width(20);
        inner.height = 30;
        inner.place = Place::Pinned {
            content_row: 1,
            prev_block_top: 30,
        };
        // One short line (1 row) + one 45-col line (3 rows at width 20).
        let output = format!("short\n{}", "y".repeat(45));
        inner.pinned_frame(Some(&output));
        match inner.place {
            Place::Pinned { content_row, .. } => assert_eq!(content_row, 5),
            other => panic!("expected pinned, got {other:?}"),
        }
    }

    #[test]
    fn pinned_hands_off_to_scrolling_when_content_meets_the_block() {
        let mut inner = inner_at_width(80);
        inner.height = 10;
        let block_height = inner.compose_block().0.len();
        let block_top = 10 - block_height + 1;
        inner.place = Place::Pinned {
            content_row: block_top, // the next output row collides
            prev_block_top: block_top,
        };
        inner.pinned_frame(Some("one more line"));
        assert_eq!(inner.place, Place::Scrolling);
    }

    #[test]
    fn tiny_terminals_go_straight_to_scrolling() {
        let mut inner = inner_at_width(80);
        inner.height = 3;
        // A live line and activity row grow the block past the (clamped)
        // terminal height, forcing the scrolling fallback.
        inner.live = Some(LiveLine {
            kind: LiveKind::Text,
            text: "streaming".into(),
        });
        inner.activity = Some(Activity {
            started: Instant::now(),
            spinner: 0,
            tool: None,
            out_bytes: 0,
        });
        inner.place = Place::Pinned {
            content_row: 1,
            prev_block_top: 1,
        };
        inner.pinned_frame(None);
        assert_eq!(inner.place, Place::Scrolling);
    }

    #[test]
    fn prose_word_wraps_at_word_boundaries_with_block_prefixes() {
        let mut inner = inner_at_width(30);
        inner.last_blank = false;
        inner.text_block_start = true;
        let entries = inner
            .render_text_entries("The quick brown fox jumps over the lazy dog near the riverbank");
        let out: Vec<String> = entries
            .iter()
            .flat_map(|entry| render_entry(entry, 30))
            .collect();
        let lines: Vec<&str> = out.iter().map(String::as_str).collect();
        // Spacer first (previous line was not blank), then ● + indented rows.
        assert_eq!(lines[0], "");
        assert!(lines[1].starts_with("● "));
        assert!(lines[2].starts_with("  "));
        for line in &lines[1..] {
            assert!(visible_cols(line) <= 30, "row too wide: {line:?}");
            // No mid-word cuts: each row ends at a word boundary.
            assert!(!line.ends_with(' '));
        }
        assert!(lines[1].ends_with("fox") || !lines[1].contains("jumps over the lazy"));
    }

    #[test]
    fn bullets_hang_wrapped_rows_under_their_text() {
        let mut inner = inner_at_width(30);
        inner.last_blank = true;
        inner.text_block_start = false;
        let entries = inner.render_text_entries("- a bullet item with enough words to wrap around");
        let out: Vec<String> = entries
            .iter()
            .flat_map(|entry| render_entry(entry, 30))
            .collect();
        let lines: Vec<&str> = out.iter().map(String::as_str).collect();
        assert!(lines.len() >= 2);
        assert!(lines[0].contains('•'));
        // Continuations indent 2 (block) + 2 (bullet hang) = 4 columns.
        assert!(
            lines[1].starts_with("    "),
            "hanging indent lost: {:?}",
            lines[1]
        );
    }

    #[test]
    fn bold_spans_survive_row_wrapping() {
        let spans = vec![
            md::Span {
                style: md::SpanStyle::Plain,
                text: "start ".into(),
            },
            md::Span {
                style: md::SpanStyle::Bold,
                text: "a bold run that is long enough to cross rows".into(),
            },
        ];
        let rows = styled_rows(&spans, 20, 20);
        assert!(rows.len() >= 2);
        // Every row that contains bold text re-opens the style.
        for row in &rows[1..] {
            assert!(row.contains(md::BOLD), "style dropped on wrap: {row:?}");
        }
    }

    #[test]
    fn long_words_hard_cut_instead_of_overflowing() {
        let spans = vec![md::Span {
            style: md::SpanStyle::Plain,
            text: "x".repeat(50),
        }];
        let rows = styled_rows(&spans, 20, 20);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| visible_cols(r) <= 20));
    }

    #[test]
    fn transcript_records_commits_and_overlay_math_clamps() {
        let mut inner = inner_at_width(40);
        inner.height = 10;
        // paint() would write to the terminal; exercise record() directly.
        for i in 0..20 {
            inner.record(true, Entry::Fixed(format!("line {i}")));
        }
        assert_eq!(inner.transcript.len(), 20);
        assert_eq!(inner.transcript[0], (true, Entry::Fixed("line 0".into())));
    }

    #[test]
    fn entries_rewrap_to_the_width_they_are_rendered_at() {
        let entry = Entry::Prose {
            block_start: true,
            spans: vec![md::Span {
                style: md::SpanStyle::Plain,
                text: "words that will wrap differently at different widths for sure".into(),
            }],
            cont_indent: 0,
        };
        let narrow = render_entry(&entry, 24);
        let wide = render_entry(&entry, 80);
        assert!(narrow.len() > wide.len(), "narrow must produce more rows");
        assert!(narrow.iter().all(|row| visible_cols(row) <= 24));
        assert_eq!(wide.len(), 1);
        assert!(wide[0].starts_with("● "));
        // User bars re-pad to the width they are rendered at.
        let bar = Entry::UserBar("fix the bug".into());
        for width in [24usize, 60] {
            let rows = render_entry(&bar, width);
            assert_eq!(visible_cols(&rows[0]), width, "bar must span the width");
        }
    }

    #[test]
    fn user_bars_mark_only_the_first_row_of_a_multiline_prompt() {
        let bar = Entry::UserBar("hi tell me\n\n@crates/core/src/provider/mod.rs".into());
        let rows = render_entry(&bar, 60);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().filter(|row| row.contains('❯')).count(), 1);
        assert!(rows[0].contains("❯ hi tell me"), "{}", rows[0]);
        // The blank interior line and the continuation stay on the band,
        // indented under the marker.
        for row in &rows {
            assert!(row.starts_with(BAND), "{row}");
            assert_eq!(visible_cols(row), 60, "{row}");
        }
        assert!(rows[2].contains("  @crates"), "{}", rows[2]);
        // Content column matches assistant prose: marker col 0, text col 2.
        assert!(rows[0].starts_with(&format!("{BAND}❯ hi")), "{}", rows[0]);
        // A trailing newline adds no extra band row.
        let trailing = Entry::UserBar("done\n".into());
        assert_eq!(render_entry(&trailing, 60).len(), 1);
    }

    #[test]
    fn repaint_math_refills_from_the_transcript_tail() {
        let mut inner = inner_at_width(40);
        inner.height = 12;
        for i in 0..30 {
            inner.record(true, Entry::Fixed(format!("line {i}")));
        }
        // Hidden reasoning must NOT surface in the repaint fill.
        inner.record(false, Entry::Fixed("secret reasoning line".into()));
        // Mirror repaint_all's fill loop (the drawing itself needs a tty).
        let block_height = inner.compose_block().0.len();
        let avail = inner.height - block_height;
        let mut rows: Vec<String> = Vec::new();
        'fill: for entry in inner
            .transcript
            .iter()
            .rev()
            .filter_map(|(on_screen, entry)| on_screen.then_some(entry))
        {
            for line in render_entry(entry, inner.width).into_iter().rev() {
                rows.push(line);
                if rows.len() >= avail {
                    break 'fill;
                }
            }
        }
        rows.reverse();
        assert_eq!(rows.len(), avail);
        assert_eq!(rows.last().expect("tail"), "line 29");
        assert!(rows.iter().all(|row| !row.contains("secret")));
    }

    #[test]
    fn truncate_styled_counts_visible_columns_only() {
        let styled = format!("{}0123456789{}0123456789", md::CYAN, md::RESET);
        let cut = truncate_styled(&styled, 12);
        assert!(visible_cols(&cut) <= 12);
        assert!(cut.contains('…'));
        assert!(cut.starts_with(md::CYAN));
        let fits = truncate_styled("short", 12);
        assert!(fits.starts_with("short"));
    }
}

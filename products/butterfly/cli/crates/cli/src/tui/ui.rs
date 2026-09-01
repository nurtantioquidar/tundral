//! The shared inline-TUI surface. Committed output flows into native
//! terminal scrollback; only a small block at the bottom (live stream line,
//! input editor, completion dropdown, status bar) is redrawn in place:
//! erase block → append output → redraw block. No alternate screen, so
//! scrollback, copy-paste, and tmux behave normally.

use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use butterfly_core::domain::format_microusd_as_usd;
use butterfly_core::provider::StreamSink;
use butterfly_core::session::approval::{ApprovalGate, ApprovalRequest, Decision};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use tokio::sync::oneshot;
use unicode_width::UnicodeWidthChar;

use super::completion::{active_trigger, ChoiceKind, CompletionIndex};
use super::editor::Editor;
use super::markdown::MarkdownRenderer;
use crate::repl::render_detail;

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const GREEN: &str = "\x1b[32m";
const INVERSE: &str = "\x1b[7m";
const SYNC_BEGIN: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";

const MAX_INPUT_ROWS: usize = 8;
const MAX_DROPDOWN_ROWS: usize = 6;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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

pub struct Status {
    pub model: String,
    pub workspace: String,
    pub mode: &'static str,
    pub ctx_pct: f64,
    pub session_cost_microusd: u64,
    pub cache_pct: Option<f64>,
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

struct UiInner {
    width: usize,
    height: usize,
    place: Place,
    editor: Editor,
    index: CompletionIndex,
    dropdown: Option<Dropdown>,
    menu: Option<Menu>,
    status: Status,
    activity: Option<Activity>,
    live: Option<LiveLine>,
    renderer: MarkdownRenderer,
    /// True when the next assistant text line starts a response block and
    /// gets the `●` marker; continuation lines are indented under it.
    text_block_start: bool,
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
                "✳ {} {}s · ~{} tok",
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
        if let Some(menu) = &self.menu {
            for (i, (label, _)) in MENU.iter().enumerate() {
                let marker = if i == menu.selected { "▸" } else { " " };
                let styled = if i == menu.selected {
                    format!("  {CYAN}{marker} {label}{RESET}")
                } else {
                    format!("  {DIM}{marker} {label}{RESET}")
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
            for (i, choice) in dropdown.choices.iter().take(MAX_DROPDOWN_ROWS).enumerate() {
                let marker = if i == dropdown.selected { "▸" } else { " " };
                // Row budget: 4 prefix cols + label + 2 + hint ≤ width.
                // Wide terminals reserve ~22 cols for the hint; narrow ones
                // give the label everything and drop the hint.
                let label_budget = if width >= 44 {
                    width - 28
                } else {
                    width.saturating_sub(4)
                };
                let label = truncate_cols(&choice.label, label_budget.max(8));
                let hint_budget = width.saturating_sub(4 + width_of(&label) + 2);
                let hint = if choice.hint.is_empty() || hint_budget < 4 {
                    String::new()
                } else {
                    format!("  {}", truncate_cols(&choice.hint, hint_budget))
                };
                let styled = if i == dropdown.selected {
                    format!("  {INVERSE}{marker} {label}{RESET}{DIM}{hint}{RESET}")
                } else {
                    format!("  {marker} {label}{DIM}{hint}{RESET}")
                };
                lines.push(styled);
            }
        }
        lines.push(self.compose_status(width));
        lines.push(self.compose_mode_line(width));
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
                "message · @ file · / command · ctrl+j newline"
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

    fn compose_status(&self, width: usize) -> String {
        let mut status = format!(
            "{} | {} | ctx:{:.0}% | {}",
            self.status.model,
            self.status.workspace,
            self.status.ctx_pct * 100.0,
            format_microusd_as_usd(self.status.session_cost_microusd),
        );
        if let Some(cache) = self.status.cache_pct {
            status.push_str(&format!(" | cache:{cache:.0}%"));
        }
        format!("{DIM}{}{RESET}", truncate_cols(&status, width))
    }

    fn compose_mode_line(&self, width: usize) -> String {
        let color = match self.status.mode {
            "auto" => YELLOW,
            "read-only" => GREEN,
            _ => DIM,
        };
        let line = format!("▶▶ {} mode (shift+tab to cycle)", self.status.mode);
        format!("{color}{}{RESET}", truncate_cols(&line, width))
    }

    /// Erase the block, optionally append committed output, redraw.
    fn paint(&mut self, output: Option<&str>) {
        let buf = match self.place {
            Place::Pinned { .. } => self.pinned_frame(output),
            Place::Scrolling => self.scrolling_frame(output),
        };
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
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
            self.place = Place::Pinned {
                content_row,
                prev_block_top: block_top,
            };
        }
        buf.push_str(SYNC_END);
        buf
    }

    /// Markdown-render one assistant text line, with the `●` block marker on
    /// the first line of a response block and alignment indents after it.
    fn render_text_line(&mut self, line: &str) -> String {
        let rendered = self.renderer.render_line(line);
        if line.trim().is_empty() {
            return rendered;
        }
        if self.text_block_start {
            self.text_block_start = false;
            format!("● {rendered}")
        } else {
            format!("  {rendered}")
        }
    }

    /// Commit the live line (if any) into scrollback.
    fn commit_live(&mut self) {
        if let Some(live) = self.live.take() {
            if live.text.is_empty() {
                return;
            }
            let styled = match live.kind {
                LiveKind::Reasoning => {
                    self.text_block_start = true;
                    format!("{DIM}{}{RESET}", live.text)
                }
                LiveKind::Text => self.render_text_line(&live.text),
                LiveKind::Tool => {
                    self.text_block_start = true;
                    format!("{CYAN}{}{RESET}", live.text)
                }
            };
            self.paint(Some(&styled));
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
        let mut committed = String::new();
        while let Some(at) = self.live.as_ref().and_then(|l| l.text.find('\n')) {
            let line: String = {
                let live = self.live.as_mut().expect("live line");
                live.text.drain(..=at).collect()
            };
            let line = line.trim_end_matches('\n');
            let styled = match kind {
                LiveKind::Reasoning => {
                    self.text_block_start = true;
                    format!("{DIM}{line}{RESET}")
                }
                LiveKind::Text => self.render_text_line(line),
                LiveKind::Tool => {
                    self.text_block_start = true;
                    format!("{CYAN}{line}{RESET}")
                }
            };
            if !committed.is_empty() {
                committed.push('\n');
            }
            committed.push_str(&styled);
        }
        if committed.is_empty() {
            self.paint(None);
        } else {
            self.paint(Some(&committed));
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
            self.paint(Some(&format!("  {DIM}↳ {label}{RESET}")));
            let _ = menu.responder.send(decision);
        }
    }
}

impl Ui {
    pub fn new(editor: Editor, index: CompletionIndex, status: Status) -> Arc<Self> {
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
                status,
                activity: None,
                live: None,
                renderer: MarkdownRenderer::default(),
                text_block_start: true,
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
        inner.place = Place::Pinned {
            content_row: 1,
            prev_block_top: 1,
        };
        inner.paint(None);
    }

    /// Commit styled lines into scrollback above the block.
    pub fn print(&self, text: &str) {
        let mut inner = self.lock();
        inner.commit_live();
        inner.paint(Some(text));
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
        inner.commit_live();
        inner.menu = None;
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
                // Reflow on resize is terminal-dependent; the pinned row
                // bookkeeping can't survive it reliably, so hand off to the
                // cursor-relative scrolling model.
                if let Place::Pinned { content_row, .. } = inner.place {
                    let row = content_row.min(inner.height.max(1));
                    let mut out = std::io::stdout().lock();
                    let _ = out.write_all(format!("\x1b[{row};1H\x1b[J").as_bytes());
                    let _ = out.flush();
                    inner.place = Place::Scrolling;
                    inner.cursor_row = 0;
                }
                inner.paint(None);
                None
            }
            Event::Paste(content) => {
                if inner.menu.is_none() {
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
                // Echo the prompt as a highlighted bar so user turns stand
                // apart from agent output in scrollback.
                let width = inner.width.max(20);
                let bar: Vec<String> = visible
                    .lines()
                    .map(|line| {
                        let pad = width.saturating_sub(width_of(line) + 4);
                        format!("{INVERSE}{DIM} ❯ {line}{}{RESET}", " ".repeat(pad))
                    })
                    .collect();
                inner.paint(Some(&format!("{}\n", bar.join("\n"))));
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
    show_reasoning: bool,
}

impl UiSink {
    pub fn new(ui: Arc<Ui>, show_reasoning: bool) -> Self {
        Self { ui, show_reasoning }
    }
}

impl StreamSink for UiSink {
    fn on_text(&mut self, delta: &str) {
        self.ui.lock().stream(LiveKind::Text, delta);
    }

    fn on_reasoning(&mut self, delta: &str) {
        if !self.show_reasoning {
            let mut inner = self.ui.lock();
            if let Some(activity) = &mut inner.activity {
                activity.out_bytes += delta.len() as u64;
            }
            return;
        }
        self.ui.lock().stream(LiveKind::Reasoning, delta);
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

    fn on_tool_result(&mut self, preview: &str, is_error: bool) {
        let mut inner = self.ui.lock();
        let mark = if is_error {
            format!(" {RED}✗{RESET}")
        } else {
            " ✓".to_string()
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
            status: Status {
                model: "deepseek-v4-flash".into(),
                workspace: "a-rather-long-workspace-name".into(),
                mode: "read-only",
                ctx_pct: 0.42,
                session_cost_microusd: 123_456,
                cache_pct: Some(93.4),
            },
            activity: None,
            live: None,
            renderer: MarkdownRenderer::default(),
            text_block_start: true,
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
        inner.place = Place::Pinned {
            content_row: 1,
            prev_block_top: 1,
        };
        inner.pinned_frame(None);
        assert_eq!(inner.place, Place::Scrolling);
    }
}

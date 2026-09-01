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
const INVERSE: &str = "\x1b[7m";
const SYNC_BEGIN: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";

const MAX_INPUT_ROWS: usize = 8;
const MAX_DROPDOWN_ROWS: usize = 6;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// What a handled input event means for the session loop.
pub enum UiAction {
    Submit(String),
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

struct UiInner {
    width: usize,
    editor: Editor,
    index: CompletionIndex,
    dropdown: Option<Dropdown>,
    menu: Option<Menu>,
    status: Status,
    activity: Option<Activity>,
    live: Option<LiveLine>,
    renderer: MarkdownRenderer,
    /// The cursor's row within the block at last draw: MoveUp(cursor_row)
    /// reaches the block's top line for erase-and-redraw.
    cursor_row: usize,
}

pub struct Ui {
    inner: Mutex<UiInner>,
}

fn width_of(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
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
            let text = tail_cols(&live.text, width);
            lines.push(match live.kind {
                LiveKind::Reasoning => format!("{DIM}{text}{RESET}"),
                LiveKind::Text => text,
                LiveKind::Tool => format!("{CYAN}{text}{RESET}"),
            });
        }
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
                let label = truncate_cols(&choice.label, width.saturating_sub(24).max(12));
                let hint = if choice.hint.is_empty() {
                    String::new()
                } else {
                    format!("  {}", truncate_cols(&choice.hint, 20))
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
            "{} · {} · ctx {:.0}% · {}",
            self.status.model,
            self.status.mode,
            self.status.ctx_pct * 100.0,
            format_microusd_as_usd(self.status.session_cost_microusd),
        );
        if let Some(cache) = self.status.cache_pct {
            status.push_str(&format!(" · cache {cache:.0}%"));
        }
        if let Some(activity) = &self.activity {
            status.push_str(&format!(
                " · {} {}s · ~{} tok",
                SPINNER[activity.spinner % SPINNER.len()],
                activity.started.elapsed().as_secs(),
                activity.out_bytes / 4,
            ));
            if let Some(tool) = &activity.tool {
                status.push_str(&format!(" · {tool}"));
            }
        }
        format!("{DIM}{}{RESET}", truncate_cols(&status, width))
    }

    /// Erase the block, optionally append committed output above it, redraw.
    fn paint(&mut self, output: Option<&str>) {
        let mut buf = String::from(SYNC_BEGIN);
        if self.cursor_row > 0 {
            buf.push_str(&format!("\x1b[{}A", self.cursor_row));
        }
        buf.push('\r');
        buf.push_str("\x1b[J");
        if let Some(output) = output {
            for line in output.split('\n') {
                buf.push_str(line);
                buf.push_str(RESET);
                buf.push_str("\r\n");
            }
        }
        let (lines, cursor_row, cursor_col) = self.compose_block();
        buf.push_str(&lines.join("\r\n"));
        let up = lines.len().saturating_sub(1).saturating_sub(cursor_row);
        if up > 0 {
            buf.push_str(&format!("\x1b[{up}A"));
        }
        buf.push('\r');
        if cursor_col > 0 {
            buf.push_str(&format!("\x1b[{cursor_col}C"));
        }
        buf.push_str(SYNC_END);
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
        self.cursor_row = cursor_row;
    }

    /// Commit the live line (if any) into scrollback.
    fn commit_live(&mut self) {
        if let Some(live) = self.live.take() {
            if live.text.is_empty() {
                return;
            }
            let styled = match live.kind {
                LiveKind::Reasoning => format!("{DIM}{}{RESET}", live.text),
                LiveKind::Text => self.renderer.render_line(&live.text),
                LiveKind::Tool => format!("{CYAN}{}{RESET}", live.text),
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
            let live = self.live.as_mut().expect("live line");
            let line: String = live.text.drain(..=at).collect();
            let line = line.trim_end_matches('\n');
            let styled = match kind {
                LiveKind::Reasoning => format!("{DIM}{line}{RESET}"),
                LiveKind::Text => self.renderer.render_line(line),
                LiveKind::Tool => format!("{CYAN}{line}{RESET}"),
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
        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(100);
        Arc::new(Self {
            inner: Mutex::new(UiInner {
                width,
                editor,
                index,
                dropdown: None,
                menu: None,
                status,
                activity: None,
                live: None,
                renderer: MarkdownRenderer::default(),
                cursor_row: 0,
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, UiInner> {
        self.inner.lock().expect("ui lock")
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

    /// Restore the terminal and leave the scrollback tidy.
    pub fn teardown(&self) {
        let mut inner = self.lock();
        inner.commit_live();
        inner.menu = None;
        inner.dropdown = None;
        let mut buf = String::new();
        if inner.cursor_row > 0 {
            buf.push_str(&format!("\x1b[{}A", inner.cursor_row));
        }
        buf.push('\r');
        buf.push_str("\x1b[J");
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
            Event::Resize(w, _) => {
                inner.width = w as usize;
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
                inner.paint(Some(&format!("{CYAN}❯{RESET} {visible}")));
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
            KeyCode::Tab => {
                if inner.dropdown.is_some() {
                    inner.accept_selected();
                    inner.paint(None);
                }
            }
            KeyCode::Esc => {
                inner.dropdown = None;
                inner.paint(None);
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

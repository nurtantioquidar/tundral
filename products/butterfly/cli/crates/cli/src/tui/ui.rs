//! The shared inline-TUI surface. Committed output flows into native
//! terminal scrollback; only a small block at the bottom (live stream line,
//! activity row, input editor, completion dropdown, footer) is redrawn in
//! place: erase block ⇢ append output ⇢ redraw block. No alternate screen,
//! so scrollback, copy-paste, and tmux behave normally.
//!
//! Colors and width primitives live in `theme`; every composed surface
//! (welcome card, footer, tool rows, approval card, help) lives in
//! `chrome`. This module owns state, layout math and input.

use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use butterfly_core::provider::StreamSink;
use butterfly_core::session::approval::{ApprovalGate, ApprovalRequest, Decision};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use tokio::sync::oneshot;

use super::chrome::{self, hint_row, option_row, Phase, ToolOutcome, GRID};
use super::completion::{active_trigger, ChoiceKind, CompletionIndex};
use super::editor::Editor;
use super::markdown::{self as md, MarkdownRenderer};
use super::theme::{
    self as th, tail_cols, truncate_cols, truncate_styled, visible_cols, width_of, ACCENT,
    ACCENT_BOLD, BAND, BOLD, DANGER, DIM, RESET,
};

const SYNC_BEGIN: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";

const MAX_INPUT_ROWS: usize = 8;
const MAX_DROPDOWN_ROWS: usize = 6;
/// The model picker can outgrow the block; scroll it instead of letting it
/// push the composer off a short terminal.
const MAX_SELECT_ROWS: usize = 8;
/// How much of a still-streaming table the block previews. Bounded so a
/// long table cannot push the composer off a short terminal.
const MAX_LIVE_TABLE_ROWS: usize = 6;
/// The rule above the input spans the full width. It was once capped at 20
/// columns because the block was found by counting cursor-relative rows, so
/// a line that reflowed into two stranded everything above it. The frame is
/// rebuilt absolutely now (`full_frame`) and `sync_size` stops a relative
/// erase from ever running against a width the terminal has abandoned, so
/// the cap bought nothing but a stunted-looking rule.
const RULE_MAX_COLS: usize = 120;
/// How long the resize stream must be quiet before the block is redrawn.
const RESIZE_SETTLE: std::time::Duration = std::time::Duration::from_millis(180);
const TRANSCRIPT_CAP: usize = 10_000;
const ALT_SCREEN_ENTER: &str = "\x1b[?1049h\x1b[?25l";
const ALT_SCREEN_LEAVE: &str = "\x1b[?1049l\x1b[?25h";
/// Terminal title, set once on entry. Shells reset it at their next prompt.
const TITLE: &str = "\x1b]0;butterfly\x07";

/// What a handled input event means for the session loop.
/// A finished tool call, kept beside its preview so the row can be
/// re-composed at any width.
#[derive(Clone, Debug, PartialEq)]
struct ToolRecord {
    failed: bool,
    reason: String,
    elapsed: std::time::Duration,
}

impl ToolRecord {
    fn as_outcome(&self) -> ToolOutcome {
        if self.failed {
            ToolOutcome::Failed {
                reason: self.reason.clone(),
                elapsed: self.elapsed,
            }
        } else {
            ToolOutcome::Ok {
                elapsed: self.elapsed,
            }
        }
    }
}

pub enum UiAction {
    Submit(String),
    /// Shift+Tab: advance the permission mode (ask ⇢ auto ⇢ read-only).
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
    /// How many candidates matched in total, so the list can say what it is
    /// not showing instead of silently stopping at six.
    total: usize,
}

/// The approval options, their access keys, and what each one consents to.
/// Order, keys and decisions are load-bearing safety behaviour (invariants
/// 4-5) and are never reordered for looks; only the labels are presentation.
const MENU: [(&str, char, Decision); 3] = [
    ("yes, run it", 'y', Decision::Approve),
    (
        "yes, and stop asking this session",
        'a',
        Decision::ApproveForSession,
    ),
    ("no, skip it", 'n', Decision::Deny),
];

struct Menu {
    selected: usize,
    /// What the user is deciding about (`edit README.md`), restated beside
    /// the options: the card above can scroll out of reach in a long turn.
    subject: String,
    responder: oneshot::Sender<Decision>,
}

/// In-session /connect flow: pick a provider, paste the key (masked). The
/// key lives only in this buffer until the caller saves it ‒ it never enters
/// the editor history, the transcript, or the session store.
enum ConnectStage {
    Provider { selected: usize },
    Key { buffer: String },
}

struct Connect {
    stage: ConnectStage,
    /// `(provider id, label, already has a key)` choices from the registry.
    providers: Vec<(String, String, bool)>,
    /// Index confirmed at the provider stage (the key stage saves under it).
    chosen: usize,
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
    pub session_microusd: u64,
    pub price_verified: bool,
}

struct Activity {
    started: Instant,
    spinner: usize,
    phase: Phase,
    /// Tool call in flight, with the instant it started, so a finished call
    /// can report how long it actually took.
    tool_started: Option<Instant>,
    out_bytes: u64,
}

/// Where the block lives on screen. A session starts `Pinned`: the screen is
/// taken over (old shell content scrolls into scrollback), content flows
/// from the top, and the block sits at the bottom edge ‒ Claude Code style.
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
///
/// The contract every variant owes `render_entry`: **no returned row may be
/// wider than the width it was rendered at**. The repaint counts one screen
/// row per returned string and draws with autowrap off, so an overlong row
/// does not wrap ‒ it overwrites the last column and strands garbage, which
/// is exactly what a terminal resize used to produce.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Blank,
    /// A row already composed by `chrome` at some width (cards, notes,
    /// decision echoes). Right-aligned content means re-wrapping would be
    /// wrong, so a narrower screen clamps it.
    Fixed(String),
    /// Verbatim content from a fence. Clamping would destroy code, so this
    /// hard-wraps instead ‒ nothing is lost at any width.
    Code(String),
    /// A tool call kept as DATA, not as a string, so it re-composes at
    /// whatever width the screen is now.
    Tool {
        preview: String,
        outcome: Option<ToolRecord>,
    },
    /// A parsed table, laid out at the render width.
    Table(md::Table),
    /// The user's whole prompt (may span lines), drawn as a highlighted bar
    /// with the ❯ marker on the first row only (Codex-style).
    UserBar(String),
    /// Re-wrappable prose. `marker` is the glyph for column 0 of the first
    /// row ‒ `◉` opens an answer, `▪` closes a turn, `None` is a
    /// continuation that simply sits on the content column.
    Prose {
        marker: Option<&'static str>,
        spans: Vec<md::Span>,
        cont_indent: usize,
    },
}

/// Render one entry to physical rows at `width` columns.
///
/// Layout math has a 20-column floor ‒ below that there is no room for the
/// marker grid, and the arithmetic stops meaning anything. A terminal really
/// can be narrower than that, though, and the contract above says no row may
/// exceed the width it was rendered at, so rows laid out against the floor
/// are clamped to the width that actually exists.
fn render_entry(entry: &Entry, width: usize) -> Vec<String> {
    let rows = render_entry_floored(entry, width.max(20));
    if width >= 20 {
        return rows;
    }
    rows.into_iter()
        .map(|row| truncate_styled(&row, width))
        .collect()
}

fn render_entry_floored(entry: &Entry, width: usize) -> Vec<String> {
    match entry {
        Entry::Blank => vec![String::new()],
        Entry::Fixed(line) => vec![if visible_cols(line) <= width {
            line.clone()
        } else {
            truncate_styled(line, width)
        }],
        // Code keeps the shared 2-column grid on every row, wrapped rows
        // included ‒ a continuation starting at column 0 reads as the
        // terminal having mangled the block.
        Entry::Code(line) => th::wrap_styled(line, width.saturating_sub(GRID))
            .into_iter()
            .map(|row| format!("{}{row}", " ".repeat(GRID)))
            .collect(),
        Entry::Tool { preview, outcome } => vec![chrome::tool_line(
            preview,
            outcome.as_ref().map(ToolRecord::as_outcome),
            width,
        )],
        Entry::Table(table) => chrome::table_rows(table, width),
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
                rendered.extend(md::styled_rows(
                    &spans,
                    width.saturating_sub(3),
                    width.saturating_sub(3),
                ));
            }
            // Same vertical grid as every other element: marker in column 0,
            // content in column 2 (❯/◉/⚒/▪ all line up).
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
            marker,
            spans,
            cont_indent,
        } => {
            let rows = md::styled_rows(
                spans,
                width.saturating_sub(GRID),
                width.saturating_sub(GRID + cont_indent),
            );
            let cont_prefix = format!("{}{}", " ".repeat(GRID), " ".repeat(*cont_indent));
            let head = match marker {
                // The answer marker carries the accent, the same one the
                // prompt caret uses; everything else stays quiet.
                Some(glyph) if *glyph == th::MARK_ANSWER => {
                    format!("{ACCENT}{glyph}{RESET} ")
                }
                Some(glyph) => format!("{DIM}{glyph}{RESET} "),
                None => " ".repeat(GRID),
            };
            rows.into_iter()
                .enumerate()
                .map(|(i, row)| {
                    let prefix = if i == 0 { &head } else { &cont_prefix };
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
    /// gets the `◉` marker; continuation lines are indented under it.
    text_block_start: bool,
    /// Whether the last committed scrollback line was blank (vertical
    /// rhythm: a new `◉` block inserts a spacer when it wasn't).
    last_blank: bool,
    /// Stream reasoning to the screen. Off by default, Codex-style; Ctrl+R
    /// toggles live. Hidden reasoning is still recorded in the transcript.
    show_reasoning: bool,
    /// Partial reasoning line buffered while reasoning is hidden.
    hidden_reasoning: String,
    /// Everything committed to scrollback (plus hidden reasoning), as
    /// logical entries re-renderable at any width: backs the Ctrl+T overlay
    /// and the full-screen re-wrap on resize. The bool marks whether the
    /// entry was drawn on the main screen (hidden reasoning is not ‒ the
    /// resize repaint must not resurrect it). Capped ring.
    transcript: std::collections::VecDeque<(bool, Entry)>,
    /// Ctrl+T transcript overlay: Some(scroll offset from the tail). While
    /// open, committed output is queued and flushed on close.
    overlay: Option<usize>,
    pending_output: Vec<Entry>,
    /// Scrolling mode only: the cursor's row within the block at last draw ‒
    /// MoveUp(cursor_row) reaches the block's top line for erase-and-redraw.
    cursor_row: usize,
    /// A resize arrived and has not been settled yet. Held until the event
    /// stream goes quiet so one window drag costs one redraw.
    resize_pending: bool,
    /// When the most recent resize event arrived. A window drag emits one
    /// per pixel step, so the redraw waits for the stream to go quiet ‒
    /// each intermediate redraw would erase the block with geometry the
    /// terminal has already re-wrapped, and those stale rows accumulate.
    resize_at: Option<Instant>,
    trace: Option<Trace>,
}

/// Byte-level trace of everything the TUI writes, plus the size it believes
/// the terminal to be at each resize. Enabled with `BUTTERFLY_TUI_TRACE=<path>`.
///
/// It exists because the obvious way to capture a session ‒ `script` ‒ does
/// NOT propagate window resizes to the process it wraps (verified on macOS:
/// the inner process keeps seeing the original size), so a `script` recording
/// is silent about exactly the behaviour a resize bug lives in. Tracing from
/// inside records the real byte stream AND what the CLI thought the geometry
/// was, with no wrapper in the way.
struct Trace(std::fs::File);

impl Trace {
    fn open() -> Option<Self> {
        let path = std::env::var("BUTTERFLY_TUI_TRACE").ok()?;
        std::fs::File::create(path).ok().map(Trace)
    }

    fn write(&mut self, bytes: &[u8]) {
        let _ = std::io::Write::write_all(&mut self.0, bytes);
    }

    /// Side-channel note, as an APC string: ignorable by any terminal, so a
    /// trace stays replayable byte-for-byte.
    fn note(&mut self, note: &str) {
        self.write(format!("\x1b_bf;{note}\x1b\\").as_bytes());
    }
}

pub struct Ui {
    inner: Mutex<UiInner>,
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
    ///
    /// Vertical order is fixed so the eye always finds the same thing in the
    /// same place: live stream, activity, rule, input (or whichever selector
    /// owns the block), dropdown, footer.
    fn compose_block(&self) -> (Vec<String>, usize, usize) {
        let width = self.width.max(20);
        let mut lines: Vec<String> = Vec::new();
        // A table buffers until it ends, so without this the screen would
        // sit blank for however long the model takes to stream one. Show it
        // growing in the block ‒ the region that is redrawn in place ‒ and
        // commit it once, laid out for real, when it closes.
        if let Some(table) = self.renderer.pending_table() {
            let rows = chrome::table_rows(table, width);
            let shown = rows.len().min(MAX_LIVE_TABLE_ROWS);
            if rows.len() > shown {
                lines.push(format!("{DIM}  ⋮{RESET}"));
            }
            for row in &rows[rows.len() - shown..] {
                lines.push(truncate_styled(row, width));
            }
        } else if let Some(live) = &self.live {
            // The 2-column block marker/indent counts against the width
            // budget: a block line wider than the terminal wraps to two
            // physical rows and corrupts the erase math (stranded frames).
            let plain = tail_cols(&live.text, width);
            let prefixed = tail_cols(&live.text, width.saturating_sub(GRID));
            lines.push(match live.kind {
                LiveKind::Reasoning => format!("{DIM}{plain}{RESET}"),
                LiveKind::Text if self.text_block_start => {
                    format!("{ACCENT}{}{RESET} {prefixed}", th::MARK_ANSWER)
                }
                LiveKind::Text => format!("  {prefixed}"),
                LiveKind::Tool => chrome::tool_line(&live.text, None, width),
            });
        }
        if let Some(activity) = &self.activity {
            lines.push(chrome::activity_line(
                &activity.phase,
                activity.spinner,
                activity.started.elapsed(),
                activity.out_bytes / 4,
                width,
            ));
        }
        // Deliberately SHORT, and never wider than the narrowest terminal
        // we clamp to. Every block line above the cursor must be unable to
        // wrap: a resize erases the block by moving up the cursor row
        // recorded at the last draw, and a terminal that reflows on resize
        // (Ghostty, kitty, iTerm2, Terminal.app) turns a full-width line
        // into two rows ‒ putting the block's top out of the erase's reach
        // and stranding it on screen. A full-bleed rule was also the
        // heaviest thing in the composer; a short one reads as the start of
        // the input rather than a wall.
        lines.push(format!(
            "{DIM}{}{RESET}",
            th::RULE.repeat(RULE_MAX_COLS.min(width))
        ));
        let cursor_row;
        let cursor_col;
        if let Some(connect) = &self.connect {
            match &connect.stage {
                ConnectStage::Provider { selected } => {
                    lines.push(format!("{ACCENT_BOLD}connect a provider{RESET}"));
                    for (i, (_, label, connected)) in connect.providers.iter().enumerate() {
                        let hint = if *connected { "key already saved" } else { "" };
                        lines.push(option_row(label, hint, i == *selected, width));
                    }
                    lines.push(hint_row("enter continue", "esc cancel", width));
                    cursor_row = lines.len().saturating_sub(1);
                    cursor_col = 0;
                }
                ConnectStage::Key { buffer } => {
                    lines.push(format!("{ACCENT_BOLD}paste your API key{RESET}"));
                    let masked = if buffer.is_empty() {
                        String::new()
                    } else {
                        super::connect::masked(buffer.chars().count())
                    };
                    let masked = tail_cols(&masked, width.saturating_sub(4));
                    lines.push(format!("{ACCENT}{}{RESET} {masked}", th::MARK_USER));
                    lines.push(hint_row(
                        "input is masked and never stored in history",
                        "enter save ⋅ esc back",
                        width,
                    ));
                    cursor_row = lines.len().saturating_sub(2);
                    cursor_col = 2 + width_of(&masked);
                }
            }
        } else if let Some(select) = &self.select {
            lines.push(format!("{ACCENT_BOLD}{}{RESET}", select.title));
            let rows = MAX_SELECT_ROWS.min(select.items.len());
            let first = select
                .selected
                .saturating_sub(rows.saturating_sub(1))
                .min(select.items.len().saturating_sub(rows));
            for (i, item) in select.items.iter().enumerate().skip(first).take(rows) {
                lines.push(option_row(item, "", i == select.selected, width));
            }
            let left = if select.items.len() > rows {
                format!("{} of {}", select.selected + 1, select.items.len())
            } else {
                String::new()
            };
            lines.push(hint_row(
                &left,
                "⇡⇣ move ⋅ enter select ⋅ esc cancel",
                width,
            ));
            cursor_row = lines.len().saturating_sub(1);
            cursor_col = 0;
        } else if let Some(menu) = &self.menu {
            lines.push(truncate_styled(
                &format!("{}{BOLD}{}?{RESET}", " ".repeat(GRID), menu.subject),
                width,
            ));
            for (i, (label, key, _)) in MENU.iter().enumerate() {
                lines.push(option_row(
                    label,
                    &key.to_string(),
                    i == menu.selected,
                    width,
                ));
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
            lines.extend(self.compose_dropdown(dropdown, width));
        }
        lines.push(self.compose_status(width));
        (lines, cursor_row, cursor_col)
    }

    /// Completion rows plus, when the list is cut short, what it left out.
    fn compose_dropdown(&self, dropdown: &Dropdown, width: usize) -> Vec<String> {
        // Row budget: 2 prefix cols + label + 2 + hint ≤ width. Wide
        // terminals reserve ~24 cols for the hint; narrow ones give the
        // label everything and drop the hint.
        let label_budget = if width >= 44 {
            width - 26
        } else {
            width.saturating_sub(GRID)
        };
        let shown = dropdown.choices.len().min(MAX_DROPDOWN_ROWS);
        let labels: Vec<String> = dropdown
            .choices
            .iter()
            .take(shown)
            .map(|choice| truncate_cols(&choice.label, label_budget.max(8)))
            .collect();
        // Hints start at one shared column so the eye reads two columns.
        let label_col = labels.iter().map(|l| width_of(l)).max().unwrap_or(0);
        let mut rows: Vec<String> = Vec::new();
        for (i, choice) in dropdown.choices.iter().take(shown).enumerate() {
            let hint_budget = width.saturating_sub(GRID + label_col + 2);
            let hint = if choice.hint.is_empty() || hint_budget < 4 {
                String::new()
            } else {
                truncate_cols(&choice.hint, hint_budget)
            };
            rows.push(chrome::completion_row(
                &th::pad_to(&labels[i], if hint.is_empty() { 0 } else { label_col }),
                &hint,
                i == dropdown.selected,
                width,
            ));
        }
        if dropdown.total > shown {
            rows.push(truncate_styled(
                &format!(
                    "{}{DIM}+{} more ⋅ keep typing to narrow{RESET}",
                    " ".repeat(GRID),
                    dropdown.total - shown,
                ),
                width,
            ));
        }
        rows
    }

    /// Wrapped input rows plus the cursor's (row, column) within them.
    fn compose_input(&self, width: usize) -> (Vec<String>, usize, usize) {
        let prompt = format!("{ACCENT}{}{RESET} ", th::MARK_USER);
        let continuation = "  ";
        let avail = width.saturating_sub(2).max(10);
        let text = self.editor.text();
        if text.is_empty() {
            // The placeholder is the only always-visible teaching surface,
            // so it says what is possible RIGHT NOW rather than one fixed
            // sentence: mid-turn, typing steers instead of starting a turn.
            let (hint, short) = match &self.activity {
                Some(activity) if activity.phase == Phase::Deciding => {
                    ("answer above with ⇡⇣ and enter, or y / a / n", "y / a / n")
                }
                Some(_) => (
                    "steer ‒ your message lands at the next turn boundary",
                    "steer the turn",
                ),
                None => (
                    "ask anything ⋅ @ for a file ⋅ / for commands",
                    "ask anything",
                ),
            };
            // The placeholder is a block line like any other: an unclamped
            // one is what the width invariant exists to prevent.
            let budget = width.saturating_sub(GRID);
            let hint = if width_of(hint) <= budget {
                hint
            } else {
                short
            };
            return (
                vec![format!(
                    "{prompt}{DIM}{}{RESET}",
                    truncate_cols(hint, budget)
                )],
                0,
                2,
            );
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
                let w = th::cell_width(ch);
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

    /// The footer: identity, context pressure and spend on the left, the
    /// permission mode on the right. It carries the running totals so the
    /// per-turn summary above can stay short.
    fn compose_status(&self, width: usize) -> String {
        chrome::footer(
            &chrome::Footer {
                model_id: &self.status.model,
                mode: self.status.mode,
                ctx_used: self.status.ctx_pct,
                session_microusd: self.status.session_microusd,
                price_verified: self.status.price_verified,
            },
            width,
        )
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
                } else if !self.resize_pending {
                    // Mid-drag: the screen is about to be re-wrapped again,
                    // and redrawing now would erase the block using
                    // geometry that no longer holds. `tick` settles it once
                    // the drag stops.
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
        if self.resize_pending {
            // Recorded above; the settle reprojects the whole viewport from
            // the transcript, so this reaches the screen re-wrapped rather
            // than appended at a width that is about to change again.
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

    /// Re-read the terminal's real size before drawing.
    ///
    /// A resize reaches the input thread as an event, but a paint can be
    /// issued from the session thread in the window between the terminal
    /// re-wrapping itself and that event being handled. Drawing then would
    /// append output wrapped to a width that no longer holds, and erase
    /// relative to a block the terminal has already moved. `terminal::size`
    /// is a `TIOCGWINSZ` ioctl ‒ no round trip, nothing to wait for ‒ so the
    /// renderer can simply never treat its width as current without asking.
    ///
    /// Returns true when the size moved, meaning the caller must rebuild the
    /// frame rather than draw against the old one.
    fn sync_size(&mut self) -> bool {
        let Ok((w, h)) = crossterm::terminal::size() else {
            return false;
        };
        let (w, h) = (w as usize, h as usize);
        if w == self.width && h == self.height {
            return false;
        }
        self.width = w;
        self.height = h;
        true
    }

    /// Erase the block, optionally append committed output, redraw the main
    /// screen. Bypasses transcript recording (callers already recorded).
    fn paint_screen(&mut self, output: Option<&str>) {
        if self.sync_size() {
            // The terminal changed shape under us. `output` is already in
            // the transcript (every caller records before it draws), so the
            // settled reprojection will render it at the width that now
            // actually holds ‒ drawing it here would only strand it.
            self.place = Place::Scrolling;
            self.resize_pending = true;
            self.resize_at = Some(Instant::now());
            return;
        }
        let buf = match self.place {
            Place::Pinned { .. } => self.pinned_frame(output),
            Place::Scrolling => self.scrolling_frame(output),
        };
        self.emit(buf.as_bytes());
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
        // Where you are in the scroll matters as much as the keys do: a
        // transcript with no position indicator feels bottomless.
        let position = if total <= rows {
            "all".to_string()
        } else {
            format!("{}‒{} of {total}", start + 1, end)
        };
        let footer = th::justify(
            &format!(" transcript ⋅ {position}"),
            "⇡⇣ pgup/pgdn scroll ⋅ q closes ",
            width,
        );
        buf.push_str(&format!("{BAND}{}{RESET}", truncate_styled(&footer, width)));
        buf.push_str("\x1b[?7h");
        buf.push_str(SYNC_END);
        self.emit(buf.as_bytes());
    }

    fn open_overlay(&mut self) {
        if self.overlay.is_some() || self.menu.is_some() {
            return;
        }
        self.emit(ALT_SCREEN_ENTER.as_bytes());
        self.overlay = Some(0);
        self.draw_overlay();
    }

    fn close_overlay(&mut self) {
        if self.overlay.take().is_none() {
            return;
        }
        {
            self.emit(ALT_SCREEN_LEAVE.as_bytes());
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

    /// Settle a resize by redrawing ONLY the block.
    ///
    /// Committed output belongs to the terminal the moment it is printed ‒
    /// that is the whole inline contract. Repainting it from the transcript
    /// looks tempting (old lines keep their old wrap points) but duplicates
    /// content: narrowing a window makes the terminal reflow its screen
    /// buffer, pushing the overflow up into scrollback, and the repaint then
    /// writes those same rows again. A window drag fires dozens of resize
    /// events, so the same paragraphs pile up over and over ‒ the bug this
    /// replaces. Old lines keep whatever wrap the terminal gives them, which
    /// is how every other inline CLI behaves.
    ///
    /// Settle a resize by RE-PROJECTING the visible screen from the
    /// transcript at the new width, rather than redrawing only the block.
    ///
    /// This is the load-bearing rule of the whole module. Text committed to
    /// scrollback is wrapped at the width in force when it was written, and
    /// those are real newlines in the terminal's grid: a wider window cannot
    /// re-join them (the text keeps its old, narrow column) and a narrower
    /// one shears every row in half against the new edge. Neither is
    /// recoverable by touching the block alone, because the damage is in the
    /// content above it.
    ///
    /// So the screen is redrawn as what it always should have been ‒ a pure
    /// projection of `(transcript, width, height)`. `repaint_all` erases the
    /// viewport absolutely and refills it from the transcript tail, so no
    /// relative cursor arithmetic has to survive the terminal's own reflow
    /// and nothing that is on screen keeps a stale wrap.
    ///
    /// Rows that scrolled ABOVE the viewport keep the wrapping they were
    /// committed with: they belong to the terminal's scrollback and are not
    /// ours to rewrite. Scrolling far enough back therefore crosses a seam.
    /// That is the accepted cost of staying inline and keeping native
    /// scrollback, copy-paste and mouse selection.
    fn repaint_after_resize(&mut self) {
        // Anything committed mid-drag was recorded into the transcript
        // already; the projection below picks it up by construction.
        self.repaint_all();
    }

    /// Draw a complete frame: erase the viewport and rebuild it from the
    /// transcript at the current width, with the block pinned to the bottom
    /// edge. Safe at any point in a session ‒ every committed entry is in
    /// the transcript (`commit_entries` records before it draws), so what is
    /// erased is exactly what is redrawn, re-wrapped to the width now.
    fn repaint_all(&mut self) {
        let buf = self.full_frame();
        self.emit(buf.as_bytes());
    }

    /// The bytes of a complete frame: viewport erased, transcript tail
    /// re-rendered at the current width, block pinned to the bottom edge.
    fn full_frame(&mut self) -> String {
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
            return buf;
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
        self.place = Place::Pinned {
            content_row: rows.len() + 1,
            prev_block_top: block_top,
        };
        self.cursor_row = cursor_row;
        buf
    }

    /// Append the block draw (wrapped in autowrap-off) and cursor placement
    /// for scrolling mode, where positions are cursor-relative.
    fn append_scrolling_block(&mut self, buf: &mut String) {
        let (lines, cursor_row, cursor_col) = self.compose_block();
        // Autowrap is disabled while the block draws: even if a glyph the
        // width tables call narrow renders wide (◉, ⚒, ▸ on some terminals),
        // an overlong line clamps at the right edge instead of wrapping ‒
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
    /// block region ‒ or the terminal is too small ‒ hand off to scrolling.
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
            // Tracked in both modes: a resize hands off to the scrolling
            // model, which reaches the block's top by moving up this many
            // rows from the cursor.
            self.cursor_row = cursor_row;
        }
        buf.push_str(SYNC_END);
        buf
    }

    /// Markdown-render one assistant text line into logical entries:
    /// re-wrappable prose with the `◉` block marker on the first line of a
    /// response block (plus a spacer entry before a new block), code kept
    /// verbatim.
    fn render_text_entries(&mut self, line: &str) -> Vec<Entry> {
        let rendered = self.renderer.render(line);
        self.entries_of(rendered, line)
    }

    /// Anything the markdown renderer still holds (a table that ended the
    /// response) must reach the screen when the text run ends.
    fn flush_markdown(&mut self) -> Vec<Entry> {
        let rendered = self.renderer.flush();
        self.entries_of(rendered, "")
    }

    fn entries_of(&mut self, rendered: Vec<md::Rendered>, line: &str) -> Vec<Entry> {
        /// One blank line is a paragraph break; two in a row is a hole.
        fn push_blank(entries: &mut Vec<Entry>, last_blank: bool) {
            let already = match entries.last() {
                Some(entry) => matches!(entry, Entry::Blank),
                None => last_blank,
            };
            if !already {
                entries.push(Entry::Blank);
            }
        }
        let mut entries = Vec::new();
        for item in rendered {
            match item {
                md::Rendered::Raw(styled) => {
                    if styled.is_empty() {
                        push_blank(&mut entries, self.last_blank);
                    } else {
                        entries.push(Entry::Code(styled));
                    }
                }
                md::Rendered::Table(table) => {
                    // A table is a block: give it the same air as one.
                    if !self.last_blank && !matches!(entries.last(), Some(Entry::Blank)) {
                        entries.push(Entry::Blank);
                    }
                    entries.push(Entry::Table(table));
                    entries.push(Entry::Blank);
                    // The trailing spacer above is the table's own; a blank
                    // line following it in the source must not double it.
                    self.text_block_start = false;
                }
                md::Rendered::Prose { spans, cont_indent } => {
                    if line.trim().is_empty() && spans.iter().all(|s| s.text.trim().is_empty()) {
                        push_blank(&mut entries, self.last_blank);
                        continue;
                    }
                    let starting = self.text_block_start;
                    self.text_block_start = false;
                    if starting && !self.last_blank && !matches!(entries.last(), Some(Entry::Blank))
                    {
                        entries.push(Entry::Blank);
                    }
                    entries.push(Entry::Prose {
                        marker: starting.then_some(th::MARK_ANSWER),
                        spans,
                        cont_indent,
                    });
                }
            }
        }
        entries
    }

    /// One committed line of `kind` as logical entries.
    fn line_entries(&mut self, kind: LiveKind, line: &str) -> Vec<Entry> {
        match kind {
            LiveKind::Reasoning => {
                self.text_block_start = true;
                vec![Entry::Prose {
                    marker: None,
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
                // A tool live line only reaches here when the call never
                // reported a result (an interrupt): record the row without
                // an outcome rather than dropping the evidence that it ran.
                vec![Entry::Tool {
                    preview: line.to_string(),
                    outcome: None,
                }]
            }
        }
    }

    /// Everything the TUI writes goes through here, so a trace is a
    /// complete record of what the terminal was told.
    fn emit(&mut self, bytes: &[u8]) {
        if let Some(trace) = &mut self.trace {
            trace.write(bytes);
        }
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
    }

    /// Drop a pending tool live line without committing it. Used when the
    /// approval card is about to restate the very same call.
    fn discard_live_tool(&mut self) {
        if self
            .live
            .as_ref()
            .is_some_and(|live| live.kind == LiveKind::Tool)
        {
            self.live = None;
        }
    }

    /// Commit the live line (if any) into scrollback, then flush anything
    /// the markdown renderer is still buffering. Both halves matter: a
    /// table that ends the response has no following line to close it.
    fn commit_live(&mut self) {
        let mut entries = Vec::new();
        if let Some(live) = self.live.take() {
            if !live.text.is_empty() {
                let text = live.text;
                entries = self.line_entries(live.kind, &text);
            }
        }
        entries.extend(self.flush_markdown());
        if !entries.is_empty() {
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
                let (choices, total) = self.index.suggest(&trigger, MAX_DROPDOWN_ROWS);
                Dropdown {
                    choices,
                    selected: 0,
                    token_start: trigger.start(),
                    total,
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

    /// Record the answer above the block and hand it to the waiting gate.
    /// The index ⇢ `Decision` mapping is `MENU`'s and is never reinterpreted
    /// here.
    fn resolve_menu(&mut self, choice: usize) {
        if let Some(menu) = self.menu.take() {
            let (label, _, decision) = MENU[choice];
            let color = if decision == Decision::Deny {
                DANGER
            } else {
                DIM
            };
            if let Some(activity) = &mut self.activity {
                activity.phase = Phase::Starting;
            }
            self.paint(Some(&format!(
                "{}{color}↳ {label}{RESET}",
                " ".repeat(GRID)
            )));
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
                resize_pending: false,
                resize_at: None,
                trace: Trace::open(),
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, UiInner> {
        self.inner.lock().expect("ui lock")
    }

    /// Take over the visible screen, Claude Code-style: newline-flood the
    /// old shell content into scrollback (NOT erased ‒ no alternate screen,
    /// history stays reachable), then draw the block at the bottom edge.
    pub fn enter_screen(&self) {
        let mut inner = self.lock();
        // Name the terminal (OSC 0). Multiplexers put it in the pane frame,
        // which is how a zellij or tmux user tells this pane from the shell
        // next to it — Codex and OpenCode both do it.
        inner.emit(TITLE.as_bytes());
        let flood = "\n".repeat(inner.height.max(1));
        inner.emit(flood.as_bytes());
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
        self.print_marked(None, text);
    }

    /// Commit plain text as dim, word-wrapped, re-wrappable prose under
    /// `marker` (turn receipts and other long notes that must survive a
    /// resize tidily).
    pub fn print_marked(&self, marker: Option<&'static str>, text: &str) {
        let mut inner = self.lock();
        inner.commit_live();
        // A marked block opens a new unit of meaning; give it the same
        // breathing room an answer block gets.
        let mut entries: Vec<Entry> = Vec::new();
        if marker.is_some() && !inner.last_blank {
            entries.push(Entry::Blank);
        }
        entries.extend(text.lines().enumerate().map(|(i, line)| Entry::Prose {
            marker: if i == 0 { marker } else { None },
            spans: vec![md::Span {
                style: md::SpanStyle::Dim,
                text: line.to_string(),
            }],
            cont_indent: 2,
        }));
        inner.commit_entries(entries);
    }

    /// Draw the opening card at the current width.
    pub fn print_welcome(&self, info: &chrome::Welcome<'_>) {
        let mut inner = self.lock();
        let width = inner.width.max(20);
        let rows = chrome::welcome(info, width);
        inner.paint(Some(&rows.join("\n")));
    }

    /// Lay out model-picker rows at the current width (the picker itself is
    /// the generic selector, which takes ready-made strings).
    pub fn render_model_rows(&self, rows: &[chrome::ModelRow<'_>]) -> Vec<String> {
        let width = self.lock().width.max(20);
        // The selector prefixes a marker and a space, and reserves the same
        // on the right, so rows compose at the width they will be drawn at.
        chrome::model_rows(rows, width.saturating_sub(chrome::GRID * 2))
    }

    /// Current terminal width, for surfaces composed outside this module.
    pub fn width(&self) -> usize {
        self.lock().width.max(20)
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
            phase: Phase::Starting,
            tool_started: None,
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
                marker: None,
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

    /// Called on the input thread's poll timeout ‒ i.e. whenever the event
    /// stream has been quiet for a moment. Advances the spinner and settles
    /// a resize once the user stops dragging.
    pub fn tick(&self) {
        let mut inner = self.lock();
        let settled = inner
            .resize_at
            .is_some_and(|at| at.elapsed() >= RESIZE_SETTLE);
        if inner.resize_pending && settled {
            inner.resize_pending = false;
            inner.resize_at = None;
            let (w, h) = (inner.width, inner.height);
            if let Some(trace) = &mut inner.trace {
                trace.note(&format!("settle;{w}x{h};reproject"));
            }
            inner.repaint_after_resize();
            return;
        }
        if inner.resize_pending {
            // Still dragging: hold the screen rather than redraw into
            // geometry that is about to change again.
            return;
        }
        if let Some(activity) = &mut inner.activity {
            activity.spinner += 1;
            inner.paint(None);
        }
    }

    /// Open the /connect flow; resolves with Some((provider, api_key)) on
    /// submit or None if the user backs out. The key is never echoed,
    /// recorded, or persisted by the UI ‒ the caller owns saving it.
    pub fn open_connect(
        &self,
        providers: Vec<(String, String, bool)>,
    ) -> oneshot::Receiver<Option<(String, String)>> {
        let mut inner = self.lock();
        let (sender, receiver) = oneshot::channel();
        if inner.connect.is_some() || inner.menu.is_some() || providers.is_empty() {
            // Already mid-flow: refuse a second concurrent one.
            let _ = sender.send(None);
            return receiver;
        }
        inner.connect = Some(Connect {
            stage: ConnectStage::Provider { selected: 0 },
            providers,
            chosen: 0,
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
        // The pending `⚒` line is dropped rather than committed: the card
        // below restates the same call, and seeing it three times (live
        // line, card, spinner) at the one moment that needs attention is
        // noise. `on_tool_result` re-prints the row once the call runs.
        inner.discard_live_tool();
        inner.commit_live();
        let width = inner.width.max(20);
        let mut card = chrome::approval_card(request, request.detail.as_deref(), width);
        if !inner.last_blank {
            card.insert(0, String::new());
        }
        let (tool, target) = chrome::split_preview(&request.preview);
        let (sender, receiver) = oneshot::channel();
        inner.menu = Some(Menu {
            selected: 0,
            subject: chrome::approval_subject(tool, target),
            responder: sender,
        });
        // Nothing is running while the user decides, and Esc means "no"
        // here ‒ the spinner's interrupt hint would misdescribe the key.
        if let Some(activity) = &mut inner.activity {
            activity.phase = Phase::Deciding;
        }
        inner.paint(Some(&card.join("\n")));
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
        inner.emit(buf.as_bytes());
        inner.editor.save_history();
    }

    /// Handle one terminal event. Editing is absorbed internally; only
    /// session-relevant actions surface.
    pub fn handle_event(&self, event: Event) -> Option<UiAction> {
        let mut inner = self.lock();
        match event {
            Event::Resize(w, h) => {
                let changed = inner.width != w as usize || inner.height != h as usize;
                if let Some(trace) = &mut inner.trace {
                    trace.note(&format!("resize;{w}x{h};changed={changed}"));
                }
                inner.width = w as usize;
                inner.height = h as usize;
                if inner.overlay.is_some() {
                    // The overlay owns the alternate screen outright, so it
                    // can safely re-wrap its whole view from the transcript.
                    inner.draw_overlay();
                } else {
                    // Absolute row arithmetic cannot survive a resize, so
                    // hand off to the cursor-relative model immediately ‒
                    // any paint arriving before the redraw settles (a
                    // stream delta, say) must not use the old geometry.
                    inner.place = Place::Scrolling;
                    // A width change re-wraps the content; a height change
                    // moves the bottom edge the block is pinned to. Both
                    // invalidate the frame, so both are settled.
                    //
                    // Dragging a window edge emits a resize per pixel step.
                    // Redrawing on each one is both wasteful and visibly
                    // jumpy, so the work is deferred to `tick`, which runs
                    // when the event stream goes quiet ‒ once per drag.
                    if changed {
                        inner.resize_pending = true;
                        inner.resize_at = Some(Instant::now());
                    }
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
                        *selected = (*selected + 1).min(connect.providers.len().saturating_sub(1));
                    }
                    (ConnectStage::Provider { selected }, KeyCode::Enter) => {
                        connect.chosen = *selected;
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
                            let provider = connect
                                .providers
                                .get(connect.chosen)
                                .map(|(id, ..)| id.clone())
                                .unwrap_or_default();
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
        let mut inner = self.ui.lock();
        if let Some(activity) = &mut inner.activity {
            if activity.phase != Phase::Deciding {
                activity.phase = Phase::Writing;
            }
        }
        inner.stream(LiveKind::Text, delta);
    }

    fn on_reasoning(&mut self, delta: &str) {
        let mut inner = self.ui.lock();
        if let Some(activity) = &mut inner.activity {
            if activity.phase != Phase::Deciding {
                activity.phase = Phase::Thinking;
            }
        }
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
                    marker: None,
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
            activity.phase = Phase::Running(preview.to_string());
            activity.tool_started = Some(Instant::now());
        }
        // The live line holds the raw core preview; `chrome::tool_line`
        // turns it into the aligned row at paint time, at the current width.
        inner.live = Some(LiveLine {
            kind: LiveKind::Tool,
            text: preview.to_string(),
        });
        inner.paint(None);
    }

    fn on_tool_result(&mut self, preview: &str, is_error: bool, error: Option<&str>) {
        let mut inner = self.ui.lock();
        let elapsed = inner
            .activity
            .as_ref()
            .and_then(|activity| activity.tool_started)
            .map(|started| started.elapsed())
            .unwrap_or_default();
        let outcome = if is_error {
            ToolOutcome::Failed {
                reason: error.unwrap_or_default().to_string(),
                elapsed,
            }
        } else {
            ToolOutcome::Ok { elapsed }
        };
        let width = inner.width.max(20);
        // Prefer the live line's own preview; fall back to the result's when
        // an approval prompt consumed it.
        let text = match inner
            .live
            .as_ref()
            .filter(|live| live.kind == LiveKind::Tool)
        {
            Some(_) => inner.live.take().expect("tool live line").text,
            None => preview.to_string(),
        };
        inner.paint(Some(&chrome::tool_line(&text, Some(outcome), width)));
        inner.text_block_start = true;
        if let Some(activity) = &mut inner.activity {
            activity.phase = Phase::Thinking;
            activity.tool_started = None;
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

    /// A frame's visible text: escapes stripped, `\r\n` folded to rows.
    fn plain(frame: &str) -> String {
        let mut out = String::new();
        let mut chars = frame.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\x1b' => {
                    if chars.peek() == Some(&'[') {
                        chars.next();
                        for esc in chars.by_ref() {
                            if esc.is_ascii_alphabetic() {
                                // A cursor jump starts a new screen row just
                                // as surely as a newline does.
                                if esc == 'H' {
                                    out.push('\n');
                                }
                                break;
                            }
                        }
                    }
                }
                '\r' => {}
                other => out.push(other),
            }
        }
        out
    }

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
                session_microusd: 2_916,
                price_verified: true,
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
            resize_pending: false,
            resize_at: None,
            trace: None,
        }
    }

    /// The invariant behind the erase math: no composed block line may be
    /// wider than the terminal, or it wraps to two physical rows and every
    /// later repaint strands a stale frame in scrollback (user-reported).
    #[test]
    fn no_block_line_exceeds_terminal_width() {
        for width in [24usize, 40, 60, 80] {
            let mut inner = inner_at_width(width);
            // Long streaming text line WITH the ◉ prefix budget in play.
            inner.live = Some(LiveLine {
                kind: LiveKind::Text,
                text: "x".repeat(width * 3),
            });
            inner.activity = Some(Activity {
                started: Instant::now(),
                spinner: 3,
                phase: Phase::Running("read_file(some/deeply/nested/path.rs)".into()),
                tool_started: Some(Instant::now()),
                out_bytes: 123_456,
            });
            inner.dropdown = Some(Dropdown {
                choices: vec![super::super::completion::Choice {
                    insert: "@x".into(),
                    label: "a/very/long/path/that/goes/on/forever/and/ever/mod.rs".into(),
                    hint: "struct ⋅ a/very/long/location.rs:1234".into(),
                    kind: ChoiceKind::Symbol,
                }],
                selected: 0,
                token_start: 0,
                total: 42,
            });
            // A table still buffering previews in the block, so it is
            // subject to the same width budget as everything else.
            inner.renderer.render("| Metric | Value | Where |");
            inner.renderer.render("|---|---:|---|");
            inner.renderer.render(&format!(
                "| {} | {} | {} |",
                "a-very-long-metric-name".repeat(2),
                "~46.9k lines Python / 393 files, plus YAML",
                "crates/core/src/failure.rs:294-313",
            ));
            assert!(inner.renderer.pending_table().is_some());
            // Both composer states: the empty placeholder and typed text.
            for typed in [false, true] {
                if typed {
                    inner
                        .editor
                        .insert_str("some typed input that is fairly long too");
                }
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

    /// The contract `render_entry` owes the repaint, for EVERY variant:
    /// one returned string is exactly one screen row, and it fits.
    ///
    /// Both halves have been violated in production. An over-wide row
    /// overwrites the last column with garbage when autowrap is off (a
    /// terminal resize re-rendered pre-composed rows at the old width); a
    /// row carrying an embedded newline moves the cursor down WITHOUT a
    /// carriage return, stranding text at the right edge (table rows were
    /// returned joined by newlines).
    #[test]
    fn every_entry_renders_to_single_rows_that_fit() {
        let long = "supercalifragilistic ".repeat(12);
        let table = md::Table {
            headers: vec![
                vec![md::Span {
                    style: md::SpanStyle::Plain,
                    text: "Metric".into(),
                }],
                vec![md::Span {
                    style: md::SpanStyle::Plain,
                    text: "Value".into(),
                }],
            ],
            aligns: vec![md::Align::Left, md::Align::Right],
            rows: vec![
                vec![
                    vec![md::Span {
                        style: md::SpanStyle::Plain,
                        text: "Code".into(),
                    }],
                    vec![md::Span {
                        style: md::SpanStyle::Code,
                        text: long.clone(),
                    }],
                ],
                vec![
                    vec![md::Span {
                        style: md::SpanStyle::Plain,
                        text: "Commits".into(),
                    }],
                    vec![md::Span {
                        style: md::SpanStyle::Plain,
                        text: "346".into(),
                    }],
                ],
            ],
        };
        let entries = vec![
            Entry::Blank,
            // Composed for a 200-column screen, rendered on a narrow one.
            Entry::Fixed(chrome::tool_line(&format!("read_file({long})"), None, 200)),
            Entry::Code(format!("{ACCENT}{long}{RESET}")),
            Entry::Tool {
                preview: format!("run_command(cargo test {long})"),
                outcome: Some(ToolRecord {
                    failed: true,
                    reason: long.clone(),
                    elapsed: std::time::Duration::from_millis(1200),
                }),
            },
            Entry::UserBar(format!("{long}\n\nand a second paragraph")),
            Entry::Prose {
                marker: Some(th::MARK_ANSWER),
                spans: vec![md::Span {
                    style: md::SpanStyle::Bold,
                    text: long.clone(),
                }],
                cont_indent: 4,
            },
            Entry::Table(table),
        ];
        for width in [1usize, 2, 5, 12, 16, 19, 20, 21, 24, 31, 40, 68, 80, 120] {
            for entry in &entries {
                for row in render_entry(entry, width) {
                    assert!(
                        !row.contains('\n') && !row.contains('\r'),
                        "embedded newline at {width}: {row:?}"
                    );
                    // The real width, not the layout floor: a terminal
                    // narrower than 20 columns still must not be overflowed.
                    assert!(
                        visible_cols(&row) <= width,
                        "row exceeds {width} cols ({}): {row:?}",
                        visible_cols(&row),
                    );
                }
            }
        }
    }

    /// Nothing the composer draws ABOVE the cursor may be wide enough to
    /// reflow.
    ///
    /// A resize erases the block by moving up the cursor row recorded at
    /// the last draw. Terminals that reflow on resize (Ghostty, kitty,
    /// iTerm2, Terminal.app) re-wrap their own screen buffer first, so a
    /// full-width line above the cursor becomes TWO rows and the block's top
    /// moves out of the erase's reach ‒ it survives on screen as a ghost
    /// composer. That is what a full-width composer rule did: measured with
    /// a reflow-faithful replay, one resize left two rules and two footers.
    ///
    /// Keeping these lines inside the 20-column floor `compose_block` clamps
    /// to makes them unwrappable at ANY terminal size, so the recorded
    /// cursor row is exact on reflowing and non-reflowing terminals alike.
    #[test]
    fn the_block_draws_no_ambiguous_width_glyph() {
        use unicode_width::UnicodeWidthChar as _;
        // The block is the rule, the input, the hint row and the footer --
        // every one of them width-critical. A glyph the terminal may render
        // two columns wide turns "exactly the width" into "double it".
        for width in [24usize, 40, 80, 100, 120, 160] {
            let mut inner = inner_at_width(width);
            inner.editor.insert_str("a half-typed question");
            let (lines, _, _) = inner.compose_block();
            for line in &lines {
                for ch in line.chars() {
                    assert!(
                        ch.width() == ch.width_cjk(),
                        "ambiguous glyph {ch:?} (U+{:04X}) in a block line at \
                         {width} cols: {line:?}",
                        ch as u32
                    );
                }
            }
        }
    }

    #[test]
    fn no_block_line_can_reflow_at_any_width() {
        // The 20-column cap on lines above the cursor is retired: the frame
        // is rebuilt absolutely after a resize and `sync_size` keeps the
        // relative erase from running against a stale width. What still has
        // to hold is that no block line exceeds the width it was composed
        // at ‒ the block draws with autowrap off, so an overlong line
        // overwrites the last column instead of wrapping.
        for width in [20usize, 21, 40, 80, 119, 120, 121, 200] {
            let mut inner = inner_at_width(width);
            inner.editor.insert_str("a half-typed question");
            let (lines, _, _) = inner.compose_block();
            for line in &lines {
                assert!(
                    visible_cols(line) <= width.max(20),
                    "composed at {width}, a {}-col block line would overflow: {line:?}",
                    visible_cols(line),
                );
            }
        }
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
            phase: Phase::Thinking,
            tool_started: None,
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
        // Spacer first (previous line was not blank), then ◉ + indented rows.
        assert_eq!(lines[0], "");
        assert!(
            lines[1].starts_with(&format!("{ACCENT}◉{RESET} ")),
            "{:?}",
            lines[1]
        );
        assert!(lines[2].starts_with("  "));
        for line in &lines[1..] {
            assert!(visible_cols(line) <= 30, "row too wide: {line:?}");
            // No mid-word cuts: each row ends at a word boundary.
            assert!(!line.ends_with(' '));
        }
        assert!(lines[1].ends_with("fox") || !lines[1].contains("jumps over the lazy"));
        // Markers are styled but occupy the same two columns as before, so
        // the grid survives: content starts at column 2 on every row.
        assert_eq!(
            visible_cols(&lines[1][..lines[1].find('T').expect("prose")]),
            2
        );
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
        assert!(lines[0].contains('‣'));
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
        let rows = md::styled_rows(&spans, 20, 20);
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
        let rows = md::styled_rows(&spans, 20, 20);
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
            marker: Some(th::MARK_ANSWER),
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
        assert!(
            wide[0].starts_with(&format!("{ACCENT}◉{RESET} ")),
            "{:?}",
            wide[0]
        );
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

    /// A resize must never re-emit committed output.
    ///
    /// It used to repaint the visible screen from the transcript, which
    /// reads as reasonable until you remember that narrowing a window makes
    /// the terminal reflow its own screen buffer and push the overflow into
    /// scrollback ‒ the repaint then writes those rows a second time. One
    /// window drag fires dozens of resize events, so whole paragraphs piled
    /// up over and over (measured: the same sentence emitted 29 times in a
    /// single drag). Committed output belongs to the terminal.
    /// The whole point of the cursor-position report: it tells us whether
    /// the terminal re-wrapped its screen buffer, which decides how far up
    /// the block's top actually is.
    ///
    /// A block drawn at 300 columns whose above-cursor lines are wide
    /// occupies more rows once a reflowing terminal re-wraps it at 150. Both
    /// hypotheses predict a different cursor row ‒ the block's last row sits
    /// on the screen's last row ‒ so the reported row picks the right one.
    #[test]
    fn a_resize_reprojects_committed_content_at_the_new_width() {
        // The bug this whole path exists for: text committed at one width
        // keeps that wrap forever, because the terminal owns those newlines.
        // A settled resize must redraw it from the transcript instead.
        let sentence = "the screen is a deterministic projection of application \
             state and the current terminal dimensions, never a pile of mutations";
        let mut inner = inner_at_width(100);
        inner.height = 24;
        inner.record(
            true,
            Entry::Prose {
                marker: Some(th::MARK_ANSWER),
                spans: vec![md::Span {
                    style: md::SpanStyle::Plain,
                    text: sentence.to_string(),
                }],
                cont_indent: 0,
            },
        );

        // Narrow: every row must fit the new width, not shear against it.
        inner.width = 60;
        let narrow = inner.full_frame();
        assert!(
            narrow.contains("\x1b[2J"),
            "viewport not erased: {narrow:?}"
        );
        for row in plain(&narrow).lines() {
            assert!(
                visible_cols(row) <= 60,
                "row overflows the narrowed screen: {row:?}"
            );
        }
        // Wide: the SAME entry now occupies fewer rows ‒ it re-expanded
        // instead of keeping the wrap it was committed with.
        let entry = &inner.transcript.back().expect("recorded").1;
        let (at60, at100, at160) = (
            render_entry(entry, 60).len(),
            render_entry(entry, 100).len(),
            render_entry(entry, 160).len(),
        );
        assert!(
            at60 > at100 && at100 > at160,
            "entry did not re-wrap with width: 60->{at60} 100->{at100} 160->{at160}"
        );

        inner.width = 160;
        let wide = inner.full_frame();
        // And the widened frame really is using the extra columns, rather
        // than redrawing the old narrow shape into a bigger window.
        assert!(
            plain(&wide)
                .lines()
                .any(|row| visible_cols(row) > 100 && row.contains("projection")),
            "widened frame never exceeds the old width"
        );
        for row in plain(&wide).lines() {
            assert!(visible_cols(row) <= 160, "row overflows: {row:?}");
        }
    }

    #[test]
    fn a_resize_rebuilds_the_frame_from_the_transcript_tail() {
        let mut inner = inner_at_width(80);
        inner.height = 24;
        for i in 0..40 {
            inner.record(true, Entry::Fixed(format!("committed line {i}")));
        }
        inner.editor.insert_str("a half-typed question");
        inner.width = 60;
        let frame = inner.full_frame();
        // The viewport is rebuilt absolutely: no relative cursor arithmetic
        // has to survive the terminal's own reflow.
        assert!(
            frame.contains("\x1b[2J\x1b[1;1H"),
            "no absolute erase: {frame:?}"
        );
        // It refills from the TAIL, so the newest lines are there and the
        // ones that scrolled away stay in scrollback where they belong.
        let text = plain(&frame);
        assert!(text.contains("committed line 39"), "tail missing: {text:?}");
        assert!(
            !text.contains("committed line 0\n"),
            "refilled too far back"
        );
        // Editor state survives the resize untouched.
        assert!(text.contains("a half-typed question"), "input lost");
        assert!(text.contains("read-only"), "footer lost");
        // Block pinned to the bottom edge, content above it.
        assert!(matches!(inner.place, Place::Pinned { .. }));
    }

    #[test]
    fn truncate_styled_counts_visible_columns_only() {
        let styled = format!("{}0123456789{}0123456789", md::CYAN, md::RESET);
        let cut = truncate_styled(&styled, 12);
        assert!(visible_cols(&cut) <= 12);
        assert!(cut.contains('⋯'));
        assert!(cut.starts_with(md::CYAN));
        let fits = truncate_styled("short", 12);
        assert!(fits.starts_with("short"));
    }
}

//! The shared inline-TUI surface, on the same model Codex's TUI uses.
//!
//! Committed output is written into the terminal's own scrollback ONCE, as
//! pre-wrapped hard-terminated rows (`Term::insert_history`), and is never
//! redrawn. The renderer owns exactly one region — the composer block at the
//! bottom of the screen — and a paint re-lays only that. No alternate screen
//! during a session, so scrollback, copy-paste and mouse selection behave
//! normally; the Ctrl+T transcript overlay is the one sanctioned use of it.
//!
//! Colors and width primitives live in `theme`; every composed surface
//! (welcome card, footer, tool rows, approval card, help) lives in
//! `chrome`; all terminal I/O lives in `term`. This module owns state,
//! layout math and input.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use butterfly_core::provider::StreamSink;
use butterfly_core::session::approval::{ApprovalGate, ApprovalRequest, Decision};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::text::{Line, Span};
use tokio::sync::oneshot;

use super::chrome::{self, hint_row, option_row, option_text, Phase, ToolOutcome, GRID};
use super::completion::{active_trigger, ChoiceKind, CompletionIndex};
use super::editor::Editor;
use super::markdown::{self as md, MarkdownRenderer};
use super::term::Term;
use super::theme::{
    self as th, blank, indent, line_cols, span, tail_cols, truncate_cols, truncate_line, width_of,
};

const MAX_INPUT_ROWS: usize = 8;
const MAX_DROPDOWN_ROWS: usize = 6;
/// The model picker can outgrow the block; scroll it instead of letting it
/// push the composer off a short terminal.
const MAX_SELECT_ROWS: usize = 8;
/// How much of a still-streaming table the block previews. Bounded so a
/// long table cannot push the composer off a short terminal.
const MAX_LIVE_TABLE_ROWS: usize = 6;
/// How long the resize stream must be quiet before the frame is rebuilt.
///
/// Cell diffing makes a repaint cheap, but the terminal's OWN reflow during
/// a drag is not ours to race: a frame drawn mid-drag lays out against
/// geometry the terminal is about to change again. One drag therefore costs
/// one redraw.
const RESIZE_SETTLE: std::time::Duration = std::time::Duration::from_millis(180);
const TRANSCRIPT_CAP: usize = 10_000;
/// Rows replayed into scrollback when a resize reflows the session. Past this
/// the terminal is dropping them anyway, and a window drag must not get slower
/// the longer the session has run.
const REFLOW_ROW_CAP: usize = 4_000;
/// Below this there is no room for the marker grid and the layout
/// arithmetic stops meaning anything; rows composed against the floor are
/// clamped to the width that actually exists.
const MIN_LAYOUT_COLS: usize = 20;

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

/// What a handled input event means for the session loop.
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
    items: Vec<Line<'static>>,
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

/// One logical transcript entry. Entries render to physical rows at a GIVEN
/// width, so the visible screen can be re-wrapped from source on resize and
/// the Ctrl+T overlay wraps instead of truncating.
///
/// The contract every variant owes `render_entry`: **one returned `Line` is
/// exactly one screen row, and it fits the width it was rendered at**. The
/// frame places one line per screen row, so an overlong row would be
/// silently cut by the buffer and a row carrying a newline would corrupt the
/// grid. Entries are therefore kept as DATA (preview + outcome, parsed
/// table, spans), never as pre-rendered rows — except `Fixed`, which is a
/// row already composed at some width and is clamped when the screen is
/// narrower.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Blank,
    /// A row already composed by `chrome` (cards, notes, decision echoes).
    /// Right-aligned content means re-wrapping would be wrong, so a
    /// narrower screen clamps it.
    Fixed(Line<'static>),
    /// Verbatim content from a fence. Clamping would destroy code, so this
    /// hard-wraps instead ‒ nothing is lost at any width.
    Code(Line<'static>),
    /// A tool call kept as DATA, not as a row, so it re-composes at
    /// whatever width the screen is now.
    Tool {
        preview: String,
        outcome: Option<ToolRecord>,
    },
    /// A parsed table, laid out at the render width.
    Table(md::Table),
    /// The opening card, kept as data for the same reason `Tool` is: it is a
    /// box, and a box that can only be clamped loses its right border the
    /// first time the window narrows.
    Welcome(chrome::Welcome),
    /// The user's whole prompt (may span lines), drawn as a highlighted bar
    /// with the ❯ marker on the first row only.
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
fn render_entry(entry: &Entry, width: usize) -> Vec<Line<'static>> {
    let rows = render_entry_floored(entry, width.max(MIN_LAYOUT_COLS));
    if width >= MIN_LAYOUT_COLS {
        return rows;
    }
    rows.into_iter()
        .map(|row| truncate_line(row, width))
        .collect()
}

fn render_entry_floored(entry: &Entry, width: usize) -> Vec<Line<'static>> {
    match entry {
        Entry::Blank => vec![blank()],
        Entry::Fixed(line) => vec![truncate_line(line.clone(), width)],
        // Code keeps the shared 2-column grid on every row, wrapped rows
        // included ‒ a continuation starting at column 0 reads as the
        // terminal having mangled the block.
        Entry::Code(line) => th::wrap_line(line, width.saturating_sub(GRID))
            .into_iter()
            .map(|row| {
                let mut spans = vec![indent(GRID)];
                spans.extend(row.spans);
                Line::from(spans)
            })
            .collect(),
        Entry::Tool { preview, outcome } => vec![chrome::tool_line(
            preview,
            outcome.as_ref().map(ToolRecord::as_outcome),
            width,
        )],
        Entry::Table(table) => chrome::table_rows(table, width),
        Entry::Welcome(info) => chrome::welcome(info, width),
        Entry::UserBar(text) => {
            let measure = width;
            let mut rendered: Vec<Line<'static>> = Vec::new();
            for line in text.trim_end_matches('\n').split('\n') {
                if line.is_empty() {
                    rendered.push(blank());
                    continue;
                }
                let spans = vec![md::Span {
                    style: md::SpanStyle::Plain,
                    text: line.to_string(),
                }];
                rendered.extend(md::styled_lines(
                    &spans,
                    measure.saturating_sub(GRID),
                    measure.saturating_sub(GRID),
                ));
            }
            // Codex marks a user turn with a dim caret and nothing else — no
            // filled band. The marker sits in column 0, content in column 2,
            // the same grid everything else uses.
            rendered
                .into_iter()
                .enumerate()
                .map(|(i, line)| {
                    let mut spans = if i == 0 {
                        vec![span(
                            format!("{} ", th::MARK_USER),
                            th::dim().add_modifier(ratatui::style::Modifier::BOLD),
                        )]
                    } else {
                        vec![indent(GRID)]
                    };
                    spans.extend(line.spans);
                    Line::from(spans)
                })
                .collect()
        }
        Entry::Prose {
            marker,
            spans,
            cont_indent,
        } => {
            // Prose uses the WHOLE terminal, as Codex does. A typographic
            // measure cap was tried here and the user rejected it on sight:
            // on a wide window it leaves most of the screen black, and a
            // terminal is not a book — the reader chose that width.
            let measure = width;
            let rows = md::styled_lines(
                spans,
                measure.saturating_sub(GRID),
                measure.saturating_sub(GRID + cont_indent),
            );
            let head: Vec<Span<'static>> = match marker {
                // The answer marker carries the accent, the same one the
                // prompt caret uses; everything else stays quiet.
                Some(glyph) if *glyph == th::MARK_ANSWER => {
                    vec![span(*glyph, th::accent()), indent(1)]
                }
                Some(glyph) => vec![span(*glyph, th::dim()), indent(1)],
                None => vec![indent(GRID)],
            };
            rows.into_iter()
                .enumerate()
                .map(|(i, row)| {
                    let mut out = if i == 0 {
                        head.clone()
                    } else {
                        vec![indent(GRID + cont_indent)]
                    };
                    out.extend(row.spans);
                    Line::from(out)
                })
                .collect()
        }
    }
}

struct UiInner {
    term: Term,
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
    /// Stream reasoning to the screen. Off by default; Ctrl+R toggles live.
    /// Hidden reasoning is still recorded in the transcript.
    show_reasoning: bool,
    /// Partial reasoning line buffered while reasoning is hidden.
    hidden_reasoning: String,
    /// Everything committed to scrollback (plus hidden reasoning), as
    /// logical entries re-renderable at any width: backs the Ctrl+T overlay
    /// and the re-wrap on resize. The bool marks whether the entry was drawn
    /// on the main screen (hidden reasoning is not ‒ the resize repaint must
    /// not resurrect it). Capped ring.
    transcript: std::collections::VecDeque<Entry>,
    /// Ctrl+T transcript overlay: Some(scroll offset from the tail). While
    /// open, committed output is queued and flushed on close.
    overlay: Option<usize>,
    pending_output: Vec<Entry>,
    /// A resize arrived and has not been settled yet.
    resize_pending: bool,
    /// When the most recent resize event arrived. A window drag emits one
    /// per pixel step, so the redraw waits for the stream to go quiet.
    resize_at: Option<Instant>,
}

pub struct Ui {
    inner: Mutex<UiInner>,
}

impl UiInner {
    fn width(&self) -> usize {
        self.term.size().0.max(MIN_LAYOUT_COLS)
    }

    fn height(&self) -> usize {
        self.term.size().1.max(4)
    }

    /// The terminal cursor is hidden while a selector owns the block (there
    /// is nothing to type at); it shows for the editor and key entry.
    fn selecting(&self) -> bool {
        self.menu.is_some()
            || self.select.is_some()
            || matches!(
                &self.connect,
                Some(Connect {
                    stage: ConnectStage::Provider { .. },
                    ..
                })
            )
    }

    /// Compose the block: (rows, cursor row, cursor column).
    ///
    /// Vertical order is fixed so the eye always finds the same thing in the
    /// same place: live stream, activity, rule, input (or whichever selector
    /// owns the block), dropdown, footer.
    fn compose_block(&self) -> (Vec<Line<'static>>, usize, usize) {
        let width = self.width();
        let mut lines: Vec<Line<'static>> = Vec::new();
        // A table buffers until it ends, so without this the screen would
        // sit blank for however long the model takes to stream one. Show it
        // growing in the block ‒ the region that is redrawn in place ‒ and
        // commit it once, laid out for real, when it closes.
        if let Some(table) = self.renderer.pending_table() {
            let rows = chrome::table_rows(table, width);
            let shown = rows.len().min(MAX_LIVE_TABLE_ROWS);
            if rows.len() > shown {
                lines.push(Line::from(vec![indent(GRID), span("⋮", th::dim())]));
            }
            for row in &rows[rows.len() - shown..] {
                lines.push(truncate_line(row.clone(), width));
            }
        } else if let Some(live) = &self.live {
            // The 2-column block marker/indent counts against the width
            // budget: a block line wider than the terminal would be cut by
            // the frame instead of clamped where the layout intended.
            let plain = tail_cols(&live.text, width);
            let prefixed = tail_cols(&live.text, width.saturating_sub(GRID));
            lines.push(match live.kind {
                LiveKind::Reasoning => Line::from(vec![span(plain, th::dim())]),
                LiveKind::Text if self.text_block_start => Line::from(vec![
                    span(th::MARK_ANSWER, th::accent()),
                    indent(1),
                    th::text(prefixed),
                ]),
                LiveKind::Text => Line::from(vec![indent(GRID), th::text(prefixed)]),
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
        // No rule above the composer: Codex separates it with a blank row,
        // which reads as air rather than as a wall.
        lines.push(blank());
        let cursor_row;
        let cursor_col;
        if let Some(connect) = &self.connect {
            match &connect.stage {
                ConnectStage::Provider { selected } => {
                    lines.push(Line::from(vec![span(
                        "connect a provider",
                        th::accent_bold(),
                    )]));
                    for (i, (_, label, connected)) in connect.providers.iter().enumerate() {
                        let hint = if *connected { "key already saved" } else { "" };
                        lines.push(option_text(label, hint, i == *selected, width));
                    }
                    lines.push(hint_row("enter continue", "esc cancel", width));
                    cursor_row = lines.len().saturating_sub(1);
                    cursor_col = 0;
                }
                ConnectStage::Key { buffer } => {
                    lines.push(Line::from(vec![span(
                        "paste your API key",
                        th::accent_bold(),
                    )]));
                    let masked = if buffer.is_empty() {
                        String::new()
                    } else {
                        super::connect::masked(buffer.chars().count())
                    };
                    let masked = tail_cols(&masked, width.saturating_sub(4));
                    lines.push(Line::from(vec![
                        span(th::MARK_USER, th::accent()),
                        indent(1),
                        th::text(masked.clone()),
                    ]));
                    lines.push(hint_row(
                        "input is masked and never stored in history",
                        &format!("enter save {} esc back", th::SEP),
                        width,
                    ));
                    cursor_row = lines.len().saturating_sub(2);
                    cursor_col = 2 + width_of(&masked);
                }
            }
        } else if let Some(select) = &self.select {
            lines.push(Line::from(vec![span(
                select.title.clone(),
                th::accent_bold(),
            )]));
            let rows = MAX_SELECT_ROWS.min(select.items.len());
            let first = select
                .selected
                .saturating_sub(rows.saturating_sub(1))
                .min(select.items.len().saturating_sub(rows));
            for (i, item) in select.items.iter().enumerate().skip(first).take(rows) {
                lines.push(option_row(item.clone(), "", i == select.selected, width));
            }
            let left = if select.items.len() > rows {
                format!("{} of {}", select.selected + 1, select.items.len())
            } else {
                String::new()
            };
            lines.push(hint_row(
                &left,
                &format!("⇡⇣ move {} enter select {} esc cancel", th::SEP, th::SEP),
                width,
            ));
            cursor_row = lines.len().saturating_sub(1);
            cursor_col = 0;
        } else if let Some(menu) = &self.menu {
            lines.push(truncate_line(
                Line::from(vec![indent(GRID), span(menu.subject.clone(), th::bold())]),
                width,
            ));
            for (i, (label, key, _)) in MENU.iter().enumerate() {
                lines.push(chrome::numbered_row(
                    i,
                    label,
                    &key.to_string(),
                    i == menu.selected,
                    width,
                ));
            }
            lines.push(hint_row("", "enter to confirm · esc to cancel", width));
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
        lines.push(blank());
        lines.push(self.compose_status(width));
        (lines, cursor_row, cursor_col)
    }

    /// Completion rows plus, when the list is cut short, what it left out.
    fn compose_dropdown(&self, dropdown: &Dropdown, width: usize) -> Vec<Line<'static>> {
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
        let mut rows: Vec<Line<'static>> = Vec::new();
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
            rows.push(truncate_line(
                Line::from(vec![
                    indent(GRID),
                    span(
                        format!(
                            "+{} more {} keep typing to narrow",
                            dropdown.total - shown,
                            th::SEP
                        ),
                        th::dim(),
                    ),
                ]),
                width,
            ));
        }
        rows
    }

    /// Wrapped input rows plus the cursor's (row, column) within them.
    fn compose_input(&self, width: usize) -> (Vec<Line<'static>>, usize, usize) {
        let prompt = || vec![span(th::MARK_USER, th::accent()), indent(1)];
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
                None => ("Ask Butterfly to do anything", "Ask Butterfly"),
            };
            let budget = width.saturating_sub(GRID);
            let hint = if width_of(hint) <= budget {
                hint
            } else {
                short
            };
            let mut spans = prompt();
            spans.push(span(truncate_cols(hint, budget), th::dim()));
            return (vec![Line::from(spans)], 0, 2);
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
        let visible: Vec<Line<'static>> = rows
            .iter()
            .skip(start)
            .take(MAX_INPUT_ROWS)
            .enumerate()
            .map(|(i, row)| {
                let mut spans = if start + i == 0 {
                    prompt()
                } else {
                    vec![indent(GRID)]
                };
                spans.push(th::text(row.clone()));
                Line::from(spans)
            })
            .collect();
        (visible, cursor_row - start, cursor_col)
    }

    /// The footer: identity, context pressure and spend on the left, the
    /// permission mode on the right. It carries the running totals so the
    /// per-turn summary above can stay short.
    fn compose_status(&self, width: usize) -> Line<'static> {
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

    /// Append an entry to the transcript ring.
    ///
    /// The ring backs the Ctrl+T overlay and nothing else: what is on screen
    /// belongs to the terminal's scrollback the moment it is committed, so
    /// there is no projection for this to feed. Reasoning the stream hides is
    /// recorded here too, which is why the overlay can show it.
    fn record(&mut self, entry: Entry) {
        if self.transcript.len() >= TRANSCRIPT_CAP {
            self.transcript.pop_front();
        }
        self.transcript.push_back(entry);
    }

    /// Enforce the vertical rhythm in ONE place: never two blank rows in a
    /// row, never a blank row opening a commit, never a run of them trailing
    /// it.
    ///
    /// Blank entries are produced by several independent paths — a table's own
    /// spacers, a prose block opening, a hidden fence delimiter, a marked note
    /// — and each used to decide for itself whether one was needed, using a
    /// `last_blank` flag that only tracked the previous line. A reply that put
    /// a table next to a heading therefore stacked up a hole. Deciding here
    /// instead means no path can get it wrong.
    ///
    /// Trailing blanks are dropped rather than kept: the next block opens with
    /// its own spacer, so keeping them would double the gap, and at the end of
    /// a turn they leave a hole above the composer.
    fn collapse_blanks(&mut self, entries: Vec<Entry>) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::with_capacity(entries.len());
        // At the very start of a session there is nothing to separate from.
        let mut previous_blank = self.last_blank;
        for entry in entries {
            let blank = matches!(entry, Entry::Blank);
            if blank && previous_blank {
                continue;
            }
            previous_blank = blank;
            out.push(entry);
        }
        while matches!(out.last(), Some(Entry::Blank)) {
            out.pop();
        }
        out
    }

    /// Commit logical entries: record them into the transcript ring, make
    /// room on screen for the rows they add, then redraw.
    fn commit_entries(&mut self, entries: Vec<Entry>) {
        if entries.is_empty() {
            return;
        }
        let entries = self.collapse_blanks(entries);
        if entries.is_empty() {
            return;
        }
        // Trailing blanks are gone, so the next block always opens its own
        // separator.
        self.last_blank = false;
        if self.overlay.is_some() {
            // The overlay owns the alternate screen; committed rows wait in
            // the transcript and are written to history when it closes.
            for entry in entries {
                self.record(entry);
            }
            self.draw_overlay();
            return;
        }
        let width = self.width();
        let mut lines: Vec<Line<'static>> = Vec::new();
        for entry in entries {
            lines.extend(render_entry(&entry, width));
            self.record(entry);
        }
        // Committed rows go into the terminal's own scrollback, once, and are
        // never redrawn. Everything the renderer does after this touches only
        // the composer.
        self.term.insert_history(&lines);
        self.paint();
    }

    /// Redraw the composer.
    ///
    /// This is the whole of the render loop. The composer is the only region
    /// the renderer owns; everything above it was handed to the terminal's
    /// scrollback when it was committed and is not ours to redraw.
    fn paint(&mut self) {
        if self.overlay.is_some() {
            self.draw_overlay();
            return;
        }
        if self.term.sync_size() {
            // The terminal changed shape under us. Re-laying the composer
            // against geometry that is about to change again is wasted work,
            // so the settle does it once the drag stops.
            self.resize_pending = true;
            self.resize_at = Some(Instant::now());
            return;
        }
        if self.resize_pending {
            return;
        }
        let (block, cursor_row, cursor_col) = self.compose_block();
        let height = self.height();
        let block_height = block.len();
        // A terminal barely taller than the composer: show the block's tail so
        // the input and footer stay reachable.
        let rows: Vec<Line<'static>> = if block_height > height {
            block[block_height - height..].to_vec()
        } else {
            block
        };
        let top = height.saturating_sub(rows.len());
        // The composer changed height, so it has taken rows from the history
        // region or handed some back. Repaint that region in place from the
        // transcript: at a fixed width those are exactly the rows already
        // there, so nothing scrolls, nothing reaches scrollback twice, and the
        // composer never leaves a hole behind it.
        if self.term.viewport_top_for(rows.len()) != self.term.viewport_top() {
            let width = self.width();
            let region = self.term.viewport_top_for(rows.len());
            let mut history: Vec<Line<'static>> = Vec::new();
            'fill: for entry in self.transcript.iter().rev() {
                for line in render_entry(entry, width).into_iter().rev() {
                    // Same rhythm as `insert_history`: this path renders the
                    // transcript raw, so without the rule a repaint could
                    // reintroduce the stacked blanks the commit path strips.
                    let blank = line_cols(&line) == 0;
                    if blank && history.last().is_some_and(|l| line_cols(l) == 0) {
                        continue;
                    }
                    history.push(line);
                    if history.len() >= region {
                        break 'fill;
                    }
                }
            }
            while history.last().is_some_and(|l| line_cols(l) == 0) {
                history.pop();
            }
            history.reverse();
            self.term.fill_history(&history, region);
        }
        let cursor = (!self.selecting()).then(|| {
            let row = (top + cursor_row).min(height.saturating_sub(1));
            (cursor_col as u16, row as u16)
        });
        self.term.draw_viewport(&rows, cursor);
    }

    /// Settle a resize.
    ///
    /// The composer is re-laid at the new width and nothing else happens.
    /// History belongs to the terminal's scrollback the moment it is written
    /// (`Term::insert_history`), and it is never re-projected: rows keep the
    /// wrapping they were committed with, so narrowing shears an over-long
    /// row — the same trade Codex makes — but no text is ever drawn twice.
    /// Re-projecting is what put the same paragraph on screen and in history
    /// at once.
    /// Settle a resize by REFLOWING: wipe the screen and the scrollback, then
    /// replay the transcript at the new width.
    ///
    /// This is what makes the UI actually responsive to a resize, and it is
    /// Codex's strategy (`app::resize_reflow` + `clear_scrollback_and_visible_screen_ansi`).
    /// The in-memory transcript is the source of truth; the terminal's copy is
    /// a rendering of it that goes stale the moment the width changes. Leaving
    /// that copy alone is what "the UI does not resize" looks like, and
    /// re-projecting over the top of it is what duplicates text — the terminal
    /// has already reflowed some of those rows into scrollback. Owning the
    /// scrollback and rewriting it is the only option that does neither.
    ///
    /// Replay is capped: past a few thousand rows the terminal is discarding
    /// them anyway, and a drag should not get slower the longer the session
    /// runs.
    fn settle_resize(&mut self) {
        let width = self.width();
        self.term.reset_for_reflow();
        let mut lines: Vec<Line<'static>> = Vec::new();
        for entry in self.transcript.iter() {
            lines.extend(render_entry(entry, width));
        }
        if lines.len() > REFLOW_ROW_CAP {
            lines.drain(..lines.len() - REFLOW_ROW_CAP);
        }
        self.term.insert_history(&lines);
        self.paint();
    }

    /// Draw the transcript overlay on the alternate screen: the full session
    /// including reasoning that streaming keeps hidden.
    fn draw_overlay(&mut self) {
        let Some(offset) = self.overlay else {
            return;
        };
        let (width, height) = (self.width(), self.height());
        let rows = height - 1;
        // Entries re-wrap to the overlay's width at draw time.
        let lines: Vec<Line<'static>> = self
            .transcript
            .iter()
            .flat_map(|entry| render_entry(entry, width))
            .collect();
        let total = lines.len();
        let max_offset = total.saturating_sub(rows);
        let offset = offset.min(max_offset);
        self.overlay = Some(offset);
        let end = total - offset;
        let start = end.saturating_sub(rows);
        let mut frame: Vec<Line<'static>> = lines[start..end].to_vec();
        while frame.len() < rows {
            frame.push(blank());
        }
        // Where you are in the scroll matters as much as the keys do: a
        // transcript with no position indicator feels bottomless.
        let position = if total <= rows {
            "all".to_string()
        } else {
            format!("{}‒{} of {total}", start + 1, end)
        };
        let footer = th::justify(
            Line::from(vec![span(
                format!(" transcript {} {position}", th::SEP),
                th::band(),
            )]),
            Line::from(vec![span(
                format!("⇡⇣ pgup/pgdn scroll {} q closes ", th::SEP),
                th::band(),
            )]),
            width,
        );
        frame.push(th::pad_line(
            truncate_line(footer, width),
            width,
            th::band(),
        ));
        self.term.draw_full(&frame);
    }

    fn open_overlay(&mut self) {
        if self.overlay.is_some() || self.menu.is_some() {
            return;
        }
        self.term.enter_alt_screen();
        self.overlay = Some(0);
        self.draw_overlay();
    }

    fn close_overlay(&mut self) {
        if self.overlay.take().is_none() {
            return;
        }
        self.term.leave_alt_screen();
        // Output that arrived while the overlay was open is already in the
        // transcript; account for the rows it added and redraw.
        // Rows that arrived while the overlay was open are already in the
        // transcript; write them to history now, in order, then redraw.
        let pending = std::mem::take(&mut self.pending_output);
        let width = self.width();
        let lines: Vec<Line<'static>> = pending
            .iter()
            .flat_map(|entry| render_entry(entry, width))
            .collect();
        self.term.insert_history(&lines);
        self.paint();
    }

    /// Markdown-render one assistant text line into logical entries.
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
                    if line_cols(&styled) == 0 {
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
                    // A heading opens a section, so it gets a blank row above
                    // it even when the model wrote none. `collapse_blanks`
                    // drops it if there is already one.
                    if spans
                        .iter()
                        .all(|span| span.style == md::SpanStyle::Heading)
                        && !spans.is_empty()
                    {
                        entries.push(Entry::Blank);
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
            self.paint();
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
            let style = if decision == Decision::Deny {
                th::danger()
            } else {
                th::dim()
            };
            if let Some(activity) = &mut self.activity {
                activity.phase = Phase::Starting;
            }
            self.commit_entries(vec![Entry::Fixed(Line::from(vec![
                indent(GRID),
                span(format!("↳ {label}"), style),
            ]))]);
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
        Arc::new(Self {
            inner: Mutex::new(UiInner {
                term: Term::new(),
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
                resize_pending: false,
                resize_at: None,
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, UiInner> {
        self.inner.lock().expect("ui lock")
    }

    /// Take over the visible screen: the old shell content is flooded into
    /// scrollback (NOT erased ‒ no alternate screen, history stays
    /// reachable), then the block draws at the bottom edge.
    pub fn enter_screen(&self) {
        let mut inner = self.lock();
        inner.term.set_title();
        inner.term.take_over();
        inner.paint();
    }

    /// Commit composed rows into scrollback above the block.
    pub fn print_lines(&self, lines: Vec<Line<'static>>) {
        let mut inner = self.lock();
        inner.commit_live();
        // Every committed block opens with one blank row. `collapse_blanks`
        // drops it at the very start of a session and wherever it would
        // double, so this is "give this block air" rather than "add a line".
        let mut entries: Vec<Entry> = vec![Entry::Blank];
        entries.extend(lines.into_iter().map(|line| {
            // Whitespace-only counts as blank, or a source that indents its
            // empty lines slips a row past `collapse_blanks` and the gap
            // doubles.
            if line.to_string().trim().is_empty() {
                Entry::Blank
            } else {
                Entry::Fixed(line)
            }
        }));
        inner.commit_entries(entries);
    }

    /// Commit plain (unstyled) text into scrollback, one row per line.
    pub fn print(&self, text: &str) {
        self.print_lines(
            text.split('\n')
                .map(|line| {
                    if line.is_empty() {
                        blank()
                    } else {
                        Line::from(vec![th::text(line.to_string())])
                    }
                })
                .collect(),
        );
    }

    /// Commit an error, in the one color reserved for things that failed.
    pub fn print_error(&self, text: &str) {
        self.print_lines(vec![Line::from(vec![span(text.to_string(), th::danger())])]);
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

    /// Commit the opening card. It is stored as data, so it re-lays out at the
    /// width of whatever screen it is next drawn on.
    pub fn print_welcome(&self, info: chrome::Welcome) {
        let mut inner = self.lock();
        inner.commit_entries(vec![Entry::Blank, Entry::Welcome(info)]);
    }

    /// Lay out model-picker rows at the current width (the picker itself is
    /// the generic selector, which takes ready-made rows).
    pub fn render_model_rows(&self, rows: &[chrome::ModelRow<'_>]) -> Vec<Line<'static>> {
        let width = self.lock().width();
        // The selector prefixes a marker and a space, and reserves the same
        // on the right, so rows compose at the width they will be drawn at.
        chrome::model_rows(rows, width.saturating_sub(GRID * 2))
    }

    /// Current terminal width, for surfaces composed outside this module.
    pub fn width(&self) -> usize {
        self.lock().width()
    }

    pub fn set_status(&self, status: Status) {
        let mut inner = self.lock();
        inner.status = status;
        inner.paint();
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
        inner.paint();
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
            inner.record(entry);
        }
        inner.activity = None;
        inner.paint();
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
            inner.settle_resize();
            return;
        }
        if inner.resize_pending {
            // Still dragging: hold the screen rather than redraw into
            // geometry that is about to change again.
            return;
        }
        if let Some(activity) = &mut inner.activity {
            activity.spinner += 1;
            inner.paint();
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
        inner.paint();
        receiver
    }

    /// Open a generic arrow-key selector; resolves with the chosen index or
    /// None on cancel.
    pub fn open_select(
        &self,
        title: &str,
        items: Vec<Line<'static>>,
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
        inner.paint();
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
        let width = inner.width();
        let card = chrome::approval_card(request, request.detail.as_deref(), width);
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
        let mut entries: Vec<Entry> = Vec::new();
        if !inner.last_blank {
            entries.push(Entry::Blank);
        }
        entries.extend(card.into_iter().map(|line| {
            if line_cols(&line) == 0 {
                Entry::Blank
            } else {
                Entry::Fixed(line)
            }
        }));
        inner.commit_entries(entries);
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
        inner.activity = None;
        inner.live = None;
        // Clear the composer and leave the shell prompt directly under the
        // conversation; history stays exactly where the terminal has it.
        inner.term.teardown();
        inner.editor.save_history();
    }

    /// Handle one terminal event. Editing is absorbed internally; only
    /// session-relevant actions surface.
    pub fn handle_event(&self, event: Event) -> Option<UiAction> {
        let mut inner = self.lock();
        match event {
            Event::Resize(..) => {
                // A width change re-wraps the content; a height change moves
                // the bottom edge the block is pinned to. Both invalidate
                // the frame, so both are settled.
                //
                // Dragging a window edge emits a resize per pixel step.
                // Redrawing on each one is both wasteful and visibly jumpy,
                // so the work is deferred to `tick`, which runs when the
                // event stream goes quiet ‒ once per drag.
                if inner.term.sync_size() {
                    if inner.overlay.is_some() {
                        // The overlay owns the alternate screen outright, so
                        // it can safely re-wrap its whole view right away.
                        inner.draw_overlay();
                    } else {
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
                    inner.paint();
                } else if inner.menu.is_none() && inner.connect.is_none() {
                    inner.editor.insert_paste(&content);
                    inner.update_dropdown();
                    inner.paint();
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
            let page = inner.height() - 1;
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
            inner.paint();
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
            inner.paint();
            return None;
        }
        // Approval menu captures everything until resolved.
        if inner.menu.is_some() {
            match code {
                KeyCode::Up => {
                    if let Some(menu) = &mut inner.menu {
                        menu.selected = (menu.selected + MENU.len() - 1) % MENU.len();
                    }
                    inner.paint();
                }
                KeyCode::Down => {
                    if let Some(menu) = &mut inner.menu {
                        menu.selected = (menu.selected + 1) % MENU.len();
                    }
                    inner.paint();
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
                inner.paint();
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
                inner.commit_entries(vec![Entry::Fixed(Line::from(vec![span(
                    notice,
                    th::dim(),
                )]))]);
            }
            KeyCode::Char('j') if control => {
                inner.editor.insert('\n');
                inner.dropdown = None;
                inner.paint();
            }
            KeyCode::Char('a') if control => {
                inner.editor.move_home();
                inner.paint();
            }
            KeyCode::Char('e') if control => {
                inner.editor.move_end();
                inner.paint();
            }
            KeyCode::Char('u') if control => {
                inner.editor.clear();
                inner.dropdown = None;
                inner.paint();
            }
            KeyCode::Char('k') if control => {
                let (start, end) = (inner.editor.cursor(), inner.editor.line_end());
                inner.editor.replace_range(start, end, "");
                inner.update_dropdown();
                inner.paint();
            }
            KeyCode::Char('w') if control => {
                inner.editor.delete_word_back();
                inner.update_dropdown();
                inner.paint();
            }
            KeyCode::Char(ch) if !control => {
                inner.editor.insert(ch);
                inner.update_dropdown();
                inner.paint();
            }
            KeyCode::Enter if modifiers.contains(KeyModifiers::ALT) => {
                inner.editor.insert('\n');
                inner.dropdown = None;
                inner.paint();
            }
            KeyCode::Enter => {
                if let Some(dropdown) = &inner.dropdown {
                    // If accepting would not change the token (the user typed
                    // it out in full), Enter means submit, not re-accept.
                    let token = &inner.editor.text()[dropdown.token_start..inner.editor.cursor()];
                    if dropdown.choices[dropdown.selected].insert != token {
                        inner.accept_selected();
                        inner.paint();
                        return None;
                    }
                    inner.dropdown = None;
                }
                let visible = inner.editor.text().trim().to_string();
                let submitted = inner.editor.submit();
                inner.dropdown = None;
                if submitted.is_empty() {
                    inner.paint();
                    return None;
                }
                // Bare picker commands are composer interactions, not
                // conversation turns: the picker opens in place of the
                // input and nothing echoes into the transcript.
                if matches!(submitted.as_str(), "/model" | "/connect") {
                    inner.paint();
                    return Some(UiAction::Submit(submitted));
                }
                // Echo the prompt as a highlighted bar so user turns stand
                // apart from agent output in scrollback. One entry for the
                // whole message: the marker draws on the first row only.
                // The separator goes BEFORE the turn: `collapse_blanks` drops
                // trailing blanks, so a turn that carried its spacer behind it
                // ended up flush against whatever came next.
                inner.commit_entries(vec![Entry::Blank, Entry::UserBar(visible.to_string())]);
                return Some(UiAction::Submit(submitted));
            }
            KeyCode::Backspace => {
                inner.editor.backspace();
                inner.update_dropdown();
                inner.paint();
            }
            KeyCode::Delete => {
                inner.editor.delete_forward();
                inner.update_dropdown();
                inner.paint();
            }
            KeyCode::Left => {
                inner.editor.move_left();
                inner.dropdown = None;
                inner.paint();
            }
            KeyCode::Right => {
                inner.editor.move_right();
                inner.dropdown = None;
                inner.paint();
            }
            KeyCode::Home => {
                inner.editor.move_home();
                inner.paint();
            }
            KeyCode::End => {
                inner.editor.move_end();
                inner.paint();
            }
            KeyCode::BackTab => {
                return Some(UiAction::CycleMode);
            }
            KeyCode::Tab => {
                if inner.dropdown.is_some() {
                    inner.accept_selected();
                    inner.paint();
                }
            }
            KeyCode::Esc => {
                if inner.dropdown.is_some() {
                    inner.dropdown = None;
                    inner.paint();
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
                inner.paint();
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
                inner.paint();
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
                inner.record(entry);
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
        inner.paint();
    }

    fn on_tool_result(&mut self, preview: &str, is_error: bool, error: Option<&str>) {
        let mut inner = self.ui.lock();
        let elapsed = inner
            .activity
            .as_ref()
            .and_then(|activity| activity.tool_started)
            .map(|started| started.elapsed())
            .unwrap_or_default();
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
        inner.commit_entries(vec![Entry::Tool {
            preview: text,
            outcome: Some(ToolRecord {
                failed: is_error,
                reason: error.unwrap_or_default().to_string(),
                elapsed,
            }),
        }]);
        inner.text_block_start = true;
        if let Some(activity) = &mut inner.activity {
            activity.phase = Phase::Thinking;
            activity.tool_started = None;
        }
        inner.paint();
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

    /// A headless `UiInner` at a fixed geometry. `Term` never touches the
    /// terminal until something is drawn, so composition and layout are
    /// testable without a tty.
    fn inner_at(width: usize, height: usize) -> UiInner {
        let mut inner = UiInner {
            term: Term::new(),
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
            resize_pending: false,
            resize_at: None,
        };
        inner.term.set_test_size(width as u16, height as u16);
        inner
    }

    /// The invariant behind the whole layout: no composed block row may be
    /// wider than the terminal. An overlong row is silently cut by the
    /// frame, so the composer would lose its right-hand half rather than
    /// clamping where the layout intended.
    #[test]
    fn no_block_line_exceeds_terminal_width() {
        for width in [24usize, 40, 60, 80] {
            let mut inner = inner_at(width, 30);
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
                    hint: "struct · a/very/long/location.rs:1234".into(),
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
                        line_cols(line) <= width,
                        "line exceeds {width} cols ({}): {:?}",
                        line_cols(line),
                        line.to_string(),
                    );
                }
            }
        }
    }

    /// Tool and reasoning live lines (no prefix) must also fit exactly.
    #[test]
    fn unprefixed_live_lines_fit_too() {
        let mut inner = inner_at(50, 30);
        inner.live = Some(LiveLine {
            kind: LiveKind::Tool,
            text: format!("⚒ read_file({})", "y".repeat(200)),
        });
        let (lines, _, _) = inner.compose_block();
        assert!(lines.iter().all(|line| line_cols(line) <= 50));
    }

    /// Nothing the composer draws may carry a glyph whose width the terminal
    /// gets to choose: the block is the rule, the input, the hint row and
    /// the footer, every one of them width-critical.
    #[test]
    fn the_block_draws_no_ambiguous_width_glyph() {
        for width in [24usize, 40, 80, 100, 120, 160] {
            let mut inner = inner_at(width, 30);
            inner.editor.insert_str("a half-typed question");
            let (lines, _, _) = inner.compose_block();
            for line in &lines {
                for ch in line.to_string().chars() {
                    // Chrome glyphs are measured, not assumed — see
                    // `theme::CHROME_GLYPHS`. Anything else the block draws
                    // must have one unambiguous width.
                    if th::cell_width(ch) == 1 && th::is_ambiguous_width(ch) {
                        continue;
                    }
                    assert!(
                        !th::is_ambiguous_width(ch),
                        "ambiguous glyph {ch:?} (U+{:04X}) in a block line at \
                         {width} cols: {:?}",
                        ch as u32,
                        line.to_string(),
                    );
                }
            }
        }
    }

    /// The contract `render_entry` owes the frame, for EVERY variant: one
    /// returned line is exactly one screen row, and it fits.
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
            Entry::Code(Line::from(vec![span(long.clone(), th::accent())])),
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
                for line in render_entry(entry, width) {
                    let plain = line.to_string();
                    assert!(
                        !plain.contains('\n') && !plain.contains('\r'),
                        "embedded newline at {width}: {plain:?}"
                    );
                    // The real width, not the layout floor: a terminal
                    // narrower than 20 columns still must not be overflowed.
                    assert!(
                        line_cols(&line) <= width,
                        "row exceeds {width} cols ({}): {plain:?}",
                        line_cols(&line),
                    );
                }
            }
        }
    }

    /// Vertical rhythm: never two blank rows in a row, and never a blank row
    /// opening a commit. Blank entries are produced by several independent
    /// paths — a table's own spacers, a prose block opening, a fence
    /// delimiter, a marked note — and each used to decide for itself, so a
    /// model reply that put a table next to a heading stacked up a hole.
    #[test]
    fn committed_rows_never_stack_blank_lines() {
        let mut inner = inner_at(100, 40);
        let reply = "\
Here are the kinds.

| Kind | Trigger |
|---|---|
| syntax_failure | SyntaxError |
| test_failure | panicked at |


## Payload


| Field | Shape |
|---|---|
| source | tool{name} |

Done.
";
        let mut entries: Vec<Entry> = Vec::new();
        for line in reply.lines() {
            entries.extend(inner.render_text_entries(line));
        }
        entries.extend(inner.flush_markdown());
        let kept = inner.collapse_blanks(entries);
        assert!(
            !matches!(kept.first(), Some(Entry::Blank)),
            "a commit must not open with a blank row"
        );
        let mut previous_blank = false;
        for (i, entry) in kept.iter().enumerate() {
            let blank = matches!(entry, Entry::Blank);
            assert!(
                !(blank && previous_blank),
                "two blank rows in a row at {i}: {kept:#?}"
            );
            previous_blank = blank;
        }
        // And the content survived the collapse.
        let text: String = kept
            .iter()
            .flat_map(|entry| render_entry(entry, 100))
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("syntax_failure") && text.contains("Payload"));
        assert!(text.contains("source") && text.contains("Done."));
    }

    /// Model prose must never overflow the width: a row that does is wrapped
    /// by the TERMINAL instead of by us, and a terminal wraps to column 0 —
    /// so the continuation loses the content grid and reads as a new item.
    #[test]
    fn model_prose_never_overflows_and_keeps_its_grid() {
        // Shapes taken from a real reply: em dashes, arrows, inline code,
        // nested bullets, long paths, and a heading.
        let reply = "\
**Purpose (1-9):** Maps tool errors, command output, provider errors and \
verification results onto one closed vocabulary so recovery can choose a \
response, telemetry can aggregate, and Nimble can learn from `previous_failure` \
without parsing prose — rule-based and conservative.

`classify_tool_output(tool, output)` **(148-211):** Order: Timeout → \
ModelFormatFailure (invalid arguments, must be json array) → ToolFailure.

- edit: `old_string`/match + not found or occurrence/matches/ambiguous → \
  IncorrectAssumption; bare not found → MissingContext; else ToolFailure.
  - read_file/list_files/search_text/symbol_search/repo_map: missing-file → \
    MissingContext; invalid pattern → ModelFormatFailure.

## Constructors (333-347)
products/butterfly/cli/crates/core/src/failure.rs (477 lines) — structured \
failure classification for Butterfly's recovery loop:
";
        for width in [40usize, 60, 80, 100, 120, 160] {
            let mut inner = inner_at(width, 40);
            let mut entries: Vec<Entry> = Vec::new();
            for line in reply.lines() {
                entries.extend(inner.render_text_entries(line));
            }
            entries.extend(inner.flush_markdown());
            for entry in &entries {
                for line in render_entry(entry, width) {
                    let plain = line.to_string();
                    assert!(
                        line_cols(&line) <= width,
                        "row overflows {width} cols ({}) and the terminal will \
                         wrap it to column 0: {plain:?}",
                        line_cols(&line),
                    );
                }
            }
        }
    }

    #[test]
    fn the_transcript_ring_keeps_everything_for_the_overlay() {
        let mut inner = inner_at(40, 10);
        for i in 0..20 {
            inner.record(Entry::Fixed(Line::from(vec![th::text(format!(
                "line {i}"
            ))])));
        }
        // Reasoning the stream hides is recorded too — that is what lets the
        // Ctrl+T overlay show it.
        inner.record(Entry::Fixed(Line::from(vec![th::text("hidden reasoning")])));
        assert_eq!(inner.transcript.len(), 21);
        let rendered: Vec<String> = inner
            .transcript
            .iter()
            .flat_map(|entry| render_entry(entry, 40))
            .map(|line| line.to_string())
            .collect();
        assert_eq!(rendered.last().expect("tail"), "hidden reasoning");
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
        assert!(narrow.iter().all(|line| line_cols(line) <= 24));
        assert_eq!(wide.len(), 1);
        assert_eq!(wide[0].spans[0].content, th::MARK_ANSWER);
        assert_eq!(wide[0].spans[0].style, th::accent());
        // A user turn is a dim caret plus the text — no filled band, so it
        // no longer spans the width.
        let bar = Entry::UserBar("fix the bug".into());
        for width in [24usize, 60] {
            let rows = render_entry(&bar, width);
            assert!(rows[0].to_string().starts_with("› fix the bug"));
            assert!(line_cols(&rows[0]) < width);
        }
    }

    #[test]
    fn user_bars_mark_only_the_first_row_of_a_multiline_prompt() {
        let bar = Entry::UserBar("hi tell me\n\n@crates/core/src/provider/mod.rs".into());
        let rows = render_entry(&bar, 60);
        assert_eq!(rows.len(), 3);
        let plain: Vec<String> = rows.iter().map(|line| line.to_string()).collect();
        assert_eq!(plain.iter().filter(|row| row.contains('›')).count(), 1);
        assert!(plain[0].starts_with("› hi tell me"), "{}", plain[0]);
        // Continuations sit on the content column, under the marker.
        for line in &rows[1..] {
            assert!(line.to_string().starts_with("  ") || line_cols(line) == 0);
        }
        assert!(plain[2].contains("  @crates"), "{}", plain[2]);
        // A trailing newline adds no extra band row.
        let trailing = Entry::UserBar("done\n".into());
        assert_eq!(render_entry(&trailing, 60).len(), 1);
    }

    #[test]
    fn prose_word_wraps_at_word_boundaries_with_block_prefixes() {
        let mut inner = inner_at(30, 30);
        inner.last_blank = false;
        inner.text_block_start = true;
        let entries = inner
            .render_text_entries("The quick brown fox jumps over the lazy dog near the riverbank");
        let out: Vec<Line<'static>> = entries
            .iter()
            .flat_map(|entry| render_entry(entry, 30))
            .collect();
        // Spacer first (previous line was not blank), then ◉ + indented rows.
        assert_eq!(line_cols(&out[0]), 0);
        assert_eq!(out[1].spans[0].content, th::MARK_ANSWER);
        assert_eq!(out[1].spans[0].style, th::accent());
        assert!(out[2].to_string().starts_with("  "));
        for line in &out[1..] {
            let plain = line.to_string();
            assert!(line_cols(line) <= 30, "row too wide: {plain:?}");
            // No mid-word cuts: each row ends at a word boundary.
            assert!(!plain.ends_with(' '));
        }
        // Markers are styled but occupy the same two columns as before, so
        // the grid survives: content starts at column 2 on every row.
        assert_eq!(
            width_of(&out[1].to_string()[..out[1].to_string().find('T').expect("prose")]),
            2
        );
    }

    #[test]
    fn bullets_hang_wrapped_rows_under_their_text() {
        let mut inner = inner_at(30, 30);
        inner.last_blank = true;
        inner.text_block_start = false;
        let entries = inner.render_text_entries("- a bullet item with enough words to wrap around");
        let out: Vec<String> = entries
            .iter()
            .flat_map(|entry| render_entry(entry, 30))
            .map(|line| line.to_string())
            .collect();
        assert!(out.len() >= 2);
        assert!(out[0].contains('‣'));
        // Continuations indent 2 (block) + 2 (bullet hang) = 4 columns.
        assert!(
            out[1].starts_with("    "),
            "hanging indent lost: {:?}",
            out[1]
        );
    }

    #[test]
    fn styled_spans_survive_row_wrapping() {
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
        let rows = md::styled_lines(&spans, 20, 20);
        assert!(rows.len() >= 2);
        // Every row that contains bold text carries the style on its spans.
        for row in &rows[1..] {
            assert!(
                row.spans.iter().any(|s| s.style == th::bold()),
                "style dropped on wrap: {row:?}"
            );
        }
    }

    #[test]
    fn long_words_hard_cut_instead_of_overflowing() {
        let spans = vec![md::Span {
            style: md::SpanStyle::Plain,
            text: "x".repeat(50),
        }];
        let rows = md::styled_lines(&spans, 20, 20);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| line_cols(r) <= 20));
    }

    /// The opening card is a BOX, so it has to be re-composed at the render
    /// width rather than clamped to it.
    ///
    /// It used to be committed as rows composed once at the startup width.
    /// Widening then left it marooned in the corner, and narrowing ran it
    /// through `truncate_line`, which cut the right-hand border off and put an
    /// ellipsis where it had been — measured in herdr at a 44-column pane.
    #[test]
    fn the_welcome_card_is_rebuilt_at_every_width() {
        let entry = Entry::Welcome(chrome::Welcome {
            workspace: "monet/products/butterfly".into(),
            branch: Some("feature/a-fairly-long-branch-name".into()),
            model_name: "DeepSeek V4 Flash (served as V4.1 Flash)".into(),
            context_tokens: 1_000_000,
            mode: "ask".into(),
        });
        for width in [44usize, 60, 80, 100, 132, 160, 200] {
            let rows = render_entry(&entry, width);
            assert_eq!(rows.len(), 7, "card shape changed at {width}");
            for row in &rows {
                assert_eq!(
                    line_cols(row),
                    width,
                    "card did not fill {width} columns: {:?}",
                    row.to_string()
                );
                let last = row.to_string().chars().last().unwrap();
                assert!(
                    [th::BOX_TR, th::BOX_BR, th::BOX_V]
                        .iter()
                        .any(|edge| edge.starts_with(last)),
                    "card lost its right border at {width}: {:?}",
                    row.to_string()
                );
            }
        }
    }

    /// Entries re-wrap at whatever width they are rendered at. That still
    /// matters after the move to Codex's model — not for a resize, which no
    /// longer redraws history, but for every fresh commit and for the Ctrl+T
    /// overlay, which re-wraps the whole session at its own width.
    #[test]
    fn entries_rewrap_at_the_width_they_are_rendered_at() {
        let sentence = "the composer is the only region this renderer owns, and everything \
             above it belongs to the terminal the moment it is committed";
        let entry = Entry::Prose {
            marker: Some(th::MARK_ANSWER),
            spans: vec![md::Span {
                style: md::SpanStyle::Plain,
                text: sentence.to_string(),
            }],
            cont_indent: 0,
        };
        let (at40, at60, at96) = (
            render_entry(&entry, 40).len(),
            render_entry(&entry, 60).len(),
            render_entry(&entry, 96).len(),
        );
        assert!(
            at40 > at60 && at60 > at96,
            "entry did not re-wrap with the width: 40->{at40} 60->{at60} 96->{at96}"
        );
        // And it keeps widening: prose uses the whole terminal, with no
        // measure cap. One was tried and removed — it left most of a wide
        // window black.
        let widest = render_entry(&entry, 200);
        assert_eq!(
            widest.len(),
            1,
            "prose stopped using the width it was given"
        );
        assert!(
            line_cols(&widest[0]) > 96,
            "prose is capped short of the terminal: {:?}",
            widest[0].to_string()
        );
        for width in [40usize, 60, 100, 160] {
            for line in render_entry(&entry, width) {
                assert!(line_cols(&line) <= width, "{:?}", line.to_string());
            }
        }
    }
}

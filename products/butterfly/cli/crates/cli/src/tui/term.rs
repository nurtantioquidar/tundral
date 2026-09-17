//! Terminal I/O, built on the model OpenAI's Codex TUI uses (mechanism read
//! from `codex-rs/tui/src/insert_history.rs`, Apache-2.0; the technique is
//! reimplemented here, no code is copied).
//!
//! The screen has exactly two parts:
//!
//! * **The viewport** — the composer block at the bottom of the screen. It is
//!   a ratatui [`Buffer`], cell-diffed against what the terminal is believed
//!   to be showing, and it is the ONLY region ever redrawn.
//! * **History** — everything the session has committed. It is written into
//!   the terminal's own scrollback exactly once by [`Term::insert_history`]
//!   and never touched again.
//!
//! Two properties of that write are what make scrollback behave, and both are
//! Codex's:
//!
//! 1. **Rows are pre-wrapped and hard-terminated.** Every line handed to
//!    `insert_history` is already at most one screen row wide, and rows are
//!    separated by a real `\r\n`. A terminal narrowing later re-wraps each row
//!    on its own; it cannot glue consecutive rows into one logical paragraph,
//!    which is what turned a scrolled-back conversation into run-together
//!    text.
//! 2. **The write happens inside a scroll region above the viewport**
//!    (`CSI 1;top r`), with the cursor parked on that region's last row. Each
//!    `\r\n` scrolls only that region, pushing its top row into native
//!    scrollback and leaving the composer untouched.
//!
//! History is never *patched*: rows already written are not redrawn in place.
//! A resize instead REFLOWS — [`Term::reset_for_reflow`] purges the screen and
//! the scrollback and the caller replays the transcript at the new width. See
//! that method for why owning the scrollback is the only strategy that both
//! reflows and never duplicates.

use std::io::{Stdout, Write};

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;

use super::theme::{self as th, ansi};

/// Terminal title, set once on entry. Shells reset it at their next prompt.
const TITLE: &str = "\x1b]0;butterfly\x07";
const ALT_SCREEN_ENTER: &str = "\x1b[?1049h";
const ALT_SCREEN_LEAVE: &str = "\x1b[?1049l";

/// The width the layout may use: one column fewer than the terminal has.
///
/// Writing the last column is the one thing terminals disagree about: xterm
/// parks the cursor in a pending-wrap state, and some multiplexers record the
/// two rows as ONE logical line which they then re-wrap together.
pub fn usable_width(columns: usize) -> usize {
    columns.saturating_sub(1)
}

/// Byte-level trace of everything the TUI writes. Enabled with
/// `BUTTERFLY_TUI_TRACE=<path>`.
///
/// It exists because the obvious way to capture a session ‒ `script` ‒ does
/// NOT propagate window resizes to the process it wraps (verified on macOS),
/// so a `script` recording is silent about exactly the behaviour a resize bug
/// lives in.
struct Trace(std::fs::File);

/// Stdout with an optional byte-for-byte copy tee'd to the trace file. Every
/// write the backend makes passes through here, so a trace is a complete
/// record of what the terminal was told.
pub struct TracedOut {
    out: Stdout,
    trace: Option<Trace>,
    /// Swallow every byte instead of writing it. A headless `Term` sets this
    /// (`set_test_size`): the cell writes a drawing test makes are full of
    /// cursor-positioning escapes, and the test harness does not capture
    /// direct writes to the stdout descriptor, so without it `cargo test`
    /// would scramble the terminal it was run from.
    silent: bool,
}

impl TracedOut {
    fn new() -> Self {
        let trace = std::env::var("BUTTERFLY_TUI_TRACE")
            .ok()
            .and_then(|path| std::fs::File::create(path).ok())
            .map(Trace);
        Self {
            out: std::io::stdout(),
            trace,
            silent: false,
        }
    }
}

impl Write for TracedOut {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(Trace(file)) = &mut self.trace {
            let _ = file.write_all(buf);
        }
        if self.silent {
            return Ok(buf.len());
        }
        self.out.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(Trace(file)) = &mut self.trace {
            let _ = file.flush();
        }
        if self.silent {
            return Ok(());
        }
        self.out.flush()
    }
}

pub struct Term {
    backend: CrosstermBackend<TracedOut>,
    /// What the viewport is believed to be showing. Frames are diffed against
    /// this; only differing cells are written.
    prev: Buffer,
    /// Screen row (0-based) the viewport starts on. History lives above it.
    viewport_top: u16,
    width: u16,
    height: u16,
    cursor_shown: bool,
    force_full: bool,
    /// Visible width of each row of the last composer frame. A resize needs
    /// it: the terminal re-wraps those rows too, and the taller shape it
    /// leaves behind has to be cleared before the composer is redrawn or the
    /// old one stays on screen and scrolls into history as a ghost.
    last_block_widths: Vec<usize>,
    /// Index of the composer row the caret sat on in the last frame.
    last_caret_row: usize,
    /// Whether the last row written to history was blank.
    last_history_blank: bool,
    /// How many rows of OUR history are currently visible above the composer.
    ///
    /// This is the band Butterfly owns and may repaint. Anything above it
    /// belongs to whatever was on screen before the session started and is
    /// never touched. Codex keeps the same number for the same reason
    /// (`visible_history_rows`).
    history_rows: usize,
}

impl Term {
    pub fn new() -> Self {
        let (columns, rows) = crossterm::terminal::size().unwrap_or((100, 30));
        let width = usable_width(columns as usize).max(1) as u16;
        let height = rows.max(1);
        let top = height - 1;
        Self {
            backend: CrosstermBackend::new(TracedOut::new()),
            prev: Buffer::empty(Rect::new(0, top, width, 1)),
            viewport_top: top,
            width,
            height,
            cursor_shown: true,
            force_full: true,
            last_block_widths: Vec::new(),
            last_caret_row: 0,
            // A session opens against the shell's prompt, so the first row it
            // writes needs no separator of its own.
            last_history_blank: true,
            history_rows: 0,
        }
    }

    /// Layout width and height, in the units every surface composes against.
    pub fn size(&self) -> (usize, usize) {
        (self.width as usize, self.height as usize)
    }

    /// Re-read the terminal's real size.
    ///
    /// A resize reaches the input thread as an event, but a paint can be
    /// issued from the session thread in the window between the terminal
    /// re-wrapping itself and that event being handled. `terminal::size` is a
    /// `TIOCGWINSZ` ioctl — no round trip — so the renderer can simply never
    /// treat its geometry as current without asking.
    pub fn sync_size(&mut self) -> bool {
        let Ok((columns, rows)) = crossterm::terminal::size() else {
            return false;
        };
        let width = usable_width(columns as usize).max(1) as u16;
        let height = rows.max(1);
        if width == self.width && height == self.height {
            return false;
        }
        self.width = width;
        self.height = height;
        self.viewport_top = self.viewport_top.min(height.saturating_sub(1));
        self.invalidate();
        true
    }

    /// Forget what the viewport is believed to show, so the next frame writes
    /// every cell of it. Never touches history.
    pub fn invalidate(&mut self) {
        self.force_full = true;
    }

    /// Name the terminal (OSC 0). Multiplexers put it in the pane frame, which
    /// is how a user tells this pane from the shell next to it.
    pub fn set_title(&mut self) {
        self.write_raw(TITLE);
    }

    /// Start inline, where the shell left the cursor.
    ///
    /// Nothing is flooded or erased: the composer claims the bottom rows and
    /// the first `draw_viewport` scrolls exactly as far as it needs, so the
    /// shell's own output stays where it was — the way Codex opens. Flooding a
    /// screenful of newlines first left a session opening against a wall of
    /// blank rows.
    pub fn take_over(&mut self) {
        self.viewport_top = self.height.saturating_sub(1);
        self.invalidate();
    }

    /// Write committed rows into the terminal's own scrollback, above the
    /// viewport, once.
    ///
    /// Every line must already fit one screen row; `render_entry` guarantees
    /// that, and anything longer is clamped here rather than being allowed to
    /// soft-wrap into a second row the scroll arithmetic does not know about.
    pub fn insert_history(&mut self, lines: &[Line<'static>]) {
        if lines.is_empty() {
            return;
        }
        let top = self.viewport_top;
        if top == 0 {
            // No room above the composer (a terminal barely taller than the
            // block). Scrolling here would eat the viewport, so the rows stay
            // in the transcript and reach the user through Ctrl+T.
            return;
        }
        let width = self.width as usize;
        let mut out = String::new();
        // Scroll region = every row above the viewport. Each `\r\n` written
        // with the cursor on its last row scrolls ONLY this region, so the top
        // row leaves into native scrollback and the composer never moves.
        out.push_str(&format!("\x1b[1;{top}r"));
        out.push_str(&format!("\x1b[{top};1H"));
        let mut wrote = false;
        for line in lines {
            // The vertical rhythm is enforced HERE, at the one point every row
            // passes through, rather than in each of the several places that
            // can produce a blank (a table's spacers, a prose block opening, a
            // command's own leading newline, a marked note). Any of them may
            // ask for air; none of them can stack it.
            let blank = line.to_string().trim().is_empty();
            if blank && self.last_history_blank {
                continue;
            }
            self.last_history_blank = blank;
            let row = if th::line_cols(line) > width {
                th::truncate_line(line.clone(), width)
            } else {
                line.clone()
            };
            wrote = true;
            out.push_str("\r\n");
            // Clear to end of line before writing: the row scrolling into view
            // may hold longer content from before.
            out.push_str("\x1b[K");
            out.push_str(&ansi::render(&row));
            out.push_str(ansi::RESET);
        }
        // Restore the full-screen scroll region, or the composer's own drawing
        // would be confined to the region above it.
        out.push_str("\x1b[r");
        if !wrote {
            return;
        }
        self.write_raw(&out);
        self.history_rows = (self.history_rows + lines.len()).min(top as usize);
        // The composer's rows were not touched, but the cursor is now
        // somewhere else entirely; the next viewport draw puts it back.
        self.invalidate();
    }

    /// Draw the composer block at the bottom of the screen.
    ///
    /// The viewport is exactly as tall as the block. When the block grows the
    /// screen scrolls to make room (pushing history up, as any shell does);
    /// when it shrinks, the rows it vacates are cleared.
    pub fn draw_viewport(&mut self, rows: &[Line<'static>], cursor: Option<(u16, u16)>) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let block_height = (rows.len() as u16).clamp(1, self.height);
        let top = self.height - block_height;
        // Rows the composer gives back when it shrinks are NOT cleared here.
        // They used to be — otherwise a short session left the old input line
        // stranded above the new one — but `fill_history` now clears the union
        // of the old region and the new one before it draws, so the rows are
        // already clean, and clearing them a second time here erased the
        // history rows it had just written into them.
        if top != self.viewport_top {
            // The composer changed height — a completion dropdown opening, the
            // input gaining a line. It simply takes the rows it needs and
            // gives them back; the caller repaints the history region above it
            // (`fill_history`) so nothing is stolen or left blank.
            //
            // Scrolling on growth was wrong and visible: typing `/` opened an
            // eleven-row dropdown, seven rows of conversation were pushed into
            // scrollback to make space, and when the list narrowed again the
            // composer gave back six rows of nothing.
            self.invalidate();
        }
        self.viewport_top = top;
        self.history_rows = self.history_rows.min(top as usize);
        self.last_block_widths = rows
            .iter()
            .take(block_height as usize)
            .map(th::line_cols)
            .collect();
        self.last_caret_row = cursor
            .map(|(_, y)| y.saturating_sub(top) as usize)
            .unwrap_or(0);
        let area = Rect::new(0, top, self.width, block_height);
        if self.prev.area != area {
            self.prev = Buffer::empty(area);
            self.force_full = true;
        }
        let mut next = Buffer::empty(area);
        for (i, line) in rows.iter().take(block_height as usize).enumerate() {
            next.set_line(0, top + i as u16, line, self.width);
        }
        if self.force_full {
            // Clear each row first: the layout never writes the last column,
            // so a cell diff alone cannot reach residue a reflow left there.
            let mut out = String::new();
            for row in top..top + block_height {
                out.push_str(&format!("\x1b[{};1H\x1b[2K", row + 1));
            }
            self.write_raw(&out);
            let all: Vec<(u16, u16, &ratatui::buffer::Cell)> = next
                .content
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    let (x, y) = (i as u16 % self.width, top + i as u16 / self.width);
                    (x, y, cell)
                })
                .collect();
            let _ = self.backend.draw(all.into_iter());
            self.force_full = false;
        } else {
            let updates = self.prev.diff(&next);
            if !updates.is_empty() {
                let _ = self.backend.draw(updates.into_iter());
            }
        }
        self.prev = next;
        match cursor {
            Some((x, y)) => {
                let _ = self
                    .backend
                    .set_cursor_position(ratatui::layout::Position::new(
                        x.min(self.width.saturating_sub(1)),
                        y.min(self.height.saturating_sub(1)),
                    ));
                if !self.cursor_shown {
                    let _ = self.backend.show_cursor();
                    self.cursor_shown = true;
                }
            }
            None => {
                if self.cursor_shown {
                    let _ = self.backend.hide_cursor();
                    self.cursor_shown = false;
                }
            }
        }
        let _ = Backend::flush(&mut self.backend);
    }

    /// Wipe the screen AND the scrollback, ready for the transcript to be
    /// replayed at the current width.
    ///
    /// This is Codex's resize strategy (`clear_scrollback_and_visible_screen_ansi`)
    /// and it is the only one that actually reflows. Once rows are written to
    /// scrollback the terminal owns them; a renderer that leaves them there can
    /// either re-project over the top (which duplicates, because the terminal's
    /// own reflow has already moved some of them) or leave them at their old
    /// wrapping (which is what "the UI does not resize" looks like). Owning the
    /// whole scrollback and rewriting it is the third option, and it is why
    /// Codex reflows cleanly.
    ///
    /// The sequence mirrors a shell's `clear` plus a scrollback purge: reset
    /// the scroll region, reset styles, home, erase the display, **erase the
    /// scrollback** (`CSI 3J`), home again.
    ///
    /// The cost is explicit: anything on screen before the session started is
    /// purged too. The session already takes the screen over, and Codex makes
    /// the same trade.
    pub fn reset_for_reflow(&mut self) {
        self.last_history_blank = true;
        self.history_rows = 0;
        self.write_raw("\x1b[r\x1b[0m\x1b[H\x1b[2J\x1b[3J\x1b[H");
        self.viewport_top = self.height.saturating_sub(1);
        self.last_block_widths.clear();
        self.last_caret_row = 0;
        self.invalidate();
    }

    /// Screen row the composer starts on.
    pub fn viewport_top(&self) -> usize {
        self.viewport_top as usize
    }

    /// The composer's height for a block of `block_len` rows.
    pub fn viewport_top_for(&self, block_len: usize) -> usize {
        let block_height = (block_len as u16).clamp(1, self.height);
        (self.height - block_height) as usize
    }

    /// Repaint the visible history region — the rows between the top of the
    /// screen and the composer — from `rows`, bottom-aligned on `region`.
    ///
    /// `region` is the height the composer is ABOUT to leave above itself, not
    /// the one it currently occupies. Reading `self.viewport_top` here instead
    /// was a real bug: the caller repaints before it draws, so a composer that
    /// is growing (a spinner arriving the moment a turn starts) bottom-aligned
    /// history on the OLD, taller region and then drew itself over the last
    /// rows of it. Those rows are the newest ones, so a message vanished the
    /// instant it was sent and came back only when the reply scrolled it clear
    /// — and the tail of the conversation went missing right after a resize,
    /// where the gap between the two regions is largest.
    ///
    /// This writes in place and scrolls nothing, so it adds nothing to
    /// scrollback and cannot duplicate: at a fixed width the rows it draws are
    /// exactly the rows already there. It exists for when the composer changes
    /// height and hands rows back, and it deliberately fills only as many rows
    /// as it has content for, leaving whatever the shell put above a short
    /// session untouched.
    pub fn fill_history(&mut self, rows: &[Line<'static>], region: usize) {
        let region = (region as u16).min(self.height);
        if region == 0 || self.width == 0 {
            return;
        }
        let shown = rows.len().min(region as usize);
        let new_start = region - shown as u16;
        // Clear the WHOLE region above the composer — the union of the old and
        // the new one, so a composer that is shrinking cannot leave its own
        // rows stranded above the redraw — not a computed band.
        //
        // Three narrower rules were tried and each left something behind: by
        // length (misses where the band moved to), by the union of old and new
        // positions (still off by the rows the composer covered while it was
        // tall), and by clearing only what is about to be written. Each one
        // showed up as a fragment of the opening card stranded above the
        // redraw. The region is ours to own — `reset_for_reflow` already
        // purges the whole scrollback on a resize — so there is nothing to
        // protect by being clever here, and "clear it all" cannot be wrong.
        let mut out = String::new();
        for row in 0..region.max(self.viewport_top) {
            out.push_str(&format!("\x1b[{};1H\x1b[2K", row + 1));
        }
        self.write_raw(&out);
        if shown == 0 {
            self.history_rows = 0;
            return;
        }
        let top = new_start;
        let area = Rect::new(0, top, self.width, shown as u16);
        let mut buffer = Buffer::empty(area);
        for (i, line) in rows[rows.len() - shown..].iter().enumerate() {
            buffer.set_line(0, top + i as u16, line, self.width);
        }
        let cells: Vec<(u16, u16, &ratatui::buffer::Cell)> = buffer
            .content
            .iter()
            .enumerate()
            .map(|(i, cell)| (i as u16 % self.width, top + i as u16 / self.width, cell))
            .collect();
        let _ = self.backend.draw(cells.into_iter());
        let _ = Backend::flush(&mut self.backend);
        self.history_rows = shown;
    }

    /// Draw a full screen. Used only by the Ctrl+T transcript overlay, which
    /// owns the alternate screen outright and has no history to preserve.
    pub fn draw_full(&mut self, rows: &[Line<'static>]) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let area = Rect::new(0, 0, self.width, self.height);
        let mut next = Buffer::empty(area);
        for (y, line) in rows.iter().take(self.height as usize).enumerate() {
            next.set_line(0, y as u16, line, self.width);
        }
        let mut out = String::new();
        for row in 0..self.height {
            out.push_str(&format!("\x1b[{};1H\x1b[2K", row + 1));
        }
        self.write_raw(&out);
        let all: Vec<(u16, u16, &ratatui::buffer::Cell)> = next
            .content
            .iter()
            .enumerate()
            .map(|(i, cell)| (i as u16 % self.width, i as u16 / self.width, cell))
            .collect();
        let _ = self.backend.draw(all.into_iter());
        if self.cursor_shown {
            let _ = self.backend.hide_cursor();
            self.cursor_shown = false;
        }
        let _ = Backend::flush(&mut self.backend);
        // The overlay leaves the viewport belief meaningless.
        self.invalidate();
    }

    pub fn enter_alt_screen(&mut self) {
        self.write_raw(ALT_SCREEN_ENTER);
        self.invalidate();
    }

    pub fn leave_alt_screen(&mut self) {
        self.write_raw(ALT_SCREEN_LEAVE);
        self.invalidate();
    }

    /// Clear the composer and leave the shell prompt directly under the
    /// conversation.
    pub fn teardown(&mut self) {
        let mut out = String::new();
        for row in self.viewport_top..self.height {
            out.push_str(&format!("\x1b[{};1H\x1b[2K", row + 1));
        }
        out.push_str(&format!("\x1b[{};1H", self.viewport_top + 1));
        out.push_str("\x1b[?25h");
        self.write_raw(&out);
        self.cursor_shown = true;
    }

    /// Fix the geometry without asking the terminal, so layout and
    /// composition are testable headlessly.
    #[cfg(test)]
    pub fn set_test_size(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.viewport_top = height.saturating_sub(1);
        self.backend.writer_mut().silent = true;
        self.invalidate();
    }

    /// The escapes ratatui's backend has no API for. Everything that carries
    /// styled content is drawn as cells or rendered through `theme::ansi`.
    fn write_raw(&mut self, bytes: &str) {
        let _ = self.backend.writer_mut().write_all(bytes.as_bytes());
        let _ = Backend::flush(&mut self.backend);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_width_never_touches_the_last_column() {
        assert_eq!(usable_width(80), 79);
        assert_eq!(usable_width(1), 0);
        assert_eq!(usable_width(0), 0);
    }

    /// History is bottom-aligned on the region the composer is ABOUT to leave,
    /// not the one it currently occupies.
    ///
    /// The caller repaints history before it draws the composer, so reading
    /// `viewport_top` here aligned on the old region and the composer then
    /// drew over the newest rows of it. Every way the composer changes height
    /// hit it: the spinner arriving on submit made the message just sent
    /// disappear until the reply scrolled it clear, and a resize — where the
    /// two regions differ most — cut the tail of the conversation off.
    #[test]
    fn history_is_bottom_aligned_on_the_region_it_is_given() {
        let rows: Vec<Line<'static>> = (0..100).map(|i| Line::from(format!("row {i}"))).collect();

        // The composer is growing: three rows taller than it was.
        let mut term = Term::new();
        term.set_test_size(80, 40);
        assert_eq!(term.viewport_top, 39);
        term.fill_history(&rows, 36);
        assert_eq!(term.history_rows, 36, "aligned on the old region");

        // And shrinking: the region it hands back is filled, not left blank.
        let mut term = Term::new();
        term.set_test_size(80, 40);
        term.viewport_top = 30;
        term.fill_history(&rows, 38);
        assert_eq!(term.history_rows, 38);

        // A region taller than the screen cannot be asked for.
        let mut term = Term::new();
        term.set_test_size(80, 10);
        term.fill_history(&rows, 999);
        assert_eq!(term.history_rows, 10);
    }
}

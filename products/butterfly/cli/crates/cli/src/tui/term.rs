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
    /// What a silent (headless test) terminal would have written, so a test
    /// can assert the escapes a step emits.
    #[cfg(test)]
    captured: Vec<u8>,
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
            #[cfg(test)]
            captured: Vec::new(),
        }
    }
}

impl Write for TracedOut {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(Trace(file)) = &mut self.trace {
            let _ = file.write_all(buf);
        }
        if self.silent {
            #[cfg(test)]
            self.captured.extend_from_slice(buf);
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
    /// The band of screen rows above the composer that hold content:
    /// `[content_start, content_end)`, 0-based. Rows above the band are blank
    /// (a shrinking composer pulled the content down and left them), rows
    /// between the band and the composer are blank (a session opening under
    /// a shell prompt, or a screen just wiped for a reflow). The shell's own
    /// rows count as content: they are never redrawn, only scrolled away.
    ///
    /// The invariant this bookkeeping keeps is the whole point of the
    /// renderer: **the terminal's scrollback plus the band is the transcript,
    /// in order, each row exactly once.** Rows leave the band only by
    /// scrolling into scrollback, and only `insert_history` and a growing
    /// composer scroll. Nothing is ever redrawn from the transcript at a
    /// fixed width — the earlier design did, bottom-aligned on the region the
    /// composer was about to leave, and it broke the invariant: growth
    /// clipped rows off the top of the band without scrolling them, a commit
    /// then scrolled the region, and the shrink repainted rows that were
    /// already in scrollback. Measured on a streaming table, whose preview
    /// changed the composer height on every row: rows lost, rows twice.
    content_start: u16,
    content_end: u16,
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
            // Until `take_over` asks where the cursor is, assume the shell has
            // filled the screen: scrolling too much loses nothing, clipping
            // would.
            content_start: 0,
            content_end: top,
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
        self.content_end = self.content_end.min(self.viewport_top);
        self.content_start = self.content_start.min(self.content_end);
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
    /// Nothing is flooded or erased: the composer claims the bottom row, the
    /// rows above the shell's cursor count as content that is only ever
    /// scrolled away, and the first rows committed land directly under the
    /// prompt — the way Codex opens. The one cursor-position query the
    /// renderer makes happens here, once, before the input thread exists;
    /// if the terminal does not answer, the screen is assumed full and the
    /// first commit simply scrolls.
    pub fn take_over(&mut self) {
        self.viewport_top = self.height.saturating_sub(1);
        let row = crossterm::cursor::position()
            .map(|(_, y)| y)
            .unwrap_or(self.viewport_top)
            .min(self.viewport_top);
        self.content_start = 0;
        self.content_end = row;
        self.invalidate();
    }

    /// Rows above the composer that hold content, as `(start, end)`.
    #[cfg(test)]
    fn band(&self) -> (u16, u16) {
        (self.content_start, self.content_end)
    }

    #[cfg(test)]
    fn written(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.backend.writer_mut().captured))
            .into_owned()
    }

    /// Write committed rows above the composer, into the terminal's own
    /// scrollback once they leave the screen.
    ///
    /// Every line must already fit one screen row; `render_entry` guarantees
    /// that, and anything longer is clamped here rather than being allowed to
    /// soft-wrap into a second row the scroll arithmetic does not know about.
    ///
    /// Rows go where a shell would put them. Blank rows directly under the
    /// content are filled first (a session opening under the prompt, a screen
    /// just wiped for a reflow). Blank rows ABOVE the content — left when a
    /// shrinking composer pulled the content down — are deleted at the top
    /// (`DL`, which never reaches scrollback) so the content moves up and
    /// frees rows at the bottom. Only when the band fills the whole region
    /// does a row scroll: a `\r\n` on the last row of a scroll region
    /// `1;top` scrolls that region alone, pushing its top row into native
    /// scrollback and leaving the composer untouched.
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
        let mut rows: Vec<Line<'static>> = Vec::with_capacity(lines.len());
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
            rows.push(if th::line_cols(line) > width {
                th::truncate_line(line.clone(), width)
            } else {
                line.clone()
            });
        }
        if rows.is_empty() {
            return;
        }
        let paint = |out: &mut String, row0: u16, line: &Line<'static>| {
            out.push_str(&format!("\x1b[{};1H\x1b[2K", row0 + 1));
            out.push_str(&ansi::render(line));
            out.push_str(ansi::RESET);
        };
        let mut out = String::new();
        let mut remaining = rows.as_slice();
        // 1. Blank rows under the content.
        let below = (top - self.content_end) as usize;
        let take = remaining.len().min(below);
        for (i, line) in remaining[..take].iter().enumerate() {
            paint(&mut out, self.content_end + i as u16, line);
        }
        self.content_end += take as u16;
        remaining = &remaining[take..];
        // 2. Blank rows above the content: delete them at the top of the
        //    region, which shifts the content up and frees rows at the
        //    bottom. Deleted lines are discarded, never saved, so nothing
        //    blank reaches scrollback.
        if !remaining.is_empty() && self.content_start > 0 {
            let take = remaining.len().min(self.content_start as usize) as u16;
            out.push_str(&format!("\x1b[1;{top}r\x1b[1;1H\x1b[{take}M\x1b[r"));
            self.content_start -= take;
            self.content_end -= take;
            for (i, line) in remaining[..take as usize].iter().enumerate() {
                paint(&mut out, self.content_end + i as u16, line);
            }
            self.content_end += take;
            remaining = &remaining[take as usize..];
        }
        // 3. The band fills the region: scroll, and the top row leaves into
        //    native scrollback.
        if !remaining.is_empty() {
            out.push_str(&format!("\x1b[1;{top}r\x1b[{top};1H"));
            for line in remaining {
                out.push_str("\r\n\x1b[K");
                out.push_str(&ansi::render(line));
                out.push_str(ansi::RESET);
            }
            // Restore the full-screen scroll region, or the composer's own
            // drawing would be confined to the region above it.
            out.push_str("\x1b[r");
            self.content_start = 0;
            self.content_end = top;
        }
        self.write_raw(&out);
        // The composer's rows were not touched, but the cursor is now
        // somewhere else entirely; the next viewport draw puts it back.
        self.invalidate();
    }

    /// Draw the composer block at the bottom of the screen.
    ///
    /// The viewport is exactly as tall as the block, and a change of height
    /// moves the content band, never redraws it:
    ///
    /// - **Growing** by `g` rows, the composer needs the `g` rows above it.
    ///   Blank rows above the band are deleted first (`DL` at the top of the
    ///   region — discarded, not saved), and only the remainder scrolls the
    ///   region, pushing that many content rows into native scrollback, as a
    ///   shell would. Nothing is clipped: every row is either on screen or in
    ///   scrollback.
    /// - **Shrinking** by `s` rows, a band that touched the composer is pulled
    ///   down to keep touching it (`IL` at the top of the region — the old
    ///   composer rows fall off the bottom, `s` blank rows appear at the top,
    ///   where the next growth or commit consumes them). A band that did not
    ///   touch it stays put and the vacated rows are cleared.
    ///
    /// The earlier design let growth clip the band and repainted it from the
    /// transcript on every height change; see `content_start`.
    pub fn draw_viewport(&mut self, rows: &[Line<'static>], cursor: Option<(u16, u16)>) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let block_height = (rows.len() as u16).clamp(1, self.height);
        let top = self.height - block_height;
        let old_top = self.viewport_top;
        if top < old_top {
            let mut out = String::new();
            if self.content_end > top {
                let excess = self.content_end - top;
                let from_blanks = excess.min(self.content_start);
                if from_blanks > 0 {
                    out.push_str(&format!(
                        "\x1b[1;{old_top}r\x1b[1;1H\x1b[{from_blanks}M\x1b[r"
                    ));
                    self.content_start -= from_blanks;
                    self.content_end -= from_blanks;
                }
                let scroll = excess - from_blanks;
                if scroll > 0 {
                    out.push_str(&format!("\x1b[1;{old_top}r\x1b[{old_top};1H"));
                    out.push_str(&"\n".repeat(scroll as usize));
                    out.push_str("\x1b[r");
                    self.content_start = 0;
                    self.content_end -= scroll;
                }
            }
            self.write_raw(&out);
            self.invalidate();
        } else if top > old_top {
            let shift = top - old_top;
            let mut out = String::new();
            if self.content_end == old_top && self.content_end > 0 {
                out.push_str(&format!("\x1b[1;{top}r\x1b[1;1H\x1b[{shift}L\x1b[r"));
                self.content_start += shift;
                self.content_end += shift;
            } else {
                for row in old_top..top {
                    out.push_str(&format!("\x1b[{};1H\x1b[2K", row + 1));
                }
            }
            self.write_raw(&out);
            self.invalidate();
        }
        self.viewport_top = top;
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
        self.content_start = 0;
        self.content_end = 0;
        self.write_raw("\x1b[r\x1b[0m\x1b[H\x1b[2J\x1b[3J\x1b[H");
        self.viewport_top = self.height.saturating_sub(1);
        self.last_block_widths.clear();
        self.last_caret_row = 0;
        self.invalidate();
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

    fn term_at(height: u16, band: (u16, u16)) -> Term {
        let mut term = Term::new();
        term.set_test_size(80, height);
        term.viewport_top = height - 1;
        term.content_start = band.0;
        term.content_end = band.1;
        term.written();
        term
    }

    fn rows(n: usize) -> Vec<Line<'static>> {
        (0..n).map(|i| Line::from(format!("row {i}"))).collect()
    }

    /// A commit lands where a shell's output would: in the blank rows under
    /// the content first, without scrolling anything.
    #[test]
    fn commits_fill_the_rows_under_the_content_before_scrolling() {
        let mut term = term_at(40, (0, 10));
        term.insert_history(&rows(5));
        assert_eq!(term.band(), (0, 15));
        let out = term.written();
        assert!(
            !out.contains("\r\n"),
            "scrolled with room to spare: {out:?}"
        );
        assert!(
            out.contains("\x1b[11;1H"),
            "first row not under the content: {out:?}"
        );
        // Fill the region and keep going: the overflow scrolls, once per row.
        term.insert_history(&rows(30));
        assert_eq!(term.band(), (0, 39));
        let out = term.written();
        assert_eq!(out.matches("\r\n").count(), 6, "{out:?}");
        assert!(out.contains("\x1b[1;39r"), "scroll region not set: {out:?}");
    }

    /// Blank rows above the band (left by a shrink) are consumed by deleting
    /// them at the top, which never touches scrollback.
    #[test]
    fn commits_consume_blank_rows_above_the_content_without_scrolling() {
        let mut term = term_at(40, (5, 39));
        term.insert_history(&rows(2));
        assert_eq!(term.band(), (3, 39));
        let out = term.written();
        assert!(out.contains("\x1b[2M"), "no DL at the top: {out:?}");
        assert!(!out.contains("\r\n"), "{out:?}");
        // More rows than blanks: the blanks go first, then it scrolls.
        term.insert_history(&rows(5));
        assert_eq!(term.band(), (0, 39));
        let out = term.written();
        assert!(out.contains("\x1b[3M"), "{out:?}");
        assert_eq!(out.matches("\r\n").count(), 2, "{out:?}");
    }

    /// A growing composer scrolls only what it must: blank rows above the
    /// band are deleted first, and only the remainder scrolls into
    /// scrollback. Nothing is clipped off the top.
    #[test]
    fn composer_growth_scrolls_only_what_it_must() {
        let mut term = term_at(40, (3, 39));
        term.draw_viewport(&rows(6), None);
        assert_eq!(term.viewport_top, 34);
        assert_eq!(term.band(), (0, 34));
        let out = term.written();
        assert!(out.contains("\x1b[1;39r\x1b[1;1H\x1b[3M"), "{out:?}");
        assert!(out.contains("\x1b[39;1H\n\n\x1b[r"), "{out:?}");
        // Growing into blank rows under the band scrolls nothing at all.
        let mut term = term_at(40, (0, 20));
        term.draw_viewport(&rows(6), None);
        assert_eq!(term.band(), (0, 20));
        let out = term.written();
        assert!(!out.contains('\n') && !out.contains('M'), "{out:?}");
    }

    /// A shrinking composer pulls a band that touched it down to keep
    /// touching it, so no gap opens above the input; a band that did not
    /// touch it stays where it is.
    #[test]
    fn composer_shrink_pulls_the_content_down_to_meet_it() {
        let mut term = term_at(40, (0, 30));
        term.viewport_top = 30;
        term.draw_viewport(&rows(1), None);
        assert_eq!(term.viewport_top, 39);
        assert_eq!(term.band(), (9, 39));
        let out = term.written();
        assert!(out.contains("\x1b[1;39r\x1b[1;1H\x1b[9L\x1b[r"), "{out:?}");
        let mut term = term_at(40, (0, 12));
        term.viewport_top = 30;
        term.draw_viewport(&rows(1), None);
        assert_eq!(term.band(), (0, 12));
        let out = term.written();
        assert!(!out.contains('L'), "{out:?}");
        assert!(
            out.contains("\x1b[31;1H\x1b[2K"),
            "vacated rows not cleared: {out:?}"
        );
    }
}

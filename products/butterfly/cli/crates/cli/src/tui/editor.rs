//! The input editor: a plain text buffer with a byte cursor, multi-line
//! support, prompt history, and paste placeholders. Pure state — no terminal
//! I/O — so every operation is unit-testable.

use std::path::PathBuf;

/// A pasted block collapsed to a short placeholder in the visible buffer and
/// expanded back to the original text on submit.
struct Paste {
    placeholder: String,
    content: String,
}

pub struct Editor {
    text: String,
    /// Byte offset into `text`, always on a char boundary.
    cursor: usize,
    pastes: Vec<Paste>,
    paste_counter: usize,
    history: Vec<String>,
    history_path: Option<PathBuf>,
    /// Some(i) while navigating history; the draft is stashed aside.
    history_index: Option<usize>,
    draft: String,
}

const HISTORY_CAP: usize = 500;
/// Pastes below both limits are inserted literally.
const PASTE_INLINE_MAX_CHARS: usize = 200;

impl Editor {
    pub fn new(history_path: Option<PathBuf>) -> Self {
        let history = history_path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| {
                text.lines()
                    .filter(|line| !line.is_empty())
                    .map(|line| line.replace("\\n", "\n"))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            text: String::new(),
            cursor: 0,
            pastes: Vec::new(),
            paste_counter: 0,
            history,
            history_path,
            history_index: None,
            draft: String::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn insert(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.history_index = None;
    }

    pub fn insert_str(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.history_index = None;
    }

    /// Insert pasted text: short single-line pastes go in literally, anything
    /// else collapses to a `[pasted #N: X lines]` placeholder.
    pub fn insert_paste(&mut self, content: &str) {
        let content = content.replace("\r\n", "\n").replace('\r', "\n");
        if !content.contains('\n') && content.chars().count() <= PASTE_INLINE_MAX_CHARS {
            self.insert_str(&content);
            return;
        }
        self.paste_counter += 1;
        let lines = content.lines().count().max(1);
        let placeholder = format!("[pasted #{}: {} lines]", self.paste_counter, lines);
        self.insert_str(&placeholder);
        self.pastes.push(Paste {
            placeholder,
            content,
        });
    }

    fn prev_boundary(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 {
            i -= 1;
            if self.text.is_char_boundary(i) {
                break;
            }
        }
        i
    }

    fn next_boundary(&self, from: usize) -> usize {
        let mut i = from;
        while i < self.text.len() {
            i += 1;
            if self.text.is_char_boundary(i) {
                break;
            }
        }
        i
    }

    /// If the cursor sits right after a paste placeholder, remove the whole
    /// placeholder atomically; otherwise delete one char.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if let Some(index) = self.pastes.iter().position(|paste| {
            self.cursor >= paste.placeholder.len()
                && self.text[..self.cursor].ends_with(&paste.placeholder)
        }) {
            let paste = self.pastes.remove(index);
            let start = self.cursor - paste.placeholder.len();
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
        } else {
            let start = self.prev_boundary(self.cursor);
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
        }
        self.history_index = None;
    }

    pub fn delete_forward(&mut self) {
        if self.cursor < self.text.len() {
            let end = self.next_boundary(self.cursor);
            self.text.replace_range(self.cursor..end, "");
            self.history_index = None;
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.prev_boundary(self.cursor);
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor = self.next_boundary(self.cursor);
        }
    }

    /// Start of the current visual line (after the previous '\n').
    pub fn line_start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    /// End of the current visual line (before the next '\n').
    pub fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map(|i| self.cursor + i)
            .unwrap_or(self.text.len())
    }

    pub fn move_home(&mut self) {
        self.cursor = self.line_start();
    }

    pub fn move_end(&mut self) {
        self.cursor = self.line_end();
    }

    pub fn on_first_line(&self) -> bool {
        !self.text[..self.cursor].contains('\n')
    }

    pub fn on_last_line(&self) -> bool {
        !self.text[self.cursor..].contains('\n')
    }

    pub fn move_up_line(&mut self) {
        let column = self.cursor - self.line_start();
        let prev_end = self.line_start().saturating_sub(1);
        let prev_start = self.text[..prev_end]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        self.cursor = (prev_start + column).min(prev_end);
        while !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }

    pub fn move_down_line(&mut self) {
        let column = self.cursor - self.line_start();
        let next_start = self.line_end() + 1;
        if next_start > self.text.len() {
            return;
        }
        let next_end = self.text[next_start..]
            .find('\n')
            .map(|i| next_start + i)
            .unwrap_or(self.text.len());
        self.cursor = (next_start + column).min(next_end);
        while !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }

    /// Delete the word before the cursor (Ctrl-W).
    pub fn delete_word_back(&mut self) {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end_matches(|c: char| c.is_whitespace());
        let start = trimmed
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.history_index = None;
    }

    /// Clear the whole buffer (Ctrl-U / Ctrl-C with pending input).
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.pastes.clear();
        self.history_index = None;
    }

    /// Replace an arbitrary byte range (completion acceptance). The cursor
    /// lands at the end of the replacement.
    pub fn replace_range(&mut self, start: usize, end: usize, replacement: &str) {
        self.text.replace_range(start..end, replacement);
        self.cursor = start + replacement.len();
        self.history_index = None;
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next_index = match self.history_index {
            None => {
                self.draft = self.text.clone();
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_index = Some(next_index);
        self.text = self.history[next_index].clone();
        self.cursor = self.text.len();
    }

    pub fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            self.history_index = Some(index + 1);
            self.text = self.history[index + 1].clone();
        } else {
            self.history_index = None;
            self.text = std::mem::take(&mut self.draft);
        }
        self.cursor = self.text.len();
    }

    /// Take the buffer for submission: expands paste placeholders, records
    /// history (with the compact placeholder form), and clears the editor.
    pub fn submit(&mut self) -> String {
        let visible = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.history_index = None;
        let trimmed = visible.trim();
        if !trimmed.is_empty() && self.history.last().map(String::as_str) != Some(trimmed) {
            self.history.push(trimmed.to_string());
            if self.history.len() > HISTORY_CAP {
                let excess = self.history.len() - HISTORY_CAP;
                self.history.drain(..excess);
            }
        }
        let mut expanded = visible;
        for paste in self.pastes.drain(..) {
            if let Some(at) = expanded.find(&paste.placeholder) {
                expanded.replace_range(at..at + paste.placeholder.len(), &paste.content);
            }
        }
        expanded.trim().to_string()
    }

    pub fn save_history(&self) {
        let Some(path) = &self.history_path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text: String = self
            .history
            .iter()
            .map(|entry| format!("{}\n", entry.replace('\n', "\\n")))
            .collect();
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        Editor::new(None)
    }

    #[test]
    fn insert_move_and_delete_respect_char_boundaries() {
        let mut e = editor();
        e.insert_str("héllo");
        assert_eq!(e.cursor(), "héllo".len());
        e.move_left();
        e.move_left();
        e.backspace(); // removes the second 'l'
        assert_eq!(e.text(), "hélo");
        e.move_home();
        e.delete_forward();
        assert_eq!(e.text(), "élo");
    }

    #[test]
    fn small_single_line_pastes_insert_literally() {
        let mut e = editor();
        e.insert_paste("just a path");
        assert_eq!(e.text(), "just a path");
        assert_eq!(e.submit(), "just a path");
    }

    #[test]
    fn large_pastes_collapse_and_expand_on_submit() {
        let mut e = editor();
        e.insert_str("look at this: ");
        e.insert_paste("line one\nline two\nline three");
        assert_eq!(e.text(), "look at this: [pasted #1: 3 lines]");
        let submitted = e.submit();
        assert_eq!(submitted, "look at this: line one\nline two\nline three");
    }

    #[test]
    fn backspace_removes_a_placeholder_atomically() {
        let mut e = editor();
        e.insert_paste("a\nb");
        assert!(e.text().starts_with("[pasted #1"));
        e.backspace();
        assert_eq!(e.text(), "");
        assert_eq!(e.submit(), "");
    }

    #[test]
    fn history_navigation_round_trips_the_draft() {
        let mut e = editor();
        e.insert_str("first");
        e.submit();
        e.insert_str("second");
        e.submit();
        e.insert_str("draft");
        e.history_prev();
        assert_eq!(e.text(), "second");
        e.history_prev();
        assert_eq!(e.text(), "first");
        e.history_next();
        e.history_next();
        assert_eq!(e.text(), "draft");
    }

    #[test]
    fn vertical_movement_tracks_columns() {
        let mut e = editor();
        e.insert_str("short\nlonger line");
        assert!(e.on_last_line());
        e.move_up_line();
        assert!(e.on_first_line());
        assert_eq!(&e.text()[e.cursor()..], "\nlonger line");
        // Column 5 is preserved when moving back down.
        e.move_down_line();
        assert_eq!(&e.text()[e.cursor()..], "r line");
        // From a column past the shorter line's end, up clamps to its end.
        e.move_end();
        e.move_up_line();
        assert_eq!(&e.text()[e.cursor()..], "\nlonger line");
    }

    #[test]
    fn word_delete_and_clear() {
        let mut e = editor();
        e.insert_str("fix the parser bug");
        e.delete_word_back();
        assert_eq!(e.text(), "fix the parser ");
        e.clear();
        assert!(e.is_empty());
    }
}

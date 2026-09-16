//! First-run connect wizard: a tiny standalone raw-mode flow used when no
//! API key exists anywhere (env or credential store). Pick a provider,
//! paste the key (masked), save. Runs before the session TUI boots; the
//! in-session `/connect` flow lives in `ui.rs`.

use std::io::Write as _;

use anyhow::{Context, Result};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

use super::chrome::option_row;
use super::theme::{self as th, ACCENT, ACCENT_BOLD, DIM, RESET};

/// Mask a key for display: all bullets, fixed floor so length leaks little.
pub fn masked(len: usize) -> String {
    th::BULLET.repeat(len.max(1))
}

struct RawGuard;

impl RawGuard {
    fn new() -> Result<Self> {
        crossterm::terminal::enable_raw_mode().context("cannot enable raw terminal mode")?;
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);
        Ok(Self)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn draw(lines: &[String], previous_rows: usize, typing: bool) -> usize {
    let mut buf = String::new();
    if previous_rows > 0 {
        buf.push_str(&format!("\x1b[{previous_rows}A"));
    }
    buf.push_str("\r\x1b[J");
    buf.push_str(&lines.join("\r\n"));
    // No cursor while selecting; show it when the key prompt expects input.
    buf.push_str(if typing { "\x1b[?25h" } else { "\x1b[?25l" });
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(buf.as_bytes());
    let _ = out.flush();
    lines.len().saturating_sub(1)
}

/// Run the wizard; returns `Some((provider, api_key))` on submit, `None` if
/// the user backed out (Esc / Ctrl+C / Ctrl+D).
pub fn standalone_connect(
    providers: &[(String, String, bool)],
) -> Result<Option<(String, String)>> {
    if providers.is_empty() {
        return Ok(None);
    }
    let _guard = RawGuard::new()?;
    let width = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80)
        .max(20);
    let mut selected = 0usize;
    let mut stage_key: Option<String> = None;
    let mut rows = 0usize;
    loop {
        let lines: Vec<String> = match &stage_key {
            None => {
                // The very first thing a new user sees. Say what is about
                // to happen and where the key will live, then get out of
                // the way.
                let mut lines = vec![
                    format!("{ACCENT_BOLD}{} butterfly{RESET}", th::MARK_BRAND),
                    String::new(),
                    format!("{DIM}  pick the provider you have a key for{RESET}"),
                    String::new(),
                ];
                for (i, (_, label, connected)) in providers.iter().enumerate() {
                    let hint = if *connected { "key already saved" } else { "" };
                    lines.push(option_row(label, hint, i == selected, width));
                }
                lines.push(String::new());
                lines.push(format!("{DIM}  ⇡⇣ move ⋅ enter continue ⋅ esc quit{RESET}"));
                lines
            }
            Some(key) => vec![
                format!("{ACCENT_BOLD}{} butterfly{RESET}", th::MARK_BRAND),
                String::new(),
                format!(
                    "{DIM}  paste your {} API key ‒ input is masked{RESET}",
                    providers[selected].1
                ),
                String::new(),
                format!(
                    "  {ACCENT}{}{RESET} {}",
                    th::MARK_USER,
                    if key.is_empty() {
                        String::new()
                    } else {
                        masked(key.chars().count())
                    }
                ),
                String::new(),
                format!("{DIM}  stored 0600 in your config dir, never in shell history{RESET}"),
                format!("{DIM}  enter save ⋅ esc back{RESET}"),
            ],
        };
        rows = draw(&lines, rows, stage_key.is_some());
        let event = crossterm::event::read()?;
        let Event::Key(key_event) = event else {
            if let Event::Paste(text) = event {
                if let Some(key) = &mut stage_key {
                    key.push_str(text.trim());
                }
            }
            continue;
        };
        if key_event.kind != KeyEventKind::Press && key_event.kind != KeyEventKind::Repeat {
            continue;
        }
        let control = key_event.modifiers.contains(KeyModifiers::CONTROL);
        match (&mut stage_key, key_event.code) {
            (_, KeyCode::Char('c')) if control => return finish(rows, None),
            (_, KeyCode::Char('d')) if control => return finish(rows, None),
            (None, KeyCode::Esc) => return finish(rows, None),
            (None, KeyCode::Up) => selected = selected.saturating_sub(1),
            (None, KeyCode::Down) => selected = (selected + 1).min(providers.len() - 1),
            (None, KeyCode::Enter) => stage_key = Some(String::new()),
            (Some(_), KeyCode::Esc) => stage_key = None,
            (Some(key), KeyCode::Backspace) => {
                key.pop();
            }
            (Some(key), KeyCode::Enter) => {
                let entered = key.trim().to_string();
                if !entered.is_empty() {
                    let provider = providers[selected].0.clone();
                    return finish(rows, Some((provider, entered)));
                }
            }
            (Some(key), KeyCode::Char(ch)) if !control => key.push(ch),
            _ => {}
        }
    }
}

fn finish(rows: usize, result: Option<(String, String)>) -> Result<Option<(String, String)>> {
    let mut buf = String::new();
    if rows > 0 {
        buf.push_str(&format!("\x1b[{rows}A"));
    }
    buf.push_str("\r\x1b[J\x1b[?25h");
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(buf.as_bytes());
    let _ = out.flush();
    Ok(result)
}

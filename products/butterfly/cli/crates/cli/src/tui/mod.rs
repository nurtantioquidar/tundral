//! Inline terminal UI: streamed output in native scrollback, a custom
//! bottom block (editor with @-mention completion, dropdown, status bar),
//! arrow-key approvals, paste placeholders. Active only when stdin and
//! stdout are TTYs; pipes get the plain loop in `repl`.

mod completion;
pub mod connect;
mod editor;
mod markdown;
mod ui;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use butterfly_core::domain::Model;
use butterfly_core::session::store::SessionStore;
use butterfly_core::session::Agent;
use tokio_util::sync::CancellationToken;

use crate::repl::{handle_command, mode_name, turn_status_line, CommandResult};
use completion::CompletionIndex;
use editor::Editor;
use ui::{Status, Ui, UiAction, UiGate, UiSink};

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const RED: &str = "\x1b[31m";

const COMMANDS: [(&str, &str); 8] = [
    ("compact", "force a compaction now; optional focus text"),
    ("connect", "save a provider API key"),
    ("exit", "exit"),
    ("help", "show commands"),
    ("model", "show or switch model"),
    ("plan", "show the tracked working state"),
    ("quit", "exit"),
    ("usage", "tokens, cost, cache-hit rate"),
];

const SYMBOL_CAP: usize = 8_000;

/// Restores the terminal even on early return.
struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self> {
        crossterm::terminal::enable_raw_mode().context("cannot enable raw terminal mode")?;
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn status_of(agent: &Agent) -> Status {
    Status {
        model: agent.model().wire_name().to_string(),
        mode: mode_name(agent.permission_mode()),
        ctx_pct: agent.utilization(),
    }
}

fn next_mode(mode: butterfly_core::session::approval::PermissionMode) -> &'static str {
    use butterfly_core::session::approval::PermissionMode;
    match mode {
        PermissionMode::Ask => "auto",
        PermissionMode::Auto => "read-only",
        PermissionMode::ReadOnly => "ask",
    }
}

fn set_mode(agent: &mut Agent, name: &str) {
    use butterfly_core::session::approval::PermissionMode;
    agent.set_permission_mode(match name {
        "auto" => PermissionMode::Auto,
        "read-only" => PermissionMode::ReadOnly,
        _ => PermissionMode::Ask,
    });
}

/// Files the session already touched, most useful first (changed then read).
fn recency_of(agent: &Agent) -> Vec<String> {
    let state = agent.working_state();
    let mut recency: Vec<String> = state.files_changed.iter().cloned().collect();
    for path in &state.files_read {
        if !state.files_changed.contains(path) {
            recency.push(path.clone());
        }
    }
    recency
}

fn collect_symbols(agent: &mut Agent) -> Vec<(String, String, String)> {
    if !agent.atlas_enabled() {
        return Vec::new();
    }
    let mut symbols = Vec::new();
    for (path, index) in agent.atlas_mut().files() {
        for def in &index.extraction.definitions {
            symbols.push((
                def.name.clone(),
                format!("{}:{}", path.display(), def.line),
                def.kind.label().to_string(),
            ));
            if symbols.len() >= SYMBOL_CAP {
                return symbols;
            }
        }
    }
    symbols
}

/// The event thread owns stdin for the whole session: keys edit the block
/// (or answer an approval menu) locally; only submits, interrupts, and EOF
/// surface to the session loop. Poll timeouts drive the spinner.
fn spawn_event_thread(
    ui: std::sync::Arc<Ui>,
    sender: tokio::sync::mpsc::UnboundedSender<UiAction>,
) {
    std::thread::spawn(move || loop {
        match crossterm::event::poll(Duration::from_millis(120)) {
            Ok(true) => match crossterm::event::read() {
                Ok(event) => {
                    if let Some(action) = ui.handle_event(event) {
                        let eof = matches!(action, UiAction::Eof);
                        if sender.send(action).is_err() || eof {
                            break;
                        }
                    }
                }
                Err(_) => {
                    let _ = sender.send(UiAction::Eof);
                    break;
                }
            },
            Ok(false) => ui.tick(),
            Err(_) => {
                let _ = sender.send(UiAction::Eof);
                break;
            }
        }
    });
}

pub async fn run(mut agent: Agent, root: PathBuf, show_reasoning: bool) -> Result<()> {
    if agent.atlas_enabled() {
        let _ = agent.atlas_mut().refresh();
    }
    let symbols = collect_symbols(&mut agent);
    let history_path = SessionStore::state_root().join("repl-history");
    let editor = Editor::new(Some(history_path));
    let index = CompletionIndex::new(root, COMMANDS.to_vec());

    let guard = TerminalGuard::new()?;
    let ui = Ui::new(editor, index, status_of(&agent), show_reasoning);
    ui.enter_screen();
    ui.set_session_data(recency_of(&agent), symbols);
    // Model and mode already live in the footer; the header stays lean.
    ui.print_note(&format!(
        "butterfly · session {} · /help for commands",
        agent.session_id,
    ));

    let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
    spawn_event_thread(ui.clone(), sender);

    // Steered messages left over when a turn ends become the next turns.
    let mut pending: VecDeque<String> = VecDeque::new();
    loop {
        let input = if let Some(queued) = pending.pop_front() {
            ui.print(&format!("{DIM}[queued]{RESET} {queued}"));
            queued
        } else {
            match events.recv().await {
                Some(UiAction::Submit(text)) => text,
                Some(UiAction::CycleMode) => {
                    let next = next_mode(agent.permission_mode());
                    set_mode(&mut agent, next);
                    ui.set_status(status_of(&agent));
                    continue;
                }
                Some(UiAction::Interrupt) => continue,
                Some(UiAction::Eof) | None => break,
            }
        };
        match handle_command(&mut agent, &input).await {
            CommandResult::Quit => break,
            CommandResult::Connect => {
                // Cancel resolves as None/Err: the composer returns silently.
                if let Ok(Some((provider, api_key))) = ui.open_connect().await {
                    match crate::credentials::save_key(&provider, &api_key) {
                        Ok(path) => {
                            let mut note = format!(
                                "connected · {provider} key saved to {} · new sessions use it automatically",
                                path.display()
                            );
                            if std::env::var("DEEPSEEK_API_KEY").is_ok() {
                                note.push_str(
                                    " (note: DEEPSEEK_API_KEY is set and takes precedence)",
                                );
                            }
                            ui.print_note(&note);
                        }
                        Err(error) => {
                            ui.print(&format!("{RED}could not save the key: {error}{RESET}"));
                        }
                    }
                }
                continue;
            }
            CommandResult::PickModel => {
                const MODELS: [Model; 2] = [Model::DeepseekV4Flash, Model::DeepseekV4Pro];
                let current = agent.model();
                let items: Vec<String> = MODELS
                    .iter()
                    .map(|model| {
                        let mut label = format!("{} ({})", model.display_name(), model.wire_name());
                        if *model == current {
                            label.push_str(" · current");
                        }
                        label
                    })
                    .collect();
                let initial = MODELS.iter().position(|m| *m == current).unwrap_or(0);
                match ui.open_select("switch model", items, initial).await {
                    Ok(Some(index)) if MODELS[index] != current => {
                        let model = MODELS[index];
                        agent.set_model(model);
                        crate::state::remember_model(model.wire_name());
                        ui.print_note(&format!(
                            "model set: {} ({}); next request re-caches the prefix",
                            model.display_name(),
                            model.wire_name()
                        ));
                    }
                    // Cancel / unchanged: the composer returns, no residue.
                    _ => {}
                }
                ui.set_status(status_of(&agent));
                continue;
            }
            CommandResult::Output(text) => {
                ui.print(text.trim_end_matches('\n'));
                ui.set_status(status_of(&agent));
                continue;
            }
            CommandResult::NotACommand => {}
        }

        let cancel = CancellationToken::new();
        let steer = agent.steer_handle();
        ui.begin_activity();
        let gate = UiGate::new(ui.clone());
        let turn_started = std::time::Instant::now();
        // Shift+Tab presses during a turn apply once it ends (the running
        // turn's gate keeps the mode it started with).
        let mut mode_cycles = 0usize;
        let result = {
            let mut sink = UiSink::new(ui.clone());
            let turn = agent.run_turn(input, &gate, &mut sink, &cancel);
            tokio::pin!(turn);
            loop {
                tokio::select! {
                    result = &mut turn => break result,
                    event = events.recv() => match event {
                        Some(UiAction::Submit(line)) => {
                            steer.lock().expect("steer lock").push_back(line);
                            ui.print(&format!(
                                "{DIM}[queued: delivered at the next turn boundary]{RESET}"
                            ));
                        }
                        Some(UiAction::CycleMode) => {
                            mode_cycles += 1;
                            ui.print(&format!(
                                "{DIM}[mode change applies when the turn ends]{RESET}"
                            ));
                        }
                        Some(UiAction::Interrupt) => {
                            ui.print(&format!("{DIM}[cancelling…]{RESET}"));
                            cancel.cancel();
                        }
                        Some(UiAction::Eof) | None => cancel.cancel(),
                    }
                }
            }
        };
        ui.end_activity();
        match result {
            Ok(report) => {
                if report.hit_output_limit {
                    ui.print(&format!(
                        "{RED}⚠ truncated: the response hit the output-token limit{RESET}"
                    ));
                }
                if !report.cancelled && report.assistant_text.trim().is_empty() {
                    ui.print_note(
                        "no answer text arrived — the model spent its output budget                          reasoning (ctrl+t shows it); try again or narrow the ask",
                    );
                }
                ui.print_note(&turn_status_line(&agent, &report, turn_started.elapsed()));
            }
            Err(error) => ui.print(&format!("{RED}error: {error}{RESET}")),
        }
        for _ in 0..mode_cycles {
            let next = next_mode(agent.permission_mode());
            set_mode(&mut agent, next);
        }
        ui.set_status(status_of(&agent));
        if agent.atlas_enabled() {
            let _ = agent.atlas_mut().refresh();
        }
        let symbols = collect_symbols(&mut agent);
        ui.set_session_data(recency_of(&agent), symbols);
        pending.extend(steer.lock().expect("steer lock").drain(..));
    }
    ui.teardown();
    drop(guard);
    Ok(())
}

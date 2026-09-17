//! Inline terminal UI: streamed output in native scrollback, a custom
//! bottom block (editor with @-mention completion, dropdown, status bar),
//! arrow-key approvals, paste placeholders. Active only when stdin and
//! stdout are TTYs; pipes get the plain loop in `repl`.

pub(crate) mod chrome;
mod completion;
pub mod connect;
mod editor;
mod markdown;
pub(crate) mod term;
pub(crate) mod theme;
mod ui;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use butterfly_core::session::store::SessionStore;
use butterfly_core::session::{Agent, TaskOptions};
use tokio_util::sync::CancellationToken;

use crate::providers::ModelHub;
use crate::repl::{handle_command, mode_name, turn_status_line, CommandResult, Outcome};
use completion::CompletionIndex;
use editor::Editor;
use ui::{Status, Ui, UiAction, UiGate, UiSink};

const COMMANDS: [(&str, &str); 9] = [
    (
        "task",
        "run an autonomous, verified task; ends with the verdict",
    ),
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
        model: agent.model().id.clone(),
        mode: mode_name(agent.permission_mode()),
        ctx_pct: agent.utilization(),
        session_microusd: agent.total_actual_microusd(),
        price_verified: agent.model().pricing.verified,
    }
}

/// Where the workspace sits and what is checked out, for the welcome card:
/// the repository name plus the path inside it, and the branch read straight
/// from `.git/HEAD`. One small file read, no subprocess, and no failure mode
/// worth reporting ‒ a detached head, a linked worktree, or no repository at
/// all simply means less to show. Purely cosmetic, and deliberately outside
/// the prompt: nothing dynamic may reach the cached prefix (invariant 1).
fn workspace_label(root: &std::path::Path) -> (String, Option<String>) {
    let basename = |path: &std::path::Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    };
    let Some(repo) = root.ancestors().find(|dir| dir.join(".git").exists()) else {
        return (basename(root), None);
    };
    let label = match root.strip_prefix(repo) {
        Ok(rest) if rest.as_os_str().is_empty() => basename(repo),
        Ok(rest) => format!("{}/{}", basename(repo), rest.display()),
        Err(_) => basename(root),
    };
    let branch = std::fs::read_to_string(repo.join(".git/HEAD"))
        .ok()
        .and_then(|head| {
            head.trim()
                .strip_prefix("ref: refs/heads/")
                .map(str::to_string)
        })
        .filter(|branch| !branch.is_empty());
    (label, branch)
}

fn next_mode(mode: butterfly_core::session::approval::PermissionMode) -> &'static str {
    use butterfly_core::session::approval::PermissionMode;
    match mode {
        PermissionMode::Ask => "auto",
        PermissionMode::Auto => "read-only",
        PermissionMode::ReadOnly => "ask",
    }
}

/// The mode `times` Shift+Tab presses past `mode`. Pure, so the running
/// turn ‒ which holds the agent borrow ‒ can still name the mode its queued
/// presses will land on.
fn cycled(mode: butterfly_core::session::approval::PermissionMode, times: usize) -> &'static str {
    use butterfly_core::session::approval::PermissionMode;
    let mut current = mode;
    let mut name = mode_name(mode);
    for _ in 0..times {
        name = next_mode(current);
        current = match name {
            "auto" => PermissionMode::Auto,
            "read-only" => PermissionMode::ReadOnly,
            _ => PermissionMode::Ask,
        };
    }
    name
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

pub async fn run(
    mut agent: Agent,
    root: PathBuf,
    show_reasoning: bool,
    hub: &ModelHub,
    task_options: &TaskOptions,
) -> Result<()> {
    if agent.atlas_enabled() {
        let _ = agent.atlas_mut().refresh();
    }
    let symbols = collect_symbols(&mut agent);
    let history_path = SessionStore::state_root().join("repl-history");
    let editor = Editor::new(Some(history_path));
    let (workspace, branch) = workspace_label(&root);
    let index = CompletionIndex::new(root, COMMANDS.to_vec());

    let guard = TerminalGuard::new()?;
    let ui = Ui::new(editor, index, status_of(&agent), show_reasoning);
    ui.enter_screen();
    ui.set_session_data(recency_of(&agent), symbols);
    // A session used to open on an empty screen above a one-line note whose
    // only content was a session id nobody types. The card answers the
    // questions a first keystroke actually depends on; the id stays
    // available in /usage.
    ui.print_welcome(chrome::Welcome {
        workspace: workspace.clone(),
        branch,
        model_name: agent.model().display_name.clone(),
        context_tokens: agent.model().limits.context_tokens,
        mode: mode_name(agent.permission_mode()).to_string(),
    });

    let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
    spawn_event_thread(ui.clone(), sender);

    // Steered messages left over when a turn ends become the next turns.
    let mut pending: VecDeque<String> = VecDeque::new();
    // Modes whose blurb has already been shown. The footer always names the
    // current mode, so the sentence explaining what it gates is worth one line
    // the first time you reach that mode and is pure noise on every later
    // Shift+Tab — six identical lines in the scrollback of one session.
    let mut explained: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
    loop {
        let input = if let Some(queued) = pending.pop_front() {
            ui.print_note(&format!("queued message · sending now: {queued}"));
            queued
        } else {
            match events.recv().await {
                Some(UiAction::Submit(text)) => text,
                Some(UiAction::CycleMode) => {
                    let next = next_mode(agent.permission_mode());
                    set_mode(&mut agent, next);
                    // The mode names do not say what they gate, and this is
                    // the one setting where guessing wrong has consequences —
                    // so it is said once per mode, not once per keypress.
                    if explained.insert(next) {
                        ui.print_note(&format!("{next} · {}", chrome::mode_blurb(next)));
                    }
                    ui.set_status(status_of(&agent));
                    continue;
                }
                Some(UiAction::Interrupt) => continue,
                Some(UiAction::Eof) | None => break,
            }
        };
        let mut task_statement: Option<String> = None;
        match handle_command(&mut agent, &input, hub, ui.width()).await {
            CommandResult::Quit => break,
            CommandResult::Task(statement) => task_statement = Some(statement),
            CommandResult::Connect => {
                // Cancel resolves as None/Err: the composer returns silently.
                if let Ok(Some((provider, api_key))) = ui.open_connect(hub.connect_choices()).await
                {
                    match crate::credentials::save_key(&provider, &api_key) {
                        Ok(path) => {
                            let mut note = format!(
                                "connected · {provider} key saved to {} · new sessions use it automatically",
                                path.display()
                            );
                            if let Some(env) = hub
                                .registry
                                .provider(&provider)
                                .filter(|p| std::env::var(&p.api_key_env).is_ok())
                                .map(|p| p.api_key_env.clone())
                            {
                                note.push_str(&format!(
                                    " (note: {env} is set and takes precedence)"
                                ));
                            }
                            ui.print_note(&note);
                        }
                        Err(error) => {
                            ui.print_error(&format!("could not save the key: {error}"));
                        }
                    }
                }
                continue;
            }
            CommandResult::PickModel => {
                let models = hub.registry.models().to_vec();
                let current = agent.model();
                let rows: Vec<chrome::ModelRow> = models
                    .iter()
                    .map(|model| chrome::ModelRow {
                        display_name: &model.display_name,
                        id: &model.id,
                        context_tokens: model.limits.context_tokens,
                        current: model.id == current.id,
                        // A model you have no key for is one you cannot
                        // pick; say so before the switch fails.
                        has_key: hub
                            .provider_descriptor(model)
                            .ok()
                            .is_some_and(|p| ModelHub::resolve_key(p).is_some()),
                    })
                    .collect();
                let items = ui.render_model_rows(&rows);
                let initial = models.iter().position(|m| m.id == current.id).unwrap_or(0);
                match ui.open_select("switch model", items, initial).await {
                    Ok(Some(index)) if models[index].id != current.id => {
                        let note = crate::repl::switch_model(&mut agent, &models[index].id, hub);
                        ui.print_lines(vec![note]);
                    }
                    // Cancel / unchanged: the composer returns, no residue.
                    _ => {}
                }
                ui.set_status(status_of(&agent));
                continue;
            }
            CommandResult::Output(lines) => {
                ui.print_lines(lines);
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
        let mode_at_start = agent.permission_mode();
        let result = {
            let mut sink = UiSink::new(ui.clone());
            let turn: std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = std::result::Result<
                                Outcome,
                                butterfly_core::session::SessionError,
                            >,
                        > + '_,
                >,
            > = match task_statement.take() {
                Some(statement) => Box::pin(async {
                    agent
                        .run_task(statement, task_options, &gate, &mut sink, &cancel)
                        .await
                        .map(Outcome::Task)
                }),
                None => Box::pin(async {
                    agent
                        .run_turn(input, &gate, &mut sink, &cancel)
                        .await
                        .map(Outcome::Turn)
                }),
            };
            let mut turn = turn;
            loop {
                tokio::select! {
                    result = &mut turn => break result,
                    event = events.recv() => match event {
                        Some(UiAction::Submit(line)) => {
                            steer.lock().expect("steer lock").push_back(line);
                            ui.print_note("queued · delivered at the next turn boundary");
                        }
                        Some(UiAction::CycleMode) => {
                            mode_cycles += 1;
                            ui.print_note(&format!(
                                "{} mode · applies when this turn ends",
                                cycled(mode_at_start, mode_cycles),
                            ));
                        }
                        Some(UiAction::Interrupt) => {
                            ui.print_note("cancelling ‒ finishing the current tool call");
                            cancel.cancel();
                        }
                        Some(UiAction::Eof) | None => cancel.cancel(),
                    }
                }
            }
        };
        ui.end_activity();
        match result {
            Ok(Outcome::Task(report)) => {
                ui.print(report.render_summary().trim_end());
            }
            Ok(Outcome::Turn(report)) => {
                if report.hit_output_limit {
                    ui.print_error("⚠ truncated: the response hit the output-token limit");
                }
                if !report.cancelled && report.assistant_text.trim().is_empty() {
                    ui.print_note(
                        "no answer text arrived ‒ the model spent its output budget reasoning (ctrl+t shows it); try again or narrow the ask",
                    );
                }
                ui.print_marked(
                    Some(theme::MARK_SUMMARY),
                    &turn_status_line(&agent, &report, turn_started.elapsed()),
                );
            }
            Err(error) => ui.print_error(&format!("error: {error}")),
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

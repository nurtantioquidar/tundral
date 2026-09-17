//! Non-TUI terminal paths and shared session-loop pieces: the streaming
//! sink and approval gate for `-p` runs, the slash-command handler used by
//! both loops, and a plain line-based loop for non-TTY stdin (pipes,
//! scripts). Interactive TTY sessions run the inline TUI in `tui`.

use std::io::Write as _;

use anyhow::Result;
use butterfly_core::provider::StreamSink;
use butterfly_core::session::approval::{ApprovalGate, ApprovalRequest, Decision, PermissionMode};
use butterfly_core::session::{Agent, TaskOptions, TaskReport, TurnReport};
use tokio_util::sync::CancellationToken;

use crate::providers::ModelHub;
use crate::tui::chrome;
use crate::tui::theme::ansi::{render_lines, ACCENT as CYAN, DANGER as RED, DIM, RESET};
use crate::tui::theme::{self as th, span};
use ratatui::text::Line;

/// Width assumed when nothing else knows better (pipes, `-p` runs). The
/// interactive loop passes the real terminal width instead.
pub const DEFAULT_WIDTH: usize = 100;

#[derive(PartialEq)]
enum RenderMode {
    Idle,
    Reasoning,
    Text,
}

pub struct TerminalSink {
    mode: RenderMode,
    show_reasoning: bool,
}

impl TerminalSink {
    pub fn new(show_reasoning: bool) -> Self {
        Self {
            mode: RenderMode::Idle,
            show_reasoning,
        }
    }

    fn finish_line(&mut self) {
        if self.mode == RenderMode::Reasoning {
            print!("{RESET}");
        }
        if self.mode != RenderMode::Idle {
            println!();
        }
        self.mode = RenderMode::Idle;
        let _ = std::io::stdout().flush();
    }
}

impl StreamSink for TerminalSink {
    fn on_text(&mut self, delta: &str) {
        if self.mode == RenderMode::Reasoning {
            print!("{RESET}\n\n");
        }
        self.mode = RenderMode::Text;
        print!("{delta}");
        let _ = std::io::stdout().flush();
    }

    fn on_reasoning(&mut self, delta: &str) {
        if !self.show_reasoning {
            return;
        }
        if self.mode != RenderMode::Reasoning {
            print!("{DIM}");
        }
        self.mode = RenderMode::Reasoning;
        print!("{delta}");
        let _ = std::io::stdout().flush();
    }

    fn on_tool_start(&mut self, preview: &str) {
        self.finish_line();
        print!("{CYAN}⚒ {preview}{RESET} ");
        let _ = std::io::stdout().flush();
    }

    fn on_tool_result(&mut self, _preview: &str, is_error: bool, error: Option<&str>) {
        if is_error {
            match error {
                Some(reason) if !reason.is_empty() => {
                    println!("{RED}✗{RESET} {DIM}{reason}{RESET}");
                }
                _ => println!("{RED}✗{RESET}"),
            }
        } else {
            println!("✓");
        }
        let _ = std::io::stdout().flush();
    }
}

pub struct TerminalGate;

/// Colorize a rich approval detail and clamp every row to `width`. The
/// clamp is the point: an unclamped diff line wraps at column 0 and shreds
/// the layout of the one prompt that has to stay readable.
pub(crate) fn render_detail(detail: &str, width: usize) -> String {
    let mut rendered = render_lines(&chrome::detail_rows(detail, width));
    rendered.push('\n');
    rendered
}

#[async_trait::async_trait]
impl ApprovalGate for TerminalGate {
    async fn review(&self, request: &ApprovalRequest) -> Decision {
        let preview = request.preview.clone();
        let detail = request.detail.clone();
        let warning = request.policy_warning.clone();
        tokio::task::spawn_blocking(move || {
            eprintln!("\n  {CYAN}{preview}{RESET}");
            if let Some(warning) = warning {
                eprintln!("  {RED}⚠ {warning}{RESET}");
            }
            if let Some(detail) = detail {
                eprint!("{}", render_detail(&detail, DEFAULT_WIDTH));
            }
            eprint!("  allow? [y]es / [a]lways for session / [N]o: ");
            let _ = std::io::stderr().flush();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() {
                return Decision::Deny;
            }
            match line.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => Decision::Approve,
                "a" | "always" => Decision::ApproveForSession,
                _ => Decision::Deny,
            }
        })
        .await
        .unwrap_or(Decision::Deny)
    }

    async fn answer_question(&self, question: &str) -> Option<String> {
        let question = question.to_string();
        tokio::task::spawn_blocking(move || {
            eprintln!("\n  {CYAN}The model has questions before it edits:{RESET}");
            for line in question.lines() {
                eprintln!("  {line}");
            }
            eprintln!("  {DIM}answer below; finish with an empty line (empty answer = proceed on its assumptions){RESET}");
            let _ = std::io::stderr().flush();
            let mut answer = String::new();
            loop {
                let mut line = String::new();
                match std::io::stdin().read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) if line.trim().is_empty() => break,
                    Ok(_) => answer.push_str(&line),
                    Err(_) => break,
                }
            }
            Some(answer)
        })
        .await
        .unwrap_or(None)
    }
}

/// One-line dim turn receipt, shared by both loops. Session totals and
/// context pressure live in the interactive footer, so this reports only
/// what this turn did.
pub(crate) fn turn_status_line(
    agent: &Agent,
    report: &TurnReport,
    elapsed: std::time::Duration,
) -> String {
    chrome::turn_summary(&chrome::TurnFacts {
        elapsed,
        calls: report.provider_calls,
        input_tokens: report.usage.input_tokens,
        output_tokens: report.usage.output_tokens,
        cache_rate: report.usage.cache_hit_rate(),
        microusd: report.actual_microusd,
        price_verified: agent.model().pricing.verified,
        elisions: report.elisions,
        compactions: report.compactions,
        cancelled: report.cancelled,
    })
}

fn usage_text(agent: &Agent, width: usize) -> Vec<Line<'static>> {
    let usage = agent.total_usage();
    chrome::usage_card(
        &chrome::UsageFacts {
            session_id: &agent.session_id,
            model_id: &agent.model().id,
            input_tokens: usage.input_tokens,
            cache_hits: usage.cache_hit_input_tokens,
            cache_misses: usage.cache_miss_input_tokens,
            cache_rate: usage.cache_hit_rate(),
            output_tokens: usage.output_tokens,
            microusd: agent.total_actual_microusd(),
            price_verified: agent.model().pricing.verified,
            ctx_used: agent.utilization(),
        },
        width,
    )
}

/// What one interactive submission produced: a chat turn, or an autonomous
/// task (`/task`) that ends with the run summary.
pub enum Outcome {
    Turn(TurnReport),
    Task(TaskReport),
}

/// Outcome of trying `input` as a slash command.
pub enum CommandResult {
    /// It was a command; show these composed rows.
    Output(Vec<Line<'static>>),
    /// Run the interactive provider-connect flow (TUI only).
    Connect,
    /// Open the interactive model picker (TUI; plain loops print instead).
    PickModel,
    /// Run this statement as an autonomous task (`run_task`): the completion
    /// gate and verification apply, and the run ends with the summary.
    Task(String),
    Quit,
    /// Not a command — run it as a user turn.
    NotACommand,
}

/// Handle the shared slash commands (used by both the TUI and plain loops).
pub async fn handle_command(
    agent: &mut Agent,
    input: &str,
    hub: &ModelHub,
    width: usize,
) -> CommandResult {
    match input {
        "/quit" | "/exit" => CommandResult::Quit,
        "/connect" => CommandResult::Connect,
        "/help" => CommandResult::Output(chrome::help_card(width)),
        "/usage" => CommandResult::Output(usage_text(agent, width)),
        "/plan" => CommandResult::Output(plain_lines(&agent.working_state().render())),
        _ if input == "/compact" || input.starts_with("/compact ") => {
            let focus = input.strip_prefix("/compact").unwrap_or("").trim();
            let focus = (!focus.is_empty()).then_some(focus);
            let cancel = CancellationToken::new();
            CommandResult::Output(vec![match agent.compact_now(&cancel, focus).await {
                Ok(()) => Line::from(vec![span(
                    format!("[compacted; ctx now {:.0}%]", agent.utilization() * 100.0),
                    th::dim(),
                )]),
                Err(error) => Line::from(vec![span(
                    format!("compaction failed: {error}"),
                    th::danger(),
                )]),
            }])
        }
        _ if input == "/task" || input.starts_with("/task ") => {
            let statement = input.strip_prefix("/task").unwrap_or("").trim();
            if statement.is_empty() {
                CommandResult::Output(plain_lines("usage: /task <statement>"))
            } else {
                CommandResult::Task(statement.to_string())
            }
        }
        _ if input.starts_with("/model") => {
            CommandResult::Output(vec![match input.trim_start_matches("/model").trim() {
                "" => return CommandResult::PickModel,
                name => switch_model(agent, name, hub),
            }])
        }
        _ if input.starts_with('/') => {
            CommandResult::Output(plain_lines(&format!("unknown command: {input} (/help)")))
        }
        _ => CommandResult::NotACommand,
    }
}

/// Resolve `name`, build its provider (key lookup included), and switch the
/// agent. Any failure leaves the session on its current model.
pub fn switch_model(agent: &mut Agent, name: &str, hub: &ModelHub) -> Line<'static> {
    let model = match hub.resolve(name) {
        Ok(model) => model,
        Err(error) => return Line::from(vec![span(error.to_string(), th::danger())]),
    };
    let provider = match hub.build_provider(&model) {
        Ok(provider) => provider,
        Err(error) => return Line::from(vec![span(error.to_string(), th::danger())]),
    };
    agent.set_model(model.clone(), provider);
    // Model choice sticks across sessions.
    crate::state::remember_model(&model.id);
    Line::from(vec![span(
        format!(
            "[model set: {} ({}); next request re-caches the prefix]",
            model.display_name, model.id
        ),
        th::dim(),
    )])
}

/// Unstyled rows, for command output that is plain prose.
fn plain_lines(text: &str) -> Vec<Line<'static>> {
    text.split('\n')
        .map(|line| Line::from(vec![th::text(line.to_string())]))
        .collect()
}

/// Run an autonomous task (attempts + verification) with Ctrl-C wired to the
/// cancellation token (`-p` runs).
pub async fn run_task_interruptible(
    agent: &mut Agent,
    input: String,
    options: &TaskOptions,
    gate: &dyn ApprovalGate,
    show_reasoning: bool,
) -> Result<TaskReport> {
    let cancel = CancellationToken::new();
    let mut sink = TerminalSink::new(show_reasoning);
    let task = agent.run_task(input, options, gate, &mut sink, &cancel);
    tokio::pin!(task);
    let report = loop {
        tokio::select! {
            result = &mut task => break result?,
            signal = tokio::signal::ctrl_c() => {
                signal?;
                eprintln!("\n{DIM}[cancelling…]{RESET}");
                cancel.cancel();
            }
        }
    };
    println!();
    Ok(report)
}

/// Everything the plain input thread can report.
enum InputEvent {
    Line(String),
    Eof,
}

/// Plain stdin reader for non-TTY sessions (pipes, scripts): lines arrive
/// whole; no raw mode, no completion, no history.
fn spawn_line_thread(sender: tokio::sync::mpsc::UnboundedSender<InputEvent>) {
    std::thread::spawn(move || {
        use std::io::BufRead as _;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let input = line.trim().to_string();
            if input.is_empty() {
                continue;
            }
            if sender.send(InputEvent::Line(input)).is_err() {
                return;
            }
        }
        let _ = sender.send(InputEvent::Eof);
    });
}

/// Approval gate for the plain loop: the answer arrives on the shared input
/// stream. While a claim is pending, the next line resolves it instead of
/// being queued as steering.
struct ChannelGate {
    claim: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<String>>>>,
}

#[async_trait::async_trait]
impl ApprovalGate for ChannelGate {
    async fn review(&self, request: &ApprovalRequest) -> Decision {
        eprintln!("\n  {CYAN}{}{RESET}", request.preview);
        if let Some(warning) = &request.policy_warning {
            eprintln!("  {RED}⚠ {warning}{RESET}");
        }
        if let Some(detail) = &request.detail {
            eprint!("{}", render_detail(detail, DEFAULT_WIDTH));
        }
        eprintln!("  allow? [y]es / [a]lways for session / [N]o — answer on the input line");
        let (sender, receiver) = tokio::sync::oneshot::channel();
        *self.claim.lock().expect("claim lock") = Some(sender);
        match receiver.await {
            Ok(answer) => match answer.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => Decision::Approve,
                "a" | "always" => Decision::ApproveForSession,
                _ => Decision::Deny,
            },
            Err(_) => Decision::Deny,
        }
    }
}

/// Line-based session loop for non-TTY stdin.
pub async fn plain_repl(
    mut agent: Agent,
    show_reasoning: bool,
    hub: &ModelHub,
    task_options: &TaskOptions,
) -> Result<()> {
    let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
    spawn_line_thread(sender);
    let claim: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<String>>>> =
        std::sync::Arc::default();
    let gate = ChannelGate {
        claim: claim.clone(),
    };
    println!(
        "{DIM}butterfly · {} · session {} · {} mode · /help for commands{RESET}",
        agent.model().id,
        agent.session_id,
        mode_name(agent.permission_mode()),
    );
    // Steered messages left over when a turn ends become the next turns.
    let mut pending: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    loop {
        let input = if let Some(queued) = pending.pop_front() {
            println!("{DIM}[queued]{RESET} {queued}");
            queued
        } else {
            match events.recv().await {
                Some(InputEvent::Line(line)) => line,
                Some(InputEvent::Eof) | None => break,
            }
        };
        let mut task_statement: Option<String> = None;
        match handle_command(&mut agent, &input, hub, DEFAULT_WIDTH).await {
            CommandResult::Quit => break,
            CommandResult::Task(statement) => task_statement = Some(statement),
            CommandResult::Connect => {
                println!(
                    "/connect needs an interactive terminal; run butterfly in a TTY \
                     (or export the provider's API key variable)"
                );
                continue;
            }
            CommandResult::PickModel => {
                let model = agent.model();
                println!(
                    "model: {} ({}) — switch with /model <id>; known: {}",
                    model.display_name,
                    model.id,
                    hub.registry.model_ids().join(" | ")
                );
                continue;
            }
            CommandResult::Output(lines) => {
                println!("{}", render_lines(&lines));
                continue;
            }
            CommandResult::NotACommand => {}
        }
        // Run the turn while continuing to consume input: lines answer a
        // pending approval or queue as steering; Ctrl-C cancels.
        let cancel = CancellationToken::new();
        let steer = agent.steer_handle();
        let turn_started = std::time::Instant::now();
        let result = {
            let mut sink = TerminalSink::new(show_reasoning);
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
                        Some(InputEvent::Line(line)) => {
                            let pending_claim = claim.lock().expect("claim lock").take();
                            match pending_claim {
                                Some(sender) => {
                                    let _ = sender.send(line);
                                }
                                None => {
                                    steer.lock().expect("steer lock").push_back(line);
                                    eprintln!("{DIM}[queued: delivered at the next turn boundary]{RESET}");
                                }
                            }
                        }
                        Some(InputEvent::Eof) | None => cancel.cancel(),
                    },
                    signal = tokio::signal::ctrl_c() => {
                        if signal.is_ok() {
                            eprintln!("\n{DIM}[cancelling…]{RESET}");
                        }
                        cancel.cancel();
                    }
                }
            }
        };
        println!();
        match result {
            Ok(Outcome::Task(report)) => {
                println!("{}", report.render_summary().trim_end());
            }
            Ok(Outcome::Turn(report)) => {
                if report.hit_output_limit {
                    eprintln!("{RED}⚠ truncated: the response hit the output-token limit{RESET}");
                }
                println!(
                    "{DIM}{}{RESET}",
                    turn_status_line(&agent, &report, turn_started.elapsed())
                );
            }
            Err(error) => eprintln!("{RED}error: {error}{RESET}"),
        }
        pending.extend(steer.lock().expect("steer lock").drain(..));
    }
    Ok(())
}

pub fn mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "ask",
        PermissionMode::Auto => "auto",
        PermissionMode::ReadOnly => "read-only",
    }
}

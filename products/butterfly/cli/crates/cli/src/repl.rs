//! Non-TUI terminal paths and shared session-loop pieces: the streaming
//! sink and approval gate for `-p` runs, the slash-command handler used by
//! both loops, and a plain line-based loop for non-TTY stdin (pipes,
//! scripts). Interactive TTY sessions run the inline TUI in `tui`.

use std::io::Write as _;

use anyhow::Result;
use butterfly_core::domain::format_microusd_as_usd;
use butterfly_core::provider::StreamSink;
use butterfly_core::session::approval::{ApprovalGate, ApprovalRequest, Decision, PermissionMode};
use butterfly_core::session::{Agent, TaskOptions, TaskReport, TurnReport};
use tokio_util::sync::CancellationToken;

use crate::providers::ModelHub;

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";

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

/// Colorize a rich approval detail: `-` lines red, `+` lines green.
pub(crate) fn render_detail(detail: &str) -> String {
    let mut rendered = String::new();
    for line in detail.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- ") || trimmed == "-" {
            rendered.push_str(&format!("  {RED}{line}{RESET}\n"));
        } else if trimmed.starts_with("+ ") || trimmed == "+" {
            rendered.push_str(&format!("  {GREEN}{line}{RESET}\n"));
        } else {
            rendered.push_str(&format!("  {DIM}{line}{RESET}\n"));
        }
    }
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
                eprint!("{}", render_detail(&detail));
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
}

/// One-line dim turn summary, shared by both loops.
pub(crate) fn turn_status_line(
    agent: &Agent,
    report: &TurnReport,
    elapsed: std::time::Duration,
) -> String {
    let cache_rate = (report.usage.cache_hit_rate() * 100.0).round();
    let utilization = (agent.utilization() * 100.0).round();
    let mut extras = String::new();
    if report.elisions > 0 || report.compactions > 0 {
        extras = format!(
            " · compacted ({} elision, {} summarize)",
            report.elisions, report.compactions
        );
    }
    if report.cancelled {
        extras.push_str(" · cancelled");
    }
    format!(
        "∴ done in {:.1}s · {} calls · in {} ({cache_rate:.0}% cached) · out {} · {} | session {} · ctx {utilization:.0}%{extras}",
        elapsed.as_secs_f64(),
        report.provider_calls,
        format_tokens(report.usage.input_tokens),
        format_tokens(report.usage.output_tokens),
        format_microusd_as_usd(report.actual_microusd),
        format_microusd_as_usd(agent.total_actual_microusd()),
    )
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn usage_text(agent: &Agent) -> String {
    let usage = agent.total_usage();
    format!(
        "session {}\n  model            {}\n  input tokens     {}\n    cache hits     {}\n    cache misses   {}\n    hit rate       {:.1}%\n  output tokens    {}\n  actual cost      {}\n  context used     {:.0}%",
        agent.session_id,
        agent.model().id,
        usage.input_tokens,
        usage.cache_hit_input_tokens,
        usage.cache_miss_input_tokens,
        usage.cache_hit_rate() * 100.0,
        usage.output_tokens,
        format_microusd_as_usd(agent.total_actual_microusd()),
        agent.utilization() * 100.0,
    )
}

const HELP: &str = "\
commands:
  /connect  save a provider API key (persists across sessions)
  /usage    tokens, cost, cache-hit rate, context utilization
  /plan     show the tracked working state (objective, tasks, files, commands)
  /compact  force a compaction now; optional focus: /compact keep the auth details
  /model    pick a model (arrow keys), or /model <name> to switch directly
  /help     this help
  /quit     exit (also Ctrl-D)
input:
  @path      mention a file (type @ for fuzzy completion; Tab/Enter accepts)
  Shift+Tab  cycle permission mode: ask → auto → read-only
  Ctrl-T     toggle the full-transcript overlay (includes hidden reasoning)
  Ctrl-R     toggle the live reasoning stream (hidden by default)
  Ctrl-J     insert a newline · Ctrl-C clears the line or cancels the turn
  Esc        close the dropdown, or interrupt the running turn
  paste      large pastes collapse to a placeholder and expand on submit
  Type while a turn runs: messages are delivered at the next turn boundary.";

/// Outcome of trying `input` as a slash command.
pub enum CommandResult {
    /// It was a command; show this text.
    Output(String),
    /// Run the interactive provider-connect flow (TUI only).
    Connect,
    /// Open the interactive model picker (TUI; plain loops print instead).
    PickModel,
    Quit,
    /// Not a command — run it as a user turn.
    NotACommand,
}

/// Handle the shared slash commands (used by both the TUI and plain loops).
pub async fn handle_command(agent: &mut Agent, input: &str, hub: &ModelHub) -> CommandResult {
    match input {
        "/quit" | "/exit" => CommandResult::Quit,
        "/connect" => CommandResult::Connect,
        "/help" => CommandResult::Output(HELP.to_string()),
        "/usage" => CommandResult::Output(usage_text(agent)),
        "/plan" => CommandResult::Output(agent.working_state().render()),
        _ if input == "/compact" || input.starts_with("/compact ") => {
            let focus = input.strip_prefix("/compact").unwrap_or("").trim();
            let focus = (!focus.is_empty()).then_some(focus);
            let cancel = CancellationToken::new();
            CommandResult::Output(match agent.compact_now(&cancel, focus).await {
                Ok(()) => format!(
                    "{DIM}[compacted; ctx now {:.0}%]{RESET}",
                    agent.utilization() * 100.0
                ),
                Err(error) => format!("{RED}compaction failed: {error}{RESET}"),
            })
        }
        _ if input.starts_with("/model") => {
            CommandResult::Output(match input.trim_start_matches("/model").trim() {
                "" => return CommandResult::PickModel,
                name => switch_model(agent, name, hub),
            })
        }
        _ if input.starts_with('/') => {
            CommandResult::Output(format!("unknown command: {input} (/help)"))
        }
        _ => CommandResult::NotACommand,
    }
}

/// Resolve `name`, build its provider (key lookup included), and switch the
/// agent. Any failure leaves the session on its current model.
pub fn switch_model(agent: &mut Agent, name: &str, hub: &ModelHub) -> String {
    let model = match hub.resolve(name) {
        Ok(model) => model,
        Err(error) => return format!("{RED}{error}{RESET}"),
    };
    let provider = match hub.build_provider(&model) {
        Ok(provider) => provider,
        Err(error) => return format!("{RED}{error}{RESET}"),
    };
    agent.set_model(model.clone(), provider);
    // Model choice sticks across sessions.
    crate::state::remember_model(&model.id);
    format!(
        "{DIM}[model set: {} ({}); next request re-caches the prefix]{RESET}",
        model.display_name, model.id
    )
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
            eprint!("{}", render_detail(detail));
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
pub async fn plain_repl(mut agent: Agent, show_reasoning: bool, hub: &ModelHub) -> Result<()> {
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
        match handle_command(&mut agent, &input, hub).await {
            CommandResult::Quit => break,
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
            CommandResult::Output(text) => {
                println!("{text}");
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
            let turn = agent.run_turn(input, &gate, &mut sink, &cancel);
            tokio::pin!(turn);
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
            Ok(report) => {
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

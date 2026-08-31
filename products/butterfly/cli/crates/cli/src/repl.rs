//! Line-based streaming REPL: rustyline input, ANSI streaming output,
//! terminal approval prompts, Ctrl-C turn cancellation.

use std::io::Write as _;

use anyhow::Result;
use butterfly_core::domain::format_microusd_as_usd;
use butterfly_core::provider::StreamSink;
use butterfly_core::session::approval::{ApprovalGate, ApprovalRequest, Decision, PermissionMode};
use butterfly_core::session::{Agent, TurnReport};
use tokio_util::sync::CancellationToken;

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";

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

    fn on_tool_result(&mut self, _preview: &str, is_error: bool) {
        if is_error {
            println!("{RED}✗{RESET}");
        } else {
            println!("✓");
        }
        let _ = std::io::stdout().flush();
    }
}

pub struct TerminalGate;

const GREEN: &str = "\x1b[32m";

/// Colorize a rich approval detail: `-` lines red, `+` lines green.
fn render_detail(detail: &str) -> String {
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

fn print_turn_status(agent: &Agent, report: &TurnReport) {
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
    println!(
        "{DIM}[{} calls · in {} ({cache_rate:.0}% cached) · out {} · {} | session {} · ctx {utilization:.0}%{extras}]{RESET}",
        report.provider_calls,
        format_tokens(report.usage.input_tokens),
        format_tokens(report.usage.output_tokens),
        format_microusd_as_usd(report.actual_microusd),
        format_microusd_as_usd(agent.total_actual_microusd()),
    );
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn print_usage(agent: &Agent) {
    let usage = agent.total_usage();
    println!("session {}", agent.session_id);
    println!("  model            {}", agent.model().wire_name());
    println!("  input tokens     {}", usage.input_tokens);
    println!("    cache hits     {}", usage.cache_hit_input_tokens);
    println!("    cache misses   {}", usage.cache_miss_input_tokens);
    println!("    hit rate       {:.1}%", usage.cache_hit_rate() * 100.0);
    println!("  output tokens    {}", usage.output_tokens);
    println!(
        "  actual cost      {}",
        format_microusd_as_usd(agent.total_actual_microusd())
    );
    println!("  context used     {:.0}%", agent.utilization() * 100.0);
}

const HELP: &str = "\
commands:
  /usage    tokens, cost, cache-hit rate, context utilization
  /plan     show the tracked working state (objective, tasks, files, commands)
  /compact  force a compaction now; optional focus: /compact keep the auth details
  /mode     show or set permission mode: /mode ask|auto|read-only
  /model    show or switch model: /model deepseek-v4-flash|deepseek-v4-pro
  /help     this help
  /quit     exit (also Ctrl-D)
End a line with \\ to continue on the next line. Ctrl-C cancels the turn.";

/// Run one turn with Ctrl-C wired to the cancellation token.
pub async fn run_turn_interruptible(
    agent: &mut Agent,
    input: String,
    gate: &dyn ApprovalGate,
    show_reasoning: bool,
) -> Result<TurnReport> {
    let cancel = CancellationToken::new();
    let mut sink = TerminalSink::new(show_reasoning);
    let turn = agent.run_turn(input, gate, &mut sink, &cancel);
    tokio::pin!(turn);
    let report = loop {
        tokio::select! {
            result = &mut turn => break result?,
            signal = tokio::signal::ctrl_c() => {
                signal?;
                eprintln!("\n{DIM}[cancelling…]{RESET}");
                cancel.cancel();
            }
        }
    };
    // The sink was moved into scope above; ensure the cursor is on a fresh line.
    println!();
    Ok(report)
}

/// Everything the input thread can report.
enum InputEvent {
    Line(String),
    Interrupt,
    Eof,
}

/// The input thread owns stdin and rustyline for the whole session so lines
/// can arrive WHILE a turn is running (pi-style steering). It only produces
/// events; the async loop decides what each line means (approval answer,
/// steering message, or new turn).
fn spawn_input_thread(sender: tokio::sync::mpsc::UnboundedSender<InputEvent>) {
    std::thread::spawn(move || {
        let Ok(mut editor) = rustyline::DefaultEditor::new() else {
            let _ = sender.send(InputEvent::Eof);
            return;
        };
        let history_path =
            butterfly_core::session::store::SessionStore::state_root().join("repl-history");
        let _ = editor.load_history(&history_path);
        loop {
            let mut line = match editor.readline("butterfly> ") {
                Ok(line) => line,
                Err(rustyline::error::ReadlineError::Interrupted) => {
                    if sender.send(InputEvent::Interrupt).is_err() {
                        break;
                    }
                    continue;
                }
                Err(_) => {
                    let _ = sender.send(InputEvent::Eof);
                    break;
                }
            };
            // Trailing backslash continues the input on the next line.
            while line.trim_end().ends_with('\\') {
                let trimmed = line.trim_end();
                line = format!("{}\n", &trimmed[..trimmed.len() - 1]);
                match editor.readline("...> ") {
                    Ok(next) => line.push_str(&next),
                    Err(_) => {
                        line.clear();
                        break;
                    }
                }
            }
            let input = line.trim().to_string();
            if input.is_empty() {
                continue;
            }
            let _ = editor.add_history_entry(&input);
            if sender.send(InputEvent::Line(input)).is_err() {
                break;
            }
        }
        let _ = editor.save_history(&history_path);
    });
}

/// Approval gate for the REPL: the answer arrives on the shared input line.
/// While a claim is pending, the next typed line resolves it instead of being
/// queued as steering.
pub struct ChannelGate {
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

pub async fn repl(mut agent: Agent, show_reasoning: bool) -> Result<()> {
    let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
    spawn_input_thread(sender);
    let claim: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<String>>>> =
        std::sync::Arc::default();
    let gate = ChannelGate {
        claim: claim.clone(),
    };
    println!(
        "{DIM}butterfly · {} · session {} · {} mode · /help for commands · type while it works: messages land at the next turn boundary{RESET}",
        agent.model().wire_name(),
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
                Some(InputEvent::Interrupt) => continue,
                Some(InputEvent::Eof) | None => break,
            }
        };
        match input.as_str() {
            "/quit" | "/exit" => break,
            "/help" => {
                println!("{HELP}");
                continue;
            }
            "/usage" => {
                print_usage(&agent);
                continue;
            }
            "/plan" => {
                print!("{}", agent.working_state().render());
                continue;
            }
            _ if input == "/compact" || input.starts_with("/compact ") => {
                let focus = input.strip_prefix("/compact").unwrap_or("").trim();
                let focus = (!focus.is_empty()).then_some(focus);
                let cancel = CancellationToken::new();
                match agent.compact_now(&cancel, focus).await {
                    Ok(()) => println!(
                        "{DIM}[compacted; ctx now {:.0}%]{RESET}",
                        agent.utilization() * 100.0
                    ),
                    Err(error) => eprintln!("{RED}compaction failed: {error}{RESET}"),
                }
                continue;
            }
            _ if input.starts_with("/model") => {
                match input.trim_start_matches("/model").trim() {
                    "" => println!("model: {}", agent.model().wire_name()),
                    name => match butterfly_core::domain::Model::parse(name) {
                        Some(model) => {
                            agent.set_model(model);
                            println!(
                                "{DIM}[model set: {}; next request re-caches the prefix]{RESET}",
                                model.wire_name()
                            );
                        }
                        None => {
                            eprintln!("unknown model: {name} (deepseek-v4-flash | deepseek-v4-pro)")
                        }
                    },
                }
                continue;
            }
            _ if input.starts_with("/mode") => {
                match input.trim_start_matches("/mode").trim() {
                    "" => println!("mode: {}", mode_name(agent.permission_mode())),
                    "ask" => agent.set_permission_mode(PermissionMode::Ask),
                    "auto" => agent.set_permission_mode(PermissionMode::Auto),
                    "read-only" | "readonly" => agent.set_permission_mode(PermissionMode::ReadOnly),
                    other => eprintln!("unknown mode: {other} (ask|auto|read-only)"),
                }
                continue;
            }
            _ if input.starts_with('/') => {
                eprintln!("unknown command: {input} (/help)");
                continue;
            }
            _ => {}
        }
        // Run the turn while continuing to consume input events: lines answer
        // a pending approval or queue as steering; Ctrl-C cancels.
        let cancel = CancellationToken::new();
        let steer = agent.steer_handle();
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
                        Some(InputEvent::Interrupt) => {
                            eprintln!("\n{DIM}[cancelling…]{RESET}");
                            cancel.cancel();
                        }
                        Some(InputEvent::Eof) | None => cancel.cancel(),
                    }
                }
            }
        };
        println!();
        match result {
            Ok(report) => print_turn_status(&agent, &report),
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

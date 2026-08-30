//! Line-based streaming REPL: rustyline input, ANSI streaming output,
//! terminal approval prompts, Ctrl-C turn cancellation.

use std::io::Write as _;

use anyhow::Result;
use hx_core::domain::format_microusd_as_usd;
use hx_core::provider::StreamSink;
use hx_core::session::approval::{ApprovalGate, ApprovalRequest, Decision, PermissionMode};
use hx_core::session::{Agent, TurnReport};
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
        tokio::task::spawn_blocking(move || {
            eprintln!("\n  {CYAN}{preview}{RESET}");
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
  /compact  force a summarization compaction now
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

pub async fn repl(mut agent: Agent, show_reasoning: bool) -> Result<()> {
    let mut editor = rustyline::DefaultEditor::new()?;
    let history_path = hx_core::session::store::SessionStore::state_root().join("repl-history");
    let _ = editor.load_history(&history_path);
    println!(
        "{DIM}hx · {} · session {} · {} mode · /help for commands{RESET}",
        agent.model().wire_name(),
        agent.session_id,
        mode_name(agent.permission_mode()),
    );
    loop {
        let mut line = match editor.readline("hx> ") {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(error) => return Err(error.into()),
        };
        // Trailing backslash continues the input on the next line.
        while line.trim_end().ends_with('\\') {
            let trimmed = line.trim_end();
            line = format!("{}\n", &trimmed[..trimmed.len() - 1]);
            match editor.readline("...> ") {
                Ok(next) => line.push_str(&next),
                Err(rustyline::error::ReadlineError::Interrupted) => {
                    line.clear();
                    break;
                }
                Err(rustyline::error::ReadlineError::Eof) => break,
                Err(error) => return Err(error.into()),
            }
        }
        let input = line.trim().to_string();
        if input.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(&input);
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
            "/compact" => {
                let cancel = CancellationToken::new();
                match agent.compact_now(&cancel).await {
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
                    name => match hx_core::domain::Model::parse(name) {
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
        match run_turn_interruptible(&mut agent, input, &TerminalGate, show_reasoning).await {
            Ok(report) => print_turn_status(&agent, &report),
            Err(error) => eprintln!("{RED}error: {error}{RESET}"),
        }
    }
    let _ = editor.save_history(&history_path);
    Ok(())
}

pub fn mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "ask",
        PermissionMode::Auto => "auto",
        PermissionMode::ReadOnly => "read-only",
    }
}

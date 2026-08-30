//! hx — fast interactive coding agent CLI (harness-experimental).

mod repl;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use hx_core::domain::{usd_to_microusd, Model};
use hx_core::provider::deepseek::DeepSeekProvider;
use hx_core::provider::ReasoningEffort;
use hx_core::session::approval::PermissionMode;
use hx_core::session::store::SessionStore;
use hx_core::session::{Agent, SessionConfig};

#[derive(Parser)]
#[command(
    name = "hx",
    about = "Fast interactive coding agent (DeepSeek-native, cache-first)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Workspace root (defaults to the current directory).
    #[arg(long)]
    root: Option<PathBuf>,

    /// Model: deepseek-v4-flash | deepseek-v4-pro.
    #[arg(long, default_value = "deepseek-v4-flash")]
    model: String,

    /// Reasoning effort: high | max.
    #[arg(long, default_value = "high")]
    effort: String,

    /// Approve every tool call without prompting.
    #[arg(long)]
    auto: bool,

    /// Deny all mutating tools and commands.
    #[arg(long)]
    read_only: bool,

    /// Hard session cost cap in decimal USD, e.g. "1" or "0.25".
    #[arg(long)]
    max_cost_usd: Option<String>,

    /// Resume a session: `--resume` for the latest, `--resume <id>` for one.
    #[arg(long, num_args = 0..=1, default_missing_value = "latest")]
    resume: Option<String>,

    /// Do not persist this session to disk.
    #[arg(long)]
    no_save: bool,

    /// Hide streamed reasoning output.
    #[arg(long)]
    quiet_reasoning: bool,

    /// Run a single prompt non-interactively and exit.
    #[arg(short = 'p', long)]
    prompt: Option<String>,

    /// With --prompt: suppress streaming and emit one machine-readable JSON
    /// result on stdout. Requires --auto or --read-only (no prompts).
    #[arg(long)]
    json: bool,

    /// Override the DeepSeek endpoint (or set DEEPSEEK_BASE_URL).
    #[arg(long)]
    base_url: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// List saved sessions, newest first.
    Sessions,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(Command::Sessions) = cli.command {
        let root = SessionStore::state_root();
        for session in SessionStore::list_sessions(&root) {
            println!("{session}");
        }
        return Ok(());
    }

    let root = match cli.root {
        Some(root) => root,
        None => std::env::current_dir().context("cannot resolve current directory")?,
    };
    let root = root
        .canonicalize()
        .with_context(|| format!("workspace root not found: {}", root.display()))?;

    let Some(model) = Model::parse(&cli.model) else {
        bail!(
            "unknown model: {} (deepseek-v4-flash | deepseek-v4-pro)",
            cli.model
        );
    };
    let effort = match cli.effort.as_str() {
        "high" => ReasoningEffort::High,
        "max" => ReasoningEffort::Max,
        other => bail!("unknown effort: {other} (high | max)"),
    };
    let permission_mode = match (cli.auto, cli.read_only) {
        (true, true) => bail!("--auto and --read-only are mutually exclusive"),
        (true, false) => PermissionMode::Auto,
        (false, true) => PermissionMode::ReadOnly,
        (false, false) => PermissionMode::Ask,
    };
    let max_session_cost_microusd = match &cli.max_cost_usd {
        Some(text) => Some(
            usd_to_microusd(text)
                .with_context(|| format!("invalid --max-cost-usd value: {text}"))?,
        ),
        None => None,
    };
    let config = SessionConfig {
        model,
        effort,
        permission_mode,
        max_session_cost_microusd,
        ..SessionConfig::default()
    };

    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .context("DEEPSEEK_API_KEY is required (export it in your shell)")?;
    let base_url = cli
        .base_url
        .or_else(|| std::env::var("DEEPSEEK_BASE_URL").ok());
    let provider = Arc::new(DeepSeekProvider::new(api_key, base_url));

    let agent = match &cli.resume {
        Some(selector) => {
            let state_root = SessionStore::state_root();
            let session_id = if selector == "latest" {
                SessionStore::list_sessions(&state_root)
                    .into_iter()
                    .next()
                    .context("no saved sessions to resume")?
            } else {
                selector.clone()
            };
            let store = SessionStore::create_under(&state_root, &session_id)?;
            let mut agent = Agent::resume(provider, root, config, store, session_id.clone())?;
            agent.session_id = session_id;
            agent
        }
        None => {
            let store = if cli.no_save {
                None
            } else {
                let session_id = uuid::Uuid::new_v4().simple().to_string();
                Some((
                    session_id.clone(),
                    SessionStore::create_under(&SessionStore::state_root(), &session_id)?,
                ))
            };
            match store {
                Some((session_id, store)) => {
                    let mut agent = Agent::new(provider, root, config, Some(store))?;
                    agent.session_id = session_id;
                    agent
                }
                None => Agent::new(provider, root, config, None)?,
            }
        }
    };

    let show_reasoning = !cli.quiet_reasoning;
    if let Some(prompt) = cli.prompt {
        let mut agent = agent;
        if cli.json {
            if agent.permission_mode() == PermissionMode::Ask {
                bail!("--json cannot prompt for approvals; pass --auto or --read-only");
            }
            let report = agent
                .run_turn(
                    prompt,
                    &hx_core::session::approval::AutoApprove,
                    &mut hx_core::provider::NullSink,
                    &tokio_util::sync::CancellationToken::new(),
                )
                .await?;
            let usage = report.usage;
            println!(
                "{}",
                serde_json::json!({
                    "session_id": agent.session_id,
                    "model": agent.model().wire_name(),
                    "response": report.assistant_text,
                    "provider_calls": report.provider_calls,
                    "tool_calls": report.tool_calls,
                    "input_tokens": usage.input_tokens,
                    "cache_hit_input_tokens": usage.cache_hit_input_tokens,
                    "cache_miss_input_tokens": usage.cache_miss_input_tokens,
                    "output_tokens": usage.output_tokens,
                    "cache_hit_percent": (usage.cache_hit_rate() * 1000.0).round() / 10.0,
                    "actual_cost_microusd": report.actual_microusd,
                    "normalized_cache_miss_cost_microusd": report.normalized_microusd,
                    "session_cost_microusd": agent.total_actual_microusd(),
                    "context_utilization_percent": (agent.utilization() * 1000.0).round() / 10.0,
                    "compactions": report.compactions,
                    "elisions": report.elisions,
                    "cancelled": report.cancelled,
                    "budget_exhausted": report.budget_exhausted,
                })
            );
            if report.cancelled {
                std::process::exit(130);
            }
            return Ok(());
        }
        let report =
            repl::run_turn_interruptible(&mut agent, prompt, &repl::TerminalGate, show_reasoning)
                .await?;
        if report.cancelled {
            std::process::exit(130);
        }
        return Ok(());
    }
    repl::repl(agent, show_reasoning).await
}

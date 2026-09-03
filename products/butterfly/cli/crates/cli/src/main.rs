//! butterfly — the Harness interactive coding agent CLI.

mod config;
mod credentials;
mod export;
mod repl;
mod state;
mod tui;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use butterfly_core::domain::{usd_to_microusd, Model};
use butterfly_core::provider::deepseek::DeepSeekProvider;
use butterfly_core::provider::ReasoningEffort;
use butterfly_core::session::approval::PermissionMode;
use butterfly_core::session::store::SessionStore;
use butterfly_core::session::{Agent, SessionConfig};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "butterfly",
    about = "Fast interactive coding agent (DeepSeek-native, cache-first)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Workspace root (defaults to the current directory).
    #[arg(long)]
    root: Option<PathBuf>,

    /// Model: deepseek-v4-flash | deepseek-v4-pro (default from config, else flash).
    #[arg(long)]
    model: Option<String>,

    /// Reasoning effort: high | max (default from config, else high).
    #[arg(long)]
    effort: Option<String>,

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

    /// Override the context window in tokens (tuning/testing; default is the
    /// model's real window).
    #[arg(long)]
    window_tokens: Option<u64>,

    /// Disable the Atlas code-graph tools (repo_map, symbol_search).
    #[arg(long)]
    no_atlas: bool,

    /// Force-enable Atlas regardless of repository size.
    #[arg(long)]
    atlas: bool,
}

#[derive(Subcommand)]
enum Command {
    /// List saved sessions, newest first.
    Sessions,
    /// Export a session transcript to a self-contained HTML file.
    Export {
        /// Session id (defaults to the latest session).
        session: Option<String>,
        /// Output path (defaults to butterfly-session-<id>.html).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Some(Command::Sessions) => {
            let root = SessionStore::state_root();
            for session in SessionStore::list_sessions(&root) {
                println!("{session}");
            }
            return Ok(());
        }
        Some(Command::Export { session, out }) => {
            let path = export::export_session(session.as_deref(), out.clone())?;
            println!("{}", path.display());
            return Ok(());
        }
        None => {}
    }

    let root = match cli.root {
        Some(root) => root,
        None => std::env::current_dir().context("cannot resolve current directory")?,
    };
    let root = root
        .canonicalize()
        .with_context(|| format!("workspace root not found: {}", root.display()))?;
    let workspace_root = root.clone();

    // Layered configuration: defaults ← global file ← repo file ← flags.
    let file = config::load(&root)?;

    // Model resolution: explicit pins (flag, config files) win; otherwise
    // the last model the user ran with sticks across sessions.
    let model_name = cli
        .model
        .or(file.model)
        .or_else(state::last_model)
        .unwrap_or_else(|| "deepseek-v4-flash".to_string());
    let Some(model) = Model::parse(&model_name) else {
        bail!("unknown model: {model_name} (deepseek-v4-flash | deepseek-v4-pro)");
    };
    state::remember_model(model.wire_name());
    let effort_name = cli
        .effort
        .or(file.effort)
        .unwrap_or_else(|| "high".to_string());
    let effort = match effort_name.as_str() {
        "high" => ReasoningEffort::High,
        "max" => ReasoningEffort::Max,
        other => bail!("unknown effort: {other} (high | max)"),
    };
    let permission_mode = match (cli.auto, cli.read_only) {
        (true, true) => bail!("--auto and --read-only are mutually exclusive"),
        (true, false) => PermissionMode::Auto,
        (false, true) => PermissionMode::ReadOnly,
        (false, false) => match file.mode.as_deref() {
            Some("auto") => PermissionMode::Auto,
            Some("read-only") | Some("readonly") => PermissionMode::ReadOnly,
            Some("ask") | None => PermissionMode::Ask,
            Some(other) => bail!("unknown mode in config: {other} (ask | auto | read-only)"),
        },
    };
    let max_session_cost_microusd = match cli.max_cost_usd.as_ref().or(file.max_cost_usd.as_ref()) {
        Some(text) => Some(
            usd_to_microusd(text).with_context(|| format!("invalid max_cost_usd value: {text}"))?,
        ),
        None => None,
    };
    let mut context = butterfly_core::context::ContextConfig {
        window_tokens: model.context_window_tokens(),
        ..butterfly_core::context::ContextConfig::default()
    };
    if let Some(window) = cli.window_tokens.or(file.window_tokens) {
        context.window_tokens = window;
    }
    let config = SessionConfig {
        model,
        effort,
        permission_mode,
        max_session_cost_microusd,
        context,
        // Atlas defaults on only for large repos: measured 2026-09-01, index
        // tools cost turns on small ones. Flags/config force either way.
        atlas_enabled: if cli.no_atlas {
            false
        } else if cli.atlas {
            true
        } else if let Some(configured) = file.atlas {
            configured
        } else {
            butterfly_atlas::count_indexable_files(&root, 100) >= 100
        },
        ..SessionConfig::default()
    };

    // Key resolution: environment > credential store > first-run wizard.
    use std::io::IsTerminal as _;
    let api_key = match std::env::var("DEEPSEEK_API_KEY") {
        Ok(key) if !key.trim().is_empty() => key,
        _ => match credentials::load_key("deepseek") {
            Some(key) => key,
            None if std::io::stdin().is_terminal()
                && std::io::stdout().is_terminal()
                && !cli.json =>
            {
                // Genuine first run: connect once, never ask again.
                match tui::connect::standalone_connect()? {
                    Some((provider, key)) => {
                        let path = credentials::save_key(&provider, &key)?;
                        eprintln!("connected · key saved to {}", path.display());
                        key
                    }
                    None => bail!("no API key configured (run butterfly and complete /connect, or export DEEPSEEK_API_KEY)"),
                }
            }
            None => bail!(
                "DEEPSEEK_API_KEY is required (export it, or run butterfly in a terminal once and use /connect)"
            ),
        },
    };
    let base_url = cli
        .base_url
        .or_else(|| std::env::var("DEEPSEEK_BASE_URL").ok())
        .or(file.base_url);
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

    let show_reasoning = !(cli.quiet_reasoning || file.quiet_reasoning.unwrap_or(false));
    if let Some(prompt) = cli.prompt {
        let mut agent = agent;
        if cli.json {
            if agent.permission_mode() == PermissionMode::Ask {
                bail!("--json cannot prompt for approvals; pass --auto or --read-only");
            }
            // Headless: nobody can answer a forced review, so policy-warned
            // calls (e.g. non-allowlisted commands) are denied, not approved.
            struct HeadlessGate;
            #[async_trait::async_trait]
            impl butterfly_core::session::approval::ApprovalGate for HeadlessGate {
                async fn review(
                    &self,
                    request: &butterfly_core::session::approval::ApprovalRequest,
                ) -> butterfly_core::session::approval::Decision {
                    if request.policy_warning.is_some() {
                        butterfly_core::session::approval::Decision::Deny
                    } else {
                        butterfly_core::session::approval::Decision::Approve
                    }
                }
            }
            let report = agent
                .run_turn(
                    prompt,
                    &HeadlessGate,
                    &mut butterfly_core::provider::NullSink,
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
                    "hit_output_limit": report.hit_output_limit,
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
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        // The TUI hides the reasoning stream by default (Ctrl+R toggles it
        // live; Ctrl+T shows the full transcript). Config can opt back in
        // with an explicit `quiet_reasoning = false`.
        let tui_reasoning = !cli.quiet_reasoning && file.quiet_reasoning == Some(false);
        tui::run(agent, workspace_root, tui_reasoning).await
    } else {
        repl::plain_repl(agent, show_reasoning).await
    }
}

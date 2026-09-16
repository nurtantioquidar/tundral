//! butterfly — the Butterfly interactive coding agent CLI.

mod config;
mod credentials;
mod export;
mod providers;
mod repl;
mod state;
mod tui;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use butterfly_core::domain::usd_to_microusd;
use butterfly_core::model::ModelDescriptor;
use butterfly_core::provider::Provider;
use butterfly_core::session::approval::PermissionMode;
use butterfly_core::session::store::{replay, SessionStore};
use butterfly_core::session::{Agent, SessionConfig, TaskOptions};
use butterfly_core::verify::VerificationPlan;
use clap::{Parser, Subcommand};

use providers::ModelHub;

#[derive(Parser)]
#[command(
    name = "butterfly",
    about = "Fast, cache-first coding agent for any OpenAI-compatible model",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Workspace root (defaults to the current directory).
    #[arg(long)]
    root: Option<PathBuf>,

    /// Model id: a built-in (deepseek-flash | deepseek-v4-flash | deepseek-v4-pro | glm-5.3-flash |
    /// gpt-5.6-luna | nimble) or a `[[models]]` entry from config. Default: config, else
    /// the last model used, else deepseek-v4-flash.
    #[arg(long)]
    model: Option<String>,

    /// Reasoning effort in the model's vocabulary (DeepSeek: high | max).
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

    /// Override the selected model's provider endpoint.
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

    /// With --prompt: attempts per task. After each attempt the verification
    /// stage runs; on failure its evidence is fed back and the model tries
    /// again. 1 = a single turn with no retry.
    #[arg(long, default_value_t = 1)]
    max_attempts: u32,

    /// Acceptance command run after each attempt (argv split on whitespace,
    /// or a JSON array of strings). Repeatable. Failing = the task failed.
    #[arg(long = "verify-cmd")]
    verify_cmd: Vec<String>,

    /// Also run build/test checks detected from repository manifests
    /// (Cargo.toml, package.json, pyproject.toml, go.mod) after each attempt.
    #[arg(long)]
    detect_checks: bool,

    /// Verification fails when the git workspace shows no change.
    #[arg(long)]
    require_diff: bool,

    /// Seconds allowed per verification check.
    #[arg(long, default_value_t = 600)]
    check_timeout: u64,

    /// Provider round-trips allowed per attempt before the turn is cut off
    /// (a cut-off attempt continues on the next one).
    #[arg(long, default_value_t = 50)]
    max_calls: u32,

    /// Treat the workspace as a disposable sandbox you own (benchmark or CI
    /// container): relax path confinement and the command allowlist, expose
    /// the `shell` tool, approve everything. Never use on a machine you care
    /// about.
    #[arg(long)]
    sandboxed: bool,

    /// Write telemetry JSONL to this path (default: the session directory).
    #[arg(long)]
    telemetry: Option<PathBuf>,

    /// Coarse task class forwarded to a router (debugging | feature | refactor
    /// | test | explain | ops). Never inferred from the prompt.
    #[arg(long)]
    task_type: Option<String>,

    /// With --prompt: the task ends only when the model calls `complete_task`
    /// with evidence per deliverable, after a review of the task statement.
    /// Replies without tool calls are nudged, never accepted as completion.
    /// Model-supplied acceptance checks run after the claim. Extends the
    /// prompt prefix (appended line + one schema) only when set.
    #[arg(long)]
    completion_gate: bool,

    /// With --completion-gate: review every accepted claim in a fresh-context
    /// verifier session (read-only tools, no memory of the work) before the
    /// run treats it as finished. Requirements the verifier finds unmet become
    /// the next attempt's evidence. A verifier pass is still a claim, never a
    /// verified verdict.
    #[arg(long, requires = "completion_gate")]
    independent_verifier: bool,

    /// Append the lean-turns rules to the prompt: no read-back after a
    /// successful edit, orientation limited to the files the task involves,
    /// independent reads issued together. Extends the prompt prefix only when
    /// set (measured 2026-09-12: two re-reads per task and five minutes of
    /// orientation before the first edit on GLM).
    #[arg(long)]
    lean: bool,

    /// Clarify mode: advertise `ask_user` so the model can put the task
    /// statement's open points (names, signatures, exact messages, unspecified
    /// cases) to you once before its first edit. Interactive runs answer on the
    /// terminal; headless runs need --answers-cmd or the model is told to
    /// proceed on its stated assumptions. Extends the prompt prefix only when set.
    #[arg(long)]
    clarify: bool,

    /// Whole-file reads of files with at least this many lines return an
    /// Atlas outline (definitions with exact line ranges, imports) instead of
    /// the text, so the model orients in a large file for a few hundred tokens
    /// and then reads the ranges it needs. 0 always returns the text. Changes
    /// tool results only, never the prompt prefix.
    #[arg(long, default_value_t = 400, value_name = "LINES")]
    outline_min_lines: usize,

    /// Always answer the first complete_task with the statement review, even
    /// when the claim already names a behavioural check that ran and passed in
    /// this session with every requirement listed as met (the default skips
    /// that round trip; measured 2026-09-16 as a pure re-claim in 54 of 72 runs).
    #[arg(long, requires = "completion_gate")]
    always_review_claim: bool,

    /// With --clarify in --json runs: a command that answers questions. It gets
    /// the rendered questions on stdin and its stdout is the answer (e.g. a
    /// script that consults a ticket or a spec). Split like a shell word list.
    #[arg(long, requires = "clarify")]
    answers_cmd: Option<String>,

    /// With --independent-verifier: run the verifier on this model instead of
    /// the worker's (cross-model verification; needs that provider's key).
    #[arg(long, requires = "independent_verifier")]
    verifier_model: Option<String>,
}

/// `--verify-cmd` value → argv. JSON arrays allow arguments with spaces.
fn parse_check(text: &str) -> Result<Vec<String>> {
    let trimmed = text.trim();
    if trimmed.starts_with('[') {
        let argv: Vec<String> = serde_json::from_str(trimmed)
            .with_context(|| format!("--verify-cmd is not a JSON array of strings: {text}"))?;
        return Ok(argv);
    }
    let argv: Vec<String> = trimmed.split_whitespace().map(str::to_string).collect();
    if argv.is_empty() {
        bail!("--verify-cmd is empty");
    }
    Ok(argv)
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
    /// List the models the registry knows (built-ins plus config).
    Models,
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
        Some(Command::Models) => {
            let root = std::env::current_dir().context("cannot resolve current directory")?;
            let hub = ModelHub::from_config(&config::load(&root)?, None)?;
            for model in hub.registry.models() {
                let provider = hub.provider_descriptor(model)?;
                println!(
                    "{}\t{}\tprovider={} ({})\tcontext={}\tpricing={}{}",
                    model.id,
                    model.display_name,
                    provider.id,
                    provider.api_key_env,
                    model.limits.context_tokens,
                    model.pricing.version,
                    if model.pricing.verified {
                        ""
                    } else {
                        " (unverified)"
                    }
                );
            }
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
    let hub = ModelHub::from_config(
        &file,
        cli.base_url.clone().or_else(|| file.base_url.clone()),
    )?;

    // A resumed session continues on the model it was recorded with unless a
    // flag or config pins another one.
    let resume_target = match &cli.resume {
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
            let recorded_model = replay(&store.read_events()?).model;
            Some((session_id, store, recorded_model))
        }
        None => None,
    };

    // Model resolution: explicit pins (flag, config files) win; otherwise a
    // resumed session's own model; otherwise the last model the user ran with.
    let model_name = cli
        .model
        .clone()
        .or(file.model.clone())
        .or_else(|| resume_target.as_ref().and_then(|(_, _, m)| m.clone()))
        .or_else(state::last_model)
        .unwrap_or_else(|| butterfly_core::model::builtin::REFERENCE_MODEL_ID.to_string());
    let model = hub.resolve(&model_name)?;
    state::remember_model(&model.id);
    let effort = model
        .resolve_effort(cli.effort.as_deref().or(file.effort.as_deref()))
        .map_err(|error| anyhow::anyhow!(error))?;
    // A sandbox implies auto-approval (nothing there needs protecting from
    // the model) unless the caller explicitly asked for read-only.
    let permission_mode = match (cli.auto || (cli.sandboxed && !cli.read_only), cli.read_only) {
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
        window_tokens: model.limits.context_tokens,
        ..butterfly_core::context::ContextConfig::default()
    };
    if let Some(window) = cli.window_tokens.or(file.window_tokens) {
        context.window_tokens = window;
    }
    let config = SessionConfig {
        model: model.clone(),
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
        sandboxed: cli.sandboxed,
        task_type: cli.task_type.clone(),
        max_provider_calls_per_input: cli.max_calls.max(1),
        completion_gate: cli.completion_gate,
        lean: cli.lean,
        clarify: cli.clarify,
        independent_verifier: cli.independent_verifier,
        outline_min_lines: cli.outline_min_lines,
        always_review_claim: cli.always_review_claim,
        ..SessionConfig::default()
    };
    let task_options = TaskOptions {
        max_attempts: cli.max_attempts.max(1),
        verification: VerificationPlan {
            explicit: cli
                .verify_cmd
                .iter()
                .map(|text| parse_check(text))
                .collect::<Result<Vec<_>>>()?,
            detect: cli.detect_checks,
            require_diff: cli.require_diff,
            check_timeout_seconds: cli.check_timeout.max(1),
        },
    };

    let provider = connect_provider(&hub, &model, cli.json)?;
    let verifier_backend = match cli.verifier_model.as_deref() {
        Some(id) => {
            let verifier_model = hub.resolve(id)?;
            let verifier_provider = connect_provider(&hub, &verifier_model, cli.json)?;
            Some((verifier_provider, verifier_model))
        }
        None => None,
    };

    let agent = match resume_target {
        Some((session_id, store, _)) => {
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
    let mut agent = agent;
    if let Some((verifier_provider, verifier_model)) = verifier_backend {
        agent.set_verifier_backend(verifier_provider, verifier_model);
    }
    if let Some(path) = cli.telemetry.clone() {
        agent.set_telemetry_path(path);
    }
    if let Some(prompt) = cli.prompt {
        if cli.json {
            if agent.permission_mode() == PermissionMode::Ask {
                bail!("--json cannot prompt for approvals; pass --auto or --read-only");
            }
            // Headless: nobody can answer a forced review, so policy-warned
            // calls (e.g. non-allowlisted commands) are denied, not approved.
            struct HeadlessGate {
                answers_cmd: Option<Vec<String>>,
            }
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
                async fn answer_question(&self, question: &str) -> Option<String> {
                    let argv = self.answers_cmd.clone()?;
                    let question = question.to_string();
                    tokio::task::spawn_blocking(move || {
                        use std::io::Write;
                        let mut child = std::process::Command::new(&argv[0])
                            .args(&argv[1..])
                            .stdin(std::process::Stdio::piped())
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::inherit())
                            .spawn()
                            .ok()?;
                        child.stdin.take()?.write_all(question.as_bytes()).ok()?;
                        let output = child.wait_with_output().ok()?;
                        output
                            .status
                            .success()
                            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
                    })
                    .await
                    .ok()
                    .flatten()
                }
            }
            // Same shape as --verify-cmd: whitespace-split argv or a JSON array.
            let answers_cmd = match cli.answers_cmd.as_deref() {
                Some(text) => Some(parse_check(text)?),
                None => None,
            };
            let report = match agent
                .run_task(
                    prompt,
                    &task_options,
                    &HeadlessGate { answers_cmd },
                    &mut butterfly_core::provider::NullSink,
                    &tokio_util::sync::CancellationToken::new(),
                )
                .await
            {
                Ok(report) => report,
                Err(error) => {
                    // A failed run still reports what it spent: benchmarks
                    // join on this JSON, and a bare non-zero exit loses it.
                    let model = agent.model();
                    println!(
                        "{}",
                        serde_json::json!({
                            "session_id": agent.session_id,
                            "model": model.id,
                            "provider": model.provider,
                            "error": error.to_string(),
                            "verified": false,
                            "telemetry": agent.telemetry_summary(),
                            "telemetry_path": agent.telemetry_path(),
                            "routing": agent.last_routing(),
                        })
                    );
                    std::process::exit(1);
                }
            };
            let usage = report.usage;
            let model = agent.model();
            let attempts: Vec<serde_json::Value> = report
                .attempts
                .iter()
                .map(|attempt| {
                    serde_json::json!({
                        "attempt": attempt.attempt,
                        "provider_calls": attempt.turn.provider_calls,
                        "tool_calls": attempt.turn.tool_calls,
                        "failures": attempt.turn.failures.iter().map(|f| f.kind.as_str()).collect::<Vec<_>>(),
                        "completed": attempt.turn.completed,
                        "completion_nudges": attempt.turn.completion_nudges,
                        "self_authored_checks": attempt.turn.self_authored_checks,
                        "first_claim_proven": attempt.turn.first_claim_proven,
                        "blind_edits": attempt.turn.blind_edits,
                        "outline_reads": attempt.turn.outline_reads,
                        "verifier": attempt.verifier.as_ref().map(|v| serde_json::json!({
                            "model": v.model,
                            "verdict": v.verdict.as_ref().map(|d| if d.passed() { "pass" } else { "fail" }),
                            "unmet": v.verdict.as_ref().map(|d| d.unmet().iter().map(|r| r.requirement.clone()).collect::<Vec<_>>()),
                            "requirements": v.verdict.as_ref().map(|d| &d.requirements),
                            "provider_calls": v.provider_calls,
                            "tool_calls": v.tool_calls,
                            "cost_microusd": v.actual_microusd,
                            "error": v.error,
                        })),
                        "verification": attempt.verification.as_ref().map(|v| serde_json::json!({
                            "passed": v.passed(),
                            "summary": v.summary_line(),
                            "checks": v.checks.iter().map(|c| serde_json::json!({
                                "command": c.check.label(),
                                "source": c.check.source,
                                "ok": c.ok,
                                "duration_ms": c.duration_ms,
                                "failure": c.failure,
                            })).collect::<Vec<_>>(),
                        })),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::json!({
                    "session_id": agent.session_id,
                    "model": model.id,
                    "provider": model.provider,
                    "response": report.assistant_text,
                    // Truthful: true only when an operator/detected check passed.
                    "verified": report.verified,
                    "verification_source": report.verification_source,
                    "checks_passed": report.success,
                    "completed": report.completed,
                    "completion": report.completion,
                    "attempts": attempts,
                    "latency_ms": report.latency_ms,
                    "retries": report.retries,
                    "telemetry": agent.telemetry_summary(),
                    "telemetry_path": agent.telemetry_path(),
                    "routing": agent.last_routing(),
                    "provider_calls": report.provider_calls,
                    "tool_calls": report.tool_calls,
                    "input_tokens": usage.input_tokens,
                    "cache_hit_input_tokens": usage.cache_hit_input_tokens,
                    "cache_miss_input_tokens": usage.cache_miss_input_tokens,
                    "output_tokens": usage.output_tokens,
                    "cache_hit_percent": (usage.cache_hit_rate() * 1000.0).round() / 10.0,
                    "actual_cost_microusd": report.actual_microusd,
                    "normalized_cache_miss_cost_microusd": report.normalized_microusd,
                    "price_version": model.pricing.version,
                    "price_verified": model.pricing.verified,
                    "session_cost_microusd": agent.total_actual_microusd(),
                    "context_utilization_percent": (agent.utilization() * 1000.0).round() / 10.0,
                    "compactions": report.compactions,
                    "elisions": report.elisions,
                    "summary": report.render_summary(),
                    "cost_cap_microusd": report.cost_cap_microusd,
                    "blind_edits": report.blind_edits,
                    "outline_reads": report.outline_reads,
                    "avoided_input_tokens": report.avoided_input_tokens,
                    "avoided_microusd": report.avoided_microusd,
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
        let report = repl::run_task_interruptible(
            &mut agent,
            prompt,
            &task_options,
            &repl::TerminalGate,
            show_reasoning,
        )
        .await?;
        // The verification story in one block, the same one --json carries
        // structurally: what backed the ending, what was checked and by whom,
        // and what the model merely claimed.
        eprintln!("\n{}", report.render_summary().trim_end());
        if report.cancelled {
            std::process::exit(130);
        }
        if report.success == Some(false) {
            std::process::exit(1);
        }
        return Ok(());
    }
    use std::io::IsTerminal as _;
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        // The TUI hides the reasoning stream by default (Ctrl+R toggles it
        // live; Ctrl+T shows the full transcript). Config can opt back in
        // with an explicit `quiet_reasoning = false`.
        let tui_reasoning = !cli.quiet_reasoning && file.quiet_reasoning == Some(false);
        tui::run(agent, workspace_root, tui_reasoning, &hub).await
    } else {
        repl::plain_repl(agent, show_reasoning, &hub).await
    }
}

/// Key resolution: environment > credential store > first-run wizard (TTY
/// only). Headless runs fail fast with the variable name to set.
fn connect_provider(
    hub: &ModelHub,
    model: &ModelDescriptor,
    headless_json: bool,
) -> Result<Arc<dyn Provider>> {
    use std::io::IsTerminal as _;
    let descriptor = hub.provider_descriptor(model)?;
    if let Some((key, _)) = ModelHub::resolve_key(descriptor) {
        return hub.build_provider_with_key(model, key);
    }
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() && !headless_json {
        // Genuine first run: connect once, never ask again. The wizard is
        // pre-selected on the provider the chosen model needs.
        let mut choices = hub.connect_choices();
        if let Some(index) = choices.iter().position(|(id, _)| id == &descriptor.id) {
            let preferred = choices.remove(index);
            choices.insert(0, preferred);
        }
        match tui::connect::standalone_connect(&choices)? {
            Some((provider_id, key)) => {
                let path = credentials::save_key(&provider_id, &key)?;
                eprintln!("connected · {provider_id} key saved to {}", path.display());
                if provider_id != descriptor.id {
                    bail!(
                        "saved a key for '{provider_id}', but model {} needs provider '{}' ({}); \
                         run again to connect it",
                        model.id,
                        descriptor.id,
                        descriptor.api_key_env
                    );
                }
                return hub.build_provider_with_key(model, key);
            }
            None => bail!(
                "no API key configured for provider '{}' (export {} or complete /connect)",
                descriptor.id,
                descriptor.api_key_env
            ),
        }
    }
    bail!(
        "{} is required for model {} (export it, or run butterfly in a terminal once and use /connect)",
        descriptor.api_key_env,
        model.id
    )
}

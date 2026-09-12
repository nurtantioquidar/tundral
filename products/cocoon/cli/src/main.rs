use clap::{Parser, Subcommand};
use cocoon::{
    broker::{router, BrokerState, ProviderRuntime},
    config::Config,
    control::{
        bind as bind_control, issue_session, revoke_session, serve as serve_control, ControlState,
    },
    env_guard::{scan_workspace, scrubbed_environment},
    secret::read_one_password,
};
use std::{
    collections::BTreeMap,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
};
use tokio::{net::TcpListener, process::Command};

#[derive(Parser)]
#[command(name = "cocoon", version, about = "Credential firewall for AI agents")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate configuration and detect plaintext secrets in the workspace.
    Check {
        #[arg(long)]
        config: PathBuf,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Start the long-lived credential broker and resolve vault references once.
    Daemon {
        #[arg(long)]
        config: PathBuf,
        #[arg(long, default_value = "127.0.0.1:4646")]
        listen: SocketAddr,
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Run a supervised process using a short-lived daemon session.
    Run {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    match execute(Cli::parse()).await {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("cocoon: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn execute(cli: Cli) -> Result<u8, String> {
    match cli.command {
        Commands::Check { config, workspace } => {
            Config::load(&config)?;
            require_clean_environment(&workspace)?;
            println!("configuration and workspace passed Cocoon checks");
            Ok(0)
        }
        Commands::Daemon {
            config,
            listen,
            socket,
        } => daemon(config, listen, socket).await,
        Commands::Run {
            profile,
            socket,
            workspace,
            command,
        } => run(socket, &profile, workspace, command).await,
    }
}

fn require_clean_environment(workspace: &Path) -> Result<(), String> {
    let findings = scan_workspace(workspace)?;
    if findings.is_empty() {
        return Ok(());
    }
    let summary = findings
        .iter()
        .map(|finding| {
            format!(
                "{}:{} ({})",
                finding.path.display(),
                finding.line,
                finding.variable
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "plaintext secret-like values found: {summary}; values were not displayed"
    ))
}

async fn daemon(
    config_path: PathBuf,
    listen: SocketAddr,
    socket: Option<PathBuf>,
) -> Result<u8, String> {
    if !listen.ip().is_loopback() {
        return Err("the initial Cocoon daemon may listen only on loopback".to_owned());
    }
    let config = Config::load(&config_path)?;
    let mut providers = BTreeMap::new();
    for (name, provider_config) in &config.providers {
        eprintln!("cocoon: resolving {name} credential from 1Password");
        let secret = read_one_password(&provider_config.secret).await?;
        providers.insert(name.clone(), ProviderRuntime::new(provider_config, secret)?);
    }
    let broker = BrokerState::new(providers)?;
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|error| format!("could not bind local broker: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("could not inspect local broker: {error}"))?;
    let socket = control_socket(socket)?;
    let control_listener = bind_control(&socket)?;
    let control_state =
        ControlState::new(broker.clone(), config.profiles, format!("http://{address}"));
    let control_task = tokio::spawn(async move {
        if let Err(message) = serve_control(control_listener, control_state).await {
            eprintln!("cocoon: {message}");
        }
    });
    eprintln!(
        "cocoon: ready on {address}; control socket {}",
        socket.display()
    );

    let server_result = axum::serve(listener, router(broker))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    control_task.abort();
    let _ = control_task.await;
    if let Err(error) = fs::remove_file(&socket) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(format!(
                "could not remove control socket {}: {error}",
                socket.display()
            ));
        }
    }
    server_result.map_err(|error| format!("local broker failed: {error}"))?;
    Ok(0)
}

async fn run(
    socket: Option<PathBuf>,
    profile_name: &str,
    workspace: PathBuf,
    command: Vec<String>,
) -> Result<u8, String> {
    require_clean_environment(&workspace)?;
    let socket = control_socket(socket)?;
    let grant = issue_session(&socket, profile_name).await?;

    let executable = command
        .first()
        .ok_or_else(|| "a command is required".to_owned())?;
    let mut child = Command::new(executable);
    child
        .args(&command[1..])
        .current_dir(&workspace)
        .env_clear()
        .envs(scrubbed_environment())
        .env(&grant.base_url_env, &grant.base_url)
        .env(&grant.api_key_env, grant.token.as_str());

    // The session token is intentionally obtained from configuration established above, never
    // from the upstream credential. This placeholder is replaced before the request leaves Cocoon.
    let status_result = child.status().await;
    let revoke_result = revoke_session(&socket, &grant.token).await;
    let status = status_result.map_err(|error| format!("could not run {executable:?}: {error}"))?;
    revoke_result?;
    Ok(status.code().unwrap_or(1).clamp(0, u8::MAX as i32) as u8)
}

fn control_socket(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| "HOME is unavailable; pass --socket explicitly".to_owned())?;
    #[cfg(target_os = "macos")]
    let path = PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Cocoon")
        .join("cocoon.sock");
    #[cfg(not(target_os = "macos"))]
    let path = PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("cocoon")
        .join("cocoon.sock");
    Ok(path)
}

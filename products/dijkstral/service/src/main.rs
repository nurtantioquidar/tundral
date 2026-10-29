use std::path::Path;

use anyhow::{bail, Result};
use dijkstral_service::{app, builder, config::Config};
use tokio::signal::unix::{signal, SignalKind};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        // A build child (`builder::run_child`): one code graph, then exit.
        Some("build") => match args.get(2) {
            Some(dir) => builder::child_main(Path::new(dir)),
            None => bail!("usage: dijkstral-service build <dir>"),
        },
        None | Some("serve") => serve(),
        Some(other) => {
            bail!("unknown command {other:?}; usage: dijkstral-service [serve|build <dir>]")
        }
    }
}

fn serve() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = Config::from_env()?;
    tokio::runtime::Runtime::new()?.block_on(async move {
        let listen = config.listen;
        let (router, _state) = app::app(config)?;
        let listener = tokio::net::TcpListener::bind(listen).await?;
        tracing::info!(%listen, "dijkstral-service listening");
        // Ctrl-C, or SIGTERM from a supervisor (`docker stop`, systemd).
        let mut term = signal(SignalKind::terminate())?;
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            })
            .await?;
        Ok(())
    })
}

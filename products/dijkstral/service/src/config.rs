//! Service configuration, read once from the environment at start.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// Where repositories are fetched from.
#[derive(Debug, Clone)]
pub enum SourceConfig {
    /// `<base>/<owner>/<repo>.git`: local bare repositories, for tests and local runs.
    Local(PathBuf),
}

/// Limits that keep one host healthy while the service is free (`DIJKSTRAL_HOSTED_DESIGN.md` §6).
#[derive(Debug, Clone)]
pub struct Limits {
    pub repos_per_installation: usize,
    pub checkout_bytes: u64,
    pub nodes: usize,
    pub builds_per_repo_per_day: usize,
    pub build_timeout: Duration,
    pub mcp_requests_per_minute: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            repos_per_installation: 5,
            checkout_bytes: 1 << 30,
            nodes: 300_000,
            builds_per_repo_per_day: 50,
            build_timeout: Duration::from_secs(300),
            mcp_requests_per_minute: 120,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Holds `db/service.sqlite` and `repos/<repo id>/`.
    pub data: PathBuf,
    pub listen: SocketAddr,
    /// The GitHub App's webhook secret.
    pub webhook_secret: Vec<u8>,
    /// Bearer token of the web app for `/internal/*`.
    pub internal_token: String,
    pub source: SourceConfig,
    /// This executable, re-run as `build <dir>` for each build.
    pub exe: PathBuf,
    /// Threads a build may use.
    pub build_threads: usize,
    pub limits: Limits,
}

fn var(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} is not set"))
}

impl Config {
    /// `DIJKSTRAL_SERVICE_DATA`, `DIJKSTRAL_SERVICE_LISTEN` (default `127.0.0.1:8095`),
    /// `DIJKSTRAL_SERVICE_WEBHOOK_SECRET`, `DIJKSTRAL_SERVICE_INTERNAL_TOKEN` and
    /// `DIJKSTRAL_SERVICE_LOCAL_SOURCE` (the only source until the GitHub App source lands).
    pub fn from_env() -> Result<Config> {
        let webhook_secret = var("DIJKSTRAL_SERVICE_WEBHOOK_SECRET")?.into_bytes();
        let internal_token = var("DIJKSTRAL_SERVICE_INTERNAL_TOKEN")?;
        if webhook_secret.len() < 16 || internal_token.len() < 32 {
            bail!("the webhook secret needs at least 16 bytes and the internal token 32");
        }
        Ok(Config {
            data: var("DIJKSTRAL_SERVICE_DATA")?.into(),
            listen: std::env::var("DIJKSTRAL_SERVICE_LISTEN")
                .unwrap_or_else(|_| "127.0.0.1:8095".into())
                .parse()
                .context("DIJKSTRAL_SERVICE_LISTEN")?,
            webhook_secret,
            internal_token,
            source: SourceConfig::Local(var("DIJKSTRAL_SERVICE_LOCAL_SOURCE")?.into()),
            exe: std::env::current_exe()?,
            build_threads: 2,
            limits: Limits::default(),
        })
    }
}

//! The `dijkstral-live` helper (`src/bin/dijkstral-live/`): `watch` and `extract --postgres` run
//! there, so this binary links no file-watcher or database client and starts as fast as a program
//! that does nothing (every assistant hook call starts it). The helper ships next to `dijkstral`;
//! a missing or mismatched one fails only these two commands.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use dijkstral_core::inputs::postgres::Snapshot;

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// Directory to watch; its dijkstral-out/ is rebuilt.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Seconds without changes before a rebuild starts.
    #[arg(long, value_name = "SECS", default_value_t = 1.0)]
    debounce: f64,
}

const HELPER: &str = if cfg!(windows) {
    "dijkstral-live.exe"
} else {
    "dijkstral-live"
};

/// The helper next to this executable.
fn helper() -> Result<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().context("locating the dijkstral binary")?;
    let path = exe.with_file_name(HELPER);
    if !path.is_file() {
        bail!(
            "{} is missing: `dijkstral watch` and `extract --postgres` run in it; reinstall \
             dijkstral so both binaries are installed together",
            path.display()
        );
    }
    Ok((path, exe))
}

fn command(path: &Path) -> Command {
    let mut cmd = Command::new(path);
    cmd.arg(env!("CARGO_PKG_VERSION"));
    cmd
}

/// `dijkstral watch`: hand the terminal to the helper (on Unix it replaces this process, so
/// Ctrl-C reaches the watcher directly).
pub fn watch(a: WatchArgs) -> ExitCode {
    let (root, debounce) = (a.path, a.debounce);
    let (path, exe) = match helper() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let mut cmd = command(&path);
    cmd.arg("watch")
        .arg(&exe)
        .arg(&root)
        .arg(debounce.to_string());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = cmd.exec();
        eprintln!("error: running {}: {e}", path.display());
        ExitCode::FAILURE
    }
    #[cfg(not(unix))]
    match cmd.status() {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("error: running {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

/// `extract --postgres DSN`: the catalog snapshot, read by the helper. The DSN goes over stdin,
/// never the command line, since it may hold a password.
pub fn postgres(dsn: &str) -> Result<Snapshot> {
    let (path, _) = helper()?;
    let mut child = command(&path)
        .arg("postgres")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {}", path.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(dsn.as_bytes())
            .with_context(|| format!("writing to {}", path.display()))?;
    }
    let out = child
        .wait_with_output()
        .with_context(|| format!("running {}", path.display()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        let err = err.strip_prefix("error: ").unwrap_or(err);
        return Err(anyhow!("{err}"));
    }
    serde_json::from_slice(&out.stdout).context("reading the catalog from dijkstral-live")
}

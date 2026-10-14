//! Assistant integration commands: `install`, `uninstall`, `claude`, `codex`, and the hidden
//! `hook-guard` / `hook-check` hooks, which are dispatched before clap parses anything so the
//! per-tool-call path stays as short as possible.

use std::ffi::OsString;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand, ValueEnum};
use dijkstral_agents::guard;
use dijkstral_agents::install::{self, Change, Platform};

#[derive(Subcommand, Debug)]
pub enum AgentCommand {
    /// Install the dijkstral skill for an assistant: user-wide by default, or into this project
    /// with --project (which also writes the project's instructions and hooks).
    Install(InstallArgs),
    /// Remove every dijkstral integration: the user-wide skills and this project's Claude Code
    /// and Codex files.
    Uninstall,
    /// Claude Code integration for this project: a CLAUDE.md section and PreToolUse hooks that
    /// point searches and source reads at the graph.
    #[command(subcommand)]
    Claude(ClaudeAction),
    /// Codex integration for this project: an AGENTS.md section and a .codex/hooks.json entry.
    #[command(subcommand)]
    Codex(CodexAction),
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Assistant to install for.
    #[arg(long, value_enum, default_value_t = PlatformArg::Claude)]
    platform: PlatformArg,
    /// Install into the current project instead of the user's home directory.
    #[arg(long)]
    project: bool,
    /// With --project for Claude Code: deny the first source read of each session until the
    /// graph has been consulted.
    #[arg(long, requires = "project")]
    strict: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum PlatformArg {
    Claude,
    Codex,
}

#[derive(Subcommand, Debug)]
pub enum ClaudeAction {
    /// Write the CLAUDE.md section and .claude/settings.json hooks.
    Install {
        /// Deny the first source read of each session until the graph has been consulted.
        #[arg(long)]
        strict: bool,
    },
    /// Remove them, keeping everything else in those files.
    Uninstall,
}

#[derive(Subcommand, Debug)]
pub enum CodexAction {
    /// Write the AGENTS.md section and .codex/hooks.json hook.
    Install,
    /// Remove them, keeping everything else in those files.
    Uninstall,
}

/// Runs `hook-guard` or `hook-check` when they are the subcommand; `None` otherwise.
pub fn early_dispatch() -> Option<ExitCode> {
    let mut args = std::env::args_os().skip(1);
    match args.next()?.to_str()? {
        "hook-guard" => {
            hook_guard(args.collect());
            Some(ExitCode::SUCCESS)
        }
        "hook-check" => {
            // Codex rejects `additionalContext` on PreToolUse, so its guidance lives in
            // AGENTS.md and this hook only drains its input.
            read_stdin();
            Some(ExitCode::SUCCESS)
        }
        _ => None,
    }
}

/// Never fails and never writes to stderr: a hook that errors would surface as noise on every
/// tool call. A panic is swallowed and treated as "allow".
fn hook_guard(args: Vec<OsString>) {
    std::panic::set_hook(Box::new(|_| {}));
    let _ = std::panic::catch_unwind(|| {
        let input = read_stdin();
        let Some((mode, strict)) = guard::parse_args(&args) else {
            return;
        };
        let Ok(cwd) = std::env::current_dir() else {
            return;
        };
        if let Some(line) = guard::decide(mode, strict, &input, &cwd).to_stdout() {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(line.as_bytes()).and_then(|()| out.flush());
        }
    });
}

fn read_stdin() -> Vec<u8> {
    let mut buf = Vec::new();
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        let _ = stdin.lock().read_to_end(&mut buf);
    }
    buf
}

pub fn run(cmd: AgentCommand) -> ExitCode {
    match execute(cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn execute(cmd: AgentCommand) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    match cmd {
        AgentCommand::Install(args) => {
            let platform = match args.platform {
                PlatformArg::Claude => Platform::Claude,
                PlatformArg::Codex => Platform::Codex,
            };
            if args.project {
                // A project install is shared with collaborators, so it names the program, not
                // this machine's path to it.
                report(&install::skill_install(platform, &cwd)?);
                match platform {
                    Platform::Claude => {
                        report(&install::claude_project_install(
                            &cwd,
                            "dijkstral",
                            args.strict,
                        )?);
                        claude_note();
                    }
                    Platform::Codex => {
                        report(&install::codex_project_install(&cwd, "dijkstral")?);
                        codex_note();
                    }
                }
            } else {
                report(&install::skill_install(platform, &home()?)?);
            }
        }
        AgentCommand::Uninstall => {
            let home = home()?;
            report(&install::skill_uninstall(Platform::Claude, &home)?);
            report(&install::skill_uninstall(Platform::Codex, &home)?);
            report(&install::claude_project_uninstall(&cwd)?);
            report(&install::codex_project_uninstall(&cwd)?);
        }
        AgentCommand::Claude(ClaudeAction::Install { strict }) => {
            report(&install::claude_project_install(&cwd, &program(), strict)?);
            claude_note();
        }
        AgentCommand::Claude(ClaudeAction::Uninstall) => {
            report(&install::claude_project_uninstall(&cwd)?);
        }
        AgentCommand::Codex(CodexAction::Install) => {
            report(&install::codex_project_install(&cwd, &program())?);
            codex_note();
        }
        AgentCommand::Codex(CodexAction::Uninstall) => {
            report(&install::codex_project_uninstall(&cwd)?);
        }
    }
    Ok(())
}

fn report(changes: &[Change]) {
    for change in changes {
        println!("{change}");
    }
}

fn claude_note() {
    println!(
        "\nClaude Code will steer searches and source reads toward `dijkstral query` whenever \
         dijkstral-out/graph.json exists here. Build it with `dijkstral extract .`."
    );
}

fn codex_note() {
    println!(
        "\nCodex reads the AGENTS.md section. Its PreToolUse hook (`dijkstral hook-check`) is a \
         placeholder that does nothing, because Codex does not accept added context from \
         PreToolUse hooks. Build the graph with `dijkstral extract .`."
    );
}

fn home() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

/// The hook command's program: `dijkstral` as found on PATH (not resolved through symlinks, so a
/// package manager's stable link keeps working after upgrades), else this executable.
fn program() -> String {
    let on_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("dijkstral"))
            .find(|p| is_executable(p))
    });
    let exe = on_path
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("dijkstral"));
    install::hook_program(&exe.to_string_lossy())
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

//! `dijkstral hook install|uninstall|status`, the hidden `hook run` the hook scripts call, and
//! `dijkstral merge-driver` (`docs/graphify-git.md`), over `dijkstral_git`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use dijkstral_git::install::{self, Report};
use dijkstral_git::rebuild;
use dijkstral_git::repo::Repo;

#[derive(Args, Debug)]
pub struct HookArgs {
    #[command(subcommand)]
    action: HookAction,
}

#[derive(Subcommand, Debug)]
enum HookAction {
    /// Add dijkstral to the post-commit and post-checkout hooks (rebuilding dijkstral-out/ in the
    /// background) and register the graph.json merge driver. Existing hooks are kept.
    Install,
    /// Remove what `hook install` added, keeping everything else in the hooks and .gitattributes.
    Uninstall,
    /// Report whether the hooks and the merge driver are installed.
    Status,
    /// What the hook scripts run (not for direct use).
    #[command(hide = true)]
    Run {
        /// The hook: post-commit or post-checkout.
        hook: String,
        /// The arguments git passed to the hook.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Args, Debug)]
pub struct MergeDriverArgs {
    /// The common ancestor's version (git's %O); may be empty.
    base: PathBuf,
    /// This branch's version (%A); receives the result.
    current: PathBuf,
    /// The other branch's version (%B).
    other: PathBuf,
}

pub fn run_hook(a: HookArgs) -> ExitCode {
    if let HookAction::Run { hook, args } = &a.action {
        // Fails open: whatever happens, git sees success.
        if let Ok(exe) = std::env::current_exe() {
            let _ = rebuild::run(&rebuild::hook_top(), hook, args, &exe);
        }
        return ExitCode::SUCCESS;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let repo = match Repo::find(&cwd) {
        Ok(Some(repo)) => repo,
        Ok(None) => {
            eprintln!("error: not inside a git work tree");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let report = match a.action {
        HookAction::Install => match std::env::current_exe() {
            Ok(exe) => install::install(&repo, &exe.canonicalize().unwrap_or(exe)),
            Err(e) => {
                eprintln!("error: locating the dijkstral binary: {e}");
                return ExitCode::FAILURE;
            }
        },
        HookAction::Uninstall => install::uninstall(&repo),
        HookAction::Status => install::status(&repo),
        HookAction::Run { .. } => unreachable!(),
    };
    print(report)
}

fn print(r: Report) -> ExitCode {
    for line in &r.lines {
        println!("{line}");
    }
    if r.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Exit 0 with the merged graph in `current`; exit 1 with `current` untouched, which git reports
/// as a conflict.
pub fn run_merge_driver(a: MergeDriverArgs) -> ExitCode {
    let read = |p: &PathBuf| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    let result = (|| {
        let base = read(&a.base).unwrap_or_default();
        let merged = dijkstral_git::merge::merge(&base, &read(&a.current)?, &read(&a.other)?)
            .map_err(|e| format!("{e:#}"))?;
        dijkstral_core::pipeline::write_atomic(&a.current, merged.json.as_bytes())
            .map_err(|e| format!("writing {}: {e:#}", a.current.display()))
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dijkstral merge-driver: cannot merge graph.json ({e}); left as a conflict");
            ExitCode::FAILURE
        }
    }
}

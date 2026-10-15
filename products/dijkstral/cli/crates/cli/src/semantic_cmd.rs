//! `dijkstral semantic import` (the skill-driven hand-off's second half) and
//! `dijkstral check-update`, over `dijkstral_semantic::run`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use dijkstral_semantic::run;

#[derive(Args, Debug)]
pub struct SemanticArgs {
    #[command(subcommand)]
    command: SemanticCommand,
}

#[derive(Subcommand, Debug)]
enum SemanticCommand {
    /// Validate the host assistant's replies to dijkstral-out/semantic/pending/<n>.json and cache
    /// them; the next `extract --backend handoff` merges them without a model call.
    Import(ImportArgs),
}

#[derive(Args, Debug)]
struct ImportArgs {
    /// Directory holding the replies as <n>.json (default: dijkstral-out/semantic/replies).
    #[arg(value_name = "DIR")]
    replies: Option<PathBuf>,
    /// The indexed directory.
    #[arg(long, default_value = ".")]
    path: PathBuf,
    /// The dijkstral-out directory, when not under --path.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct CheckUpdateArgs {
    /// The indexed directory.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// The dijkstral-out directory, when not under PATH.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
}

fn out_dir(path: &Path, out: Option<PathBuf>) -> PathBuf {
    out.unwrap_or_else(|| run::default_out_dir(path))
}

pub fn run_semantic(a: SemanticArgs) -> ExitCode {
    match a.command {
        SemanticCommand::Import(a) => {
            let out = out_dir(&a.path, a.out);
            let replies = a
                .replies
                .unwrap_or_else(|| run::backend::handoff::replies_dir(&out));
            match run::import(&out, &replies) {
                Ok(report) => {
                    println!("{}", report.summary());
                    if !report.imported.is_empty() {
                        println!(
                            "run `dijkstral extract {} --backend handoff` to merge them",
                            a.path.display()
                        );
                    }
                    if report.rejected.is_empty() {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::FAILURE
                    }
                }
                Err(e) => {
                    eprintln!("error: {e:#}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// Prints nothing when the semantic pass is current, else two lines; exits 0 either way.
pub fn run_check_update(a: CheckUpdateArgs) -> ExitCode {
    let out = out_dir(&a.path, a.out);
    if let Some(why) = run::pending_changes(&a.path, &out) {
        let abs = std::path::absolute(&a.path).unwrap_or(a.path);
        println!(
            "[dijkstral check-update] Pending non-code changes in {} ({why}).",
            abs.display()
        );
        println!(
            "[dijkstral check-update] Run `dijkstral extract` (or /dijkstral in your assistant) \
             to apply semantic re-extraction."
        );
    }
    ExitCode::SUCCESS
}

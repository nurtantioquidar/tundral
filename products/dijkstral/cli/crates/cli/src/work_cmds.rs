//! The commands beside the graph (`docs/graphify-memory.md`): the work-memory loop
//! (`memory_cmd.rs`), the PR dashboard (`prs_cmd.rs`) and `diagnose multigraph`
//! (`diagnose_cmd.rs`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;

use crate::{diagnose_cmd, memory_cmd, prs_cmd};

#[derive(Subcommand, Debug)]
pub enum WorkCommand {
    /// Save a question and its answer to dijkstral-out/memory/ (the work memory), optionally
    /// marked useful, dead_end or corrected, citing the graph nodes it relied on.
    SaveResult(memory_cmd::SaveArgs),
    /// Fold the saved results into dijkstral-out/reflections/LESSONS.md: the nodes worth starting
    /// from, dead ends and corrections, by community when a graph is available. No model.
    Reflect(memory_cmd::ReflectArgs),
    /// Open pull requests from gh: CI and review state, the worktree that has each checked out,
    /// and, for one PR, the graph nodes its changes touch. Read-only.
    Prs(prs_cmd::PrsArgs),
    /// Diagnostics over a graph file.
    Diagnose(diagnose_cmd::DiagnoseArgs),
}

pub fn run(cmd: WorkCommand) -> ExitCode {
    match cmd {
        WorkCommand::SaveResult(a) => memory_cmd::run_save(a),
        WorkCommand::Reflect(a) => memory_cmd::run_reflect(a),
        WorkCommand::Prs(a) => prs_cmd::run(a),
        WorkCommand::Diagnose(a) => diagnose_cmd::run(a),
    }
}

/// Graph files looked for in a project, in preference order.
pub const PROJECT_GRAPHS: &[&str] = &[
    "dijkstral-out/graph.dij",
    "dijkstral-out/graph.json",
    "graphify-out/graph.json",
];

/// The first project graph that exists under `dir`.
pub fn project_graph(dir: &Path) -> Option<PathBuf> {
    PROJECT_GRAPHS
        .iter()
        .map(|p| dir.join(p))
        .find(|p| p.is_file())
}

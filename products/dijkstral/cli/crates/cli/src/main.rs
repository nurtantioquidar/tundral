use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use dijkstral_core::{build, BuildOptions, BuildReport};

// mimalloc keeps fragmentation and the retained high-water mark well below the system allocator's
// on large builds (Kubernetes: measured in bench/README.md).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Dijkstral turns a repository into a queryable knowledge graph for AI coding agents.
#[derive(Parser, Debug)]
#[command(name = "dijkstral", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Build the graph for a directory and write dijkstral-out/graph.json.
    Extract(BuildArgs),
    /// Rebuild incrementally after changes (only changed files are re-extracted).
    Update(BuildArgs),
}

#[derive(Args, Debug)]
struct BuildArgs {
    /// Directory to index.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Index code only (no documents, papers or images). The only mode before the semantic pass.
    #[arg(long)]
    code_only: bool,
    /// Skip clustering. Clustering arrives in stage 2, so this is currently always the case.
    #[arg(long)]
    no_cluster: bool,
    /// Write dijkstral-out/ under this directory instead of the indexed one.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
    /// Worker threads (default: all cores).
    #[arg(long, value_name = "N")]
    threads: Option<usize>,
    /// Print per-phase timings to stderr.
    #[arg(long)]
    timings: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (Command::Extract(args) | Command::Update(args)) = cli.command;
    let threads = args
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    let started = Instant::now();
    let opts = BuildOptions {
        root: args.path,
        out: args.out,
        threads,
    };
    match build(&opts) {
        Ok(report) => {
            print_report(&report, args.timings, started);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn print_report(r: &BuildReport, timings: bool, started: Instant) {
    println!(
        "{} files ({} extracted, {} cached) -> {} nodes, {} edges in {:.2}s\nwrote {}",
        r.files,
        r.extracted,
        r.cached,
        r.nodes,
        r.edges,
        started.elapsed().as_secs_f64(),
        r.out_dir.join("graph.json").display()
    );
    if timings {
        for (name, d) in &r.phases {
            eprintln!("  {name:<12} {:>8.1} ms", d.as_secs_f64() * 1e3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}

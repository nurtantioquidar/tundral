//! `dijkstral mcp`: the MCP server over stdio (`docs/graphify-query.md` §10), implemented in
//! `dijkstral_serve::mcp`.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use dijkstral_serve::mcp;

#[derive(Args, Debug)]
pub struct McpArgs {
    /// Graph to serve: a graph.dij, or any graph.json (Graphify's included). Default: the project
    /// in the current directory (dijkstral-out/graph.dij, else dijkstral-out/graph.json, else
    /// graphify-out/graph.json), re-resolved on every call.
    #[arg(value_name = "GRAPH", conflicts_with = "graph")]
    graph_path: Option<PathBuf>,
    /// Same as the positional GRAPH.
    #[arg(long, value_name = "PATH")]
    graph: Option<PathBuf>,
    /// Log graph loads and per-request timings to stderr.
    #[arg(long)]
    verbose: bool,
}

pub fn run(a: McpArgs) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read the current directory: {e}");
            return ExitCode::FAILURE;
        }
    };
    let opts = mcp::Options {
        graph: a.graph.or(a.graph_path),
        cwd,
        verbose: a.verbose,
        hosted: false,
    };
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    match mcp::serve(opts, stdin, io::BufWriter::new(stdout)) {
        Ok(()) => ExitCode::SUCCESS,
        // A closed stdout means the client went away.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

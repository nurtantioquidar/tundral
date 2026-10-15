//! `diagnose multigraph` (`docs/graphify-memory.md` §6): the edge-collapse report of a graph or
//! extraction JSON. Writes nothing.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use dijkstral_serve::diagnose::{self, Options, Scan, DEFAULT_MAX_EXAMPLES};

use crate::work_cmds::PROJECT_GRAPHS;

#[derive(Args, Debug)]
pub struct DiagnoseArgs {
    #[command(subcommand)]
    what: Diagnostic,
}

#[derive(Subcommand, Debug)]
enum Diagnostic {
    /// How many edges collapse when the JSON is loaded with one edge per endpoint pair: missing,
    /// dangling, self-loop and duplicate edges, same-endpoint groups and examples.
    Multigraph(MultigraphArgs),
}

#[derive(Args, Debug)]
#[command(args_override_self = true)]
struct MultigraphArgs {
    /// Graph or extraction JSON. Default: dijkstral-out/graph.json, else graphify-out/graph.json.
    #[arg(long, value_name = "PATH")]
    graph: Option<PathBuf>,
    /// Machine-readable JSON report.
    #[arg(long)]
    json: bool,
    /// Same-endpoint examples to show.
    #[arg(long, value_name = "N", default_value_t = DEFAULT_MAX_EXAMPLES)]
    max_examples: usize,
    /// Count as a directed graph (default: as the JSON's `directed` flag says; directed when
    /// absent).
    #[arg(long, conflicts_with = "undirected")]
    directed: bool,
    /// Count as an undirected graph.
    #[arg(long)]
    undirected: bool,
    /// Python source to scan for de-duplication sites (`seen_*` sets) that could drop edges
    /// before the graph is written.
    #[arg(long, value_name = "PATH")]
    extract_path: Option<PathBuf>,
}

pub fn run(a: DiagnoseArgs) -> ExitCode {
    let Diagnostic::Multigraph(a) = a.what;
    let graph = a.graph.clone().unwrap_or_else(|| {
        PROJECT_GRAPHS[1..]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from(PROJECT_GRAPHS[1]))
    });
    let opts = Options {
        directed: if a.directed {
            Some(true)
        } else if a.undirected {
            Some(false)
        } else {
            None
        },
        max_examples: a.max_examples,
    };
    let mut report = match diagnose::diagnose_file(&graph, &shown(&graph), &opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    report.scan = match &a.extract_path {
        Some(p) => match diagnose::scan(p, &shown(p)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {e:#}");
                return ExitCode::FAILURE;
            }
        },
        None => Scan::default(),
    };
    if a.json {
        println!("{}", diagnose::render_json(&report));
    } else {
        print!("{}", diagnose::render_text(&report));
    }
    ExitCode::SUCCESS
}

fn shown(p: &Path) -> String {
    p.display().to_string()
}

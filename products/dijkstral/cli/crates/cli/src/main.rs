mod agents_cmd;
mod export_cmd;
mod exports;
mod git_cmd;
mod mcp_cmd;
mod query_cmds;
mod semantic_build;
mod semantic_cmd;
mod watch_cmd;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use dijkstral_core::pipeline::cluster_only_files;
use dijkstral_core::{build_full, BuildOptions, BuildReport};
use dijkstral_semantic::markdown::MarkdownPass;

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
    /// Build the graph for a directory and write dijkstral-out/graph.json. Documents, images and
    /// other non-code files go through a model (the semantic pass) unless --code-only.
    Extract(BuildArgs),
    /// Rebuild incrementally after changes (only changed files are re-extracted), adding the
    /// structure of Markdown pages: headings, links and backtick references to code. Never calls
    /// a model.
    Update(BuildArgs),
    /// Re-cluster the graph and name its communities with a model.
    Label(semantic_build::LabelArgs),
    /// Fetch an http(s) page into raw/ for the next semantic pass.
    Add(semantic_build::AddArgs),
    /// Re-cluster an existing graph.json and rewrite it with GRAPH_REPORT.md and
    /// .graphify_analysis.json (`graphify cluster-only --no-label`: communities named
    /// "Community N", no file statistics).
    ClusterOnly(ClusterOnlyArgs),
    #[command(flatten)]
    Query(query_cmds::QueryCommand),
    /// Serve the graph to AI assistants over MCP (stdio): Graphify's graphify-mcp tools.
    Mcp(mcp_cmd::McpArgs),
    /// Export a graph: interactive HTML, Markdown wiki, GraphML, Cypher (Neo4j, FalkorDB),
    /// Obsidian vault, SVG or Mermaid call-flow HTML.
    Export(export_cmd::ExportArgs),
    /// Write a D3 collapsible-tree HTML page of the graph (files, then their symbols).
    Tree(exports::tree::TreeArgs),
    #[command(flatten)]
    Agents(agents_cmd::AgentCommand),
    /// Semantic pass hand-off: import the host assistant's replies.
    Semantic(semantic_cmd::SemanticArgs),
    /// Report non-code changes the semantic pass has not seen (silent when current).
    CheckUpdate(semantic_cmd::CheckUpdateArgs),
    /// Git hooks that rebuild the graph after commits and branch switches (in the background),
    /// plus the graph.json merge driver: install, uninstall or status.
    Hook(git_cmd::HookArgs),
    /// Git merge driver for graph.json: three-way merge of nodes, links and hyperedges (`hook
    /// install` registers it).
    MergeDriver(git_cmd::MergeDriverArgs),
    /// Watch a directory and run `update` whenever its code or documents change (Ctrl-C stops).
    Watch(watch_cmd::WatchArgs),
}

#[derive(Args, Debug)]
struct ClusterOnlyArgs {
    /// Directory whose dijkstral-out/graph.json is re-clustered.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Graph to re-cluster instead (Graphify's graphify-out/graph.json works too); outputs are
    /// written next to it.
    #[arg(long, value_name = "FILE")]
    graph: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct BuildArgs {
    /// Directory to index.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Index code only: no documents, papers or images, no model.
    #[arg(long)]
    code_only: bool,
    /// Skip clustering, analysis and the report: graph.json gets no `community`, and
    /// GRAPH_REPORT.md and .graphify_analysis.json are not written.
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
    #[command(flatten)]
    exports: export_cmd::BuildExports,
    #[command(flatten)]
    semantic: semantic_build::SemanticFlags,
}

fn main() -> ExitCode {
    if let Some(code) = agents_cmd::early_dispatch() {
        return code;
    }
    let cli = Cli::parse();
    let (args, update) = match cli.command {
        Command::Agents(cmd) => return agents_cmd::run(cmd),
        Command::Extract(args) => (args, false),
        Command::Update(args) => (args, true),
        Command::Label(args) => return semantic_build::run_label(args),
        Command::Add(args) => return semantic_build::run_add(args),
        Command::Query(cmd) => return query_cmds::run(cmd),
        Command::Mcp(args) => return mcp_cmd::run(args),
        Command::Export(args) => return export_cmd::run(args),
        Command::Tree(args) => return exports::tree::run_cmd(args),
        Command::Semantic(args) => return semantic_cmd::run_semantic(args),
        Command::CheckUpdate(args) => return semantic_cmd::run_check_update(args),
        Command::Hook(args) => return git_cmd::run_hook(args),
        Command::MergeDriver(args) => return git_cmd::run_merge_driver(args),
        Command::Watch(args) => return watch_cmd::run(args),
        Command::ClusterOnly(args) => {
            return match cluster_only_files(&args.path, args.graph.as_deref()) {
                Ok((dir, communities)) => {
                    println!("{communities} communities\nwrote {}", dir.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e:#}");
                    ExitCode::FAILURE
                }
            };
        }
    };
    let threads = args
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    let started = Instant::now();
    let exporters = export_cmd::exporters(&args.exports, &args.path);
    let opts = BuildOptions {
        root: args.path,
        out: args.out,
        threads,
        cluster: !args.no_cluster,
    };
    let exporters: Vec<_> = exporters.iter().map(|e| e.as_ref()).collect();
    // `update` adds the markdown structure as `graphify update` does; `extract` sends the
    // non-code files to a model instead (the semantic pass). `--code-only` does neither.
    let markdown = MarkdownPass::new();
    let semantic = if update || args.code_only {
        None
    } else {
        match semantic_build::prepare(&opts.root, opts.out.as_deref(), &args.semantic) {
            Ok(pass) => pass,
            Err(e) => {
                eprintln!("error: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    };
    let documents: Option<&dyn dijkstral_core::Documents> = match &semantic {
        Some(pass) => Some(pass),
        None if update && !args.code_only => Some(&markdown),
        None => None,
    };
    match build_full(&opts, &exporters, documents) {
        Ok(report) => {
            print_report(&report, args.timings, started);
            if semantic.is_none() {
                // A no-model build: flag non-code changes for `check-update`.
                if let Err(e) =
                    dijkstral_semantic::pass::note_pending_changes(&opts.root, &report.out_dir)
                {
                    eprintln!("warning: {e:#}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn print_report(r: &BuildReport, timings: bool, started: Instant) {
    if r.unchanged {
        println!(
            "{} files unchanged; {} is current ({:.2}s)",
            r.files,
            r.out_dir.join("graph.json").display(),
            started.elapsed().as_secs_f64()
        );
        return;
    }
    if let Some(s) = &r.semantic {
        for note in &s.notes {
            println!("{note}");
        }
    }
    let communities = if r.communities > 0 {
        format!(", {} communities", r.communities)
    } else {
        String::new()
    };
    println!(
        "{} files ({} extracted, {} cached) -> {} nodes, {} edges{} in {:.2}s\nwrote {}",
        r.files,
        r.extracted,
        r.cached,
        r.nodes,
        r.edges,
        communities,
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

    #[test]
    fn hook_guard_watches_the_files_builds_write() {
        use dijkstral_core::pipeline;
        assert_eq!(dijkstral_agents::OUT_DIR, pipeline::OUT_DIR);
        assert_eq!(dijkstral_agents::DIGEST_FILE, pipeline::DIGEST_FILE);
    }
}

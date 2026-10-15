//! `save-result` and `reflect` (`docs/graphify-memory.md` §2, §4): Graphify's result files and
//! lessons, under `dijkstral-out/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use clap::{Args, ValueEnum};
use dijkstral_serve::clock::{self, Instant};
use dijkstral_serve::memory::{self, NewResult, Outcome, MEMORY_DIR};
use dijkstral_serve::reflect::{self, LESSONS_FILE};

use crate::work_cmds::project_graph;

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum OutcomeArg {
    #[value(name = "useful")]
    Useful,
    #[value(name = "dead_end")]
    DeadEnd,
    #[value(name = "corrected")]
    Corrected,
}

impl From<OutcomeArg> for Outcome {
    fn from(o: OutcomeArg) -> Outcome {
        match o {
            OutcomeArg::Useful => Outcome::Useful,
            OutcomeArg::DeadEnd => Outcome::DeadEnd,
            OutcomeArg::Corrected => Outcome::Corrected,
        }
    }
}

#[derive(Args, Debug)]
#[command(args_override_self = true)]
pub struct SaveArgs {
    /// The question that was asked.
    #[arg(long)]
    question: String,
    /// The answer to save.
    #[arg(long)]
    answer: Option<String>,
    /// Read the answer from this file instead (surrounding whitespace removed).
    #[arg(long, value_name = "FILE")]
    answer_file: Option<PathBuf>,
    /// What kind of question: query, path_query or explain.
    #[arg(long = "type", value_name = "T", default_value = "query")]
    kind: String,
    /// Labels or ids of the graph nodes the answer cites.
    #[arg(long, value_name = "N", num_args = 0..)]
    nodes: Vec<String>,
    /// How it went: useful, dead_end or corrected.
    #[arg(long, value_enum, value_name = "O")]
    outcome: Option<OutcomeArg>,
    /// What the right answer was (pairs with --outcome corrected).
    #[arg(long, value_name = "TEXT")]
    correction: Option<String>,
    /// Directory to save into.
    #[arg(long, value_name = "DIR", default_value = MEMORY_DIR)]
    memory_dir: PathBuf,
}

pub fn run_save(a: SaveArgs) -> ExitCode {
    let answer = match (&a.answer_file, a.answer.filter(|s| !s.is_empty())) {
        (Some(f), _) => match fs::read(f) {
            Ok(b) => String::from_utf8_lossy(&b).trim().to_string(),
            Err(e) => {
                eprintln!("error: cannot read {}: {e}", f.display());
                return ExitCode::FAILURE;
            }
        },
        (None, Some(text)) => text,
        (None, None) => {
            eprintln!("error: give the answer with --answer or --answer-file");
            return ExitCode::from(2);
        }
    };
    let result = NewResult {
        question: a.question,
        answer,
        kind: a.kind,
        nodes: a.nodes,
        outcome: a.outcome.map(Outcome::from),
        correction: a.correction,
    };
    match memory::save(&a.memory_dir, &result, Instant::now()) {
        Ok(path) => {
            println!("Saved {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Args, Debug)]
#[command(args_override_self = true)]
pub struct ReflectArgs {
    /// Directory of saved results.
    #[arg(long, value_name = "DIR", default_value = MEMORY_DIR)]
    memory_dir: PathBuf,
    /// Where to write the lessons.
    #[arg(long, value_name = "FILE", default_value = LESSONS_FILE)]
    out: PathBuf,
    /// Graph used to drop names that no longer exist and to group lessons by community.
    /// Default: dijkstral-out/graph.dij or graph.json, else graphify-out/graph.json, when present.
    #[arg(long, value_name = "PATH")]
    graph: Option<PathBuf>,
    /// Community assignment (default: .graphify_analysis.json next to the graph).
    #[arg(long, value_name = "PATH")]
    analysis: Option<PathBuf>,
    /// Community names (default: .graphify_labels.json next to the graph).
    #[arg(long, value_name = "PATH")]
    labels: Option<PathBuf>,
    /// A result's weight halves every N days (0 disables the decay).
    #[arg(long, value_name = "N", default_value_t = reflect::DEFAULT_HALF_LIFE_DAYS, allow_hyphen_values = true)]
    half_life_days: f64,
    /// Distinct useful results a node needs before it is recommended.
    #[arg(long, value_name = "N", default_value_t = reflect::DEFAULT_MIN_CORROBORATION, allow_hyphen_values = true)]
    min_corroboration: i64,
    /// Do nothing when the lessons are newer than every input.
    #[arg(long)]
    if_stale: bool,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub fn run_reflect(a: ReflectArgs) -> ExitCode {
    let graph = a
        .graph
        .clone()
        .or_else(|| project_graph(Path::new(".")))
        .filter(|p| p.is_file());
    let side = |explicit: &Option<PathBuf>, name: &str| -> Option<PathBuf> {
        explicit
            .clone()
            .or_else(|| graph.as_ref().map(|g| g.with_file_name(name)))
            .filter(|p| p.is_file())
    };
    let analysis = side(&a.analysis, ".graphify_analysis.json");
    let labels = side(&a.labels, ".graphify_labels.json");

    if a.if_stale {
        if let Some(out_time) = mtime(&a.out) {
            let mut inputs: Vec<PathBuf> = vec![a.memory_dir.clone()];
            if let Ok(entries) = fs::read_dir(&a.memory_dir) {
                inputs.extend(
                    entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x == "md")),
                );
            }
            inputs.extend(graph.iter().cloned());
            inputs.extend(analysis.iter().cloned());
            inputs.extend(labels.iter().cloned());
            if inputs
                .iter()
                .filter_map(|p| mtime(p))
                .all(|t| t <= out_time)
            {
                println!(
                    "{} is up to date; run without --if-stale to rewrite it",
                    a.out.display()
                );
                return ExitCode::SUCCESS;
            }
        }
    }

    let results = memory::load_dir(&a.memory_dir);
    let context = graph
        .as_deref()
        .and_then(|g| reflect::load_context(g, analysis.as_deref(), labels.as_deref()));
    let opts = reflect::Options {
        half_life_days: a.half_life_days,
        min_corroboration: a.min_corroboration,
        now: clock::now(),
    };
    let lessons = reflect::reflect(&results, context.as_ref(), &opts);
    let text = reflect::render(&lessons, &a.memory_dir, &opts);
    if let Some(parent) = a.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Err(e) = fs::create_dir_all(parent) {
            eprintln!("error: creating {}: {e}", parent.display());
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = fs::write(&a.out, text) {
        eprintln!("error: writing {}: {e}", a.out.display());
        return ExitCode::FAILURE;
    }
    let c = lessons.counts;
    println!(
        "{}: {} results ({} useful, {} dead ends, {} corrected, {} unmarked)",
        a.out.display(),
        c.total(),
        c.useful,
        c.dead_ends,
        c.corrected,
        c.unmarked
    );
    ExitCode::SUCCESS
}

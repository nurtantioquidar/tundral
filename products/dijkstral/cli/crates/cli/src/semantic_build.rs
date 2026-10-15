//! The semantic pass on the command line: `extract`'s model flags, `label` and `add <url>`
//! (`docs/graphify-semantic.md` §4, §10, §11), over `dijkstral_semantic::pass`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, ValueEnum};
use dijkstral_semantic::add::{self, AddOptions};
use dijkstral_semantic::pass::{self, PassOptions, SemanticPass, DEFAULT_TOKEN_BUDGET};
use dijkstral_semantic::run::{self, BackendKind, BackendSettings, Env, LabelOptions, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ModeArg {
    Normal,
    Deep,
}

/// Backend selection, shared by `extract` and `label`.
#[derive(Args, Debug, Clone)]
pub struct BackendFlags {
    /// Model backend: codex (default when the Codex CLI is logged in), openai, anthropic,
    /// gemini, kimi, deepseek, replay or handoff. Without it, the first configured one.
    #[arg(long, value_name = "NAME")]
    pub backend: Option<String>,
    /// Model to ask (overrides the backend's default and environment variable).
    #[arg(long, value_name = "NAME")]
    pub model: Option<String>,
    /// Codex reasoning effort (`low`, `medium`, `high`, …).
    #[arg(long, value_name = "EFFORT")]
    pub reasoning_effort: Option<String>,
    /// Seconds before a model request times out.
    #[arg(long, value_name = "SECONDS", default_value_t = 600)]
    pub api_timeout: u64,
    /// Requests in flight at once.
    #[arg(long, value_name = "N", default_value_t = 4)]
    pub max_concurrency: usize,
    /// Transcript directory for `--backend replay`.
    #[arg(long, value_name = "DIR")]
    pub replay_dir: Option<PathBuf>,
}

impl BackendFlags {
    fn settings(&self) -> Result<BackendSettings> {
        Ok(BackendSettings {
            kind: self
                .backend
                .as_deref()
                .map(BackendKind::parse)
                .transpose()?,
            model: self.model.clone(),
            reasoning_effort: self.reasoning_effort.clone(),
            api_timeout: Duration::from_secs(self.api_timeout),
            replay_dir: self.replay_dir.clone(),
        })
    }
}

/// `extract`'s semantic-pass flags (ignored with `--code-only` and by `update`, which never
/// calls a model).
#[derive(Args, Debug, Clone)]
pub struct SemanticFlags {
    #[command(flatten)]
    pub backend: BackendFlags,
    /// `deep`: finer concepts and more inferred links (its own cache).
    #[arg(long, value_enum, default_value_t = ModeArg::Normal)]
    pub mode: ModeArg,
    /// Ignore the semantic cache and the unchanged-build check: ask the model again.
    #[arg(long)]
    pub force: bool,
    /// Estimated input tokens per request.
    #[arg(long, value_name = "N", default_value_t = DEFAULT_TOKEN_BUDGET)]
    pub token_budget: usize,
    /// Accept files whose re-extraction produced fewer nodes than before.
    #[arg(long)]
    pub allow_partial: bool,
}

/// The pass for `root`, or `None` when nothing needs a model.
pub fn prepare(root: &Path, flags: &SemanticFlags) -> Result<Option<SemanticPass>> {
    let opts = PassOptions {
        backend: flags.backend.settings()?,
        mode: match flags.mode {
            ModeArg::Normal => Mode::Normal,
            ModeArg::Deep => Mode::Deep,
        },
        max_concurrency: flags.backend.max_concurrency,
        token_budget: flags.token_budget,
        force: flags.force,
        allow_partial: flags.allow_partial,
        progress: true,
    };
    SemanticPass::prepare(root, opts, &Env::system())
}

#[derive(Args, Debug)]
pub struct LabelArgs {
    /// Directory whose dijkstral-out/graph.json is re-clustered and named.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Graph to label instead; outputs are written next to it.
    #[arg(long, value_name = "FILE")]
    graph: Option<PathBuf>,
    /// Communities per naming request.
    #[arg(long, value_name = "N", default_value_t = 100)]
    batch_size: usize,
    #[command(flatten)]
    backend: BackendFlags,
}

/// `dijkstral label`: re-cluster and name the communities through the backend.
pub fn run_label(a: LabelArgs) -> ExitCode {
    let result = (|| -> Result<()> {
        let backend = run::select(&a.backend.settings()?, &Env::system())?;
        let opts = LabelOptions {
            batch_size: a.batch_size,
            max_concurrency: a.backend.max_concurrency,
            ..LabelOptions::default()
        };
        let (dir, communities, outcome) =
            pass::label(&a.path, a.graph.as_deref(), backend.as_ref(), &opts)?;
        for w in &outcome.warnings {
            eprintln!("[dijkstral label] {w}");
        }
        println!("{}", outcome.summary());
        println!("{}", outcome.usage.summary());
        println!("{communities} communities\nwrote {}", dir.display());
        Ok(())
    })();
    finish(result)
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// http(s) URL to fetch (private and loopback addresses are refused).
    url: String,
    /// Directory to save into.
    #[arg(long, value_name = "DIR", default_value = "raw")]
    dir: PathBuf,
    /// Author of the content (front matter).
    #[arg(long, value_name = "NAME")]
    author: Option<String>,
    /// Who added it to the corpus (front matter).
    #[arg(long, value_name = "NAME")]
    contributor: Option<String>,
}

/// `dijkstral add <url>`.
pub fn run_add(a: AddArgs) -> ExitCode {
    let opts = AddOptions {
        dir: Some(a.dir),
        author: a.author,
        contributor: a.contributor,
        timeout: None,
    };
    let result = add::add(&a.url, &opts).map(|path| {
        println!("saved {}", path.display());
        println!("run `dijkstral extract` to add it to the graph");
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: ingest: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn finish(result: Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

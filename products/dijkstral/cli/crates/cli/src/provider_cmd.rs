//! `dijkstral provider list|show|add|remove`: custom OpenAI-compatible model providers for the
//! semantic pass (`docs/graphify-inputs.md` §6), over `dijkstral_semantic::run::backend::provider`.

use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, Subcommand};
use dijkstral_semantic::run::backend::provider::{Origin, Provider, Registry};

#[derive(Args, Debug)]
pub struct ProviderArgs {
    #[command(subcommand)]
    command: Option<ProviderCommand>,
}

#[derive(Subcommand, Debug)]
enum ProviderCommand {
    /// List the providers: dijkstral's (~/.dijkstral/providers.json), then Graphify's
    /// (~/.graphify/providers.json, read only).
    List,
    /// Print a provider's settings as JSON (the key's variable name, never the key).
    Show { name: String },
    /// Register an OpenAI-compatible endpoint under NAME, for `--backend NAME`.
    Add(AddArgs),
    /// Remove a provider from dijkstral's registry.
    Remove { name: String },
}

#[derive(Args, Debug)]
struct AddArgs {
    /// Letters, digits, '.', '_' and '-'; not a built-in backend name.
    name: String,
    /// Chat Completions base URL (requests go to <URL>/chat/completions).
    #[arg(long, value_name = "URL")]
    base_url: String,
    /// Model used when --model is not given.
    #[arg(long, value_name = "MODEL")]
    default_model: String,
    /// Name of the environment variable that holds the API key (the key itself is never stored).
    #[arg(long, value_name = "VAR")]
    env_key: String,
    /// USD per million input tokens (stored for reference).
    #[arg(long, value_name = "USD", default_value_t = 0.0)]
    pricing_input: f64,
    /// USD per million output tokens (stored for reference).
    #[arg(long, value_name = "USD", default_value_t = 0.0)]
    pricing_output: f64,
}

pub fn run(a: ProviderArgs) -> ExitCode {
    let Some(command) = a.command else {
        println!("usage: dijkstral provider list|show <name>|add <name> …|remove <name>");
        return ExitCode::SUCCESS;
    };
    match execute(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn execute(command: ProviderCommand) -> Result<()> {
    let mut registry = Registry::system()?;
    match command {
        ProviderCommand::List => {
            let all = registry.all();
            if all.is_empty() {
                println!("No custom providers.");
            }
            for p in all {
                let from = match p.origin {
                    Origin::Dijkstral => "",
                    Origin::Graphify => "  [graphify]",
                };
                println!("  {}  ({}){from}", p.name, p.base_url);
            }
        }
        ProviderCommand::Show { name } => {
            let Some(p) = registry.get(&name) else {
                anyhow::bail!("provider '{name}' is not registered");
            };
            let mut doc = serde_json::Map::new();
            doc.insert(p.name.clone(), p.to_json());
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::Value::Object(doc))?
            );
        }
        ProviderCommand::Add(a) => {
            let p = Provider {
                name: a.name,
                base_url: a.base_url,
                default_model: a.default_model,
                env_key: a.env_key,
                pricing: (a.pricing_input, a.pricing_output),
                origin: Origin::Dijkstral,
            };
            if let Some(warning) = registry.add(&p)? {
                eprintln!("{warning}");
            }
            println!(
                "added provider '{}' (key from ${}); use it with `dijkstral extract . --backend {}`",
                p.name, p.env_key, p.name
            );
        }
        ProviderCommand::Remove { name } => {
            registry.remove(&name)?;
            println!("removed provider '{name}'");
        }
    }
    Ok(())
}

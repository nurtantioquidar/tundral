use cicada::{Balance, BalanceChecker, CheckResult, Provider, Quota, Status};
use clap::{Parser, ValueEnum};
use futures_util::future::join_all;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "cicada",
    version,
    about = "Check every model API balance without opening four browser tabs"
)]
struct Cli {
    /// Providers to check; use `all` or omit this argument to check every provider concurrently
    #[arg(value_enum, value_name = "PROVIDER")]
    providers: Vec<Target>,

    /// Emit a stable JSON array
    #[arg(long)]
    json: bool,

    /// Per-provider HTTP timeout in milliseconds
    #[arg(long, default_value_t = 8_000, value_parser = clap::value_parser!(u64).range(1..))]
    timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Target {
    All,
    Openai,
    #[value(alias = "muse", alias = "meta-ai", alias = "meta-model-api")]
    Meta,
    #[value(alias = "z-ai", alias = "glm")]
    Zai,
    Deepseek,
}

impl Target {
    fn provider(self) -> Option<Provider> {
        match self {
            Self::All => None,
            Self::Openai => Some(Provider::Openai),
            Self::Meta => Some(Provider::Meta),
            Self::Zai => Some(Provider::Zai),
            Self::Deepseek => Some(Provider::Deepseek),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let providers = select_targets(cli.providers);
    let checker = match BalanceChecker::new(Duration::from_millis(cli.timeout_ms)) {
        Ok(checker) => checker,
        Err(error) => {
            eprintln!("cicada: {error}");
            return ExitCode::FAILURE;
        }
    };
    let results = join_all(
        providers
            .into_iter()
            .map(|provider| checker.check(provider)),
    )
    .await;

    if cli.json {
        match serde_json::to_string_pretty(&results) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("cicada: could not serialize results: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print_human(&results);
    }

    if results.iter().all(|result| result.status == Status::Ok) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn deduplicate(providers: Vec<Provider>) -> Vec<Provider> {
    let mut selected = Vec::with_capacity(providers.len());
    for provider in providers {
        if !selected.contains(&provider) {
            selected.push(provider);
        }
    }
    selected
}

fn select_targets(targets: Vec<Target>) -> Vec<Provider> {
    if targets.is_empty() || targets.contains(&Target::All) {
        Provider::ALL.to_vec()
    } else {
        deduplicate(targets.into_iter().filter_map(Target::provider).collect())
    }
}

fn print_human(results: &[CheckResult]) {
    let width = results
        .iter()
        .map(|result| result.provider.to_string().len())
        .max()
        .unwrap_or_default();
    for result in results {
        let summary = match result.status {
            Status::Ok => {
                let mut values = result
                    .balances
                    .iter()
                    .map(render_balance)
                    .collect::<Vec<_>>();
                values.extend(result.quotas.iter().map(render_quota));
                values.join(", ")
            }
            Status::Unconfigured => "not configured".into(),
            Status::Unavailable => "unavailable".into(),
            Status::Error => "error".into(),
        };
        println!(
            "{:<width$}  {:<20}  {} ({} ms)",
            result.provider, summary, result.detail, result.elapsed_ms
        );
    }
}

fn render_quota(quota: &Quota) -> String {
    match (&quota.remaining_percent, &quota.remaining) {
        (Some(percent), Some(remaining)) => format!(
            "{}: {percent}% left ({remaining} {} left)",
            quota.label, quota.unit
        ),
        (Some(percent), None) => format!("{}: {percent}% left", quota.label),
        (None, Some(remaining)) => {
            format!("{}: {remaining} {} left", quota.label, quota.unit)
        }
        (None, None) => quota.label.clone(),
    }
}

fn render_balance(balance: &Balance) -> String {
    let amount = match balance.currency.as_str() {
        "USD" => format!("${}", balance.total),
        "CNY" => format!("¥{}", balance.total),
        _ => format!("{} {}", balance.total, balance.currency),
    };
    let mut components = Vec::new();
    if let Some(granted) = &balance.granted {
        components.push(format!("grant {granted}"));
    }
    if let Some(topped_up) = &balance.topped_up {
        components.push(format!("top-up {topped_up}"));
    }
    if components.is_empty() {
        amount
    } else {
        format!("{amount} [{}]", components.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_arguments_are_deduplicated_in_order() {
        assert_eq!(
            deduplicate(vec![Provider::Meta, Provider::Meta, Provider::Deepseek]),
            vec![Provider::Meta, Provider::Deepseek]
        );
    }

    #[test]
    fn all_is_explicit_or_implicit() {
        assert_eq!(select_targets(Vec::new()), Provider::ALL);
        assert_eq!(
            select_targets(vec![Target::Deepseek, Target::All]),
            Provider::ALL
        );
    }

    #[test]
    fn provider_aliases_parse() {
        let cli = Cli::try_parse_from(["cicada", "muse", "z-ai"]).unwrap();
        assert_eq!(cli.providers, vec![Target::Meta, Target::Zai]);
    }

    #[test]
    fn display_includes_deepseek_components() {
        assert_eq!(
            render_balance(&Balance {
                currency: "USD".into(),
                total: "12.5".into(),
                granted: Some("2.5".into()),
                topped_up: Some("10".into()),
            }),
            "$12.5 [grant 2.5, top-up 10]"
        );
    }

    #[test]
    fn display_includes_zai_quota() {
        assert_eq!(
            render_quota(&Quota {
                label: "weekly credits".into(),
                remaining_percent: Some("80".into()),
                remaining: Some("7994".into()),
                unit: "credits".into(),
                resets_at_unix_ms: Some(1_787_210_351_998),
            }),
            "weekly credits: 80% left (7994 credits left)"
        );
    }
}

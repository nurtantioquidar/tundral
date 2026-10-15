//! Model backends and their selection.
//!
//! | Name | Answers through | Configured by |
//! |---|---|---|
//! | `codex` | `codex exec` on the user's Codex login (default when installed and logged in) | `--model`, `--reasoning-effort` |
//! | `gemini` | Gemini `generateContent` | `GEMINI_API_KEY` / `GOOGLE_API_KEY`, `GEMINI_MODEL`, `GEMINI_BASE_URL` |
//! | `kimi` | Moonshot, OpenAI-compatible | `MOONSHOT_API_KEY`, `MOONSHOT_MODEL`, `MOONSHOT_BASE_URL` |
//! | `anthropic` (`claude`) | Anthropic Messages | `ANTHROPIC_API_KEY`, `ANTHROPIC_MODEL`, `ANTHROPIC_BASE_URL` |
//! | `openai` | any OpenAI-compatible Chat Completions server (OpenAI, Ollama, z.ai, Inver, the bench shim) | `OPENAI_API_KEY`, `OPENAI_MODEL`, `OPENAI_BASE_URL` |
//! | `deepseek` | DeepSeek, OpenAI-compatible | `DEEPSEEK_API_KEY`, `DEEPSEEK_MODEL`, `DEEPSEEK_BASE_URL` |
//! | `replay` | a recorded transcript directory (tests, benchmarks) | `--replay-dir` |
//! | `handoff` (`skill`) | nobody: requests are written for the host assistant (§ `handoff`) | — |
//!
//! Without `--backend`: Codex when it is installed and logged in, else the first API key set in
//! Graphify's order (Gemini, Kimi, Anthropic, OpenAI, DeepSeek), else an error that says how to
//! enable the pass or use the hand-off. `openai`, `kimi` and `gemini` ship no default model (their
//! ids change too often): they need `--model` or their model variable. Keys are read only to authenticate the request they
//! belong to; they are never logged, stored or put in an error.

pub mod codex;
pub mod handoff;
pub mod http;
pub mod provider;
pub mod record;
pub mod replay;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};

use crate::types::Backend;

/// Default `--api-timeout` (§4.4).
pub const DEFAULT_API_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Codex,
    Gemini,
    Kimi,
    Anthropic,
    OpenAi,
    DeepSeek,
    Replay,
    Handoff,
}

impl BackendKind {
    pub fn parse(name: &str) -> Result<BackendKind> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "codex" => BackendKind::Codex,
            "gemini" | "google" => BackendKind::Gemini,
            "kimi" | "moonshot" => BackendKind::Kimi,
            "anthropic" | "claude" => BackendKind::Anthropic,
            "openai" => BackendKind::OpenAi,
            "deepseek" => BackendKind::DeepSeek,
            "replay" => BackendKind::Replay,
            "handoff" | "skill" => BackendKind::Handoff,
            other => bail!(
                "unknown backend '{other}' (codex, gemini, kimi, anthropic, openai, deepseek, \
                 replay, handoff)"
            ),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            BackendKind::Codex => "codex",
            BackendKind::Gemini => "gemini",
            BackendKind::Kimi => "kimi",
            BackendKind::Anthropic => "anthropic",
            BackendKind::OpenAi => "openai",
            BackendKind::DeepSeek => "deepseek",
            BackendKind::Replay => "replay",
            BackendKind::Handoff => "handoff",
        }
    }
}

/// What the command line says about the backend.
#[derive(Debug, Clone)]
pub struct BackendSettings {
    /// `--backend`; `None` selects automatically.
    pub kind: Option<BackendKind>,
    /// `--model` (overrides the backend's environment variable and default).
    pub model: Option<String>,
    /// `--reasoning-effort` (Codex).
    pub reasoning_effort: Option<String>,
    /// `--api-timeout`.
    pub api_timeout: Duration,
    /// Transcript directory for `replay`.
    pub replay_dir: Option<PathBuf>,
    /// `--backend <name>` naming a custom provider (`provider add`) rather than a built-in.
    pub provider: Option<String>,
}

impl Default for BackendSettings {
    fn default() -> Self {
        BackendSettings {
            kind: None,
            model: None,
            reasoning_effort: None,
            api_timeout: DEFAULT_API_TIMEOUT,
            replay_dir: None,
            provider: None,
        }
    }
}

type VarFn = dyn Fn(&str) -> Option<String> + Send + Sync;

/// Where selection looks: environment variables and the Codex CLI. Injected so tests never see
/// the real environment.
pub struct Env {
    vars: Box<VarFn>,
    codex: Box<dyn Fn() -> CodexStatus + Send + Sync>,
    providers: Box<dyn Fn() -> Result<Vec<provider::Provider>> + Send + Sync>,
}

/// The Codex CLI as selection sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexStatus {
    Missing,
    LoggedOut(PathBuf),
    Ready(PathBuf),
}

impl Env {
    /// The process environment and the `codex` on `PATH` (or `DIJKSTRAL_CODEX`).
    pub fn system() -> Env {
        Env {
            vars: Box::new(|k| std::env::var(k).ok().filter(|v| !v.is_empty())),
            codex: Box::new(|| match codex::find_binary(&|k| std::env::var(k).ok()) {
                None => CodexStatus::Missing,
                Some(bin) if codex::logged_in(&bin) => CodexStatus::Ready(bin),
                Some(bin) => CodexStatus::LoggedOut(bin),
            }),
            providers: Box::new(|| Ok(provider::Registry::system()?.all())),
        }
    }

    /// Fixed variables and Codex status (tests).
    pub fn fake(vars: &[(&str, &str)], codex: CodexStatus) -> Env {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Env {
            vars: Box::new(move |k| {
                vars.iter()
                    .find(|(name, _)| name == k)
                    .map(|(_, v)| v.clone())
            }),
            codex: Box::new(move || codex.clone()),
            providers: Box::new(|| Ok(Vec::new())),
        }
    }

    /// The same, with these custom providers registered (tests).
    pub fn with_providers(mut self, providers: Vec<provider::Provider>) -> Env {
        self.providers = Box::new(move || Ok(providers.clone()));
        self
    }

    /// The registered custom providers.
    pub fn providers(&self) -> Result<Vec<provider::Provider>> {
        (self.providers)()
    }

    pub fn var(&self, name: &str) -> Option<String> {
        (self.vars)(name)
    }

    pub fn codex(&self) -> CodexStatus {
        (self.codex)()
    }
}

/// The automatic choice (no `--backend`), or `None` when nothing is available.
pub fn auto_kind(env: &Env) -> Option<BackendKind> {
    if matches!(env.codex(), CodexStatus::Ready(_)) {
        return Some(BackendKind::Codex);
    }
    let has = |k: &str| env.var(k).is_some();
    if has("GEMINI_API_KEY") || has("GOOGLE_API_KEY") {
        Some(BackendKind::Gemini)
    } else if has("MOONSHOT_API_KEY") {
        Some(BackendKind::Kimi)
    } else if has("ANTHROPIC_API_KEY") {
        Some(BackendKind::Anthropic)
    } else if has("OPENAI_API_KEY") {
        Some(BackendKind::OpenAi)
    } else if has("DEEPSEEK_API_KEY") {
        Some(BackendKind::DeepSeek)
    } else {
        None
    }
}

/// How to enable the semantic pass when no backend is available.
pub const NO_BACKEND: &str = "\
no model backend available for the semantic pass. Either:
  - install the Codex CLI and run `codex login` (dijkstral then uses it by default, no API key);
  - set one of GEMINI_API_KEY or GOOGLE_API_KEY (gemini), MOONSHOT_API_KEY (kimi),
    ANTHROPIC_API_KEY (anthropic), OPENAI_API_KEY (openai; OPENAI_BASE_URL for any compatible
    server), DEEPSEEK_API_KEY (deepseek), or pass --backend;
  - pass --backend handoff to write the requests to dijkstral-out/semantic/pending/ for your AI
    assistant (the /dijkstral skill) and load its answers with `dijkstral semantic import`;
  - or pass --code-only to skip documents, papers and images.";

/// The backend `settings` and `env` select.
pub fn select(settings: &BackendSettings, env: &Env) -> Result<Box<dyn Backend>> {
    if let Some(name) = &settings.provider {
        let providers = env.providers()?;
        let Some(p) = providers.iter().find(|p| &p.name == name) else {
            let mut known: Vec<&str> = vec![
                "codex",
                "openai",
                "anthropic",
                "gemini",
                "kimi",
                "deepseek",
                "replay",
                "handoff",
            ];
            known.extend(providers.iter().map(|p| p.name.as_str()));
            bail!("unknown backend '{name}' (known: {})", known.join(", "));
        };
        return custom(p, settings, env);
    }
    let kind = match settings.kind {
        Some(k) => k,
        None => match auto_kind(env) {
            Some(k) => k,
            None => {
                // Then the first custom provider whose key is set (Graphify's order: rule R4).
                if let Some(p) = env
                    .providers()?
                    .iter()
                    .find(|p| !p.env_key.is_empty() && env.var(&p.env_key).is_some())
                {
                    return custom(p, settings, env);
                }
                bail!("{NO_BACKEND}")
            }
        },
    };
    let model = |var: &str, default: &str| {
        settings
            .model
            .clone()
            .or_else(|| env.var(var))
            .unwrap_or_else(|| default.to_string())
    };
    // Backends whose model ids change too often to ship a default: the user names one.
    let required_model = |var: &str, backend: &str| -> Result<String> {
        settings
            .model
            .clone()
            .or_else(|| env.var(var))
            .ok_or_else(|| {
                anyhow::anyhow!("--backend {backend} needs a model: pass --model or set {var}")
            })
    };
    let timeout = settings.api_timeout;
    Ok(match kind {
        BackendKind::Codex => {
            let bin = match env.codex() {
                CodexStatus::Ready(bin) => bin,
                CodexStatus::LoggedOut(_) => {
                    bail!("the Codex CLI is not logged in: run `codex login` first")
                }
                CodexStatus::Missing => {
                    bail!("the Codex CLI is not installed (or set DIJKSTRAL_CODEX to its path)")
                }
            };
            Box::new(codex::Codex::new(
                bin,
                settings
                    .model
                    .clone()
                    .unwrap_or_else(|| codex::DEFAULT_MODEL.into()),
                settings
                    .reasoning_effort
                    .clone()
                    .unwrap_or_else(|| codex::DEFAULT_EFFORT.into()),
                timeout,
            ))
        }
        BackendKind::OpenAi => {
            let base = env.var("OPENAI_BASE_URL");
            let key = env.var("OPENAI_API_KEY");
            if key.is_none() && base.is_none() {
                bail!(
                    "--backend openai needs OPENAI_API_KEY (or OPENAI_BASE_URL for a local server)"
                );
            }
            Box::new(http::OpenAi::new(
                "openai",
                base.unwrap_or_else(|| "https://api.openai.com/v1".into()),
                key,
                required_model("OPENAI_MODEL", "openai")?,
                http::TokenField::MaxCompletionTokens,
                None,
                timeout,
            ))
        }
        BackendKind::DeepSeek => Box::new(http::OpenAi::new(
            "deepseek",
            env.var("DEEPSEEK_BASE_URL")
                .unwrap_or_else(|| "https://api.deepseek.com".into()),
            Some(require(env, &["DEEPSEEK_API_KEY"], "deepseek")?),
            model("DEEPSEEK_MODEL", "deepseek-v4-flash"),
            http::TokenField::MaxTokens,
            Some(8192),
            timeout,
        )),
        BackendKind::Kimi => Box::new(http::OpenAi::new(
            "kimi",
            env.var("MOONSHOT_BASE_URL")
                .unwrap_or_else(|| "https://api.moonshot.ai/v1".into()),
            Some(require(env, &["MOONSHOT_API_KEY"], "kimi")?),
            required_model("MOONSHOT_MODEL", "kimi")?,
            http::TokenField::MaxTokens,
            None,
            timeout,
        )),
        BackendKind::Anthropic => Box::new(http::Anthropic::new(
            env.var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| "https://api.anthropic.com".into()),
            require(env, &["ANTHROPIC_API_KEY"], "anthropic")?,
            model("ANTHROPIC_MODEL", "claude-opus-5-5"),
            timeout,
        )),
        BackendKind::Gemini => Box::new(http::Gemini::new(
            env.var("GEMINI_BASE_URL")
                .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".into()),
            require(env, &["GEMINI_API_KEY", "GOOGLE_API_KEY"], "gemini")?,
            required_model("GEMINI_MODEL", "gemini")?,
            timeout,
        )),
        BackendKind::Replay => {
            let Some(dir) = &settings.replay_dir else {
                bail!("--backend replay needs a transcript directory");
            };
            Box::new(replay::Replay::open(dir)?)
        }
        BackendKind::Handoff => Box::new(handoff::Handoff),
    })
}

/// A custom provider: OpenAI Chat Completions at its `base_url`, keyed by its `env_key`.
fn custom(
    p: &provider::Provider,
    settings: &BackendSettings,
    env: &Env,
) -> Result<Box<dyn Backend>> {
    if p.env_key.is_empty() {
        bail!(
            "provider '{}' names no env_key; register it again with --env-key",
            p.name
        );
    }
    let Some(key) = env.var(&p.env_key) else {
        bail!(
            "--backend {} needs {} (the variable its key is read from)",
            p.name,
            p.env_key
        );
    };
    if let Some(host) = provider::check_base_url(&p.base_url)? {
        eprintln!(
            "[dijkstral] {}",
            provider::plaintext_warning(&p.name, &host)
        );
    }
    let model = settings
        .model
        .clone()
        .or_else(|| (!p.default_model.is_empty()).then(|| p.default_model.clone()))
        .ok_or_else(|| anyhow::anyhow!("--backend {} needs --model", p.name))?;
    Ok(Box::new(http::OpenAi::new(
        p.name.clone(),
        p.base_url.clone(),
        Some(key),
        model,
        http::TokenField::MaxCompletionTokens,
        None,
        settings.api_timeout,
    )))
}

fn require(env: &Env, vars: &[&str], backend: &str) -> Result<String> {
    for v in vars {
        if let Some(k) = env.var(v) {
            return Ok(k);
        }
    }
    bail!("--backend {backend} needs {}", vars.join(" or "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(vars: &[(&str, &str)], codex: CodexStatus) -> Option<BackendKind> {
        auto_kind(&Env::fake(vars, codex))
    }

    #[test]
    fn automatic_selection_order() {
        let ready = CodexStatus::Ready("/bin/codex".into());
        let out = CodexStatus::LoggedOut("/bin/codex".into());
        let all = [
            ("OPENAI_API_KEY", "x"),
            ("DEEPSEEK_API_KEY", "x"),
            ("ANTHROPIC_API_KEY", "x"),
            ("GOOGLE_API_KEY", "x"),
            ("MOONSHOT_API_KEY", "x"),
        ];
        assert_eq!(kind(&all, ready), Some(BackendKind::Codex));
        assert_eq!(kind(&all, out.clone()), Some(BackendKind::Gemini));
        assert_eq!(
            kind(&all[..3], CodexStatus::Missing),
            Some(BackendKind::Anthropic)
        );
        assert_eq!(
            kind(&all[..2], CodexStatus::Missing),
            Some(BackendKind::OpenAi)
        );
        assert_eq!(kind(&all[1..2], out), Some(BackendKind::DeepSeek));
        assert_eq!(kind(&[], CodexStatus::Missing), None);
    }

    #[test]
    fn selection_builds_backends_and_explains_failures() {
        let s = BackendSettings::default();
        let e = select(&s, &Env::fake(&[], CodexStatus::Missing))
            .err()
            .unwrap()
            .to_string();
        assert!(
            e.contains("codex login") && e.contains("--backend handoff"),
            "{e}"
        );

        let b = select(&s, &Env::fake(&[], CodexStatus::Ready("/x/codex".into()))).unwrap();
        assert_eq!((b.name(), b.model()), ("codex", codex::DEFAULT_MODEL));
        let s2 = BackendSettings {
            model: Some("m".into()),
            ..BackendSettings::default()
        };
        let b = select(&s2, &Env::fake(&[], CodexStatus::Ready("/x/codex".into()))).unwrap();
        assert_eq!(b.model(), "m");

        let env = Env::fake(
            &[("DEEPSEEK_API_KEY", "k"), ("OPENAI_MODEL", "local")],
            CodexStatus::LoggedOut("/x".into()),
        );
        let b = select(&s, &env).unwrap();
        assert_eq!((b.name(), b.model()), ("deepseek", "deepseek-v4-flash"));
        let explicit = |k| BackendSettings {
            kind: Some(k),
            ..BackendSettings::default()
        };
        let e = select(&explicit(BackendKind::Codex), &env)
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("codex login"), "{e}");
        assert!(select(&explicit(BackendKind::Anthropic), &env).is_err());
        assert!(select(&explicit(BackendKind::OpenAi), &env).is_err());
        let local = Env::fake(
            &[
                ("OPENAI_BASE_URL", "http://127.0.0.1:1/v1"),
                ("OPENAI_MODEL", "llama"),
            ],
            CodexStatus::Missing,
        );
        let b = select(&explicit(BackendKind::OpenAi), &local).unwrap();
        assert_eq!((b.name(), b.model()), ("openai", "llama"));
        let b = select(&explicit(BackendKind::Handoff), &local).unwrap();
        assert_eq!(b.name(), "handoff");
        let gemini = Env::fake(&[("GEMINI_API_KEY", "k")], CodexStatus::Missing);
        let e = select(&s, &gemini).err().unwrap().to_string();
        assert!(e.contains("--model") && e.contains("GEMINI_MODEL"), "{e}");
        assert_eq!(select(&s2, &gemini).unwrap().model(), "m");
        assert_eq!(BackendKind::parse("skill").unwrap(), BackendKind::Handoff);
        assert_eq!(
            BackendKind::parse("claude").unwrap(),
            BackendKind::Anthropic
        );
        assert!(BackendKind::parse("nope").is_err());
    }

    /// `docs/graphify-inputs.md` §6.3: `--backend <provider>`, and providers after the built-in
    /// keys in automatic selection.
    #[test]
    fn custom_providers() {
        let p = |name: &str, key: &str| provider::Provider {
            name: name.into(),
            base_url: "http://127.0.0.1:9/v1".into(),
            default_model: format!("{name}-model"),
            env_key: key.into(),
            pricing: (0.0, 0.0),
            origin: provider::Origin::Dijkstral,
        };
        let providers = vec![p("first", "FIRST_KEY"), p("second", "SECOND_KEY")];
        let by_name = |name: &str| BackendSettings {
            provider: Some(name.into()),
            ..BackendSettings::default()
        };
        let env = Env::fake(&[("SECOND_KEY", "k")], CodexStatus::Missing)
            .with_providers(providers.clone());
        let b = select(&by_name("second"), &env).unwrap();
        assert_eq!((b.name(), b.model()), ("second", "second-model"));
        let e = select(&by_name("first"), &env).err().unwrap().to_string();
        assert!(e.contains("FIRST_KEY"), "{e}");
        let e = select(&by_name("nope"), &env).err().unwrap().to_string();
        assert!(e.contains("first") && e.contains("second"), "{e}");
        let model = BackendSettings {
            model: Some("m2".into()),
            ..by_name("second")
        };
        assert_eq!(select(&model, &env).unwrap().model(), "m2");
        // Automatic: the first provider whose key is set, after the built-in keys.
        let b = select(&BackendSettings::default(), &env).unwrap();
        assert_eq!(b.name(), "second");
        let env = Env::fake(
            &[("SECOND_KEY", "k"), ("DEEPSEEK_API_KEY", "d")],
            CodexStatus::Missing,
        )
        .with_providers(providers);
        assert_eq!(
            select(&BackendSettings::default(), &env).unwrap().name(),
            "deepseek"
        );
    }
}

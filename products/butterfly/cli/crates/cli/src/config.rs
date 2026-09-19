//! Layered configuration: defaults ← ~/.config/butterfly/config.toml ← <repo>/.butterfly.toml
//! ← command-line flags. Every field is optional at every layer; later layers
//! win. `[[providers]]` and `[[models]]` tables accumulate across layers (repo
//! entries apply after global ones, so a repo can override a global model by
//! id). Credentials are NOT config — keys live in the environment or the
//! credential store.

use std::path::Path;

use butterfly_core::model::{ModelConfig, ProviderConfig};
use serde::Deserialize;

/// `[triage]`: the optional System One second opinion on failures the
/// rule-based classifier could not name. Off unless `enabled = true` AND a key
/// resolves (`TYPESAFE_API_KEY`, else the `typesafe` credential-store entry).
/// Still not a credential home — only the knobs live here.
#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TriageFileConfig {
    pub enabled: Option<bool>,
    /// Endpoint override; defaults to TypeSafe's hosted System One.
    pub endpoint: Option<String>,
    /// System One model id (default `jev-latest`).
    pub model: Option<String>,
    /// Answers below this confidence are discarded (default 0.9).
    pub min_confidence: Option<f64>,
    /// Per-call budget; one request, no retries (default 5).
    pub timeout_seconds: Option<u64>,
    /// Provider calls between progress checks (default 25); 0 disables the
    /// progress check while leaving failure triage on.
    pub progress_every: Option<u32>,
}

impl TriageFileConfig {
    fn overlay(self, over: TriageFileConfig) -> TriageFileConfig {
        TriageFileConfig {
            enabled: over.enabled.or(self.enabled),
            endpoint: over.endpoint.or(self.endpoint),
            model: over.model.or(self.model),
            min_confidence: over.min_confidence.or(self.min_confidence),
            timeout_seconds: over.timeout_seconds.or(self.timeout_seconds),
            progress_every: over.progress_every.or(self.progress_every),
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    /// Model id from the registry (built-in or `[[models]]`).
    pub model: Option<String>,
    /// Reasoning effort in the model's vocabulary (e.g. high | max).
    pub effort: Option<String>,
    /// ask | auto | read-only
    pub mode: Option<String>,
    /// Decimal USD session cap, e.g. "0.50".
    pub max_cost_usd: Option<String>,
    /// Endpoint override for the selected model's provider.
    pub base_url: Option<String>,
    pub quiet_reasoning: Option<bool>,
    /// Context window override in tokens (default: the model's real window).
    pub window_tokens: Option<u64>,
    /// Enable the Atlas code-graph tools (default true).
    pub atlas: Option<bool>,
    #[serde(default)]
    pub triage: TriageFileConfig,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

impl FileConfig {
    fn overlay(self, over: FileConfig) -> FileConfig {
        let mut providers = self.providers;
        providers.extend(over.providers);
        let mut models = self.models;
        models.extend(over.models);
        FileConfig {
            model: over.model.or(self.model),
            effort: over.effort.or(self.effort),
            mode: over.mode.or(self.mode),
            max_cost_usd: over.max_cost_usd.or(self.max_cost_usd),
            base_url: over.base_url.or(self.base_url),
            quiet_reasoning: over.quiet_reasoning.or(self.quiet_reasoning),
            window_tokens: over.window_tokens.or(self.window_tokens),
            atlas: over.atlas.or(self.atlas),
            triage: self.triage.overlay(over.triage),
            providers,
            models,
        }
    }
}

fn load_file(path: &Path) -> anyhow::Result<FileConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map_err(|error| anyhow::anyhow!("invalid config {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
        Err(error) => Err(anyhow::anyhow!("cannot read {}: {error}", path.display())),
    }
}

/// Global config path: ~/.config/butterfly/config.toml.
pub fn global_config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("butterfly/config.toml"))
}

/// Load and merge global + repo-local config (repo wins).
pub fn load(repo_root: &Path) -> anyhow::Result<FileConfig> {
    let mut merged = FileConfig::default();
    if let Some(global) = global_config_path() {
        merged = merged.overlay(load_file(&global)?);
    }
    merged = merged.overlay(load_file(&repo_root.join(".butterfly.toml"))?);
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_config_overlays_global_fields_and_accumulates_tables() {
        let base = toml::from_str::<FileConfig>(
            "model = \"deepseek-v4-pro\"\nmax_cost_usd = \"1\"\nquiet_reasoning = true\n\n[[models]]\nid = \"deepseek-v4-flash\"\ncontext_tokens = 131072\n",
        )
        .unwrap();
        let repo = toml::from_str::<FileConfig>(
            "model = \"deepseek-v4-flash\"\n\n[[models]]\nid = \"deepseek-v4-flash\"\ndefault_effort = \"max\"\n",
        )
        .unwrap();
        let merged = base.overlay(repo);
        assert_eq!(merged.model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(merged.max_cost_usd.as_deref(), Some("1"));
        assert_eq!(merged.quiet_reasoning, Some(true));
        // Global entry first, repo entry second: applied in that order.
        assert_eq!(merged.models.len(), 2);
        assert_eq!(merged.models[1].default_effort.as_deref(), Some("max"));
    }

    #[test]
    fn triage_is_absent_by_default_and_overlays_field_by_field() {
        let empty = toml::from_str::<FileConfig>("model = \"deepseek-v4-pro\"\n").unwrap();
        assert_eq!(empty.triage, TriageFileConfig::default());
        assert_eq!(empty.triage.enabled, None);

        let base = toml::from_str::<FileConfig>(
            "[triage]\nenabled = true\nmin_confidence = 0.95\ntimeout_seconds = 3\n",
        )
        .unwrap();
        let repo = toml::from_str::<FileConfig>("[triage]\nmin_confidence = 0.8\n").unwrap();
        let merged = base.overlay(repo);
        // The repo narrows one knob and inherits the rest.
        assert_eq!(merged.triage.enabled, Some(true));
        assert_eq!(merged.triage.min_confidence, Some(0.8));
        assert_eq!(merged.triage.timeout_seconds, Some(3));
    }

    #[test]
    fn a_key_in_the_triage_table_is_rejected_it_is_not_a_credential_home() {
        assert!(toml::from_str::<FileConfig>("[triage]\napi_key = \"sk-nope\"\n").is_err());
    }

    #[test]
    fn missing_files_are_fine_but_junk_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_file(&dir.path().join("absent.toml")).is_ok());
        std::fs::write(dir.path().join(".butterfly.toml"), "modle = \"typo\"").unwrap();
        assert!(load_file(&dir.path().join(".butterfly.toml")).is_err());
        std::fs::write(
            dir.path().join("bad-model.toml"),
            "[[models]]\nid = \"x\"\nbogus = 1\n",
        )
        .unwrap();
        assert!(load_file(&dir.path().join("bad-model.toml")).is_err());
    }
}

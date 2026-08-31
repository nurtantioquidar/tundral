//! Layered configuration: defaults ← ~/.config/butterfly/config.toml ← <repo>/.butterfly.toml
//! ← command-line flags. Every field is optional at every layer; later layers
//! win. Credentials are NOT config — the API key stays in the environment.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    /// deepseek-v4-flash | deepseek-v4-pro
    pub model: Option<String>,
    /// high | max
    pub effort: Option<String>,
    /// ask | auto | read-only
    pub mode: Option<String>,
    /// Decimal USD session cap, e.g. "0.50".
    pub max_cost_usd: Option<String>,
    pub base_url: Option<String>,
    pub quiet_reasoning: Option<bool>,
    /// Context window override in tokens (default: the model's real window).
    pub window_tokens: Option<u64>,
}

impl FileConfig {
    fn overlay(self, over: FileConfig) -> FileConfig {
        FileConfig {
            model: over.model.or(self.model),
            effort: over.effort.or(self.effort),
            mode: over.mode.or(self.mode),
            max_cost_usd: over.max_cost_usd.or(self.max_cost_usd),
            base_url: over.base_url.or(self.base_url),
            quiet_reasoning: over.quiet_reasoning.or(self.quiet_reasoning),
            window_tokens: over.window_tokens.or(self.window_tokens),
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
    fn repo_config_overlays_global_fields() {
        let base = toml::from_str::<FileConfig>(
            "model = \"deepseek-v4-pro\"\nmax_cost_usd = \"1\"\nquiet_reasoning = true\n",
        )
        .unwrap();
        let repo = toml::from_str::<FileConfig>("model = \"deepseek-v4-flash\"\n").unwrap();
        let merged = base.overlay(repo);
        assert_eq!(merged.model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(merged.max_cost_usd.as_deref(), Some("1"));
        assert_eq!(merged.quiet_reasoning, Some(true));
    }

    #[test]
    fn missing_files_are_fine_but_junk_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_file(&dir.path().join("absent.toml")).is_ok());
        std::fs::write(dir.path().join(".butterfly.toml"), "modle = \"typo\"").unwrap();
        assert!(load_file(&dir.path().join(".butterfly.toml")).is_err());
    }
}

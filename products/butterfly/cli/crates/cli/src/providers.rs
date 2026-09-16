//! Model resolution and provider construction for the CLI: the registry
//! (built-ins + config), API-key lookup (environment first, then the
//! credential store), and the `Provider` instance for a given model. Session
//! code never sees vendor names; it receives a descriptor and a provider.

use std::sync::Arc;

use anyhow::{bail, Result};
use butterfly_core::model::{ModelDescriptor, ProviderDescriptor, Registry};
use butterfly_core::provider::openai_compat::OpenAiCompatProvider;
use butterfly_core::provider::Provider;

use crate::config::FileConfig;
use crate::credentials;

pub struct ModelHub {
    pub registry: Registry,
    /// `--base-url` flag: applies to whichever provider the session uses.
    base_url_override: Option<String>,
}

/// Where a key came from, for user-facing notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Environment,
    Store,
}

impl ModelHub {
    pub fn from_config(file: &FileConfig, base_url_override: Option<String>) -> Result<Self> {
        let mut registry = Registry::builtin();
        for provider in &file.providers {
            registry.apply_provider(provider)?;
        }
        for model in &file.models {
            registry.apply_model(model)?;
        }
        Ok(Self {
            registry,
            base_url_override,
        })
    }

    pub fn resolve(&self, model_id: &str) -> Result<Arc<ModelDescriptor>> {
        match self.registry.resolve(model_id) {
            Some(model) => Ok(model),
            None => bail!(
                "unknown model: {model_id} (known: {}; add [[models]] entries in config for others)",
                self.registry.model_ids().join(" | ")
            ),
        }
    }

    pub fn provider_descriptor(&self, model: &ModelDescriptor) -> Result<&ProviderDescriptor> {
        self.registry.provider_for(model).ok_or_else(|| {
            anyhow::anyhow!(
                "model {} names unknown provider {}",
                model.id,
                model.provider
            )
        })
    }

    /// Environment beats the credential store, always. Providers that may run
    /// unauthenticated (a loopback router) resolve to an empty key.
    pub fn resolve_key(provider: &ProviderDescriptor) -> Option<(String, KeySource)> {
        match std::env::var(&provider.api_key_env) {
            Ok(key) if !key.trim().is_empty() => Some((key, KeySource::Environment)),
            _ => credentials::load_key(&provider.id)
                .map(|key| (key, KeySource::Store))
                .or_else(|| {
                    provider
                        .api_key_optional
                        .then(|| (String::new(), KeySource::Environment))
                }),
        }
    }

    /// Build the provider for `model`, or explain exactly which key is missing.
    pub fn build_provider(&self, model: &ModelDescriptor) -> Result<Arc<dyn Provider>> {
        let descriptor = self.provider_descriptor(model)?;
        let Some((api_key, _)) = Self::resolve_key(descriptor) else {
            bail!(
                "no API key for provider '{}' (model {}): export {} or run /connect",
                descriptor.id,
                model.id,
                descriptor.api_key_env
            );
        };
        self.build_provider_with_key(model, api_key)
    }

    pub fn build_provider_with_key(
        &self,
        model: &ModelDescriptor,
        api_key: String,
    ) -> Result<Arc<dyn Provider>> {
        let descriptor = self.provider_descriptor(model)?;
        let provider = OpenAiCompatProvider::from_descriptor(
            descriptor,
            api_key,
            self.base_url_override.clone(),
        )
        .map_err(|error| anyhow::anyhow!(error))?;
        Ok(Arc::new(provider))
    }

    /// `(provider id, display label, already has a key)` for connect
    /// pickers. The flag is what stops a user from silently overwriting a
    /// key they already saved.
    pub fn connect_choices(&self) -> Vec<(String, String, bool)> {
        self.registry
            .providers()
            .map(|p| {
                (
                    p.id.clone(),
                    provider_label(p),
                    Self::resolve_key(p).is_some(),
                )
            })
            .collect()
    }
}

fn provider_label(provider: &ProviderDescriptor) -> String {
    match provider.id.as_str() {
        "deepseek" => "DeepSeek".to_string(),
        "zai" => "Z.ai (GLM)".to_string(),
        "openai" => "OpenAI".to_string(),
        "meta" => "Meta (Muse Spark)".to_string(),
        "inver" => "Inver".to_string(),
        "nimble" => "Nimble".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_applies_config_and_reports_unknown_models() {
        let file: FileConfig = toml::from_str(
            r#"
[[providers]]
id = "local"
dialect = "open-ai"
base_url = "http://127.0.0.1:11434/v1"
api_key_env = "LOCAL_KEY"

[[models]]
id = "qwen"
provider = "local"
context_tokens = 32000
"#,
        )
        .unwrap();
        let hub = ModelHub::from_config(&file, None).unwrap();
        let qwen = hub.resolve("qwen").unwrap();
        assert_eq!(qwen.limits.context_tokens, 32_000);
        assert_eq!(
            hub.provider_descriptor(&qwen).unwrap().api_key_env,
            "LOCAL_KEY"
        );
        let error = hub.resolve("nope").unwrap_err().to_string();
        assert!(error.contains("deepseek-v4-flash"), "{error}");
        assert!(hub.connect_choices().iter().any(|(id, ..)| id == "local"));
    }

    #[test]
    fn missing_key_error_names_the_variable() {
        let hub = ModelHub::from_config(&FileConfig::default(), None).unwrap();
        let luna = hub.resolve("gpt-5.6-luna").unwrap();
        std::env::remove_var("OPENAI_API_KEY");
        let Err(error) = hub.build_provider(&luna) else {
            panic!("expected a missing-key error");
        };
        let error = error.to_string();
        assert!(error.contains("OPENAI_API_KEY"), "{error}");
    }
}

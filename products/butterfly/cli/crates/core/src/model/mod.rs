//! Model and provider descriptors. The agent loop reads *capabilities* from a
//! `ModelDescriptor` and never asks which vendor is underneath; vendor syntax
//! lives in the provider dialect. Built-in descriptors cover the reference
//! models; `[[providers]]` / `[[models]]` config tables add or override them so
//! a new model never needs a release.
//!
//! Nothing here touches the prompt prefix: descriptors change request
//! parameters (model id, thinking flags, budgets) and pricing, not the bytes of
//! the system message or tool schemas.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::{PriceCard, Pricing, TierSchedule};

/// Which OpenAI-compatible dialect a provider speaks. The dialect owns request
/// extras (thinking/effort syntax), usage-field parsing, and error
/// classification; everything else is the shared Chat Completions shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dialect {
    DeepSeek,
    Glm,
    /// Generic OpenAI-compatible Chat Completions server.
    OpenAi,
    /// OpenAI's own Responses API (`/responses`): the only endpoint that
    /// combines function tools with reasoning on OpenAI's reasoning models.
    OpenAiResponses,
    /// OpenAI-compatible plus the optional Butterfly ↔ Nimble routing
    /// extension (`docs/butterfly-nimble-contract.md`).
    Nimble,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub dialect: Dialect,
    /// Default endpoint (without `/chat/completions`). May be empty when the
    /// deployment is private and must come from `base_url_env` or config.
    pub base_url: String,
    /// Environment variable that overrides `base_url` when set.
    pub base_url_env: Option<String>,
    /// Environment variable holding the API key; the credential store entry
    /// keyed by `id` is the fallback.
    pub api_key_env: String,
    /// The endpoint may run without authentication (a loopback router): a
    /// missing key is not an error.
    #[serde(default)]
    pub api_key_optional: bool,
}

/// Whether the provider needs assistant reasoning echoed back on replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningReplay {
    /// The API rejects tool-call replays that drop `reasoning_content`; send
    /// the field always (null when absent). DeepSeek thinking models.
    Required,
    /// Send `reasoning_content` when present, omit otherwise.
    Optional,
    /// Never send reasoning back.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reasoning {
    /// Accepted values for the effort knob, in the provider's vocabulary.
    pub effort_values: Vec<String>,
    pub default_effort: Option<String>,
    pub replay: ReasoningReplay,
    /// The provider streams reasoning (deltas or summaries) while it thinks,
    /// so a healthy stream is never silent for long. False for providers that
    /// think in silence (Meta's Muse Spark: up to ~4 minutes with no bytes,
    /// measured 2026-09-08), which need a far longer idle timeout.
    #[serde(default = "default_true")]
    pub streamed: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptCache {
    /// Provider caches byte-identical prefixes on its own; byte-stability is
    /// the only lever (DeepSeek, OpenAI, GLM).
    Automatic,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub tools: bool,
    pub parallel_tools: bool,
    pub reasoning: Option<Reasoning>,
    pub structured_output: bool,
    pub prompt_cache: PromptCache,
    pub images: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub context_tokens: u64,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// Wire name sent to the provider, e.g. `deepseek-v4-flash`.
    pub id: String,
    /// `ProviderDescriptor::id`.
    pub provider: String,
    pub display_name: String,
    pub capabilities: Capabilities,
    pub pricing: Pricing,
    pub limits: Limits,
}

impl ModelDescriptor {
    pub fn supports_reasoning(&self) -> bool {
        self.capabilities.reasoning.is_some()
    }

    pub fn reasoning_replay(&self) -> ReasoningReplay {
        self.capabilities
            .reasoning
            .as_ref()
            .map_or(ReasoningReplay::Unsupported, |r| r.replay)
    }

    /// Resolve the effort to send: an explicit value must be one the model
    /// accepts; `None` falls back to the model default; models without
    /// reasoning ignore effort entirely (and reject an explicit one).
    pub fn resolve_effort(&self, requested: Option<&str>) -> Result<Option<String>, String> {
        match (&self.capabilities.reasoning, requested) {
            (None, None) => Ok(None),
            (None, Some(value)) => Err(format!(
                "model {} has no reasoning control; remove effort '{value}'",
                self.id
            )),
            (Some(reasoning), None) => Ok(reasoning.default_effort.clone()),
            // Reasoning, but no vocabulary to ask in: a router whose pool
            // picks the effort. Saying "accepted: " with an empty list would
            // read as a bug rather than as the answer.
            (Some(reasoning), Some(value)) if reasoning.effort_values.is_empty() => Err(format!(
                "model {} takes no effort setting; remove effort '{value}'",
                self.id
            )),
            (Some(reasoning), Some(value)) => {
                if reasoning.effort_values.iter().any(|v| v == value) {
                    Ok(Some(value.to_string()))
                } else {
                    Err(format!(
                        "unknown effort '{value}' for {} (accepted: {})",
                        self.id,
                        reasoning.effort_values.join(" | ")
                    ))
                }
            }
        }
    }
}

/// `[[providers]]` config entry: creates or overrides a provider by id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub id: String,
    pub dialect: Option<Dialect>,
    pub base_url: Option<String>,
    pub base_url_env: Option<String>,
    pub api_key_env: Option<String>,
    pub api_key_optional: Option<bool>,
}

/// `[[models]]` config entry: creates or overrides a model by id. Every field
/// but `id` is optional; a new model inherits generic OpenAI-compatible
/// defaults and must name an existing provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub id: String,
    pub provider: Option<String>,
    pub display_name: Option<String>,
    pub context_tokens: Option<u64>,
    pub max_output_tokens: Option<u32>,
    /// Micro-USD per million tokens at the base (peak) rate.
    pub cache_hit_input_microusd_per_million: Option<u64>,
    pub cache_miss_input_microusd_per_million: Option<u64>,
    pub output_microusd_per_million: Option<u64>,
    pub price_version: Option<String>,
    pub price_verified: Option<bool>,
    /// Drop the peak/off-peak schedule (flat pricing).
    pub flat_pricing: Option<bool>,
    pub tools: Option<bool>,
    pub parallel_tools: Option<bool>,
    /// `false` removes reasoning support; effort fields add it.
    pub reasoning: Option<bool>,
    pub effort_values: Option<Vec<String>>,
    pub default_effort: Option<String>,
    pub reasoning_replay: Option<ReasoningReplay>,
    pub structured_output: Option<bool>,
    pub prompt_cache: Option<PromptCache>,
    pub images: Option<bool>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("model '{model}' references unknown provider '{provider}'")]
    UnknownProvider { model: String, provider: String },
    #[error("new provider '{0}' needs dialect, base_url (or base_url_env), and api_key_env")]
    IncompleteProvider(String),
    #[error("new model '{0}' needs a provider")]
    IncompleteModel(String),
}

/// The set of known providers and models for one process.
#[derive(Debug, Clone)]
pub struct Registry {
    providers: BTreeMap<String, ProviderDescriptor>,
    models: Vec<Arc<ModelDescriptor>>,
}

impl Registry {
    pub fn empty() -> Self {
        Self {
            providers: BTreeMap::new(),
            models: Vec::new(),
        }
    }

    /// Reference descriptors shipped with the binary.
    pub fn builtin() -> Self {
        let mut registry = Self::empty();
        for provider in builtin::providers() {
            registry.providers.insert(provider.id.clone(), provider);
        }
        for model in builtin::models() {
            registry.models.push(Arc::new(model));
        }
        registry
    }

    pub fn models(&self) -> &[Arc<ModelDescriptor>] {
        &self.models
    }

    pub fn providers(&self) -> impl Iterator<Item = &ProviderDescriptor> {
        self.providers.values()
    }

    pub fn resolve(&self, model_id: &str) -> Option<Arc<ModelDescriptor>> {
        self.models.iter().find(|m| m.id == model_id).cloned()
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderDescriptor> {
        self.providers.get(id)
    }

    pub fn provider_for(&self, model: &ModelDescriptor) -> Option<&ProviderDescriptor> {
        self.providers.get(&model.provider)
    }

    /// Human-readable list for error messages.
    pub fn model_ids(&self) -> Vec<&str> {
        self.models.iter().map(|m| m.id.as_str()).collect()
    }

    pub fn apply_provider(&mut self, config: &ProviderConfig) -> Result<(), RegistryError> {
        let descriptor = match self.providers.get(&config.id) {
            Some(existing) => ProviderDescriptor {
                id: existing.id.clone(),
                dialect: config.dialect.unwrap_or(existing.dialect),
                base_url: config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| existing.base_url.clone()),
                base_url_env: config
                    .base_url_env
                    .clone()
                    .or_else(|| existing.base_url_env.clone()),
                api_key_env: config
                    .api_key_env
                    .clone()
                    .unwrap_or_else(|| existing.api_key_env.clone()),
                api_key_optional: config.api_key_optional.unwrap_or(existing.api_key_optional),
            },
            None => {
                let (Some(dialect), Some(api_key_env)) = (config.dialect, &config.api_key_env)
                else {
                    return Err(RegistryError::IncompleteProvider(config.id.clone()));
                };
                if config.base_url.is_none() && config.base_url_env.is_none() {
                    return Err(RegistryError::IncompleteProvider(config.id.clone()));
                }
                ProviderDescriptor {
                    id: config.id.clone(),
                    dialect,
                    base_url: config.base_url.clone().unwrap_or_default(),
                    base_url_env: config.base_url_env.clone(),
                    api_key_env: api_key_env.clone(),
                    api_key_optional: config.api_key_optional.unwrap_or(false),
                }
            }
        };
        self.providers.insert(descriptor.id.clone(), descriptor);
        Ok(())
    }

    pub fn apply_model(&mut self, config: &ModelConfig) -> Result<(), RegistryError> {
        let base = match self.resolve(&config.id) {
            Some(existing) => (*existing).clone(),
            None => {
                let Some(provider) = &config.provider else {
                    return Err(RegistryError::IncompleteModel(config.id.clone()));
                };
                builtin::generic_openai_compatible(&config.id, provider)
            }
        };
        let mut model = base;
        if let Some(provider) = &config.provider {
            model.provider = provider.clone();
        }
        if !self.providers.contains_key(&model.provider) {
            return Err(RegistryError::UnknownProvider {
                model: model.id,
                provider: model.provider,
            });
        }
        if let Some(name) = &config.display_name {
            model.display_name = name.clone();
        }
        if let Some(tokens) = config.context_tokens {
            model.limits.context_tokens = tokens;
        }
        if let Some(tokens) = config.max_output_tokens {
            model.limits.max_output_tokens = tokens;
        }
        let card = &mut model.pricing.card;
        if let Some(v) = config.cache_hit_input_microusd_per_million {
            card.cache_hit_input = v;
        }
        if let Some(v) = config.cache_miss_input_microusd_per_million {
            card.cache_miss_input = v;
        }
        if let Some(v) = config.output_microusd_per_million {
            card.output = v;
        }
        if let Some(version) = &config.price_version {
            model.pricing.version = version.clone();
        }
        if let Some(verified) = config.price_verified {
            model.pricing.verified = verified;
        }
        if config.flat_pricing == Some(true) {
            model.pricing.tiers = None;
        }
        let caps = &mut model.capabilities;
        if let Some(v) = config.tools {
            caps.tools = v;
        }
        if let Some(v) = config.parallel_tools {
            caps.parallel_tools = v;
        }
        if let Some(v) = config.structured_output {
            caps.structured_output = v;
        }
        if let Some(v) = config.prompt_cache {
            caps.prompt_cache = v;
        }
        if let Some(v) = config.images {
            caps.images = v;
        }
        if config.reasoning == Some(false) {
            caps.reasoning = None;
        } else if config.reasoning == Some(true)
            || config.effort_values.is_some()
            || config.default_effort.is_some()
            || config.reasoning_replay.is_some()
        {
            let mut reasoning = caps.reasoning.clone().unwrap_or(Reasoning {
                effort_values: Vec::new(),
                default_effort: None,
                replay: ReasoningReplay::Optional,
                streamed: true,
            });
            if let Some(values) = &config.effort_values {
                reasoning.effort_values = values.clone();
            }
            if let Some(default) = &config.default_effort {
                reasoning.default_effort = Some(default.clone());
            }
            if let Some(replay) = config.reasoning_replay {
                reasoning.replay = replay;
            }
            caps.reasoning = Some(reasoning);
        }
        let model = Arc::new(model);
        match self.models.iter().position(|m| m.id == model.id) {
            Some(index) => self.models[index] = model,
            None => self.models.push(model),
        }
        Ok(())
    }
}

/// The model tests and defaults use when nothing else is configured.
pub fn reference_model() -> Arc<ModelDescriptor> {
    Registry::builtin()
        .resolve(builtin::REFERENCE_MODEL_ID)
        .expect("reference model is built in")
}

pub mod builtin {
    use super::*;

    pub const REFERENCE_MODEL_ID: &str = "deepseek-v4-flash";

    /// DeepSeek bills by UTC time of day: peak is 01:00–04:00 and
    /// 06:00–10:00 UTC Monday–Friday; every other hour is off-peak at half
    /// the peak rate. Verified against the official pricing page 2026-08-31.
    pub fn deepseek_tiers() -> TierSchedule {
        TierSchedule {
            peak_windows_utc: vec![(1, 4), (6, 10)],
            business_days_only: true,
            off_peak_divisor: 2,
        }
    }

    fn deepseek_reasoning() -> Reasoning {
        Reasoning {
            effort_values: vec!["high".into(), "max".into()],
            default_effort: Some("high".into()),
            replay: ReasoningReplay::Required,
            streamed: true,
        }
    }

    pub fn providers() -> Vec<ProviderDescriptor> {
        vec![
            ProviderDescriptor {
                id: "deepseek".into(),
                dialect: Dialect::DeepSeek,
                base_url: "https://api.deepseek.com".into(),
                base_url_env: Some("DEEPSEEK_BASE_URL".into()),
                api_key_env: "DEEPSEEK_API_KEY".into(),
                api_key_optional: false,
            },
            // Z.ai's international OpenAI-compatible endpoint
            // (https://docs.z.ai/guides/develop/http/introduction, 2026-09-04).
            ProviderDescriptor {
                id: "zai".into(),
                dialect: Dialect::Glm,
                base_url: "https://api.z.ai/api/paas/v4".into(),
                base_url_env: Some("ZAI_BASE_URL".into()),
                api_key_env: "ZAI_API_KEY".into(),
                api_key_optional: false,
            },
            ProviderDescriptor {
                id: "openai".into(),
                dialect: Dialect::OpenAiResponses,
                base_url: "https://api.openai.com/v1".into(),
                base_url_env: Some("OPENAI_BASE_URL".into()),
                api_key_env: "OPENAI_API_KEY".into(),
                api_key_optional: false,
            },
            // Meta Model API (Muse Spark): OpenAI-compatible Chat Completions with
            // function tools and `reasoning_effort` on the same endpoint
            // (https://dev.meta.ai/docs, 2026-09-06). Meta's docs call the key
            // MODEL_API_KEY; Butterfly namespaces it per provider.
            ProviderDescriptor {
                id: "meta".into(),
                dialect: Dialect::OpenAi,
                base_url: "https://api.meta.ai/v1".into(),
                base_url_env: Some("META_BASE_URL".into()),
                api_key_env: "META_API_KEY".into(),
                api_key_optional: false,
            },
            // Monet's own Inver gateway resells OpenAI models behind a private
            // URL; no built-in model points at it, `[[models]]` can.
            ProviderDescriptor {
                id: "inver".into(),
                dialect: Dialect::OpenAi,
                base_url: String::new(),
                base_url_env: Some("INVER_BASE_URL".into()),
                api_key_env: "INVER_API_KEY".into(),
                api_key_optional: false,
            },
            ProviderDescriptor {
                id: "nimble".into(),
                dialect: Dialect::Nimble,
                base_url: "http://127.0.0.1:8092/v1".into(),
                base_url_env: Some("NIMBLE_BASE_URL".into()),
                api_key_env: "NIMBLE_API_KEY".into(),
                api_key_optional: true,
            },
        ]
    }

    pub fn models() -> Vec<ModelDescriptor> {
        vec![
            ModelDescriptor {
                id: "deepseek-v4-flash".into(),
                provider: "deepseek".into(),
                // Legacy id: since 2026-09 DeepSeek routes it to V4.1 Flash and
                // bills at V4.1 Flash prices (api-docs.deepseek.com/quick_start/pricing).
                display_name: "DeepSeek V4 Flash (served as V4.1 Flash)".into(),
                capabilities: Capabilities {
                    tools: true,
                    parallel_tools: true,
                    reasoning: Some(deepseek_reasoning()),
                    structured_output: true,
                    prompt_cache: PromptCache::Automatic,
                    images: false,
                },
                pricing: Pricing {
                    // Peak card (01–04 and 06–10 UTC weekdays); off-peak halves it.
                    card: PriceCard {
                        cache_hit_input: 6_000,
                        cache_miss_input: 300_000,
                        output: 1_200_000,
                    },
                    tiers: Some(deepseek_tiers()),
                    version: "2026-09-10-usd".into(),
                    verified: true,
                },
                limits: Limits {
                    // Advertised window is 1M (docs: "1M", OpenRouter:
                    // 1,048,576); use a conservative round million.
                    context_tokens: 1_000_000,
                    max_output_tokens: 65_536,
                },
            },
            // DeepSeek V4.1 Flash: the current Flash model under its own id.
            // Peak $0.30 miss / $0.006 hit / $1.20 out per 1M, half off-peak
            // (api-docs.deepseek.com/quick_start/pricing, 2026-09-10). The API
            // allows 384K output; the cap stays at 64K so a runaway reasoning
            // call cannot run for hours (observed on GLM: 65K tokens ≈ 30 min).
            ModelDescriptor {
                id: "deepseek-flash".into(),
                provider: "deepseek".into(),
                display_name: "DeepSeek V4.1 Flash".into(),
                capabilities: Capabilities {
                    tools: true,
                    parallel_tools: true,
                    reasoning: Some(deepseek_reasoning()),
                    structured_output: true,
                    prompt_cache: PromptCache::Automatic,
                    images: true,
                },
                pricing: Pricing {
                    card: PriceCard {
                        cache_hit_input: 6_000,
                        cache_miss_input: 300_000,
                        output: 1_200_000,
                    },
                    tiers: Some(deepseek_tiers()),
                    version: "2026-09-10-usd".into(),
                    verified: true,
                },
                limits: Limits {
                    context_tokens: 1_000_000,
                    max_output_tokens: 65_536,
                },
            },
            ModelDescriptor {
                id: "deepseek-v4-pro".into(),
                provider: "deepseek".into(),
                // DeepSeek announced that from 2026-09-14 this id is routed to
                // V4.1 Flash and billed at Flash prices; card left as Pro
                // until then.
                display_name: "DeepSeek V4 Pro".into(),
                capabilities: Capabilities {
                    tools: true,
                    parallel_tools: true,
                    reasoning: Some(deepseek_reasoning()),
                    structured_output: true,
                    prompt_cache: PromptCache::Automatic,
                    images: false,
                },
                pricing: Pricing {
                    card: PriceCard {
                        cache_hit_input: 44_000,
                        cache_miss_input: 1_320_000,
                        output: 3_960_000,
                    },
                    tiers: Some(deepseek_tiers()),
                    version: "2026-08-31-usd".into(),
                    verified: true,
                },
                limits: Limits {
                    context_tokens: 1_000_000,
                    max_output_tokens: 65_536,
                },
            },
            // GLM-5.3-Flash on Z.ai. List prices from
            // https://docs.z.ai/guides/overview/pricing (2026-09-04): $0.15 in,
            // $0.03 cached, $0.50 out per 1M; a 50% launch promo ran until
            // 2026-09-09 and is deliberately NOT encoded (list price is the
            // conservative figure). Context "1M" (OpenRouter: 1,310,720),
            // 128K output; thinking is always on for this model.
            ModelDescriptor {
                id: "glm-5.3-flash".into(),
                provider: "zai".into(),
                display_name: "GLM-5.3 Flash".into(),
                capabilities: Capabilities {
                    tools: true,
                    parallel_tools: true,
                    reasoning: Some(Reasoning {
                        // Probed 2026-09-16: thinking cannot be disabled (code 1210), but
                        // `reasoning_effort` is honoured — "low" returned zero
                        // reasoning tokens on a short prompt where the default
                        // spent 327 of 390 output tokens on reasoning.
                        effort_values: vec!["low".into(), "high".into(), "max".into()],
                        default_effort: None,
                        replay: ReasoningReplay::Optional,
                        streamed: true,
                    }),
                    structured_output: true,
                    prompt_cache: PromptCache::Automatic,
                    images: true,
                },
                pricing: Pricing {
                    card: PriceCard {
                        cache_hit_input: 30_000,
                        cache_miss_input: 150_000,
                        output: 500_000,
                    },
                    tiers: None,
                    version: "2026-09-04-usd-list".into(),
                    verified: true,
                },
                limits: Limits {
                    context_tokens: 1_000_000,
                    max_output_tokens: 128_000,
                },
            },
            // Muse Spark 1.3 Contributor on Meta Model API. Verified against
            // https://developer.meta.com/ai/models/muse-spark/ and OpenRouter
            // (2026-09-06): $0.10 in, $0.002 cached, $0.20 out per 1M;
            // 1,048,576 context; tools + tool_choice + JSON schema; reasoning
            // always on with `reasoning_effort` (minimal…xhigh per Meta's
            // cookbook). Max output is undocumented — 64K is the conservative
            // budget until a live probe says otherwise. Contributor tier:
            // prompts and outputs may be used to improve Meta's products.
            ModelDescriptor {
                id: "muse-spark-1.3-contributor".into(),
                provider: "meta".into(),
                display_name: "Muse Spark 1.3 Contributor".into(),
                capabilities: Capabilities {
                    tools: true,
                    parallel_tools: true,
                    reasoning: Some(Reasoning {
                        effort_values: ["minimal", "low", "medium", "high", "xhigh"]
                            .into_iter()
                            .map(String::from)
                            .collect(),
                        default_effort: None,
                        replay: ReasoningReplay::Optional,
                        streamed: false,
                    }),
                    structured_output: true,
                    prompt_cache: PromptCache::Automatic,
                    images: true,
                },
                pricing: Pricing {
                    card: PriceCard {
                        cache_hit_input: 2_000,
                        cache_miss_input: 100_000,
                        output: 200_000,
                    },
                    tiers: None,
                    version: "2026-09-06-usd".into(),
                    verified: true,
                },
                limits: Limits {
                    context_tokens: 1_048_576,
                    max_output_tokens: 65_536,
                },
            },
            // Muse Spark 1.3 (standard tier, no training on prompts): $1.25 in,
            // $0.15 cached, $4.25 out per 1M (same sources, 2026-09-06). Only
            // this tier accepts `reasoning_effort: max`, the setting behind
            // Meta's published benchmarks; the Contributor tier stops at xhigh
            // (probed live 2026-09-08).
            ModelDescriptor {
                id: "muse-spark-1.3".into(),
                provider: "meta".into(),
                display_name: "Muse Spark 1.3".into(),
                capabilities: Capabilities {
                    tools: true,
                    parallel_tools: true,
                    reasoning: Some(Reasoning {
                        effort_values: ["minimal", "low", "medium", "high", "xhigh", "max"]
                            .into_iter()
                            .map(String::from)
                            .collect(),
                        default_effort: None,
                        replay: ReasoningReplay::Optional,
                        streamed: false,
                    }),
                    structured_output: true,
                    prompt_cache: PromptCache::Automatic,
                    images: true,
                },
                pricing: Pricing {
                    card: PriceCard {
                        cache_hit_input: 150_000,
                        cache_miss_input: 1_250_000,
                        output: 4_250_000,
                    },
                    tiers: None,
                    version: "2026-09-06-usd".into(),
                    verified: true,
                },
                limits: Limits {
                    context_tokens: 1_048_576,
                    max_output_tokens: 65_536,
                },
            },
            // GPT-5.6 Luna on OpenAI. Verified against
            // https://developers.openai.com/api/docs/models/gpt-5.6-luna
            // (2026-09-04): $0.20 in, $0.02 cached, $1.20 out per 1M;
            // 1,050,000 context, 128,000 output; efforts none…max. Prompts
            // over 272K input tokens are billed 2x input / 1.5x output for the
            // whole request — not modelled by this flat card; keep sessions
            // below that or expect under-reporting past it.
            ModelDescriptor {
                id: "gpt-5.6-luna".into(),
                provider: "openai".into(),
                display_name: "GPT-5.6 Luna".into(),
                capabilities: Capabilities {
                    tools: true,
                    parallel_tools: true,
                    reasoning: Some(Reasoning {
                        effort_values: vec![
                            "none".into(),
                            "low".into(),
                            "medium".into(),
                            "high".into(),
                            "xhigh".into(),
                            "max".into(),
                        ],
                        default_effort: Some("high".into()),
                        replay: ReasoningReplay::Unsupported,
                        streamed: true,
                    }),
                    structured_output: true,
                    prompt_cache: PromptCache::Automatic,
                    images: true,
                },
                pricing: Pricing {
                    card: PriceCard {
                        cache_hit_input: 20_000,
                        cache_miss_input: 200_000,
                        output: 1_200_000,
                    },
                    tiers: None,
                    version: "2026-09-04-usd".into(),
                    verified: true,
                },
                limits: Limits {
                    context_tokens: 1_050_000,
                    max_output_tokens: 128_000,
                },
            },
            nimble_virtual_model(),
        ]
    }

    /// The Nimble virtual model: one id, the router picks the real model.
    /// Reasoning replay is `Required` because a DeepSeek upstream needs the
    /// echoed `reasoning_content` and the other pool members ignore it; no
    /// effort knob is sent (the router's pool decides). Pricing is unknown
    /// here — the router accounts per selected model — so costs are marked
    /// unverified.
    pub fn nimble_virtual_model() -> ModelDescriptor {
        ModelDescriptor {
            id: "nimble".into(),
            provider: "nimble".into(),
            display_name: "Nimble (routed)".into(),
            capabilities: Capabilities {
                tools: true,
                parallel_tools: true,
                reasoning: Some(Reasoning {
                    effort_values: Vec::new(),
                    default_effort: None,
                    replay: ReasoningReplay::Required,
                    streamed: true,
                }),
                structured_output: false,
                prompt_cache: PromptCache::Automatic,
                images: false,
            },
            pricing: Pricing {
                card: PriceCard {
                    cache_hit_input: 0,
                    cache_miss_input: 0,
                    output: 0,
                },
                tiers: None,
                version: "routed-by-nimble".into(),
                verified: false,
            },
            limits: Limits {
                // Conservative until the router reports the selected model's
                // real limit in its routing facts.
                context_tokens: 128_000,
                max_output_tokens: 32_768,
            },
        }
    }

    /// Defaults for a config-defined model on an OpenAI-compatible endpoint:
    /// tools on, no reasoning knob, automatic prefix cache, unverified zero
    /// pricing so cost never silently reads as real money.
    pub fn generic_openai_compatible(id: &str, provider: &str) -> ModelDescriptor {
        ModelDescriptor {
            id: id.to_string(),
            provider: provider.to_string(),
            display_name: id.to_string(),
            capabilities: Capabilities {
                tools: true,
                parallel_tools: false,
                reasoning: None,
                structured_output: false,
                prompt_cache: PromptCache::Automatic,
                images: false,
            },
            pricing: Pricing {
                card: PriceCard {
                    cache_hit_input: 0,
                    cache_miss_input: 0,
                    output: 0,
                },
                tiers: None,
                version: "unpriced".into(),
                verified: false,
            },
            limits: Limits {
                context_tokens: 128_000,
                max_output_tokens: 16_384,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_resolves_reference_models_and_providers() {
        let registry = Registry::builtin();
        let flash = registry.resolve("deepseek-v4-flash").unwrap();
        assert_eq!(flash.provider, "deepseek");
        assert_eq!(
            registry.provider_for(&flash).unwrap().dialect,
            Dialect::DeepSeek
        );
        assert!(flash.pricing.verified);
        assert_eq!(flash.reasoning_replay(), ReasoningReplay::Required);
        assert!(registry.resolve("nope").is_none());
        // Every built-in model names a built-in provider.
        for model in registry.models() {
            assert!(registry.provider_for(model).is_some(), "{}", model.id);
        }
    }

    #[test]
    fn effort_resolution_follows_capabilities() {
        let registry = Registry::builtin();
        let flash = registry.resolve("deepseek-v4-flash").unwrap();
        assert_eq!(flash.resolve_effort(None).unwrap().as_deref(), Some("high"));
        assert_eq!(
            flash.resolve_effort(Some("max")).unwrap().as_deref(),
            Some("max")
        );
        assert!(flash.resolve_effort(Some("turbo")).is_err());
        let plain = builtin::generic_openai_compatible("m", "deepseek");
        assert_eq!(plain.resolve_effort(None).unwrap(), None);
        assert!(plain.resolve_effort(Some("high")).is_err());

        // The three "nothing to set" states must not read alike: no
        // reasoning at all, a router that owns the choice, and a model whose
        // default is the provider's own.
        let router = registry.resolve("nimble").unwrap();
        assert_eq!(router.resolve_effort(None).unwrap(), None);
        let refusal = router.resolve_effort(Some("high")).unwrap_err();
        assert!(
            refusal.contains("takes no effort setting") && !refusal.contains("accepted"),
            "empty vocabulary must not offer an empty list: {refusal}"
        );
        assert!(plain
            .resolve_effort(Some("high"))
            .unwrap_err()
            .contains("no reasoning control"));
        let glm = registry.resolve("glm-5.3-flash").unwrap();
        assert_eq!(glm.resolve_effort(None).unwrap(), None);
        assert_eq!(
            glm.resolve_effort(Some("low")).unwrap().as_deref(),
            Some("low")
        );
    }

    #[test]
    fn config_overrides_existing_models_and_adds_new_ones() {
        let mut registry = Registry::builtin();
        registry
            .apply_model(&ModelConfig {
                id: "deepseek-v4-flash".into(),
                context_tokens: Some(131_072),
                default_effort: Some("max".into()),
                ..ModelConfig::default()
            })
            .unwrap();
        let flash = registry.resolve("deepseek-v4-flash").unwrap();
        assert_eq!(flash.limits.context_tokens, 131_072);
        assert_eq!(flash.resolve_effort(None).unwrap().as_deref(), Some("max"));
        // Still exactly one entry for the id.
        assert_eq!(
            registry
                .models()
                .iter()
                .filter(|m| m.id == "deepseek-v4-flash")
                .count(),
            1
        );

        // A new model needs a provider…
        assert_eq!(
            registry.apply_model(&ModelConfig {
                id: "mystery".into(),
                ..ModelConfig::default()
            }),
            Err(RegistryError::IncompleteModel("mystery".into()))
        );
        // …and that provider must exist.
        assert!(matches!(
            registry.apply_model(&ModelConfig {
                id: "mystery".into(),
                provider: Some("ghost".into()),
                ..ModelConfig::default()
            }),
            Err(RegistryError::UnknownProvider { .. })
        ));
        registry
            .apply_provider(&ProviderConfig {
                id: "local".into(),
                dialect: Some(Dialect::OpenAi),
                base_url: Some("http://127.0.0.1:11434/v1".into()),
                api_key_env: Some("LOCAL_API_KEY".into()),
                ..ProviderConfig::default()
            })
            .unwrap();
        registry
            .apply_model(&ModelConfig {
                id: "qwen-coder".into(),
                provider: Some("local".into()),
                reasoning: Some(true),
                effort_values: Some(vec!["low".into(), "high".into()]),
                ..ModelConfig::default()
            })
            .unwrap();
        let qwen = registry.resolve("qwen-coder").unwrap();
        assert_eq!(qwen.provider, "local");
        assert!(!qwen.pricing.verified);
        assert_eq!(qwen.reasoning_replay(), ReasoningReplay::Optional);
        assert!(qwen.resolve_effort(Some("high")).is_ok());
    }

    #[test]
    fn incomplete_new_providers_are_rejected() {
        let mut registry = Registry::builtin();
        assert_eq!(
            registry.apply_provider(&ProviderConfig {
                id: "half".into(),
                dialect: Some(Dialect::OpenAi),
                ..ProviderConfig::default()
            }),
            Err(RegistryError::IncompleteProvider("half".into()))
        );
        // Overriding an existing provider needs only the changed fields.
        registry
            .apply_provider(&ProviderConfig {
                id: "deepseek".into(),
                base_url: Some("https://proxy.example/v1".into()),
                ..ProviderConfig::default()
            })
            .unwrap();
        assert_eq!(
            registry.provider("deepseek").unwrap().base_url,
            "https://proxy.example/v1"
        );
        assert_eq!(
            registry.provider("deepseek").unwrap().dialect,
            Dialect::DeepSeek
        );
    }
}

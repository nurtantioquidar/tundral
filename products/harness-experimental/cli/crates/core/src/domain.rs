//! Core domain types shared by the provider, session, and context layers.
//!
//! Invariants:
//! - Monetary values are integer micro-USD end to end; decimal USD is presentation only.
//! - Provider-reported cache-hit, cache-miss, input, and output token counts remain distinct.
//! - Assistant tool-call messages preserve `reasoning_content` for provider replay.

use serde::{Deserialize, Serialize};

pub const MICROUSD_PER_USD: u64 = 1_000_000;

/// A single function/tool invocation requested by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON string exactly as the provider sent it (kept verbatim for byte-stable replay).
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant(AssistantMessage),
    Tool {
        tool_call_id: String,
        content: String,
    },
}

impl ChatMessage {
    pub fn approximate_bytes(&self) -> u64 {
        let n = match self {
            ChatMessage::System { content } | ChatMessage::User { content } => content.len(),
            ChatMessage::Assistant(m) => {
                m.content.as_deref().map_or(0, str::len)
                    + m.reasoning_content.as_deref().map_or(0, str::len)
                    + m.tool_calls
                        .iter()
                        .map(|c| c.name.len() + c.arguments.len())
                        .sum::<usize>()
            }
            ChatMessage::Tool { content, .. } => content.len(),
        };
        n as u64
    }
}

/// Provider usage for one request. The four counters stay distinct so cache
/// behavior is always observable and benchmarks can normalize away cache luck.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub cache_hit_input_tokens: u64,
    pub cache_miss_input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.cache_hit_input_tokens += other.cache_hit_input_tokens;
        self.cache_miss_input_tokens += other.cache_miss_input_tokens;
        self.output_tokens += other.output_tokens;
    }

    /// Cache hit rate over input tokens, in [0, 1].
    pub fn cache_hit_rate(&self) -> f64 {
        if self.input_tokens == 0 {
            return 0.0;
        }
        self.cache_hit_input_tokens as f64 / self.input_tokens as f64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    /// What the request actually cost given observed cache hits.
    pub actual_microusd: u64,
    /// Cost with every input token priced at the cache-miss rate: cache-blind
    /// figure for comparisons that must not be skewed by warm-cache luck.
    pub normalized_cache_miss_microusd: u64,
    pub price_version: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    DeepseekV4Flash,
    DeepseekV4Pro,
}

impl Model {
    pub fn wire_name(self) -> &'static str {
        match self {
            Model::DeepseekV4Flash => "deepseek-v4-flash",
            Model::DeepseekV4Pro => "deepseek-v4-pro",
        }
    }

    pub fn parse(name: &str) -> Option<Model> {
        match name {
            "deepseek-v4-flash" => Some(Model::DeepseekV4Flash),
            "deepseek-v4-pro" => Some(Model::DeepseekV4Pro),
            _ => None,
        }
    }

    pub fn context_window_tokens(self) -> u64 {
        128_000
    }
}

pub const PRICE_VERSION: &str = "2026-04-24-usd";

/// Micro-USD per million tokens.
#[derive(Debug, Clone, Copy)]
pub struct PriceCard {
    pub cache_hit_input: u64,
    pub cache_miss_input: u64,
    pub output: u64,
}

pub fn price_card(model: Model) -> PriceCard {
    match model {
        Model::DeepseekV4Flash => PriceCard {
            cache_hit_input: 2_800,
            cache_miss_input: 140_000,
            output: 280_000,
        },
        Model::DeepseekV4Pro => PriceCard {
            cache_hit_input: 3_625,
            cache_miss_input: 435_000,
            output: 870_000,
        },
    }
}

fn tokens_cost_microusd(tokens: u64, microusd_per_million: u64) -> u64 {
    let product = u128::from(tokens) * u128::from(microusd_per_million);
    u64::try_from(product.div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

pub fn price_usage(model: Model, usage: Usage) -> Cost {
    let card = price_card(model);
    let actual = tokens_cost_microusd(usage.cache_hit_input_tokens, card.cache_hit_input)
        .saturating_add(tokens_cost_microusd(
            usage.cache_miss_input_tokens,
            card.cache_miss_input,
        ))
        .saturating_add(tokens_cost_microusd(usage.output_tokens, card.output));
    let normalized = tokens_cost_microusd(usage.input_tokens, card.cache_miss_input)
        .saturating_add(tokens_cost_microusd(usage.output_tokens, card.output));
    Cost {
        actual_microusd: actual,
        normalized_cache_miss_microusd: normalized,
        price_version: PRICE_VERSION,
    }
}

/// Parse a decimal USD string ("1", "0.25", "1.500000") to micro-USD without
/// floating point. At most six fractional digits.
pub fn usd_to_microusd(text: &str) -> Option<u64> {
    let (whole, frac) = match text.split_once('.') {
        Some((w, f)) => (w, f),
        None => (text, ""),
    };
    if whole.is_empty() || whole.bytes().any(|b| !b.is_ascii_digit()) {
        return None;
    }
    if frac.len() > 6 || frac.bytes().any(|b| !b.is_ascii_digit()) {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let mut frac_micro: u64 = 0;
    if !frac.is_empty() {
        frac_micro = frac.parse().ok()?;
        for _ in frac.len()..6 {
            frac_micro *= 10;
        }
    }
    whole
        .checked_mul(MICROUSD_PER_USD)
        .and_then(|w| w.checked_add(frac_micro))
}

/// Presentation-only decimal rendering of a micro-USD amount.
pub fn format_microusd_as_usd(microusd: u64) -> String {
    let whole = microusd / MICROUSD_PER_USD;
    let frac = microusd % MICROUSD_PER_USD;
    format!("${whole}.{frac:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usd_parsing_is_exact_and_rejects_junk() {
        assert_eq!(usd_to_microusd("1"), Some(1_000_000));
        assert_eq!(usd_to_microusd("0.25"), Some(250_000));
        assert_eq!(usd_to_microusd("1.500000"), Some(1_500_000));
        assert_eq!(usd_to_microusd("0.000001"), Some(1));
        assert_eq!(usd_to_microusd(".5"), None);
        assert_eq!(usd_to_microusd("1.0000001"), None);
        assert_eq!(usd_to_microusd("-1"), None);
        assert_eq!(usd_to_microusd("1e3"), None);
    }

    #[test]
    fn pricing_uses_distinct_counters() {
        let usage = Usage {
            input_tokens: 1_000_000,
            cache_hit_input_tokens: 900_000,
            cache_miss_input_tokens: 100_000,
            output_tokens: 10_000,
        };
        let cost = price_usage(Model::DeepseekV4Flash, usage);
        // 0.9M * 2,800/M + 0.1M * 140,000/M + 0.01M * 280,000/M
        assert_eq!(cost.actual_microusd, 2_520 + 14_000 + 2_800);
        // normalized prices ALL input at the miss rate
        assert_eq!(cost.normalized_cache_miss_microusd, 140_000 + 2_800);
        assert_eq!(cost.price_version, PRICE_VERSION);
    }

    #[test]
    fn pricing_rounds_up_and_never_overflows() {
        let usage = Usage {
            input_tokens: 1,
            cache_hit_input_tokens: 0,
            cache_miss_input_tokens: 1,
            output_tokens: 0,
        };
        // 1 token at 140,000 micro-USD/M rounds up to 1 micro-USD.
        assert_eq!(
            price_usage(Model::DeepseekV4Flash, usage).actual_microusd,
            1
        );
        let huge = Usage {
            input_tokens: u64::MAX,
            cache_hit_input_tokens: u64::MAX,
            cache_miss_input_tokens: u64::MAX,
            output_tokens: u64::MAX,
        };
        let _ = price_usage(Model::DeepseekV4Pro, huge);
    }

    #[test]
    fn formatting_is_presentation_only() {
        assert_eq!(format_microusd_as_usd(1_234_567), "$1.234567");
        assert_eq!(format_microusd_as_usd(1), "$0.000001");
    }
}

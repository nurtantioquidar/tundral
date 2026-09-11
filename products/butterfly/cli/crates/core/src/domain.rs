//! Core domain types shared by the provider, session, and context layers.
//!
//! Invariants:
//! - Monetary values are integer micro-USD end to end; decimal USD is presentation only.
//! - Provider-reported cache-hit, cache-miss, input, and output token counts remain distinct.
//! - Assistant tool-call messages preserve `reasoning_content` for provider replay when the
//!   model requires it (see `model::ReasoningReplay`).

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
    /// Opaque provider replay items echoed back verbatim on later requests
    /// (e.g. OpenAI Responses encrypted reasoning). Never rendered, never
    /// counted toward byte estimates, absent on every other provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_state: Option<serde_json::Value>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    /// What the request actually cost given observed cache hits.
    pub actual_microusd: u64,
    /// Cost with every input token priced at the cache-miss rate: cache-blind
    /// figure for comparisons that must not be skewed by warm-cache luck.
    pub normalized_cache_miss_microusd: u64,
    pub price_version: String,
    /// False when the price card is a placeholder; consumers must not present
    /// such a cost as real money.
    pub price_verified: bool,
}

/// Time-of-day billing tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PriceTier {
    Peak,
    OffPeak,
}

/// A provider's peak/off-peak schedule in UTC. Off-peak divides every rate by
/// `off_peak_divisor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierSchedule {
    /// Half-open hour ranges `[start, end)` that are peak.
    pub peak_windows_utc: Vec<(u8, u8)>,
    /// Weekends are always off-peak when set.
    pub business_days_only: bool,
    pub off_peak_divisor: u64,
}

impl TierSchedule {
    pub fn tier_now(&self) -> PriceTier {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.tier_for_epoch_seconds(seconds)
    }

    /// 1970-01-01 was a Thursday; weekday 0 = Monday here.
    pub fn tier_for_epoch_seconds(&self, seconds: u64) -> PriceTier {
        let days = seconds / 86_400;
        let weekday = (days + 3) % 7; // 0 = Monday … 6 = Sunday
        let hour = ((seconds % 86_400) / 3_600) as u8;
        if self.business_days_only && weekday >= 5 {
            return PriceTier::OffPeak;
        }
        let in_peak = self
            .peak_windows_utc
            .iter()
            .any(|&(start, end)| (start..end).contains(&hour));
        if in_peak {
            PriceTier::Peak
        } else {
            PriceTier::OffPeak
        }
    }
}

/// Micro-USD per million tokens at the base (peak) rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceCard {
    pub cache_hit_input: u64,
    pub cache_miss_input: u64,
    pub output: u64,
}

/// A model's price card plus its provenance. `version` names the rate card so
/// settled usage is never silently repriced; `verified` says whether the card
/// was checked against the provider's published pricing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pricing {
    pub card: PriceCard,
    pub tiers: Option<TierSchedule>,
    pub version: String,
    pub verified: bool,
}

impl Pricing {
    pub fn tier_now(&self) -> PriceTier {
        self.tiers
            .as_ref()
            .map_or(PriceTier::Peak, TierSchedule::tier_now)
    }

    fn card_at(&self, tier: PriceTier) -> PriceCard {
        let divisor = match (&self.tiers, tier) {
            (Some(schedule), PriceTier::OffPeak) => schedule.off_peak_divisor.max(1),
            _ => 1,
        };
        PriceCard {
            cache_hit_input: self.card.cache_hit_input / divisor,
            cache_miss_input: self.card.cache_miss_input / divisor,
            output: self.card.output / divisor,
        }
    }
}

fn tokens_cost_microusd(tokens: u64, microusd_per_million: u64) -> u64 {
    let product = u128::from(tokens) * u128::from(microusd_per_million);
    u64::try_from(product.div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

/// Price a request at the current tier.
pub fn price_usage(pricing: &Pricing, usage: Usage) -> Cost {
    price_usage_at(pricing, usage, pricing.tier_now())
}

pub fn price_usage_at(pricing: &Pricing, usage: Usage, tier: PriceTier) -> Cost {
    let card = pricing.card_at(tier);
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
        price_version: pricing.version.clone(),
        price_verified: pricing.verified,
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
    use crate::model::builtin::deepseek_tiers;

    fn flash_pricing() -> Pricing {
        crate::model::reference_model().pricing.clone()
    }

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
    fn pricing_uses_distinct_counters_per_tier() {
        let usage = Usage {
            input_tokens: 1_000_000,
            cache_hit_input_tokens: 900_000,
            cache_miss_input_tokens: 100_000,
            output_tokens: 10_000,
        };
        let pricing = flash_pricing();
        let peak = price_usage_at(&pricing, usage, PriceTier::Peak);
        // V4.1 Flash peak card: 0.9M * 6,000/M + 0.1M * 300,000/M + 0.01M * 1,200,000/M
        assert_eq!(peak.actual_microusd, 5_400 + 30_000 + 12_000);
        // normalized prices ALL input at the miss rate
        assert_eq!(peak.normalized_cache_miss_microusd, 300_000 + 12_000);
        assert_eq!(peak.price_version, "2026-09-10-usd");
        assert!(peak.price_verified);
        let off_peak = price_usage_at(&pricing, usage, PriceTier::OffPeak);
        assert_eq!(off_peak.actual_microusd, peak.actual_microusd / 2);
        // Flat pricing ignores the tier.
        let flat = Pricing {
            tiers: None,
            ..pricing
        };
        assert_eq!(
            price_usage_at(&flat, usage, PriceTier::OffPeak).actual_microusd,
            peak.actual_microusd
        );
    }

    #[test]
    fn price_tier_follows_utc_business_windows() {
        let schedule = deepseek_tiers();
        // 2026-08-31 is a Monday. 02:00 UTC = peak; 05:00 = off-peak;
        // 07:00 = peak; 12:00 = off-peak.
        let monday_midnight = 1_788_134_400; // 2026-08-31T00:00:00Z
        let at = |hour: u64| monday_midnight + hour * 3_600;
        assert_eq!(schedule.tier_for_epoch_seconds(at(2)), PriceTier::Peak);
        assert_eq!(schedule.tier_for_epoch_seconds(at(5)), PriceTier::OffPeak);
        assert_eq!(schedule.tier_for_epoch_seconds(at(7)), PriceTier::Peak);
        assert_eq!(schedule.tier_for_epoch_seconds(at(12)), PriceTier::OffPeak);
        // Sunday 07:00 UTC is off-peak (weekend).
        let sunday = monday_midnight - 86_400;
        assert_eq!(
            schedule.tier_for_epoch_seconds(sunday + 7 * 3_600),
            PriceTier::OffPeak
        );
    }

    #[test]
    fn pricing_rounds_up_and_never_overflows() {
        let usage = Usage {
            input_tokens: 1,
            cache_hit_input_tokens: 0,
            cache_miss_input_tokens: 1,
            output_tokens: 0,
        };
        // 1 token at 440,000 micro-USD/M rounds up to 1 micro-USD.
        assert_eq!(
            price_usage_at(&flash_pricing(), usage, PriceTier::Peak).actual_microusd,
            1
        );
        let huge = Usage {
            input_tokens: u64::MAX,
            cache_hit_input_tokens: u64::MAX,
            cache_miss_input_tokens: u64::MAX,
            output_tokens: u64::MAX,
        };
        let pro = crate::model::Registry::builtin()
            .resolve("deepseek-v4-pro")
            .unwrap();
        let _ = price_usage_at(&pro.pricing, huge, PriceTier::Peak);
    }

    #[test]
    fn formatting_is_presentation_only() {
        assert_eq!(format_microusd_as_usd(1_234_567), "$1.234567");
        assert_eq!(format_microusd_as_usd(1), "$0.000001");
    }
}

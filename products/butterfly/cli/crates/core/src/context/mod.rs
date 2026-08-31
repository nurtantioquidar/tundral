//! Context management: token accounting, cache-aware history layout, and the
//! two-stage compaction pipeline (mechanical tool-result elision, then
//! LLM summarization restart).
//!
//! Governing law: once a message has been sent, its bytes and position never
//! change — except during a compaction event, the only operation allowed to
//! rewrite history. DeepSeek's prefix cache has no explicit breakpoints, so
//! byte-stability is the only lever, and every rewrite is paid for as a
//! one-time cache miss. Rewrites must therefore be rare and chunky: the
//! `min_savings_fraction` floor rejects elisions that would not reclaim
//! enough to justify invalidating the prefix.

pub mod prompt;

use tokio_util::sync::CancellationToken;

use crate::domain::{ChatMessage, Cost, Usage};
use crate::provider::{
    NullSink, Provider, ProviderError, ProviderRequest, ReasoningEffort, ToolSchema,
};

#[derive(Debug, Clone)]
pub struct ContextConfig {
    pub window_tokens: u64,
    pub output_reserve_tokens: u64,
    pub safety_margin_tokens: u64,
    pub prepare_threshold: f64,
    pub compact_threshold: f64,
    pub urgent_threshold: f64,
    /// Minimum fraction of the effective window an elision must reclaim;
    /// smaller rewrites are worse than leaving the cache prefix intact.
    pub min_savings_fraction: f64,
    pub protected_tail_tokens: u64,
    /// Maximum user turns kept verbatim by summarization — a preference, not
    /// a floor: the tail is always capped by `tail_budget_fraction` so a few
    /// giant turns can never leave the rebuilt history over threshold.
    pub keep_last_turns: usize,
    /// Hard cap on the verbatim tail as a fraction of the effective window.
    pub tail_budget_fraction: f64,
    pub max_summary_tokens: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            // Verified 2026-08-31: deepseek-v4 serves a 1M-token window.
            window_tokens: 1_000_000,
            output_reserve_tokens: 16_000,
            safety_margin_tokens: 4_000,
            prepare_threshold: 0.70,
            compact_threshold: 0.80,
            urgent_threshold: 0.90,
            min_savings_fraction: 0.25,
            protected_tail_tokens: 40_000,
            keep_last_turns: 6,
            tail_budget_fraction: 0.35,
            max_summary_tokens: 2_000,
        }
    }
}

impl ContextConfig {
    pub fn effective_window(&self) -> u64 {
        self.window_tokens
            .saturating_sub(self.output_reserve_tokens)
            .saturating_sub(self.safety_margin_tokens)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionNeed {
    None,
    /// Approaching the window; cheap preparation may start.
    Prepare,
    /// Compact before the next request.
    Compact,
    /// Compact synchronously and aggressively.
    Urgent,
}

/// Anchor + drift token estimator. The anchor is ground truth from the last
/// provider response; drift is a bytes-per-token estimate of what has been
/// appended since, recalibrated against every response so error never
/// compounds.
#[derive(Debug, Clone)]
pub struct TokenLedger {
    anchor_tokens: u64,
    drift_bytes: u64,
    drift_messages: u64,
    bytes_per_token: f64,
}

const PER_MESSAGE_OVERHEAD_TOKENS: u64 = 8;
const BYTES_PER_TOKEN_MIN: f64 = 2.0;
const BYTES_PER_TOKEN_MAX: f64 = 6.0;

impl Default for TokenLedger {
    fn default() -> Self {
        Self {
            anchor_tokens: 0,
            drift_bytes: 0,
            drift_messages: 0,
            bytes_per_token: 4.0,
        }
    }
}

impl TokenLedger {
    pub fn note_appended(&mut self, message: &ChatMessage) {
        self.drift_bytes += message.approximate_bytes();
        self.drift_messages += 1;
    }

    /// Re-anchor on a provider response and recalibrate the byte/token ratio
    /// from the observed drift.
    pub fn observe_usage(&mut self, usage: &Usage) {
        let actual_drift = usage.input_tokens.saturating_sub(self.anchor_tokens);
        if self.drift_bytes > 0 && actual_drift > 0 {
            let observed = self.drift_bytes as f64 / actual_drift as f64;
            self.bytes_per_token = (0.7 * self.bytes_per_token + 0.3 * observed)
                .clamp(BYTES_PER_TOKEN_MIN, BYTES_PER_TOKEN_MAX);
        }
        self.anchor_tokens = usage.input_tokens + usage.output_tokens;
        self.drift_bytes = 0;
        self.drift_messages = 0;
    }

    /// Reset after a history rewrite: the anchor no longer describes the live
    /// transcript, so estimate it from bytes until the next response
    /// re-grounds it.
    pub fn rebase(&mut self, messages: &[ChatMessage]) {
        let bytes: u64 = messages.iter().map(ChatMessage::approximate_bytes).sum();
        self.anchor_tokens = (bytes as f64 / self.bytes_per_token).ceil() as u64
            + PER_MESSAGE_OVERHEAD_TOKENS * messages.len() as u64;
        self.drift_bytes = 0;
        self.drift_messages = 0;
    }

    pub fn estimated_next_prompt_tokens(&self) -> u64 {
        self.anchor_tokens
            + (self.drift_bytes as f64 / self.bytes_per_token).ceil() as u64
            + PER_MESSAGE_OVERHEAD_TOKENS * self.drift_messages
    }

    pub fn bytes_per_token(&self) -> f64 {
        self.bytes_per_token
    }

    fn tokens_for_bytes(&self, bytes: u64) -> u64 {
        (bytes as f64 / self.bytes_per_token).ceil() as u64
    }
}

#[derive(Debug, Clone)]
pub struct ElisionPlan {
    /// Indices of `ChatMessage::Tool` entries to placeholder.
    pub message_indices: Vec<usize>,
    pub estimated_savings_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct CompactionOutcome {
    pub before_tokens: u64,
    pub after_tokens: u64,
    pub summarizer_usage: Usage,
    pub summarizer_cost: Cost,
    pub retained_messages: usize,
}

#[derive(Debug, Clone)]
pub enum ContextEvent {
    ElisionApplied {
        elided_messages: usize,
        estimated_savings_tokens: u64,
    },
    CompactionCompleted(CompactionOutcome),
}

pub struct ContextManager {
    pub config: ContextConfig,
    pub ledger: TokenLedger,
}

impl ContextManager {
    pub fn new(config: ContextConfig) -> Self {
        Self {
            config,
            ledger: TokenLedger::default(),
        }
    }

    pub fn utilization(&self) -> f64 {
        let window = self.config.effective_window();
        if window == 0 {
            return 1.0;
        }
        self.ledger.estimated_next_prompt_tokens() as f64 / window as f64
    }

    pub fn need(&self) -> CompactionNeed {
        let utilization = self.utilization();
        if utilization >= self.config.urgent_threshold {
            CompactionNeed::Urgent
        } else if utilization >= self.config.compact_threshold {
            CompactionNeed::Compact
        } else if utilization >= self.config.prepare_threshold {
            CompactionNeed::Prepare
        } else {
            CompactionNeed::None
        }
    }

    /// Stage 1: plan a mechanical elision of old tool results. Returns `None`
    /// when the reclaim would be too small to justify breaking the prefix
    /// cache (the chunky rule).
    pub fn plan_elision(&self, messages: &[ChatMessage]) -> Option<ElisionPlan> {
        let protected_from = self.protected_tail_start(messages);
        let mut indices = Vec::new();
        let mut saved_bytes: u64 = 0;
        for (index, message) in messages.iter().enumerate().take(protected_from) {
            if let ChatMessage::Tool { content, .. } = message {
                if content.len() > 200 && !content.starts_with("[elided") {
                    saved_bytes += (content.len() - 80) as u64;
                    indices.push(index);
                }
            }
        }
        let savings_tokens = self.ledger.tokens_for_bytes(saved_bytes);
        let floor =
            (self.config.effective_window() as f64 * self.config.min_savings_fraction) as u64;
        if indices.is_empty() || savings_tokens < floor {
            return None;
        }
        Some(ElisionPlan {
            message_indices: indices,
            estimated_savings_tokens: savings_tokens,
        })
    }

    /// First message index of the elision-protected tail: the last
    /// `protected_tail_tokens` worth of messages, and never less than the
    /// entire latest user turn (its tool results may be needed for the very
    /// next reply). Deliberately NOT a multi-turn floor: when single turns
    /// carry huge tool outputs, a turn-count floor would protect nearly the
    /// whole transcript and starve elision into expensive summarization.
    fn protected_tail_start(&self, messages: &[ChatMessage]) -> usize {
        let mut tail_bytes: u64 = 0;
        let mut user_turns = 0usize;
        let mut start = messages.len();
        for (index, message) in messages.iter().enumerate().rev() {
            if index == 0 {
                break; // never include the system message
            }
            tail_bytes += message.approximate_bytes();
            if matches!(message, ChatMessage::User { .. }) && !is_checkpoint(message) {
                user_turns += 1;
            }
            start = index;
            if self.ledger.tokens_for_bytes(tail_bytes) >= self.config.protected_tail_tokens
                && user_turns >= 1
            {
                break;
            }
        }
        start
    }

    /// Apply a planned elision in place. Only `Tool` message bodies change;
    /// user/assistant text, tool_calls, and reasoning_content are untouched,
    /// so tool_call_id pairing survives by construction.
    pub fn apply_elision(&mut self, messages: &mut [ChatMessage], plan: &ElisionPlan) {
        let previews: Vec<(usize, String)> = plan
            .message_indices
            .iter()
            .map(|&index| (index, describe_elided(messages, index)))
            .collect();
        for (index, preview) in previews {
            if let Some(ChatMessage::Tool { content, .. }) = messages.get_mut(index) {
                *content = preview;
            }
        }
        self.ledger.rebase(messages);
    }

    /// Stage 2: summarization restart. The summarizer request reuses the
    /// current byte-identical history plus one appended instruction, so it
    /// rides the warm prefix cache instead of paying a cold full-window read.
    /// `checkpoint` is the caller's structured continuity state (the tracked
    /// working state); it is data, not summarizer output, so it cannot be
    /// degraded by a weak narrative and it survives earlier compactions.
    #[allow(clippy::too_many_arguments)]
    pub async fn summarize(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        checkpoint: &str,
        focus: Option<&str>,
        tools: &[ToolSchema],
        provider: &dyn Provider,
        model: crate::domain::Model,
        effort: ReasoningEffort,
        cancel: &CancellationToken,
    ) -> Result<CompactionOutcome, ProviderError> {
        let before_tokens = self.ledger.estimated_next_prompt_tokens();
        let mut summarizer_messages = messages.clone();
        let instruction = match focus {
            Some(focus) => format!(
                "{}\n\nUser guidance — emphasize this in the summary:\n{focus}",
                prompt::SUMMARIZE_INSTRUCTION
            ),
            None => prompt::SUMMARIZE_INSTRUCTION.to_string(),
        };
        summarizer_messages.push(ChatMessage::User {
            content: instruction,
        });
        let request = ProviderRequest {
            model,
            messages: summarizer_messages,
            tools: tools.to_vec(),
            // Thinking tokens count against the output budget on DeepSeek
            // reasoning models; give the summarizer headroom so reasoning
            // cannot starve the narrative itself.
            max_output_tokens: self.config.max_summary_tokens.saturating_add(4_096),
            effort,
        };
        let turn = provider.stream(&request, cancel, &mut NullSink).await?;
        let narrative = turn.message.content.unwrap_or_default();

        let tail_start = self.summary_tail_start(messages);
        let mut rebuilt: Vec<ChatMessage> = Vec::with_capacity(messages.len() - tail_start + 2);
        rebuilt.push(messages[0].clone());
        rebuilt.push(ChatMessage::User {
            content: format!(
                "CONTEXT CHECKPOINT — the conversation above this point was compacted.\n\n\
                 {checkpoint}\n\n# Session narrative\n{narrative}\n\n\
                 Continue from this state. Do not re-do verified work."
            ),
        });
        rebuilt.extend_from_slice(&messages[tail_start..]);
        let retained_messages = rebuilt.len();
        *messages = rebuilt;
        self.ledger.rebase(messages);
        Ok(CompactionOutcome {
            before_tokens,
            after_tokens: self.ledger.estimated_next_prompt_tokens(),
            summarizer_usage: turn.usage,
            summarizer_cost: turn.cost,
            retained_messages,
        })
    }

    /// The verbatim tail kept after summarization: up to `keep_last_turns`
    /// user turns, hard-capped by `tail_budget_fraction` of the effective
    /// window so the rebuilt history is guaranteed to land well below the
    /// compact threshold (a turn-count floor here caused summarize thrash:
    /// giant turns kept the rebuilt history over threshold, forcing another
    /// full-miss summarize on every provider call). The tail always starts at
    /// a user message (no orphaned tool results, reasoning_content retained)
    /// and always includes at least the latest user turn. A previous
    /// checkpoint is never counted and never retained — the new checkpoint
    /// supersedes it.
    fn summary_tail_start(&self, messages: &[ChatMessage]) -> usize {
        let tail_budget_tokens =
            (self.config.effective_window() as f64 * self.config.tail_budget_fraction) as u64;
        let mut tail_bytes: u64 = 0;
        let mut user_turns = 0usize;
        // Default to an empty tail; never fall back to a position that would
        // re-include the system message or a superseded checkpoint.
        let mut chosen = messages.len();
        let mut turn_bytes: u64 = 0;
        for (index, message) in messages.iter().enumerate().rev() {
            if index == 0 {
                break;
            }
            if is_checkpoint(message) {
                break;
            }
            turn_bytes += message.approximate_bytes();
            if matches!(message, ChatMessage::User { .. }) {
                // A complete turn (this user message through the last counted
                // message) is included only if it fits the budget — except the
                // latest turn, which is always kept.
                let projected = tail_bytes + turn_bytes;
                if user_turns > 0 && self.ledger.tokens_for_bytes(projected) > tail_budget_tokens {
                    break;
                }
                user_turns += 1;
                tail_bytes = projected;
                turn_bytes = 0;
                chosen = index;
                if user_turns >= self.config.keep_last_turns {
                    break;
                }
            }
        }
        chosen
    }

    /// Halve the retained tail after a provider context-length error.
    pub fn shrink_keep_last_turns(&mut self) {
        self.config.keep_last_turns = (self.config.keep_last_turns / 2).max(1);
    }
}

const CHECKPOINT_PREFIX: &str = "CONTEXT CHECKPOINT";

fn is_checkpoint(message: &ChatMessage) -> bool {
    matches!(message, ChatMessage::User { content } if content.starts_with(CHECKPOINT_PREFIX))
}

/// Placeholder text for an elided tool result: names the originating call so
/// the model can re-run it if the output matters again.
fn describe_elided(messages: &[ChatMessage], tool_index: usize) -> String {
    let (id, size) = match &messages[tool_index] {
        ChatMessage::Tool {
            tool_call_id,
            content,
        } => (tool_call_id.clone(), content.len()),
        _ => return "[elided tool output]".to_string(),
    };
    let call = messages[..tool_index].iter().rev().find_map(|message| {
        if let ChatMessage::Assistant(assistant) = message {
            assistant.tool_calls.iter().find(|call| call.id == id)
        } else {
            None
        }
    });
    match call {
        Some(call) => format!(
            "[elided: output of {} ({size} bytes). Re-run the tool if this output is needed.]",
            crate::tools::describe_call(&call.name, &call.arguments)
        ),
        None => format!("[elided tool output ({size} bytes)]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AssistantMessage, Model, ToolCall};
    use crate::provider::scripted::ScriptedProvider;

    fn tool_call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    fn long_transcript() -> Vec<ChatMessage> {
        let mut messages = vec![ChatMessage::System {
            content: "system".into(),
        }];
        for index in 0..20 {
            messages.push(ChatMessage::User {
                content: format!("user turn {index}"),
            });
            messages.push(ChatMessage::Assistant(AssistantMessage {
                content: None,
                reasoning_content: Some("thinking".into()),
                tool_calls: vec![tool_call(
                    &format!("call_{index}"),
                    "read_file",
                    &format!(r#"{{"path":"file_{index}.rs"}}"#),
                )],
            }));
            messages.push(ChatMessage::Tool {
                tool_call_id: format!("call_{index}"),
                content: "line of output\n".repeat(500),
            });
            messages.push(ChatMessage::Assistant(AssistantMessage {
                content: Some(format!("done with {index}")),
                reasoning_content: None,
                tool_calls: vec![],
            }));
        }
        messages
    }

    #[test]
    fn ledger_anchors_on_usage_and_estimates_drift() {
        let mut ledger = TokenLedger::default();
        ledger.observe_usage(&Usage {
            input_tokens: 10_000,
            cache_hit_input_tokens: 9_000,
            cache_miss_input_tokens: 1_000,
            output_tokens: 500,
        });
        assert_eq!(ledger.estimated_next_prompt_tokens(), 10_500);
        ledger.note_appended(&ChatMessage::User {
            content: "x".repeat(4_000),
        });
        let estimate = ledger.estimated_next_prompt_tokens();
        assert!(estimate > 10_500 + 900 && estimate < 10_500 + 2_100);
        // Calibration clamps stay in range even with absurd observations.
        for _ in 0..50 {
            ledger.note_appended(&ChatMessage::User {
                content: "y".repeat(100_000),
            });
            ledger.observe_usage(&Usage {
                input_tokens: ledger.anchor_tokens + 1,
                ..Usage::default()
            });
        }
        assert!(ledger.bytes_per_token() <= BYTES_PER_TOKEN_MAX);
        assert!(ledger.bytes_per_token() >= BYTES_PER_TOKEN_MIN);
    }

    #[test]
    fn elision_respects_protected_tail_and_savings_floor() {
        let mut manager = ContextManager::new(ContextConfig {
            protected_tail_tokens: 2_000,
            keep_last_turns: 2,
            min_savings_fraction: 0.01,
            ..ContextConfig::default()
        });
        let mut messages = long_transcript();
        let plan = manager.plan_elision(&messages).expect("plan");
        let protected_from = manager.protected_tail_start(&messages);
        assert!(plan.message_indices.iter().all(|&i| i < protected_from));
        // Protected tail keeps at least the last two user turns verbatim.
        let last_user = messages
            .iter()
            .rposition(|m| matches!(m, ChatMessage::User { .. }))
            .unwrap();
        assert!(protected_from <= last_user);

        manager.apply_elision(&mut messages, &plan);
        for &index in &plan.message_indices {
            match &messages[index] {
                ChatMessage::Tool { content, .. } => {
                    assert!(content.starts_with("[elided"), "index {index}: {content}");
                    assert!(content.contains("read_file"));
                }
                other => panic!("expected tool message, got {other:?}"),
            }
        }
        // Assistant reasoning and tool_calls untouched.
        assert!(messages.iter().any(|m| matches!(
            m,
            ChatMessage::Assistant(a) if a.reasoning_content.is_some() && !a.tool_calls.is_empty()
        )));
        // A second plan over the same transcript finds nothing big enough.
        assert!(manager.plan_elision(&messages).is_none());
    }

    #[test]
    fn elision_below_savings_floor_is_rejected() {
        let manager = ContextManager::new(ContextConfig::default());
        let messages = vec![
            ChatMessage::System {
                content: "system".into(),
            },
            ChatMessage::User {
                content: "hi".into(),
            },
            ChatMessage::Tool {
                tool_call_id: "c".into(),
                content: "x".repeat(300),
            },
        ];
        assert!(manager.plan_elision(&messages).is_none());
    }

    #[tokio::test]
    async fn summarization_rebuilds_history_with_invariants() {
        let mut manager = ContextManager::new(ContextConfig {
            keep_last_turns: 2,
            ..ContextConfig::default()
        });
        let mut messages = long_transcript();
        manager.ledger.rebase(&messages);
        let system_before = messages[0].clone();
        let provider = ScriptedProvider::new(vec![AssistantMessage {
            content: Some("the narrative summary".into()),
            reasoning_content: None,
            tool_calls: vec![],
        }]);
        let outcome = manager
            .summarize(
                &mut messages,
                "## Working state (tracked)\nFiles read: file_0.rs",
                None,
                &crate::tools::tool_schemas(),
                &provider,
                Model::DeepseekV4Flash,
                ReasoningEffort::High,
                &CancellationToken::new(),
            )
            .await
            .unwrap();

        // System message bytes unchanged.
        assert_eq!(messages[0], system_before);
        // Exactly one checkpoint message, directly after system.
        let checkpoints = messages
            .iter()
            .filter(|m| matches!(m, ChatMessage::User { content } if content.starts_with("CONTEXT CHECKPOINT")))
            .count();
        assert_eq!(checkpoints, 1);
        match &messages[1] {
            ChatMessage::User { content } => {
                assert!(content.contains("the narrative summary"));
                assert!(content.contains("file_0.rs"));
            }
            other => panic!("expected checkpoint, got {other:?}"),
        }
        // Tail starts at a user message.
        assert!(matches!(messages[2], ChatMessage::User { .. }));
        // Every retained assistant tool-call message kept its reasoning.
        for message in &messages {
            if let ChatMessage::Assistant(assistant) = message {
                if !assistant.tool_calls.is_empty() {
                    assert!(assistant.reasoning_content.is_some());
                }
            }
        }
        // No orphaned tool results: every tool message's call id exists earlier.
        for (index, message) in messages.iter().enumerate() {
            if let ChatMessage::Tool { tool_call_id, .. } = message {
                let paired = messages[..index].iter().any(|m| {
                    matches!(m, ChatMessage::Assistant(a) if a.tool_calls.iter().any(|c| &c.id == tool_call_id))
                });
                assert!(paired, "orphaned tool result at {index}");
            }
        }
        assert!(outcome.after_tokens < outcome.before_tokens);
        // The summarizer request rode the existing history (prefix + 1).
        let recorded = provider.recorded_requests();
        assert_eq!(recorded.len(), 1);
        assert!(matches!(
            recorded[0].last(),
            Some(ChatMessage::User { content }) if content.contains("compacted")
        ));
    }

    #[tokio::test]
    async fn summarize_tail_is_capped_by_token_budget_not_turn_count() {
        // Five giant turns with keep_last_turns=6: a turn-count floor would
        // keep everything and leave the rebuilt history over threshold
        // (summarize thrash). The token budget must cap the tail instead.
        let mut manager = ContextManager::new(ContextConfig {
            window_tokens: 20_000,
            output_reserve_tokens: 0,
            safety_margin_tokens: 0,
            keep_last_turns: 6,
            tail_budget_fraction: 0.3,
            ..ContextConfig::default()
        });
        let mut messages = vec![ChatMessage::System {
            content: "system".into(),
        }];
        for index in 0..5 {
            messages.push(ChatMessage::User {
                content: format!("turn {index}"),
            });
            messages.push(ChatMessage::Assistant(AssistantMessage {
                content: None,
                reasoning_content: Some("r".into()),
                tool_calls: vec![tool_call(
                    &format!("c{index}"),
                    "read_file",
                    r#"{"path":"big.rs"}"#,
                )],
            }));
            messages.push(ChatMessage::Tool {
                tool_call_id: format!("c{index}"),
                content: "x".repeat(30_000), // ~7.5k tokens per turn
            });
        }
        manager.ledger.rebase(&messages);
        let provider = ScriptedProvider::new(vec![AssistantMessage {
            content: Some("summary".into()),
            reasoning_content: None,
            tool_calls: vec![],
        }]);
        let outcome = manager
            .summarize(
                &mut messages,
                "## Working state (tracked)\nFiles read: file_0.rs",
                None,
                &crate::tools::tool_schemas(),
                &provider,
                Model::DeepseekV4Flash,
                ReasoningEffort::High,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        // Only the latest giant turn fits the 6k-token budget.
        let user_turns = messages
            .iter()
            .filter(|m| matches!(m, ChatMessage::User { .. }) && !is_checkpoint(m))
            .count();
        assert_eq!(user_turns, 1, "tail must be budget-capped: {messages:?}");
        assert!(matches!(
            &messages[2],
            ChatMessage::User { content } if content == "turn 4"
        ));
        assert!(outcome.after_tokens < outcome.before_tokens / 2);
    }

    #[tokio::test]
    async fn repeated_summarization_never_stacks_checkpoints() {
        // keep_last_turns exceeds the available user turns: the tail budget is
        // never satisfied, which previously let the old checkpoint be counted
        // as a user turn and retained alongside the new one.
        let mut manager = ContextManager::new(ContextConfig {
            keep_last_turns: 10,
            ..ContextConfig::default()
        });
        let mut messages = vec![
            ChatMessage::System {
                content: "system".into(),
            },
            ChatMessage::User {
                content: "first ask".into(),
            },
            ChatMessage::Assistant(AssistantMessage {
                content: Some("first answer".into()),
                reasoning_content: None,
                tool_calls: vec![],
            }),
        ];
        manager.ledger.rebase(&messages);
        let provider = ScriptedProvider::new(vec![
            AssistantMessage {
                content: Some("narrative one".into()),
                reasoning_content: None,
                tool_calls: vec![],
            },
            AssistantMessage {
                content: Some("narrative two".into()),
                reasoning_content: None,
                tool_calls: vec![],
            },
        ]);
        for _ in 0..2 {
            manager
                .summarize(
                    &mut messages,
                    "checkpoint",
                    None,
                    &crate::tools::tool_schemas(),
                    &provider,
                    Model::DeepseekV4Flash,
                    ReasoningEffort::High,
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
        }
        let checkpoints = messages.iter().filter(|m| is_checkpoint(m)).count();
        assert_eq!(checkpoints, 1, "checkpoints stacked: {messages:?}");
        assert!(matches!(
            &messages[1],
            ChatMessage::User { content } if content.contains("narrative two")
        ));
        // The real conversation tail survived both compactions.
        assert!(messages.iter().any(|m| matches!(
            m,
            ChatMessage::User { content } if content == "first ask"
        )));
    }

    #[test]
    fn thresholds_map_to_needs() {
        let mut manager = ContextManager::new(ContextConfig::default());
        let window = manager.config.effective_window();
        let at = |fraction: f64| Usage {
            input_tokens: (window as f64 * fraction) as u64,
            ..Usage::default()
        };
        manager.ledger.observe_usage(&at(0.5));
        assert_eq!(manager.need(), CompactionNeed::None);
        manager.ledger.observe_usage(&at(0.72));
        assert_eq!(manager.need(), CompactionNeed::Prepare);
        manager.ledger.observe_usage(&at(0.83));
        assert_eq!(manager.need(), CompactionNeed::Compact);
        manager.ledger.observe_usage(&at(0.95));
        assert_eq!(manager.need(), CompactionNeed::Urgent);
    }
}

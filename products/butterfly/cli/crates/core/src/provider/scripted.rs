//! Deterministic fake provider for offline tests. Replays a fixed list of
//! assistant turns and records every request it receives so tests can assert
//! byte-stable prefixes and replay contracts.

use std::sync::Mutex;

use tokio_util::sync::CancellationToken;

use super::{CompletedTurn, FinishReason, Provider, ProviderError, ProviderRequest, StreamSink};
use crate::domain::{price_usage, AssistantMessage, ChatMessage, Model, Usage};

pub struct ScriptedProvider {
    turns: Mutex<std::vec::IntoIter<AssistantMessage>>,
    pub requests: Mutex<Vec<Vec<ChatMessage>>>,
    usage_per_turn: Usage,
    finish_override: Option<FinishReason>,
}

impl ScriptedProvider {
    pub fn new(turns: Vec<AssistantMessage>) -> Self {
        Self {
            turns: Mutex::new(turns.into_iter()),
            requests: Mutex::new(Vec::new()),
            usage_per_turn: Usage {
                input_tokens: 1_000,
                cache_hit_input_tokens: 800,
                cache_miss_input_tokens: 200,
                output_tokens: 100,
            },
            finish_override: None,
        }
    }

    /// Report this finish reason on every response (e.g. `Length` to test
    /// output-truncation surfacing).
    pub fn with_finish(mut self, finish: FinishReason) -> Self {
        self.finish_override = Some(finish);
        self
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage_per_turn = usage;
        self
    }

    pub fn recorded_requests(&self) -> Vec<Vec<ChatMessage>> {
        self.requests.lock().expect("requests lock").clone()
    }
}

#[async_trait::async_trait]
impl Provider for ScriptedProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        _cancel: &CancellationToken,
        sink: &mut dyn StreamSink,
    ) -> Result<CompletedTurn, ProviderError> {
        self.requests
            .lock()
            .expect("requests lock")
            .push(request.messages.clone());
        let message = self
            .turns
            .lock()
            .expect("turns lock")
            .next()
            .ok_or_else(|| ProviderError::Malformed("scripted provider exhausted".into()))?;
        if let Some(text) = &message.content {
            sink.on_text(text);
        }
        let finish = self
            .finish_override
            .unwrap_or(if message.tool_calls.is_empty() {
                FinishReason::Stop
            } else {
                FinishReason::ToolCalls
            });
        let usage = self.usage_per_turn;
        Ok(CompletedTurn {
            cost: price_usage(Model::DeepseekV4Flash, usage),
            message,
            usage,
            finish,
        })
    }
}

//! Deterministic fake provider for offline tests. Replays a fixed list of
//! assistant turns and records every request it receives so tests can assert
//! byte-stable prefixes and replay contracts.

use std::sync::Mutex;

use tokio_util::sync::CancellationToken;

use super::{
    CompletedTurn, FinishReason, Provider, ProviderError, ProviderRequest, RoutingFacts,
    RoutingMetadata, StreamSink, TaskFeedback,
};
use crate::domain::{price_usage, AssistantMessage, ChatMessage, Usage};

pub struct ScriptedProvider {
    turns: Mutex<std::vec::IntoIter<AssistantMessage>>,
    pub requests: Mutex<Vec<Vec<ChatMessage>>>,
    usage_per_turn: Usage,
    finish_override: Option<FinishReason>,
    routing_facts: Option<RoutingFacts>,
    pub routing_requests: Mutex<Vec<Option<RoutingMetadata>>>,
    pub feedback: Mutex<Vec<TaskFeedback>>,
    /// Remaining calls that fail mid-stream with a transport error.
    transport_failures: Mutex<u32>,
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
            routing_facts: None,
            routing_requests: Mutex::new(Vec::new()),
            feedback: Mutex::new(Vec::new()),
            transport_failures: Mutex::new(0),
        }
    }

    /// Fail the next `count` calls with a mid-stream transport error (the
    /// request is still recorded), then behave normally.
    pub fn with_transport_failures(self, count: u32) -> Self {
        *self.transport_failures.lock().expect("failures lock") = count;
        self
    }

    /// Return these routing facts on every turn (a fake router).
    pub fn with_routing_facts(mut self, facts: RoutingFacts) -> Self {
        self.routing_facts = Some(facts);
        self
    }

    pub fn recorded_routing(&self) -> Vec<Option<RoutingMetadata>> {
        self.routing_requests.lock().expect("routing lock").clone()
    }

    pub fn recorded_feedback(&self) -> Vec<TaskFeedback> {
        self.feedback.lock().expect("feedback lock").clone()
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
        self.routing_requests
            .lock()
            .expect("routing lock")
            .push(request.routing.clone());
        {
            let mut failures = self.transport_failures.lock().expect("failures lock");
            if *failures > 0 {
                *failures -= 1;
                return Err(ProviderError::Transport(
                    "error decoding response body".into(),
                ));
            }
        }
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
            cost: price_usage(&request.model.pricing, usage),
            message,
            usage,
            finish,
            latency_ms: 0,
            retries: 0,
            routing: self.routing_facts.clone(),
        })
    }

    async fn report_feedback(&self, feedback: &TaskFeedback) -> Result<(), ProviderError> {
        self.feedback
            .lock()
            .expect("feedback lock")
            .push(feedback.clone());
        Ok(())
    }
}

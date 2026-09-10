//! Model provider abstraction. One OpenAI-compatible implementation,
//! parameterized by provider dialect, talks to every real endpoint; a scripted
//! implementation drives offline tests.

pub mod openai_compat;
pub mod openai_responses;
pub mod scripted;
mod sse;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::domain::{AssistantMessage, ChatMessage, Cost, Usage};
use crate::model::ModelDescriptor;

/// A tool schema as sent to the provider. `parameters` is a JSON Schema value;
/// `serde_json::Value` objects serialize with sorted keys, which keeps the
/// wire bytes deterministic — a hard requirement for prefix caching.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: Arc<ModelDescriptor>,
    /// Full conversation including the system message at index 0.
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSchema>,
    pub max_output_tokens: u32,
    /// Reasoning effort in the model's vocabulary; already validated against
    /// the descriptor. None for models without a reasoning knob.
    pub effort: Option<String>,
    /// Optional routing metadata (contract `nimble/v1`). Only the Nimble
    /// dialect puts it on the wire; every other provider ignores it.
    pub routing: Option<RoutingMetadata>,
}

pub const NIMBLE_CONTRACT_VERSION: &str = "nimble/v1";

/// Butterfly → Nimble request extension (`docs/butterfly-nimble-contract.md`).
/// Every field is optional on the wire; absent fields are omitted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoutingMetadata {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_complexity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_failure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_failed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_prefix_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_stable_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_dynamic_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_version: Option<u32>,
}

/// Nimble → Butterfly routing facts, from the `x-nimble-routing` header or the
/// first stream chunk's `nimble` field. Nothing here may instruct the agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoutingFacts {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub selected_model: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model_changed: bool,
    #[serde(default)]
    pub cache_affinity: f64,
    #[serde(default)]
    pub estimated_cached_tokens: u64,
    #[serde(default)]
    pub context_limit: u64,
    #[serde(default)]
    pub routing_reason: String,
    #[serde(default)]
    pub decision_id: Option<String>,
    #[serde(default)]
    pub request_id: String,
}

/// Butterfly → Nimble task outcome (`POST /v1/feedback`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskFeedback {
    pub version: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_microusd: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    Other,
}

#[derive(Debug, Clone)]
pub struct CompletedTurn {
    pub message: AssistantMessage,
    pub usage: Usage,
    pub cost: Cost,
    pub finish: FinishReason,
    /// Wall-clock time of the whole request, including retries.
    pub latency_ms: u64,
    /// HTTP attempts beyond the first that this request needed.
    pub retries: u32,
    /// Routing facts when the provider is a router that returned them.
    pub routing: Option<RoutingFacts>,
}

/// Receives streaming deltas as they arrive. Methods are synchronous and must
/// be cheap (terminal rendering, buffering). Tool hooks are called by the
/// session loop, not by providers.
pub trait StreamSink: Send {
    fn on_text(&mut self, delta: &str);
    fn on_reasoning(&mut self, delta: &str);
    fn on_tool_start(&mut self, preview: &str) {
        let _ = preview;
    }
    /// `error` carries the failure's first line for display; None on success
    /// (and on cancellation, which the caller narrates itself).
    fn on_tool_result(&mut self, preview: &str, is_error: bool, error: Option<&str>) {
        let _ = (preview, is_error, error);
    }
    /// Harness-level notice (e.g. a stream retry); never model output.
    fn on_notice(&mut self, text: &str) {
        let _ = text;
    }
}

/// A sink that discards deltas (used by the compaction summarizer).
pub struct NullSink;

impl StreamSink for NullSink {
    fn on_text(&mut self, _delta: &str) {}
    fn on_reasoning(&mut self, _delta: &str) {}
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("request cancelled")]
    Cancelled,
    #[error("provider returned status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("provider rate limited the request (status {status}): {body}")]
    RateLimited { status: u16, body: String },
    #[error("context length exceeded")]
    ContextLengthExceeded,
    #[error("transport error: {0}")]
    Transport(String),
    #[error("malformed provider response: {0}")]
    Malformed(String),
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// Stream one completion. Deltas go to `sink`; the fully assembled turn is
    /// the return value. Implementations must honor `cancel` promptly.
    async fn stream(
        &self,
        request: &ProviderRequest,
        cancel: &CancellationToken,
        sink: &mut dyn StreamSink,
    ) -> Result<CompletedTurn, ProviderError>;

    /// Report a task outcome to a router that learns from it. Fire-and-forget
    /// semantics: failures are logged by the caller, never fatal. Providers
    /// that are not routers accept and ignore it.
    async fn report_feedback(&self, feedback: &TaskFeedback) -> Result<(), ProviderError> {
        let _ = feedback;
        Ok(())
    }
}

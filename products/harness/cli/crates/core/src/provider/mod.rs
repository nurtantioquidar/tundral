//! Model provider abstraction. One implementation talks to DeepSeek; a
//! scripted implementation drives offline tests.

pub mod deepseek;
pub mod scripted;
mod sse;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::domain::{AssistantMessage, ChatMessage, Cost, Model, Usage};

/// A tool schema as sent to the provider. `parameters` is a JSON Schema value;
/// `serde_json::Value` objects serialize with sorted keys, which keeps the
/// wire bytes deterministic — a hard requirement for prefix caching.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEffort {
    High,
    Max,
}

impl ReasoningEffort {
    pub fn wire_name(self) -> &'static str {
        match self {
            ReasoningEffort::High => "high",
            ReasoningEffort::Max => "max",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: Model,
    /// Full conversation including the system message at index 0.
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSchema>,
    pub max_output_tokens: u32,
    pub effort: ReasoningEffort,
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
    fn on_tool_result(&mut self, preview: &str, is_error: bool) {
        let _ = (preview, is_error);
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
}

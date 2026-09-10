//! OpenAI-compatible Chat Completions adapter (streaming), parameterized by
//! provider dialect. DeepSeek, GLM, OpenAI-style endpoints (Luna via Inver),
//! and Nimble all share the request/response shape; the dialect decides the
//! reasoning syntax, which usage fields carry cache counters, and how errors
//! are classified.
//!
//! Prefix caching on these providers is automatic and keyed on byte-stable
//! request prefixes, so everything here serializes deterministically: struct
//! field order is fixed by construction and `serde_json::Value` objects render
//! with sorted keys. The DeepSeek dialect produces byte-for-byte the request
//! Butterfly sent before this adapter existed.

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::sse::SseParser;
use super::{
    CompletedTurn, FinishReason, Provider, ProviderError, ProviderRequest, RoutingFacts,
    StreamSink, TaskFeedback, ToolSchema,
};
use crate::domain::{price_usage, AssistantMessage, ChatMessage, ToolCall, Usage};
use crate::model::{Dialect, ModelDescriptor, ProviderDescriptor, ReasoningReplay};

/// HTTP-level attempts per request (connect failures, 5xx, 429). The backoff
/// below sums to about a minute: long enough to ride out a network blip or a
/// router's own failover, short enough that a dead provider still fails fast
/// relative to a task. Observed 2026-09-06: a host network drop of a few
/// minutes ended three trials whose retries gave up within five seconds.
const MAX_ATTEMPTS: u32 = 6;

pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    dialect: Dialect,
}

impl OpenAiCompatProvider {
    /// Build from a provider descriptor. `base_url_override` (a flag) beats the
    /// descriptor's `base_url_env`, which beats the descriptor default.
    pub fn from_descriptor(
        provider: &ProviderDescriptor,
        api_key: String,
        base_url_override: Option<String>,
    ) -> Result<Self, String> {
        let from_env = provider
            .base_url_env
            .as_deref()
            .and_then(|name| std::env::var(name).ok())
            .filter(|value| !value.trim().is_empty());
        let base_url = base_url_override
            .or(from_env)
            .unwrap_or_else(|| provider.base_url.clone());
        if base_url.trim().is_empty() {
            return Err(format!(
                "provider '{}' has no endpoint: set {} or base_url in config",
                provider.id,
                provider
                    .base_url_env
                    .as_deref()
                    .unwrap_or("[[providers]].base_url")
            ));
        }
        Ok(Self::new(api_key, base_url, provider.dialect))
    }

    pub fn new(api_key: String, base_url: String, dialect: Dialect) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest client");
        Self {
            client,
            api_key,
            base_url,
            dialect,
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    /// POST `body` to `url` with the shared retry policy (408/409/425/429/5xx
    /// and transport errors, bounded backoff). Returns the successful response
    /// and how many retries it took.
    pub(crate) async fn post_with_retries(
        &self,
        url: &str,
        body: &Value,
        cancel: &CancellationToken,
    ) -> Result<(reqwest::Response, u32), ProviderError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let send = self
                .client
                .post(url)
                .bearer_auth(&self.api_key)
                .json(body)
                .send();
            let result = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                result = tokio::time::timeout(FIRST_BYTE_TIMEOUT, send) => match result {
                    Ok(result) => result,
                    Err(_elapsed) => {
                        if attempt < MAX_ATTEMPTS {
                            continue;
                        }
                        return Err(ProviderError::Transport(format!(
                            "no response headers within {}s",
                            FIRST_BYTE_TIMEOUT.as_secs()
                        )));
                    }
                },
            };
            match result {
                Ok(response) if response.status().is_success() => {
                    return Ok((response, attempt - 1));
                }
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body_text = response.text().await.unwrap_or_default();
                    if is_context_length_error(status, &body_text) {
                        return Err(ProviderError::ContextLengthExceeded);
                    }
                    // An exhausted balance arrives as 429 (Z.ai code 1113) or 402
                    // (DeepSeek): waiting will not refill it, so fail at once.
                    if is_balance_error(&body_text) {
                        return Err(ProviderError::Status {
                            status,
                            body: truncate_body(&body_text),
                        });
                    }
                    if is_retryable_status(status) && attempt < MAX_ATTEMPTS {
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }
                    let body = truncate_body(&body_text);
                    return Err(if is_rate_limit_status(status) {
                        ProviderError::RateLimited { status, body }
                    } else {
                        ProviderError::Status { status, body }
                    });
                }
                Err(error) => {
                    if attempt < MAX_ATTEMPTS {
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }
                    return Err(ProviderError::Transport(error.to_string()));
                }
            }
        }
    }
}

/// Routing facts a router may attach as a response header.
pub(crate) fn routing_facts_from_headers(
    headers: &reqwest::header::HeaderMap,
) -> Option<RoutingFacts> {
    headers
        .get("x-nimble-routing")
        .and_then(|value| value.to_str().ok())
        .and_then(|text| serde_json::from_str::<RoutingFacts>(text).ok())
}

/// Serialize the conversation. Key order within each message is deterministic
/// (Value maps sort keys), and a message's bytes never depend on anything
/// outside the message itself — the properties the prefix cache relies on.
pub fn wire_messages(messages: &[ChatMessage], replay: ReasoningReplay) -> Vec<Value> {
    wire_messages_with(messages, replay, false)
}

/// `include_replay` adds the opaque `nimble_replay` items a router returned
/// (Nimble dialect only; other servers would reject the unknown field).
pub fn wire_messages_with(
    messages: &[ChatMessage],
    replay: ReasoningReplay,
    include_replay: bool,
) -> Vec<Value> {
    messages
        .iter()
        .map(|message| match message {
            ChatMessage::System { content } => json!({"role": "system", "content": content}),
            ChatMessage::User { content } => json!({"role": "user", "content": content}),
            ChatMessage::Assistant(m) => wire_assistant(m, replay, include_replay),
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => json!({"role": "tool", "tool_call_id": tool_call_id, "content": content}),
        })
        .collect()
}

fn wire_assistant(
    message: &AssistantMessage,
    replay: ReasoningReplay,
    include_replay: bool,
) -> Value {
    let mut value = json!({
        "role": "assistant",
        "content": message.content,
    });
    let object = value.as_object_mut().expect("assistant message object");
    if include_replay {
        if let Some(state) = &message.replay_state {
            object.insert("nimble_replay".to_string(), state.clone());
        }
    }
    if !message.tool_calls.is_empty() {
        match replay {
            // Thinking models that require the echo: absent reasoning is
            // normalized to null, never omitted.
            ReasoningReplay::Required => {
                object.insert(
                    "reasoning_content".to_string(),
                    match &message.reasoning_content {
                        Some(text) => Value::String(text.clone()),
                        None => Value::Null,
                    },
                );
            }
            ReasoningReplay::Optional => {
                if let Some(text) = &message.reasoning_content {
                    object.insert("reasoning_content".to_string(), Value::String(text.clone()));
                }
            }
            ReasoningReplay::Unsupported => {}
        }
        let calls: Vec<Value> = message
            .tool_calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments},
                })
            })
            .collect();
        object.insert("tool_calls".to_string(), Value::Array(calls));
    }
    value
}

fn wire_tools(tools: &[ToolSchema]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                },
            })
        })
        .collect()
}

pub fn request_body(request: &ProviderRequest, dialect: Dialect) -> Value {
    let model: &ModelDescriptor = &request.model;
    let mut body = json!({
        "model": model.id,
        "messages": wire_messages_with(
            &request.messages,
            model.reasoning_replay(),
            dialect == Dialect::Nimble,
        ),
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    let object = body.as_object_mut().expect("request object");
    // OpenAI reasoning models reject `max_tokens` ("use max_completion_tokens
    // instead", verified live 2026-09-04); DeepSeek and GLM take `max_tokens`.
    // Nimble receives `max_tokens` and renames per upstream itself.
    let budget_key = match dialect {
        Dialect::OpenAi | Dialect::OpenAiResponses => "max_completion_tokens",
        Dialect::DeepSeek | Dialect::Glm | Dialect::Nimble => "max_tokens",
    };
    object.insert(budget_key.into(), json!(request.max_output_tokens));
    if model.capabilities.tools && !request.tools.is_empty() {
        object.insert("tools".into(), Value::Array(wire_tools(&request.tools)));
        object.insert("tool_choice".into(), Value::String("auto".into()));
    }
    if model.supports_reasoning() {
        match dialect {
            Dialect::DeepSeek => {
                object.insert("thinking".into(), json!({"type": "enabled"}));
            }
            // GLM-5.3-Flash: thinking.type only supports "enabled";
            // clear_thinking=false keeps reasoning across turns (Z.ai docs).
            Dialect::Glm => {
                object.insert(
                    "thinking".into(),
                    json!({"type": "enabled", "clear_thinking": false}),
                );
            }
            Dialect::OpenAi | Dialect::OpenAiResponses | Dialect::Nimble => {}
        }
        if let Some(effort) = &request.effort {
            object.insert("reasoning_effort".into(), Value::String(effort.clone()));
        }
    }
    if dialect == Dialect::Nimble {
        if let Some(routing) = &request.routing {
            if let Ok(value) = serde_json::to_value(routing) {
                object.insert("nimble".into(), value);
            }
        }
    }
    body
}

/// Assembles streamed chunks into a completed turn. Pure and synchronous so
/// fixture tests can drive it without a network.
#[derive(Debug, Default)]
pub struct TurnAssembler {
    content: String,
    reasoning: String,
    tool_calls: Vec<PartialToolCall>,
    finish: Option<FinishReason>,
    usage: Option<Usage>,
    /// Routing facts from a router's first chunk (or the response header).
    pub routing: Option<RoutingFacts>,
    /// Opaque replay items a router attached (`nimble_replay`).
    replay: Option<Value>,
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug)]
pub enum ChunkDelta {
    Text(String),
    Reasoning(String),
    None,
}

impl TurnAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one parsed SSE chunk. Returns any visible delta for rendering.
    pub fn consume(&mut self, chunk: &Value) -> Result<ChunkDelta, ProviderError> {
        // An in-stream error object (OpenAI-compatible servers and routers emit
        // `{"error": {...}}` when the upstream fails mid-stream) is a failed
        // request, never an empty answer.
        if let Some(error) = chunk.get("error").filter(|e| e.is_object()) {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("upstream error in stream")
                .to_string();
            // A rate limit relayed inside a 200 stream (a router or proxy
            // that committed its response before the upstream said 429) is
            // still a rate limit: retryable, with the upstream's hint.
            if is_rate_limit_message(&message) {
                return Err(ProviderError::RateLimited {
                    status: 200,
                    body: message,
                });
            }
            return Err(ProviderError::Status {
                status: 200,
                body: message,
            });
        }
        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some(parse_usage(usage));
        }
        if let Some(facts) = chunk.get("nimble").filter(|n| n.is_object()) {
            if let Ok(parsed) = serde_json::from_value::<RoutingFacts>(facts.clone()) {
                self.routing = Some(parsed);
            }
        }
        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return Ok(ChunkDelta::None);
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(match reason {
                "stop" => FinishReason::Stop,
                "tool_calls" => FinishReason::ToolCalls,
                "length" => FinishReason::Length,
                _ => FinishReason::Other,
            });
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(ChunkDelta::None);
        };
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                self.consume_tool_call_fragment(call)?;
            }
        }
        if let Some(replay) = delta.get("nimble_replay").filter(|r| r.is_array()) {
            self.replay = Some(replay.clone());
        }
        // DeepSeek and GLM stream `reasoning_content`; some OpenAI-compatible
        // servers stream `reasoning`. Accept both.
        let reasoning = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .and_then(Value::as_str);
        if let Some(text) = reasoning {
            if !text.is_empty() {
                self.reasoning.push_str(text);
                return Ok(ChunkDelta::Reasoning(text.to_string()));
            }
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                self.content.push_str(text);
                return Ok(ChunkDelta::Text(text.to_string()));
            }
        }
        Ok(ChunkDelta::None)
    }

    fn consume_tool_call_fragment(&mut self, fragment: &Value) -> Result<(), ProviderError> {
        let index = fragment
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| ProviderError::Malformed("tool call fragment without index".into()))?
            as usize;
        while self.tool_calls.len() <= index {
            self.tool_calls.push(PartialToolCall::default());
        }
        let partial = &mut self.tool_calls[index];
        if let Some(id) = fragment.get("id").and_then(Value::as_str) {
            partial.id.push_str(id);
        }
        if let Some(function) = fragment.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                partial.name.push_str(name);
            }
            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                partial.arguments.push_str(arguments);
            }
        }
        Ok(())
    }

    pub fn finish(
        self,
        model: &ModelDescriptor,
        latency_ms: u64,
        retries: u32,
    ) -> Result<CompletedTurn, ProviderError> {
        let tool_calls: Vec<ToolCall> = self
            .tool_calls
            .into_iter()
            .map(|partial| {
                if partial.id.is_empty() || partial.name.is_empty() {
                    return Err(ProviderError::Malformed(
                        "incomplete streamed tool call".into(),
                    ));
                }
                Ok(ToolCall {
                    id: partial.id,
                    name: partial.name,
                    arguments: partial.arguments,
                })
            })
            .collect::<Result<_, _>>()?;
        let usage = self.usage.unwrap_or_default();
        let cost = price_usage(&model.pricing, usage);
        let finish = self.finish.unwrap_or(FinishReason::Other);
        Ok(CompletedTurn {
            message: AssistantMessage {
                content: if self.content.is_empty() {
                    None
                } else {
                    Some(self.content)
                },
                reasoning_content: if self.reasoning.is_empty() {
                    None
                } else {
                    Some(self.reasoning)
                },
                tool_calls,
                replay_state: self.replay,
            },
            usage,
            cost,
            finish,
            latency_ms,
            retries,
            routing: self.routing,
        })
    }
}

/// Parse usage across dialects. DeepSeek reports
/// `prompt_cache_hit_tokens`/`prompt_cache_miss_tokens`; OpenAI-style servers
/// (and GLM) report `prompt_tokens_details.cached_tokens`. Miss is derived as
/// input − hit when the provider does not state it.
pub fn parse_usage(usage: &Value) -> Usage {
    let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or_default();
    let input_tokens = field("prompt_tokens");
    let cache_hit = usage
        .get("prompt_cache_hit_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(Value::as_u64)
        })
        .unwrap_or_default()
        .min(input_tokens);
    let cache_miss = usage
        .get("prompt_cache_miss_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| input_tokens.saturating_sub(cache_hit));
    Usage {
        input_tokens,
        cache_hit_input_tokens: cache_hit,
        cache_miss_input_tokens: cache_miss,
        output_tokens: field("completion_tokens"),
    }
}

pub fn is_context_length_error(status: u16, body: &str) -> bool {
    let lowered = body.to_ascii_lowercase();
    status == 400
        && ((lowered.contains("context") && lowered.contains("length"))
            || lowered.contains("context_length_exceeded")
            || lowered.contains("maximum context")
            || lowered.contains("too many tokens"))
}

fn is_rate_limit_status(status: u16) -> bool {
    status == 429
}

/// Account-balance refusals, which some vendors send with a rate-limit status.
pub(crate) fn is_balance_error(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("insufficient balance")
        || lower.contains("insufficient_quota")
        || lower.contains("please recharge")
        || lower.contains("no resource package")
        || lower.contains("\"code\":\"1113\"")
}

/// Upstream rate-limit text, however it was relayed.
pub(crate) fn is_rate_limit_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("too many requests")
        || lower.contains("tokens per min")
        || lower.contains("requests per min")
}

/// "Please try again in 5.323s" / "retry after 12 seconds" → the wait the
/// upstream asked for, capped by the caller.
pub(crate) fn retry_after_hint(message: &str) -> Option<std::time::Duration> {
    let lower = message.to_ascii_lowercase();
    for marker in ["try again in ", "retry after ", "retry in "] {
        if let Some(index) = lower.find(marker) {
            let rest = &lower[index + marker.len()..];
            let number: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(seconds) = number.parse::<f64>() {
                let unit = rest[number.len()..].trim_start();
                let millis = if unit.starts_with("ms") {
                    seconds
                } else if unit.starts_with('m') && !unit.starts_with("ms") {
                    seconds * 60_000.0
                } else {
                    seconds * 1000.0
                };
                return Some(std::time::Duration::from_millis(millis.max(0.0) as u64));
            }
        }
    }
    None
}

/// Longest silence tolerated inside a response stream before the call is
/// abandoned as a transport failure (and retried by the session). Reasoning
/// models stream their thinking, so a healthy upstream is never quiet this
/// long. Observed 2026-09-05: Z.ai stalls of 10+ minutes left two trials
/// waiting on a dead connection with no timeout at all.
pub(crate) const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
/// Idle allowance for models that think in silence (no reasoning stream) and
/// for a router, whose silence may be an upstream thinking in silence.
pub(crate) const SILENT_STREAM_IDLE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(900);

/// How long this model's stream may stay silent before it is presumed dead.
pub(crate) fn idle_timeout_for(model: &ModelDescriptor, dialect: Dialect) -> std::time::Duration {
    let silent_thinker = model
        .capabilities
        .reasoning
        .as_ref()
        .is_some_and(|r| !r.streamed);
    if silent_thinker || dialect == Dialect::Nimble {
        SILENT_STREAM_IDLE_TIMEOUT
    } else {
        STREAM_IDLE_TIMEOUT
    }
}
/// Longest wait for response headers after the request is sent.
pub(crate) const FIRST_BYTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Next chunk of a response stream, or a transport error after `STREAM_IDLE_TIMEOUT`
/// of silence. Cancellation wins over both.
pub(crate) async fn next_chunk<S, B>(
    stream: &mut S,
    cancel: &CancellationToken,
    idle: std::time::Duration,
) -> Result<Option<B>, ProviderError>
where
    S: futures_util::Stream<Item = reqwest::Result<B>> + Unpin,
{
    use futures_util::StreamExt as _;
    tokio::select! {
        _ = cancel.cancelled() => Err(ProviderError::Cancelled),
        next = tokio::time::timeout(idle, stream.next()) => match next {
            Err(_elapsed) => Err(ProviderError::Transport(format!(
                "stream idle for {}s",
                idle.as_secs()
            ))),
            Ok(None) => Ok(None),
            Ok(Some(Err(error))) => Err(ProviderError::Transport(error.to_string())),
            Ok(Some(Ok(chunk))) => Ok(Some(chunk)),
        },
    }
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 425 | 429) || status >= 500
}

#[async_trait::async_trait]
impl Provider for OpenAiCompatProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        cancel: &CancellationToken,
        sink: &mut dyn StreamSink,
    ) -> Result<CompletedTurn, ProviderError> {
        if self.dialect == Dialect::OpenAiResponses {
            return super::openai_responses::stream(self, request, cancel, sink).await;
        }
        let started = std::time::Instant::now();
        let body = request_body(request, self.dialect);
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let (response, retries) = self.post_with_retries(&url, &body, cancel).await?;

        let mut assembler = TurnAssembler::new();
        if self.dialect == Dialect::Nimble {
            assembler.routing = routing_facts_from_headers(response.headers());
        }
        let mut parser = SseParser::new();
        let mut byte_stream = response.bytes_stream();
        loop {
            let idle = idle_timeout_for(&request.model, self.dialect);
            let Some(chunk) = next_chunk(&mut byte_stream, cancel, idle).await? else {
                break;
            };
            for payload in parser.feed(&chunk) {
                if payload == "[DONE]" {
                    return assembler.finish(
                        &request.model,
                        started.elapsed().as_millis() as u64,
                        retries,
                    );
                }
                let value: Value = serde_json::from_str(&payload)
                    .map_err(|error| ProviderError::Malformed(error.to_string()))?;
                match assembler.consume(&value)? {
                    ChunkDelta::Text(text) => sink.on_text(&text),
                    ChunkDelta::Reasoning(text) => sink.on_reasoning(&text),
                    ChunkDelta::None => {}
                }
            }
        }
        assembler.finish(
            &request.model,
            started.elapsed().as_millis() as u64,
            retries,
        )
    }

    async fn report_feedback(&self, feedback: &TaskFeedback) -> Result<(), ProviderError> {
        FeedbackReporter::report_feedback(self, feedback).await
    }
}

#[async_trait::async_trait]
impl FeedbackReporter for OpenAiCompatProvider {
    async fn report_feedback(&self, feedback: &TaskFeedback) -> Result<(), ProviderError> {
        if self.dialect != Dialect::Nimble {
            return Ok(());
        }
        let url = format!("{}/feedback", self.base_url.trim_end_matches('/'));
        let send = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(feedback)
            .timeout(std::time::Duration::from_secs(3))
            .send();
        match send.await {
            Ok(response) if response.status().is_success() => Ok(()),
            Ok(response) => Err(ProviderError::Status {
                status: response.status().as_u16(),
                body: truncate_body(&response.text().await.unwrap_or_default()),
            }),
            Err(error) => Err(ProviderError::Transport(error.to_string())),
        }
    }
}

/// Split out so the `Provider` impl can delegate without duplicating the
/// dialect check.
#[async_trait::async_trait]
trait FeedbackReporter {
    async fn report_feedback(&self, feedback: &TaskFeedback) -> Result<(), ProviderError>;
}

fn backoff(attempt: u32) -> std::time::Duration {
    // 0.5s, 1s, 2s, 4s, 8s, 16s (capped at 30s): ~31s across MAX_ATTEMPTS.
    std::time::Duration::from_millis((500 * 2u64.pow(attempt.saturating_sub(1).min(6))).min(30_000))
}

fn truncate_body(body: &str) -> String {
    const MAX: usize = 2_000;
    if body.len() <= MAX {
        body.to_string()
    } else {
        let mut end = MAX;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &body[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{reference_model, Registry};

    fn assembled(payloads: &[&str]) -> CompletedTurn {
        let mut assembler = TurnAssembler::new();
        for payload in payloads {
            let value: Value = serde_json::from_str(payload).unwrap();
            assembler.consume(&value).unwrap();
        }
        assembler.finish(&reference_model(), 12, 0).unwrap()
    }

    fn sample_request(model: std::sync::Arc<ModelDescriptor>) -> ProviderRequest {
        let effort = model.resolve_effort(None).unwrap();
        ProviderRequest {
            model,
            messages: vec![
                ChatMessage::System {
                    content: "sys".into(),
                },
                ChatMessage::User {
                    content: "hello".into(),
                },
            ],
            tools: vec![ToolSchema {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
            }],
            max_output_tokens: 1024,
            effort,
            routing: None,
        }
    }

    #[test]
    fn assembles_fragmented_tool_calls_and_usage() {
        let turn = assembled(&[
            r#"{"choices":[{"delta":{"reasoning_content":"thinking..."}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"pa"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a.rs\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":100,"prompt_cache_hit_tokens":60,"prompt_cache_miss_tokens":40,"completion_tokens":20}}"#,
        ]);
        assert_eq!(turn.finish, FinishReason::ToolCalls);
        assert_eq!(turn.message.tool_calls.len(), 1);
        let call = &turn.message.tool_calls[0];
        assert_eq!(call.id, "call_1");
        assert_eq!(call.name, "read_file");
        assert_eq!(call.arguments, r#"{"path":"a.rs"}"#);
        assert_eq!(
            turn.message.reasoning_content.as_deref(),
            Some("thinking...")
        );
        assert_eq!(turn.usage.input_tokens, 100);
        assert_eq!(turn.usage.cache_hit_input_tokens, 60);
        assert_eq!(turn.usage.cache_miss_input_tokens, 40);
        assert_eq!(turn.usage.output_tokens, 20);
        assert!(turn.cost.actual_microusd > 0);
        assert_eq!(turn.latency_ms, 12);
    }

    #[test]
    fn usage_parses_deepseek_and_openai_cache_fields() {
        let deepseek = parse_usage(&serde_json::json!({
            "prompt_tokens": 100, "prompt_cache_hit_tokens": 30, "completion_tokens": 5
        }));
        assert_eq!(deepseek.cache_hit_input_tokens, 30);
        assert_eq!(deepseek.cache_miss_input_tokens, 70);
        let openai = parse_usage(&serde_json::json!({
            "prompt_tokens": 100, "completion_tokens": 5,
            "prompt_tokens_details": {"cached_tokens": 64}
        }));
        assert_eq!(openai.cache_hit_input_tokens, 64);
        assert_eq!(openai.cache_miss_input_tokens, 36);
        let bare = parse_usage(&serde_json::json!({"prompt_tokens": 10, "completion_tokens": 1}));
        assert_eq!(bare.cache_hit_input_tokens, 0);
        assert_eq!(bare.cache_miss_input_tokens, 10);
    }

    #[test]
    fn reasoning_delta_accepts_both_field_names() {
        let turn = assembled(&[
            r#"{"choices":[{"delta":{"reasoning":"hmm "}}]}"#,
            r#"{"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
        ]);
        assert_eq!(turn.message.reasoning_content.as_deref(), Some("hmm "));
        assert_eq!(turn.message.content.as_deref(), Some("ok"));
    }

    #[test]
    fn reasoning_replay_policy_controls_the_echo() {
        let with_calls = AssistantMessage {
            content: None,
            reasoning_content: Some("because".into()),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
            }],
            replay_state: None,
        };
        let without_reasoning = AssistantMessage {
            reasoning_content: None,
            content: Some("hi".into()),
            ..with_calls.clone()
        };
        // Required: always present, null when absent.
        let a = wire_assistant(&with_calls, ReasoningReplay::Required, false);
        assert_eq!(a["reasoning_content"], "because");
        let b = wire_assistant(&without_reasoning, ReasoningReplay::Required, false);
        assert!(b["reasoning_content"].is_null());
        // Optional: present only when there is something to echo.
        let c = wire_assistant(&with_calls, ReasoningReplay::Optional, false);
        assert_eq!(c["reasoning_content"], "because");
        let d = wire_assistant(&without_reasoning, ReasoningReplay::Optional, false);
        assert!(d.get("reasoning_content").is_none());
        // Unsupported: never.
        let e = wire_assistant(&with_calls, ReasoningReplay::Unsupported, false);
        assert!(e.get("reasoning_content").is_none());
        // Plain assistant text omits the field entirely regardless.
        let plain = wire_assistant(
            &AssistantMessage {
                content: Some("hi".into()),
                reasoning_content: Some("thought".into()),
                tool_calls: vec![],
                replay_state: None,
            },
            ReasoningReplay::Required,
            false,
        );
        assert!(plain.get("reasoning_content").is_none());
    }

    #[test]
    fn deepseek_request_body_matches_the_historical_shape() {
        let request = sample_request(reference_model());
        let a = serde_json::to_string(&request_body(&request, Dialect::DeepSeek)).unwrap();
        let b = serde_json::to_string(&request_body(&request, Dialect::DeepSeek)).unwrap();
        assert_eq!(a, b, "request bytes must be deterministic");
        // Exactly the keys the pre-descriptor DeepSeek adapter sent, with the
        // same values: this is the cross-session cache prefix contract.
        let body = request_body(&request, Dialect::DeepSeek);
        let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "max_tokens",
                "messages",
                "model",
                "reasoning_effort",
                "stream",
                "stream_options",
                "thinking",
                "tool_choice",
                "tools"
            ]
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>()
            .iter()
            .collect::<Vec<_>>()
        );
        assert_eq!(body["model"], "deepseek-v4-flash");
        assert_eq!(body["thinking"], serde_json::json!({"type": "enabled"}));
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["max_tokens"], 1024);
    }

    #[test]
    fn dialects_emit_their_own_reasoning_syntax() {
        let registry = Registry::builtin();
        let luna = registry.resolve("gpt-5.6-luna").unwrap();
        let body = request_body(&sample_request(luna), Dialect::OpenAi);
        assert_eq!(body["max_completion_tokens"], 1024);
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["max_completion_tokens"], 1024);
        assert!(
            body.get("max_tokens").is_none(),
            "OpenAI reasoning models reject max_tokens"
        );
        assert!(
            body.get("thinking").is_none(),
            "OpenAI dialect has no thinking object"
        );
        assert_eq!(body["reasoning_effort"], "high");
        let glm = registry.resolve("glm-5.3-flash").unwrap();
        let body = request_body(&sample_request(glm), Dialect::Glm);
        assert_eq!(
            body["thinking"],
            serde_json::json!({"type": "enabled", "clear_thinking": false})
        );
        assert!(
            body.get("reasoning_effort").is_none(),
            "GLM has no effort vocabulary yet"
        );
        let plain = std::sync::Arc::new(crate::model::builtin::generic_openai_compatible(
            "m", "deepseek",
        ));
        let body = request_body(&sample_request(plain), Dialect::DeepSeek);
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn nimble_dialect_carries_metadata_out_and_facts_in() {
        let registry = Registry::builtin();
        let nimble = registry.resolve("nimble").unwrap();
        let mut request = sample_request(nimble);
        request.routing = Some(super::super::RoutingMetadata {
            version: super::super::NIMBLE_CONTRACT_VERSION.into(),
            session_id: Some("abc".into()),
            attempt: Some(2),
            previous_failure: Some("test_failure".into()),
            verification_failed: Some(true),
            ..Default::default()
        });
        let body = request_body(&request, Dialect::Nimble);
        assert_eq!(body["nimble"]["session_id"], "abc");
        assert_eq!(body["nimble"]["attempt"], 2);
        assert_eq!(body["nimble"]["version"], "nimble/v1");
        assert!(
            body["nimble"].get("task_type").is_none(),
            "absent fields are omitted"
        );
        // Other dialects never leak the extension.
        let plain = request_body(&request, Dialect::OpenAi);
        assert!(plain.get("nimble").is_none());

        let mut assembler = TurnAssembler::new();
        let chunk: Value = serde_json::from_str(
            r#"{"choices":[{"delta":{"content":"hi"}}],"nimble":{"version":"nimble/v1","selected_model":"deepseek-v4-flash","provider":"deepseek","model_changed":true,"cache_affinity":0.9,"estimated_cached_tokens":100,"context_limit":1000000,"routing_reason":"escalated","request_id":"r1"}}"#,
        )
        .unwrap();
        assembler.consume(&chunk).unwrap();
        let turn = assembler.finish(&reference_model(), 1, 0).unwrap();
        let facts = turn.routing.expect("facts parsed");
        assert_eq!(facts.selected_model, "deepseek-v4-flash");
        assert!(facts.model_changed);
        assert_eq!(facts.context_limit, 1_000_000);
    }

    #[test]
    fn nimble_replay_items_round_trip_only_on_the_nimble_dialect() {
        let mut assembler = TurnAssembler::new();
        for payload in [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"nimble_replay":[{"type":"reasoning","id":"rs_1","encrypted_content":"ENC","summary":[]}]},"finish_reason":"tool_calls"}]}"#,
        ] {
            assembler
                .consume(&serde_json::from_str(payload).unwrap())
                .unwrap();
        }
        let turn = assembler.finish(&reference_model(), 1, 0).unwrap();
        let replay = turn.message.replay_state.clone().expect("replay captured");
        assert_eq!(replay[0]["encrypted_content"], "ENC");
        let messages = vec![ChatMessage::Assistant(turn.message)];
        let nimble = wire_messages_with(&messages, ReasoningReplay::Required, true);
        assert_eq!(nimble[0]["nimble_replay"][0]["id"], "rs_1");
        let plain = wire_messages_with(&messages, ReasoningReplay::Required, false);
        assert!(plain[0].get("nimble_replay").is_none());
    }

    #[test]
    fn in_stream_error_objects_fail_the_request() {
        let mut assembler = TurnAssembler::new();
        let chunk: Value = serde_json::from_str(
            r#"{"error":{"message":"upstream exploded","type":"upstream_error"}}"#,
        )
        .unwrap();
        let error = assembler.consume(&chunk).unwrap_err();
        assert!(error.to_string().contains("upstream exploded"));
    }

    #[test]
    fn silent_thinkers_and_routers_get_the_long_idle_timeout() {
        let registry = crate::model::Registry::builtin();
        let muse = registry.resolve("muse-spark-1.3-contributor").unwrap();
        let deepseek = registry.resolve("deepseek-v4-flash").unwrap();
        assert_eq!(
            idle_timeout_for(&muse, Dialect::OpenAi),
            SILENT_STREAM_IDLE_TIMEOUT
        );
        assert_eq!(
            idle_timeout_for(&deepseek, Dialect::DeepSeek),
            STREAM_IDLE_TIMEOUT
        );
        assert_eq!(
            idle_timeout_for(&deepseek, Dialect::Nimble),
            SILENT_STREAM_IDLE_TIMEOUT
        );
    }

    #[test]
    fn balance_errors_are_not_rate_limits() {
        assert!(is_balance_error(
            r#"{"error":{"code":"1113","message":"Insufficient balance or no resource package. Please recharge."}}"#
        ));
        assert!(is_balance_error(
            r#"{"error":{"message":"Insufficient Balance","type":"unknown_error"}}"#
        ));
        assert!(!is_balance_error(
            "Rate limit reached on tokens per min (TPM). Please try again in 2s."
        ));
    }

    #[test]
    fn rate_limit_messages_and_wait_hints_are_recognised() {
        let message = "Rate limit reached for gpt-5.6-luna in organization org-x on tokens per min (TPM): Limit 500000, Used 439502, Requested 104861. Please try again in 5.323s. Visit https://platform.openai.com/account/rate-limits to learn more.";
        assert!(is_rate_limit_message(message));
        assert_eq!(
            retry_after_hint(message),
            Some(std::time::Duration::from_millis(5323))
        );
        assert_eq!(
            retry_after_hint("Too many requests, retry after 12 seconds"),
            Some(std::time::Duration::from_secs(12))
        );
        assert_eq!(
            retry_after_hint("slow down, try again in 250ms"),
            Some(std::time::Duration::from_millis(250))
        );
        assert_eq!(retry_after_hint("upstream exploded"), None);
        assert!(!is_rate_limit_message("upstream exploded"));
        assert!(!is_rate_limit_message(
            "This model's maximum context length is 131072 tokens"
        ));
    }

    #[tokio::test]
    async fn in_stream_rate_limits_are_rate_limit_errors() {
        let mut assembler = TurnAssembler::new();
        let chunk: Value = serde_json::from_str(
            r#"{"error":{"message":"Rate limit reached on tokens per min (TPM). Please try again in 2s.","type":"tokens"}}"#,
        )
        .unwrap();
        let error = assembler.consume(&chunk).unwrap_err();
        assert!(matches!(
            error,
            ProviderError::RateLimited { status: 200, .. }
        ));
    }

    #[test]
    fn context_length_errors_are_detected() {
        assert!(is_context_length_error(
            400,
            r#"{"error":{"message":"This model's maximum context length is 131072 tokens"}}"#
        ));
        assert!(is_context_length_error(
            400,
            r#"{"error":{"code":"context_length_exceeded"}}"#
        ));
        assert!(!is_context_length_error(400, "invalid api key"));
        assert!(!is_context_length_error(500, "context length"));
    }

    #[test]
    fn descriptor_endpoint_resolution_needs_a_url() {
        let registry = Registry::builtin();
        let inver = registry.provider("inver").unwrap();
        std::env::remove_var("INVER_BASE_URL");
        assert!(OpenAiCompatProvider::from_descriptor(inver, "k".into(), None).is_err());
        let provider = OpenAiCompatProvider::from_descriptor(
            inver,
            "k".into(),
            Some("https://gw.example/v1".into()),
        )
        .unwrap();
        assert_eq!(provider.dialect(), Dialect::OpenAi);
        assert_eq!(provider.base_url, "https://gw.example/v1");
    }
}

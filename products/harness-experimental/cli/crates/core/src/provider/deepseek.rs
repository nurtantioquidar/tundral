//! DeepSeek Chat Completions adapter (streaming).
//!
//! DeepSeek's prefix caching is automatic and keyed on byte-stable request
//! prefixes, so everything here serializes deterministically: struct field
//! order is fixed by construction and `serde_json::Value` objects render with
//! sorted keys. `reasoning_content` is echoed back on assistant tool-call
//! messages (the API rejects replays that drop it on thinking models).

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::sse::SseParser;
use super::{
    CompletedTurn, FinishReason, Provider, ProviderError, ProviderRequest, StreamSink, ToolSchema,
};
use crate::domain::{price_usage, AssistantMessage, ChatMessage, ToolCall, Usage};

pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const MAX_ATTEMPTS: u32 = 3;

pub struct DeepSeekProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl DeepSeekProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest client");
        Self {
            client,
            api_key,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        }
    }
}

/// Serialize the conversation exactly as DeepSeek expects. Key order within
/// each message is deterministic (Value maps sort keys), and a message's bytes
/// never depend on anything outside the message itself — the properties the
/// prefix cache relies on.
pub fn wire_messages(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| match message {
            ChatMessage::System { content } => json!({"role": "system", "content": content}),
            ChatMessage::User { content } => json!({"role": "user", "content": content}),
            ChatMessage::Assistant(m) => wire_assistant(m),
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => json!({"role": "tool", "tool_call_id": tool_call_id, "content": content}),
        })
        .collect()
}

fn wire_assistant(message: &AssistantMessage) -> Value {
    let mut value = json!({
        "role": "assistant",
        "content": message.content,
    });
    let object = value.as_object_mut().expect("assistant message object");
    if !message.tool_calls.is_empty() {
        // Thinking models require reasoning_content echoed on tool-call
        // messages; absent reasoning is normalized to null, never omitted.
        object.insert(
            "reasoning_content".to_string(),
            match &message.reasoning_content {
                Some(text) => Value::String(text.clone()),
                None => Value::Null,
            },
        );
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

pub fn request_body(request: &ProviderRequest) -> Value {
    json!({
        "model": request.model.wire_name(),
        "messages": wire_messages(&request.messages),
        "tools": wire_tools(&request.tools),
        "tool_choice": "auto",
        "thinking": {"type": "enabled"},
        "reasoning_effort": request.effort.wire_name(),
        "max_tokens": request.max_output_tokens,
        "stream": true,
        "stream_options": {"include_usage": true},
    })
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
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

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
        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some(parse_usage(usage));
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
        if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
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

    pub fn finish(self, model: crate::domain::Model) -> Result<CompletedTurn, ProviderError> {
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
        let cost = price_usage(model, usage);
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
            },
            usage,
            cost,
            finish,
        })
    }
}

fn parse_usage(usage: &Value) -> Usage {
    let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or_default();
    let input_tokens = field("prompt_tokens");
    let cache_hit = field("prompt_cache_hit_tokens");
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

fn is_context_length_error(status: u16, body: &str) -> bool {
    let lowered = body.to_ascii_lowercase();
    status == 400 && lowered.contains("context") && lowered.contains("length")
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 425 | 429) || status >= 500
}

#[async_trait::async_trait]
impl Provider for DeepSeekProvider {
    async fn stream(
        &self,
        request: &ProviderRequest,
        cancel: &CancellationToken,
        sink: &mut dyn StreamSink,
    ) -> Result<CompletedTurn, ProviderError> {
        let body = request_body(request);
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut attempt = 0;
        let response = loop {
            attempt += 1;
            let send = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send();
            let result = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                result = send => result,
            };
            match result {
                Ok(response) if response.status().is_success() => break response,
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body_text = response.text().await.unwrap_or_default();
                    if is_context_length_error(status, &body_text) {
                        return Err(ProviderError::ContextLengthExceeded);
                    }
                    if is_retryable_status(status) && attempt < MAX_ATTEMPTS {
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }
                    return Err(ProviderError::Status {
                        status,
                        body: truncate_body(&body_text),
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
        };

        let mut assembler = TurnAssembler::new();
        let mut parser = SseParser::new();
        let mut byte_stream = response.bytes_stream();
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                chunk = byte_stream.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|error| ProviderError::Transport(error.to_string()))?;
            for payload in parser.feed(&chunk) {
                if payload == "[DONE]" {
                    return assembler.finish(request.model);
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
        assembler.finish(request.model)
    }
}

fn backoff(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis((250 * 2u64.pow(attempt.min(3))).min(2_000))
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
    use crate::domain::Model;
    use crate::provider::ReasoningEffort;

    fn assembled(payloads: &[&str]) -> CompletedTurn {
        let mut assembler = TurnAssembler::new();
        for payload in payloads {
            let value: Value = serde_json::from_str(payload).unwrap();
            assembler.consume(&value).unwrap();
        }
        assembler.finish(Model::DeepseekV4Flash).unwrap()
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
    }

    #[test]
    fn cache_miss_falls_back_to_input_minus_hit() {
        let usage = parse_usage(&serde_json::json!({
            "prompt_tokens": 100, "prompt_cache_hit_tokens": 30, "completion_tokens": 5
        }));
        assert_eq!(usage.cache_miss_input_tokens, 70);
    }

    #[test]
    fn assistant_tool_call_messages_always_carry_reasoning_field() {
        let with = wire_assistant(&AssistantMessage {
            content: None,
            reasoning_content: Some("because".into()),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
            }],
        });
        assert_eq!(with["reasoning_content"], "because");
        let without = wire_assistant(&AssistantMessage {
            content: Some("hi".into()),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                id: "c2".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
            }],
        });
        assert!(without["reasoning_content"].is_null());
        // Plain assistant text omits the field entirely.
        let plain = wire_assistant(&AssistantMessage {
            content: Some("hi".into()),
            reasoning_content: Some("thought".into()),
            tool_calls: vec![],
        });
        assert!(plain.get("reasoning_content").is_none());
    }

    #[test]
    fn request_body_is_deterministic() {
        let request = ProviderRequest {
            model: Model::DeepseekV4Flash,
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
            effort: ReasoningEffort::High,
        };
        let a = serde_json::to_string(&request_body(&request)).unwrap();
        let b = serde_json::to_string(&request_body(&request)).unwrap();
        assert_eq!(a, b);
        assert!(a.contains("\"stream\":true"));
        assert!(a.contains("\"include_usage\":true"));
        assert!(a.contains("\"reasoning_effort\":\"high\""));
    }

    #[test]
    fn context_length_errors_are_detected() {
        assert!(is_context_length_error(
            400,
            r#"{"error":{"message":"This model's maximum context length is 131072 tokens"}}"#
        ));
        assert!(!is_context_length_error(400, "invalid api key"));
        assert!(!is_context_length_error(500, "context length"));
    }
}

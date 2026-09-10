//! OpenAI Responses API dialect (`POST /responses`, streaming).
//!
//! Why it exists: OpenAI's Chat Completions endpoint refuses function tools
//! together with any reasoning effort other than `none` for its reasoning
//! models (verified live on `gpt-5.6-luna`, 2026-09-04). A coding harness needs
//! both, and only the Responses API offers them. This module translates
//! Butterfly's provider-neutral conversation into Responses input items and the
//! event stream back into a `CompletedTurn`, so the agent loop never notices.
//!
//! Statelessness and caching: requests are sent with `store: false` and
//! `include: ["reasoning.encrypted_content"]`; the encrypted reasoning items
//! come back on `response.output_item.done` and are kept on the assistant
//! message as opaque `replay_state`, then replayed verbatim before that turn's
//! function calls on the next request. Nothing is kept server-side, the input
//! is a byte-stable, append-only prefix, and OpenAI's automatic prompt caching
//! applies to it like any other provider's.

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{
    CompletedTurn, FinishReason, ProviderError, ProviderRequest, RoutingFacts, StreamSink,
    ToolSchema,
};
use crate::domain::{price_usage, AssistantMessage, ChatMessage, ToolCall, Usage};
use crate::model::ModelDescriptor;

/// Translate the conversation into Responses `input` items. The system
/// message becomes `instructions`; every other message maps 1:1 in order so
/// the serialized prefix stays append-only.
pub fn wire_input(messages: &[ChatMessage]) -> (Option<String>, Vec<Value>) {
    let mut instructions = None;
    let mut input = Vec::with_capacity(messages.len());
    for message in messages {
        match message {
            ChatMessage::System { content } => {
                if instructions.is_none() {
                    instructions = Some(content.clone());
                } else {
                    input.push(json!({"role": "system", "content": content}));
                }
            }
            ChatMessage::User { content } => {
                input.push(json!({"role": "user", "content": content}));
            }
            ChatMessage::Assistant(assistant) => {
                // Encrypted reasoning items first: the API requires the
                // reasoning that produced a function call to precede it.
                if let Some(Value::Array(items)) = &assistant.replay_state {
                    input.extend(items.iter().cloned());
                }
                if let Some(text) = &assistant.content {
                    if !text.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": text}],
                        }));
                    }
                }
                for call in &assistant.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments,
                    }));
                }
            }
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": content,
                }));
            }
        }
    }
    (instructions, input)
}

fn wire_tools(tools: &[ToolSchema]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": false,
            })
        })
        .collect()
}

pub fn request_body(request: &ProviderRequest) -> Value {
    let model: &ModelDescriptor = &request.model;
    let (instructions, input) = wire_input(&request.messages);
    let mut body = json!({
        "model": model.id,
        "input": input,
        "max_output_tokens": request.max_output_tokens,
        "stream": true,
        "store": false,
    });
    let object = body.as_object_mut().expect("request object");
    if let Some(instructions) = instructions {
        object.insert("instructions".into(), Value::String(instructions));
    }
    if model.capabilities.tools && !request.tools.is_empty() {
        object.insert("tools".into(), Value::Array(wire_tools(&request.tools)));
        object.insert("tool_choice".into(), Value::String("auto".into()));
    }
    if model.supports_reasoning() {
        let mut reasoning = json!({"summary": "auto"});
        if let Some(effort) = &request.effort {
            reasoning["effort"] = Value::String(effort.clone());
        }
        object.insert("reasoning".into(), reasoning);
        object.insert("include".into(), json!(["reasoning.encrypted_content"]));
    }
    body
}

/// Assembles Responses stream events into a completed turn. Pure and
/// synchronous so fixture tests can drive it without a network.
#[derive(Debug, Default)]
pub struct ResponsesAssembler {
    content: String,
    reasoning_summary: String,
    /// Function-call items in output order, keyed by item id while streaming.
    calls: Vec<PartialCall>,
    reasoning_items: Vec<Value>,
    usage: Option<Usage>,
    finish: Option<FinishReason>,
    error: Option<String>,
    pub routing: Option<RoutingFacts>,
}

#[derive(Debug, Default)]
struct PartialCall {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    done: bool,
}

pub enum EventDelta {
    Text(String),
    Reasoning(String),
    None,
}

impl ResponsesAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one parsed event payload (the JSON after `data:`).
    pub fn consume(&mut self, event: &Value) -> Result<EventDelta, ProviderError> {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "response.output_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.content.push_str(delta);
                    return Ok(EventDelta::Text(delta.to_string()));
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.reasoning_summary.push_str(delta);
                    return Ok(EventDelta::Reasoning(delta.to_string()));
                }
            }
            "response.output_item.added" => {
                if let Some(item) = event.get("item") {
                    if item.get("type").and_then(Value::as_str) == Some("function_call") {
                        self.calls.push(PartialCall {
                            item_id: str_field(item, "id"),
                            call_id: str_field(item, "call_id"),
                            name: str_field(item, "name"),
                            arguments: str_field(item, "arguments"),
                            done: false,
                        });
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                let item_id = str_field(event, "item_id");
                let delta = str_field(event, "delta");
                if let Some(call) = self.call_mut(&item_id, event) {
                    if !call.done {
                        call.arguments.push_str(&delta);
                    }
                }
            }
            "response.function_call_arguments.done" => {
                let item_id = str_field(event, "item_id");
                let arguments = str_field(event, "arguments");
                if let Some(call) = self.call_mut(&item_id, event) {
                    call.arguments = arguments;
                    call.done = true;
                }
            }
            "response.output_item.done" => {
                if let Some(item) = event.get("item") {
                    match item.get("type").and_then(Value::as_str) {
                        Some("function_call") => {
                            let item_id = str_field(item, "id");
                            let call_id = str_field(item, "call_id");
                            let name = str_field(item, "name");
                            let arguments = str_field(item, "arguments");
                            match self.calls.iter_mut().find(|c| c.item_id == item_id) {
                                Some(call) => {
                                    if !arguments.is_empty() {
                                        call.arguments = arguments;
                                    }
                                    if call.call_id.is_empty() {
                                        call.call_id = call_id;
                                    }
                                    if call.name.is_empty() {
                                        call.name = name;
                                    }
                                    call.done = true;
                                }
                                None => self.calls.push(PartialCall {
                                    item_id,
                                    call_id,
                                    name,
                                    arguments,
                                    done: true,
                                }),
                            }
                        }
                        Some("reasoning") => {
                            // Keep only what replay needs: id, encrypted
                            // content, and an empty summary (the summary text
                            // is display-only and must not be replayed).
                            let mut replay = json!({"type": "reasoning", "summary": []});
                            if let Some(id) = item.get("id") {
                                replay["id"] = id.clone();
                            }
                            if let Some(encrypted) = item.get("encrypted_content") {
                                if !encrypted.is_null() {
                                    replay["encrypted_content"] = encrypted.clone();
                                    self.reasoning_items.push(replay);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "response.completed" | "response.incomplete" | "response.failed" => {
                let response = event.get("response").cloned().unwrap_or(Value::Null);
                if let Some(usage) = response.get("usage") {
                    self.usage = Some(parse_usage(usage));
                }
                let incomplete_reason = response
                    .get("incomplete_details")
                    .and_then(|d| d.get("reason"))
                    .and_then(Value::as_str);
                self.finish = Some(match (kind, incomplete_reason) {
                    ("response.failed", _) => {
                        self.error = Some(
                            response
                                .get("error")
                                .and_then(|e| e.get("message"))
                                .and_then(Value::as_str)
                                .unwrap_or("response failed")
                                .to_string(),
                        );
                        FinishReason::Other
                    }
                    (_, Some("max_output_tokens")) => FinishReason::Length,
                    _ if !self.calls.is_empty() => FinishReason::ToolCalls,
                    _ => FinishReason::Stop,
                });
            }
            "error" => {
                self.error = Some(
                    event
                        .get("message")
                        .or_else(|| event.get("error").and_then(|e| e.get("message")))
                        .and_then(Value::as_str)
                        .unwrap_or("stream error")
                        .to_string(),
                );
            }
            _ => {}
        }
        if let Some(facts) = event.get("nimble").filter(|n| n.is_object()) {
            if let Ok(parsed) = serde_json::from_value::<RoutingFacts>(facts.clone()) {
                self.routing = Some(parsed);
            }
        }
        Ok(EventDelta::None)
    }

    fn call_mut(&mut self, item_id: &str, event: &Value) -> Option<&mut PartialCall> {
        if let Some(index) = self.calls.iter().position(|c| c.item_id == item_id) {
            return self.calls.get_mut(index);
        }
        // Some servers omit `output_item.added`; fall back to output_index.
        let output_index = event.get("output_index").and_then(Value::as_u64);
        if output_index.is_some() && !item_id.is_empty() {
            self.calls.push(PartialCall {
                item_id: item_id.to_string(),
                ..PartialCall::default()
            });
            return self.calls.last_mut();
        }
        None
    }

    pub fn finish(
        self,
        model: &ModelDescriptor,
        latency_ms: u64,
        retries: u32,
    ) -> Result<CompletedTurn, ProviderError> {
        if let Some(error) = self.error {
            return Err(ProviderError::Status {
                status: 200,
                body: error,
            });
        }
        let tool_calls: Vec<ToolCall> = self
            .calls
            .into_iter()
            .map(|call| {
                if call.call_id.is_empty() || call.name.is_empty() {
                    return Err(ProviderError::Malformed(
                        "incomplete streamed function call".into(),
                    ));
                }
                Ok(ToolCall {
                    id: call.call_id,
                    name: call.name,
                    arguments: if call.arguments.is_empty() {
                        "{}".to_string()
                    } else {
                        call.arguments
                    },
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
                reasoning_content: if self.reasoning_summary.is_empty() {
                    None
                } else {
                    Some(self.reasoning_summary)
                },
                tool_calls,
                replay_state: if self.reasoning_items.is_empty() {
                    None
                } else {
                    Some(Value::Array(self.reasoning_items))
                },
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

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Responses usage: `input_tokens`, `input_tokens_details.cached_tokens`,
/// `output_tokens` (reasoning tokens are inside output_tokens).
pub fn parse_usage(usage: &Value) -> Usage {
    let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or_default();
    let input_tokens = field("input_tokens");
    let cached = usage
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .min(input_tokens);
    Usage {
        input_tokens,
        cache_hit_input_tokens: cached,
        cache_miss_input_tokens: input_tokens.saturating_sub(cached),
        output_tokens: field("output_tokens"),
    }
}

/// Stream one Responses request through the shared HTTP plumbing.
pub async fn stream(
    provider: &super::openai_compat::OpenAiCompatProvider,
    request: &ProviderRequest,
    cancel: &CancellationToken,
    sink: &mut dyn StreamSink,
) -> Result<CompletedTurn, ProviderError> {
    let started = std::time::Instant::now();
    let body = request_body(request);
    let url = format!("{}/responses", provider.base_url().trim_end_matches('/'));
    let (response, retries) = provider.post_with_retries(&url, &body, cancel).await?;
    let mut assembler = ResponsesAssembler::new();
    assembler.routing = super::openai_compat::routing_facts_from_headers(response.headers());
    let mut parser = super::sse::SseParser::new();
    let mut byte_stream = {
        use futures_util::StreamExt as _;
        response.bytes_stream().boxed()
    };
    loop {
        let idle = super::openai_compat::idle_timeout_for(
            &request.model,
            crate::model::Dialect::OpenAiResponses,
        );
        let Some(chunk) = super::openai_compat::next_chunk(&mut byte_stream, cancel, idle).await?
        else {
            break;
        };
        for payload in parser.feed(&chunk) {
            if payload == "[DONE]" {
                continue;
            }
            let value: Value = serde_json::from_str(&payload)
                .map_err(|error| ProviderError::Malformed(error.to_string()))?;
            match assembler.consume(&value)? {
                EventDelta::Text(text) => sink.on_text(&text),
                EventDelta::Reasoning(text) => sink.on_reasoning(&text),
                EventDelta::None => {}
            }
        }
    }
    assembler.finish(
        &request.model,
        started.elapsed().as_millis() as u64,
        retries,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Registry;

    fn luna() -> std::sync::Arc<ModelDescriptor> {
        Registry::builtin().resolve("gpt-5.6-luna").unwrap()
    }

    #[test]
    fn conversation_maps_to_input_items_in_order() {
        let messages = vec![
            ChatMessage::System {
                content: "sys".into(),
            },
            ChatMessage::User {
                content: "read it".into(),
            },
            ChatMessage::Assistant(AssistantMessage {
                content: None,
                reasoning_content: Some("summary text (never replayed)".into()),
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path":"a.rs"}"#.into(),
                }],
                replay_state: Some(json!([
                    {"type": "reasoning", "id": "rs_1", "encrypted_content": "ENC", "summary": []}
                ])),
            }),
            ChatMessage::Tool {
                tool_call_id: "call_1".into(),
                content: "contents".into(),
            },
            ChatMessage::Assistant(AssistantMessage {
                content: Some("done".into()),
                reasoning_content: None,
                tool_calls: vec![],
                replay_state: None,
            }),
        ];
        let (instructions, input) = wire_input(&messages);
        assert_eq!(instructions.as_deref(), Some("sys"));
        let kinds: Vec<String> = input
            .iter()
            .map(|item| {
                item.get("type")
                    .or_else(|| item.get("role"))
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "user",
                "reasoning",
                "function_call",
                "function_call_output",
                "message"
            ]
        );
        assert_eq!(input[1]["encrypted_content"], "ENC");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[3]["output"], "contents");
        assert_eq!(input[4]["content"][0]["text"], "done");
        // Reasoning summary text never goes back on the wire.
        assert!(!serde_json::to_string(&input)
            .unwrap()
            .contains("never replayed"));
    }

    #[test]
    fn request_body_uses_responses_fields() {
        let model = luna();
        let request = ProviderRequest {
            effort: model.resolve_effort(Some("high")).unwrap(),
            model,
            messages: vec![
                ChatMessage::System {
                    content: "sys".into(),
                },
                ChatMessage::User {
                    content: "hi".into(),
                },
            ],
            tools: crate::tools::tool_schemas(),
            max_output_tokens: 2048,
            routing: None,
        };
        let body = request_body(&request);
        assert_eq!(body["instructions"], "sys");
        assert_eq!(body["max_output_tokens"], 2048);
        assert_eq!(body["store"], false);
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["include"][0], "reasoning.encrypted_content");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("messages").is_none());
        let a = serde_json::to_string(&body).unwrap();
        let b = serde_json::to_string(&request_body(&request)).unwrap();
        assert_eq!(a, b, "deterministic bytes");
    }

    #[test]
    fn assembler_builds_calls_reasoning_and_usage_from_events() {
        let events = [
            r#"{"type":"response.created","response":{"id":"resp_1"}}"#,
            r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[]}}"#,
            r#"{"type":"response.reasoning_summary_text.delta","item_id":"rs_1","delta":"thinking"}"#,
            r#"{"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs_1","encrypted_content":"ENC","summary":[{"type":"summary_text","text":"thinking"}]}}"#,
            r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file","arguments":""}}"#,
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":1,"delta":"{\"pa"}"#,
            r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":1,"delta":"th\":\"a.rs\"}"}"#,
            r#"{"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":1,"arguments":"{\"path\":\"a.rs\"}"}"#,
            r#"{"type":"response.output_item.done","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"a.rs\"}"}}"#,
            r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed","usage":{"input_tokens":1200,"input_tokens_details":{"cached_tokens":1024},"output_tokens":90,"output_tokens_details":{"reasoning_tokens":70}}}}"#,
        ];
        let mut assembler = ResponsesAssembler::new();
        let mut reasoning = String::new();
        for event in events {
            if let EventDelta::Reasoning(delta) = assembler
                .consume(&serde_json::from_str(event).unwrap())
                .unwrap()
            {
                reasoning.push_str(&delta);
            }
        }
        let turn = assembler.finish(&luna(), 5, 0).unwrap();
        assert_eq!(reasoning, "thinking");
        assert_eq!(turn.finish, FinishReason::ToolCalls);
        assert_eq!(turn.message.tool_calls.len(), 1);
        assert_eq!(turn.message.tool_calls[0].id, "call_1");
        assert_eq!(turn.message.tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
        assert_eq!(turn.message.reasoning_content.as_deref(), Some("thinking"));
        let replay = turn.message.replay_state.expect("encrypted reasoning kept");
        assert_eq!(replay[0]["encrypted_content"], "ENC");
        assert_eq!(replay[0]["summary"], json!([]));
        assert_eq!(turn.usage.input_tokens, 1200);
        assert_eq!(turn.usage.cache_hit_input_tokens, 1024);
        assert_eq!(turn.usage.cache_miss_input_tokens, 176);
        assert_eq!(turn.usage.output_tokens, 90);
        assert!(turn.cost.actual_microusd > 0);
        assert!(turn.cost.price_verified);
    }

    #[test]
    fn text_answers_and_truncation_finish_correctly() {
        let mut assembler = ResponsesAssembler::new();
        for event in [
            r#"{"type":"response.output_text.delta","delta":"ok"}"#,
            r#"{"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":10,"output_tokens":5}}}"#,
        ] {
            assembler
                .consume(&serde_json::from_str(event).unwrap())
                .unwrap();
        }
        let turn = assembler.finish(&luna(), 1, 0).unwrap();
        assert_eq!(turn.finish, FinishReason::Length);
        assert_eq!(turn.message.content.as_deref(), Some("ok"));
        assert!(turn.message.replay_state.is_none());

        let mut failed = ResponsesAssembler::new();
        failed
            .consume(
                &serde_json::from_str(
                    r#"{"type":"response.failed","response":{"status":"failed","error":{"message":"boom"}}}"#,
                )
                .unwrap(),
            )
            .unwrap();
        assert!(failed.finish(&luna(), 1, 0).is_err());
    }
}

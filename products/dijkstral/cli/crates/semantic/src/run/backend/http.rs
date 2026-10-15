//! HTTP backends: OpenAI-compatible Chat Completions (OpenAI, DeepSeek, Kimi, Ollama, z.ai,
//! Inver, the bench shim), Anthropic Messages and Gemini `generateContent`.
//!
//! Requests are never streamed. Connection failures, 408, 409, 429 and 5xx are retried twice
//! (0.5 s, then 1 s) before the runner sees an error, like the official clients. Error messages
//! carry the status and at most 300 characters of the server's body, never a header.

use std::time::Duration;

use anyhow::{bail, Result};
use base64::Engine;
use serde_json::{json, Value};

use crate::types::{Backend, Request, Response};

const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1000)];

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(timeout).build()
}

/// POSTs `body` and returns the parsed JSON reply.
fn post(agent: &ureq::Agent, url: &str, headers: &[(&str, String)], body: &Value) -> Result<Value> {
    let payload = serde_json::to_string(body)?;
    let mut attempt = 0;
    loop {
        let mut req = agent.post(url).set("Content-Type", "application/json");
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let retryable = match req.send_string(&payload) {
            Ok(resp) => {
                let text = resp.into_string()?;
                return serde_json::from_str(&text)
                    .map_err(|e| anyhow::anyhow!("unreadable reply from {url}: {e}"));
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body: String = resp
                    .into_string()
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect();
                let err = anyhow::anyhow!("HTTP {code} from {url}: {}", body.trim());
                if !(code == 408 || code == 409 || code == 429 || code >= 500) {
                    return Err(err);
                }
                err
            }
            Err(ureq::Error::Transport(t)) => anyhow::anyhow!("request to {url} failed: {t}"),
        };
        if attempt >= RETRY_DELAYS.len() {
            return Err(retryable);
        }
        std::thread::sleep(RETRY_DELAYS[attempt]);
        attempt += 1;
    }
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The Chat Completions `messages` for `req`: a system message (when there is one) and a user
/// message whose content is a list of one text part and one `image_url` part per image; a request
/// without a system message (community naming) sends its user text as a plain string.
pub fn openai_messages(req: &Request) -> Value {
    if req.system.is_empty() {
        return json!([{"role": "user", "content": req.user_text}]);
    }
    let mut content = vec![json!({"type": "text", "text": req.user_text})];
    for (mime, bytes, _) in &req.images {
        content.push(json!({
            "type": "image_url",
            "image_url": {"url": data_url(mime, bytes), "detail": "auto"}
        }));
    }
    json!([
        {"role": "system", "content": req.system},
        {"role": "user", "content": content}
    ])
}

/// Which body field carries the output limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenField {
    MaxCompletionTokens,
    MaxTokens,
}

pub struct OpenAi {
    name: &'static str,
    base: String,
    key: Option<String>,
    model: String,
    field: TokenField,
    cap: Option<u32>,
    agent: ureq::Agent,
}

impl OpenAi {
    pub fn new(
        name: &'static str,
        base: String,
        key: Option<String>,
        model: String,
        field: TokenField,
        cap: Option<u32>,
        timeout: Duration,
    ) -> OpenAi {
        OpenAi {
            name,
            base: base.trim_end_matches('/').to_string(),
            key,
            model,
            field,
            cap,
            agent: agent(timeout),
        }
    }
}

impl Backend for OpenAi {
    fn name(&self) -> &str {
        self.name
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn complete(&self, req: &Request) -> Result<Response> {
        let limit = self
            .cap
            .map_or(req.max_output_tokens, |c| c.min(req.max_output_tokens));
        let field = match self.field {
            TokenField::MaxCompletionTokens => "max_completion_tokens",
            TokenField::MaxTokens => "max_tokens",
        };
        let body = json!({
            "model": self.model,
            "stream": false,
            field: limit,
            "temperature": 0,
            "messages": openai_messages(req),
        });
        let headers: Vec<(&str, String)> = self
            .key
            .iter()
            .map(|k| ("Authorization", format!("Bearer {k}")))
            .collect();
        let v = post(
            &self.agent,
            &format!("{}/chat/completions", self.base),
            &headers,
            &body,
        )?;
        let choice = &v["choices"][0];
        let text = match &choice["message"]["content"] {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<String>(),
            _ => String::new(),
        };
        Ok(Response {
            text,
            input_tokens: v["usage"]["prompt_tokens"].as_u64(),
            output_tokens: v["usage"]["completion_tokens"].as_u64(),
            model: v["model"].as_str().unwrap_or(&self.model).to_string(),
            truncated: choice["finish_reason"] == "length",
        })
    }
}

pub struct Anthropic {
    base: String,
    key: String,
    model: String,
    agent: ureq::Agent,
}

impl Anthropic {
    pub fn new(base: String, key: String, model: String, timeout: Duration) -> Anthropic {
        Anthropic {
            base: base.trim_end_matches('/').to_string(),
            key,
            model,
            agent: agent(timeout),
        }
    }
}

impl Backend for Anthropic {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn complete(&self, req: &Request) -> Result<Response> {
        let mut content: Vec<Value> = req
            .images
            .iter()
            .map(|(mime, bytes, _)| {
                json!({"type": "image", "source": {
                    "type": "base64",
                    "media_type": mime,
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                }})
            })
            .collect();
        content.push(json!({"type": "text", "text": req.user_text}));
        // No temperature: current Claude models reject sampling parameters.
        let mut body = json!({
            "model": self.model,
            "max_tokens": req.max_output_tokens,
            "messages": [{"role": "user", "content": content}],
        });
        if !req.system.is_empty() {
            body["system"] = Value::String(req.system.clone());
        }
        let headers = [
            ("x-api-key", self.key.clone()),
            ("anthropic-version", "2023-06-01".to_string()),
        ];
        let v = post(
            &self.agent,
            &format!("{}/v1/messages", self.base),
            &headers,
            &body,
        )?;
        if v["stop_reason"] == "refusal" {
            bail!("the model declined the request (stop_reason: refusal)");
        }
        let text = v["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<String>();
        Ok(Response {
            text,
            input_tokens: v["usage"]["input_tokens"].as_u64(),
            output_tokens: v["usage"]["output_tokens"].as_u64(),
            model: v["model"].as_str().unwrap_or(&self.model).to_string(),
            truncated: v["stop_reason"] == "max_tokens",
        })
    }
}

pub struct Gemini {
    base: String,
    key: String,
    model: String,
    agent: ureq::Agent,
}

impl Gemini {
    pub fn new(base: String, key: String, model: String, timeout: Duration) -> Gemini {
        Gemini {
            base: base.trim_end_matches('/').to_string(),
            key,
            model,
            agent: agent(timeout),
        }
    }
}

impl Backend for Gemini {
    fn name(&self) -> &str {
        "gemini"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn complete(&self, req: &Request) -> Result<Response> {
        let mut parts = vec![json!({"text": req.user_text})];
        for (mime, bytes, _) in &req.images {
            parts.push(json!({"inlineData": {
                "mimeType": mime,
                "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            }}));
        }
        let mut body = json!({
            "contents": [{"role": "user", "parts": parts}],
            "generationConfig": {"temperature": 0, "maxOutputTokens": req.max_output_tokens},
        });
        if !req.system.is_empty() {
            body["systemInstruction"] = json!({"parts": [{"text": req.system}]});
        }
        let v = post(
            &self.agent,
            &format!("{}/models/{}:generateContent", self.base, self.model),
            &[("x-goog-api-key", self.key.clone())],
            &body,
        )?;
        let cand = &v["candidates"][0];
        let text = cand["content"]["parts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["thought"] != true)
            .filter_map(|p| p["text"].as_str())
            .collect::<String>();
        Ok(Response {
            text,
            input_tokens: v["usageMetadata"]["promptTokenCount"].as_u64(),
            output_tokens: v["usageMetadata"]["candidatesTokenCount"].as_u64(),
            model: self.model.clone(),
            truncated: cand["finishReason"] == "MAX_TOKENS",
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A one-thread HTTP server answering each request with the next `(status, body)`; it records
    /// `(request line, headers, body)` of every request.
    /// Request line, lower-cased headers and JSON body.
    type Seen = (String, Vec<(String, String)>, Value);

    pub(crate) struct FakeServer {
        pub url: String,
        pub seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl FakeServer {
        pub fn start(replies: Vec<(u16, String)>) -> FakeServer {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let log = seen.clone();
            std::thread::spawn(move || {
                for (status, body) in replies {
                    let Ok((stream, _)) = listener.accept() else {
                        return;
                    };
                    let mut r = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    let mut headers = Vec::new();
                    let mut len = 0;
                    loop {
                        let mut h = String::new();
                        r.read_line(&mut h).unwrap();
                        let h = h.trim_end();
                        if h.is_empty() {
                            break;
                        }
                        let (k, v) = h.split_once(':').unwrap();
                        let (k, v) = (k.to_ascii_lowercase(), v.trim().to_string());
                        if k == "content-length" {
                            len = v.parse().unwrap();
                        }
                        headers.push((k, v));
                    }
                    let mut buf = vec![0; len];
                    r.read_exact(&mut buf).unwrap();
                    let body_json = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                    log.lock()
                        .unwrap()
                        .push((line.trim().to_string(), headers, body_json));
                    let mut s = stream;
                    let _ = write!(
                        s,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            });
            FakeServer { url, seen }
        }
    }

    fn req() -> Request {
        Request {
            system: "SYS".into(),
            user_text: "USER".into(),
            images: vec![("image/png".into(), vec![1, 2, 3], "i.png".into())],
            max_output_tokens: 16384,
        }
    }

    #[test]
    fn openai_compatible_request_and_retry() {
        let ok = json!({"model": "m-served", "choices": [{"message": {"content": "{\"nodes\":[]}"},
            "finish_reason": "length"}], "usage": {"prompt_tokens": 11, "completion_tokens": 3}});
        let srv = FakeServer::start(vec![(500, "{}".into()), (200, ok.to_string())]);
        let b = OpenAi::new(
            "deepseek",
            format!("{}/v1/", srv.url),
            Some("sk-test".into()),
            "m".into(),
            TokenField::MaxTokens,
            Some(8192),
            Duration::from_secs(10),
        );
        let r = b.complete(&req()).unwrap();
        assert_eq!(r.text, "{\"nodes\":[]}");
        assert!(r.truncated);
        assert_eq!((r.input_tokens, r.output_tokens), (Some(11), Some(3)));
        let seen = srv.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let (line, headers, body) = &seen[1];
        assert_eq!(line, "POST /v1/chat/completions HTTP/1.1");
        assert!(headers.contains(&("authorization".into(), "Bearer sk-test".into())));
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["temperature"], 0);
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"][0]["text"], "USER");
        assert_eq!(
            body["messages"][1]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AQID"
        );
    }

    #[test]
    fn client_errors_are_not_retried_and_keys_stay_out_of_errors() {
        let srv = FakeServer::start(vec![(401, "{\"error\":\"bad key\"}".into())]);
        let b = OpenAi::new(
            "openai",
            srv.url.clone(),
            Some("sk-secret".into()),
            "m".into(),
            TokenField::MaxCompletionTokens,
            None,
            Duration::from_secs(10),
        );
        let e = b.complete(&req()).unwrap_err().to_string();
        assert!(e.contains("HTTP 401") && !e.contains("sk-secret"), "{e}");
        assert_eq!(srv.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn anthropic_messages() {
        let ok = json!({"model": "claude-x", "stop_reason": "end_turn", "content": [
            {"type": "thinking", "thinking": ""}, {"type": "text", "text": "{}"}],
            "usage": {"input_tokens": 5, "output_tokens": 2}});
        let srv = FakeServer::start(vec![(200, ok.to_string())]);
        let b = Anthropic::new(
            srv.url.clone(),
            "k".into(),
            "claude-x".into(),
            Duration::from_secs(10),
        );
        let r = b.complete(&req()).unwrap();
        assert_eq!(
            (r.text.as_str(), r.truncated, r.output_tokens),
            ("{}", false, Some(2))
        );
        let seen = srv.seen.lock().unwrap();
        let (line, headers, body) = &seen[0];
        assert_eq!(line, "POST /v1/messages HTTP/1.1");
        assert!(headers.contains(&("x-api-key".into(), "k".into())));
        assert!(headers.contains(&("anthropic-version".into(), "2023-06-01".into())));
        assert_eq!(body["system"], "SYS");
        assert_eq!(body["messages"][0]["content"][0]["source"]["data"], "AQID");
        assert_eq!(body["messages"][0]["content"][1]["text"], "USER");
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn gemini_generate_content() {
        let ok = json!({"candidates": [{"content": {"parts": [{"text": "{\"a\"", "thought": false},
            {"text": ":1}"}]}, "finishReason": "MAX_TOKENS"}],
            "usageMetadata": {"promptTokenCount": 9, "candidatesTokenCount": 4}});
        let srv = FakeServer::start(vec![(200, ok.to_string())]);
        let b = Gemini::new(
            srv.url.clone(),
            "g".into(),
            "gem".into(),
            Duration::from_secs(10),
        );
        let r = b.complete(&req()).unwrap();
        assert_eq!((r.text.as_str(), r.truncated), ("{\"a\":1}", true));
        let seen = srv.seen.lock().unwrap();
        let (line, headers, body) = &seen[0];
        assert_eq!(line, "POST /models/gem:generateContent HTTP/1.1");
        assert!(headers.contains(&("x-goog-api-key".into(), "g".into())));
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "SYS");
        assert_eq!(
            body["contents"][0]["parts"][1]["inlineData"]["mimeType"],
            "image/png"
        );
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 16384);
    }
}

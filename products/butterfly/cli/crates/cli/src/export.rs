//! Static HTML export of a session's event log: a shareable, self-contained
//! transcript with reasoning and tool output collapsed behind <details>.

use std::path::PathBuf;

use anyhow::{Context, Result};
use butterfly_core::domain::format_microusd_as_usd;
use butterfly_core::session::store::{SessionEvent, SessionStore};

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const STYLE: &str = "\
body{font-family:ui-monospace,Menlo,monospace;max-width:60rem;margin:2rem auto;padding:0 1rem;\
background:#111;color:#ddd;font-size:14px;line-height:1.5}\
h1{font-size:1.1rem;color:#9cf}\
.user{border-left:3px solid #9cf;padding:.4rem .8rem;margin:1rem 0;white-space:pre-wrap}\
.assistant{border-left:3px solid #6d6;padding:.4rem .8rem;margin:1rem 0;white-space:pre-wrap}\
.meta{color:#888;font-size:12px;margin:.6rem 0}\
details{margin:.4rem 0;color:#aaa}summary{cursor:pointer;color:#c9a}\
pre{background:#1a1a1a;padding:.6rem;overflow-x:auto;white-space:pre-wrap;margin:.3rem 0}\
.divider{color:#c96;margin:1rem 0;font-style:italic}";

/// Render the event log to a self-contained HTML page.
pub fn render_html(session_id: &str, events: &[SessionEvent]) -> String {
    let mut body = String::new();
    let mut total_actual: u64 = 0;
    let mut total_in: u64 = 0;
    let mut total_out: u64 = 0;
    let mut model = String::new();
    for event in events {
        match event {
            SessionEvent::SessionStarted {
                workspace,
                model: m,
                system_prompt,
                ..
            } => {
                model = m.clone();
                body.push_str(&format!(
                    "<div class=meta>workspace: {} · model: {}</div>\
                     <details><summary>system prompt</summary><pre>{}</pre></details>",
                    escape(workspace),
                    escape(m),
                    escape(system_prompt)
                ));
            }
            SessionEvent::UserMessage { content } => {
                body.push_str(&format!("<div class=user>{}</div>", escape(content)));
            }
            SessionEvent::AssistantMessage { message } => {
                if let Some(reasoning) = &message.reasoning_content {
                    body.push_str(&format!(
                        "<details><summary>reasoning</summary><pre>{}</pre></details>",
                        escape(reasoning)
                    ));
                }
                if let Some(content) = &message.content {
                    body.push_str(&format!("<div class=assistant>{}</div>", escape(content)));
                }
                for call in &message.tool_calls {
                    body.push_str(&format!(
                        "<details><summary>⚒ {}({})</summary><pre>{}</pre></details>",
                        escape(&call.name),
                        escape(&call.id),
                        escape(&call.arguments)
                    ));
                }
            }
            SessionEvent::ToolResult {
                tool_call_id,
                content,
            } => {
                body.push_str(&format!(
                    "<details><summary>result ({})</summary><pre>{}</pre></details>",
                    escape(tool_call_id),
                    escape(content)
                ));
            }
            SessionEvent::UsageRecorded {
                usage,
                actual_microusd,
                ..
            } => {
                total_actual += actual_microusd;
                total_in += usage.input_tokens;
                total_out += usage.output_tokens;
            }
            SessionEvent::ElisionApplied {
                replacements,
                estimated_savings_tokens,
            } => {
                body.push_str(&format!(
                    "<div class=divider>— elided {} tool result(s), ~{} tokens reclaimed —</div>",
                    replacements.len(),
                    estimated_savings_tokens
                ));
            }
            SessionEvent::HistoryRebased {
                before_tokens,
                after_tokens,
                ..
            } => {
                body.push_str(&format!(
                    "<div class=divider>— compacted: ~{before_tokens} → ~{after_tokens} tokens —</div>"
                ));
            }
            SessionEvent::Error { detail } => {
                body.push_str(&format!(
                    "<div class=divider>error: {}</div>",
                    escape(&detail.to_string())
                ));
            }
        }
    }
    format!(
        "<!doctype html><meta charset=utf-8><title>butterfly session {sid}</title>\
         <style>{STYLE}</style><h1>butterfly session {sid}</h1>{body}\
         <div class=meta>totals: {tin} in · {tout} out · {cost} · model {model}</div>",
        sid = escape(session_id),
        body = body,
        tin = total_in,
        tout = total_out,
        cost = format_microusd_as_usd(total_actual),
        model = escape(&model),
    )
}

/// Export a session (latest when None) to an HTML file; returns the path.
pub fn export_session(session: Option<&str>, out: Option<PathBuf>) -> Result<PathBuf> {
    let state_root = SessionStore::state_root();
    let session_id = match session {
        Some(id) => id.to_string(),
        None => SessionStore::list_sessions(&state_root)
            .into_iter()
            .next()
            .context("no saved sessions to export")?,
    };
    let events_path = state_root.join(&session_id).join("events.jsonl");
    let events = SessionStore::read_events_at(&events_path)
        .with_context(|| format!("cannot read session {session_id}"))?;
    let html = render_html(&session_id, &events);
    let out = out.unwrap_or_else(|| PathBuf::from(format!("butterfly-session-{session_id}.html")));
    std::fs::write(&out, html).with_context(|| format!("cannot write {}", out.display()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use butterfly_core::domain::{AssistantMessage, ToolCall, Usage};

    #[test]
    fn renders_escaped_transcript_with_totals() {
        let events = vec![
            SessionEvent::SessionStarted {
                at_epoch_ms: 1,
                workspace: "w".into(),
                model: "deepseek-v4-flash".into(),
                provider: None,
                system_prompt: "sys <secret>".into(),
            },
            SessionEvent::UserMessage {
                content: "<script>alert(1)</script>".into(),
            },
            SessionEvent::AssistantMessage {
                message: AssistantMessage {
                    content: Some("done & dusted".into()),
                    reasoning_content: Some("thinking".into()),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read_file".into(),
                        arguments: r#"{"path":"a"}"#.into(),
                    }],
                    replay_state: None,
                },
            },
            SessionEvent::ToolResult {
                tool_call_id: "c1".into(),
                content: "contents".into(),
            },
            SessionEvent::UsageRecorded {
                usage: Usage {
                    input_tokens: 100,
                    cache_hit_input_tokens: 90,
                    cache_miss_input_tokens: 10,
                    output_tokens: 20,
                },
                actual_microusd: 42,
                normalized_cache_miss_microusd: 100,
            },
        ];
        let html = render_html("abc", &events);
        assert!(!html.contains("<script>alert"), "must escape user content");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("done &amp; dusted"));
        assert!(html.contains("<details><summary>reasoning</summary>"));
        assert!(html.contains("⚒ read_file(c1)"));
        assert!(html.contains("totals: 100 in · 20 out · $0.000042"));
    }
}

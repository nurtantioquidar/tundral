//! Append-only JSONL session persistence. Replaying the event log rebuilds
//! the exact message history, including post-elision placeholders and
//! post-compaction rebuilt transcripts, so `--resume` restores byte-identical
//! prompts (and therefore the provider's prefix cache when it is still warm).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::{AssistantMessage, ChatMessage, Usage};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    SessionStarted {
        at_epoch_ms: u64,
        workspace: String,
        model: String,
        system_prompt: String,
    },
    UserMessage {
        content: String,
    },
    AssistantMessage {
        message: AssistantMessage,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
    UsageRecorded {
        usage: Usage,
        actual_microusd: u64,
        normalized_cache_miss_microusd: u64,
    },
    /// Tool-result bodies replaced by placeholders (stage-1 compaction). The
    /// replacement text is stored so replay is exact, not re-derived.
    ElisionApplied {
        replacements: Vec<(usize, String)>,
        estimated_savings_tokens: u64,
    },
    /// Stage-2 compaction: the full rebuilt history (system + checkpoint +
    /// verbatim tail). Replay swaps the transcript wholesale.
    HistoryRebased {
        messages: Vec<ChatMessage>,
        before_tokens: u64,
        after_tokens: u64,
    },
    Error {
        detail: serde_json::Value,
    },
}

pub struct SessionStore {
    directory: PathBuf,
    events_path: PathBuf,
}

impl SessionStore {
    pub fn state_root() -> PathBuf {
        dirs::state_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local/state")
            })
            .join("harness-experimental/sessions")
    }

    pub fn create(session_id: &str) -> std::io::Result<Self> {
        Self::create_under(&Self::state_root(), session_id)
    }

    pub fn create_under(root: &Path, session_id: &str) -> std::io::Result<Self> {
        let directory = root.join(session_id);
        std::fs::create_dir_all(&directory)?;
        restrict_permissions(&directory, 0o700)?;
        let events_path = directory.join("events.jsonl");
        if !events_path.exists() {
            std::fs::write(&events_path, b"")?;
        }
        restrict_permissions(&events_path, 0o600)?;
        Ok(Self {
            directory,
            events_path,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn append(&self, event: &SessionEvent) -> std::io::Result<()> {
        let mut line = serde_json::to_string(event)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.events_path)?;
        file.write_all(line.as_bytes())
    }

    pub fn read_events(&self) -> std::io::Result<Vec<SessionEvent>> {
        Self::read_events_at(&self.events_path)
    }

    pub fn read_events_at(path: &Path) -> std::io::Result<Vec<SessionEvent>> {
        let text = std::fs::read_to_string(path)?;
        let mut events = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let event = serde_json::from_str(line)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            events.push(event);
        }
        Ok(events)
    }

    /// List session ids under the state root, newest first by directory mtime.
    pub fn list_sessions(root: &Path) -> Vec<String> {
        let mut entries: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("events.jsonl").exists())
            .filter_map(|entry| {
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((modified, entry.file_name().to_string_lossy().into_owned()))
            })
            .collect();
        entries.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        entries.into_iter().map(|(_, name)| name).collect()
    }
}

fn restrict_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

/// Rebuild the message history and accumulated usage from an event log.
pub struct ReplayedSession {
    pub messages: Vec<ChatMessage>,
    pub total_usage: Usage,
    pub total_actual_microusd: u64,
    pub model: Option<String>,
    pub workspace: Option<String>,
}

pub fn replay(events: &[SessionEvent]) -> ReplayedSession {
    let mut messages: Vec<ChatMessage> = Vec::new();
    let mut total_usage = Usage::default();
    let mut total_actual: u64 = 0;
    let mut model = None;
    let mut workspace = None;
    for event in events {
        match event {
            SessionEvent::SessionStarted {
                system_prompt,
                model: m,
                workspace: w,
                ..
            } => {
                messages.push(ChatMessage::System {
                    content: system_prompt.clone(),
                });
                model = Some(m.clone());
                workspace = Some(w.clone());
            }
            SessionEvent::UserMessage { content } => messages.push(ChatMessage::User {
                content: content.clone(),
            }),
            SessionEvent::AssistantMessage { message } => {
                messages.push(ChatMessage::Assistant(message.clone()))
            }
            SessionEvent::ToolResult {
                tool_call_id,
                content,
            } => messages.push(ChatMessage::Tool {
                tool_call_id: tool_call_id.clone(),
                content: content.clone(),
            }),
            SessionEvent::UsageRecorded {
                usage,
                actual_microusd,
                ..
            } => {
                total_usage.add(*usage);
                total_actual += actual_microusd;
            }
            SessionEvent::ElisionApplied { replacements, .. } => {
                for (index, replacement) in replacements {
                    if let Some(ChatMessage::Tool { content, .. }) = messages.get_mut(*index) {
                        *content = replacement.clone();
                    }
                }
            }
            SessionEvent::HistoryRebased {
                messages: rebuilt, ..
            } => {
                messages = rebuilt.clone();
            }
            SessionEvent::Error { .. } => {}
        }
    }
    ReplayedSession {
        messages,
        total_usage,
        total_actual_microusd: total_actual,
        model,
        workspace,
    }
}

pub fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ToolCall;

    #[test]
    fn replay_reconstructs_history_including_rewrites() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::create_under(root.path(), "s1").unwrap();
        store
            .append(&SessionEvent::SessionStarted {
                at_epoch_ms: 1,
                workspace: "w".into(),
                model: "deepseek-v4-flash".into(),
                system_prompt: "sys".into(),
            })
            .unwrap();
        store
            .append(&SessionEvent::UserMessage {
                content: "hello".into(),
            })
            .unwrap();
        store
            .append(&SessionEvent::AssistantMessage {
                message: AssistantMessage {
                    content: None,
                    reasoning_content: Some("r".into()),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read_file".into(),
                        arguments: r#"{"path":"a"}"#.into(),
                    }],
                },
            })
            .unwrap();
        store
            .append(&SessionEvent::ToolResult {
                tool_call_id: "c1".into(),
                content: "big output".into(),
            })
            .unwrap();
        store
            .append(&SessionEvent::ElisionApplied {
                replacements: vec![(3, "[elided]".into())],
                estimated_savings_tokens: 10,
            })
            .unwrap();
        store
            .append(&SessionEvent::UsageRecorded {
                usage: Usage {
                    input_tokens: 10,
                    cache_hit_input_tokens: 5,
                    cache_miss_input_tokens: 5,
                    output_tokens: 2,
                },
                actual_microusd: 42,
                normalized_cache_miss_microusd: 100,
            })
            .unwrap();

        let replayed = replay(&store.read_events().unwrap());
        assert_eq!(replayed.messages.len(), 4);
        assert!(matches!(
            &replayed.messages[3],
            ChatMessage::Tool { content, .. } if content == "[elided]"
        ));
        assert!(matches!(
            &replayed.messages[2],
            ChatMessage::Assistant(a) if a.reasoning_content.as_deref() == Some("r")
        ));
        assert_eq!(replayed.total_actual_microusd, 42);
        assert_eq!(replayed.model.as_deref(), Some("deepseek-v4-flash"));

        let rebased = vec![ChatMessage::System {
            content: "sys".into(),
        }];
        store
            .append(&SessionEvent::HistoryRebased {
                messages: rebased.clone(),
                before_tokens: 100,
                after_tokens: 10,
            })
            .unwrap();
        let replayed = replay(&store.read_events().unwrap());
        assert_eq!(replayed.messages, rebased);
    }

    #[test]
    fn sessions_are_listed() {
        let root = tempfile::tempdir().unwrap();
        SessionStore::create_under(root.path(), "a").unwrap();
        SessionStore::create_under(root.path(), "b").unwrap();
        let sessions = SessionStore::list_sessions(root.path());
        assert_eq!(sessions.len(), 2);
    }
}

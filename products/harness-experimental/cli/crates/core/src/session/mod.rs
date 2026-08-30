//! The interactive agent session: an append-only conversation driven by a
//! deterministic loop. The model proposes tool calls; this code owns budgets,
//! approval policy, context management, and termination.

pub mod approval;
pub mod store;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::context::{CompactionNeed, ContextConfig, ContextManager};
use crate::domain::{format_microusd_as_usd, ChatMessage, Model, Usage};
use crate::provider::{
    CompletedTurn, Provider, ProviderError, ProviderRequest, ReasoningEffort, StreamSink,
    ToolSchema,
};
use crate::tools::{describe_call, tool_kind, tool_schemas, ToolExecutor};
use approval::{
    policy_outcome, ApprovalGate, ApprovalRequest, Decision, PermissionMode, PolicyOutcome,
};
use store::{replay, SessionEvent, SessionStore};

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub model: Model,
    pub effort: ReasoningEffort,
    pub permission_mode: PermissionMode,
    pub max_output_tokens: u32,
    /// Hard cap on total actual session cost; None = uncapped.
    pub max_session_cost_microusd: Option<u64>,
    /// Provider round-trips allowed for a single user input.
    pub max_provider_calls_per_input: u32,
    pub context: ContextConfig,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            model: Model::DeepseekV4Flash,
            effort: ReasoningEffort::High,
            permission_mode: PermissionMode::Ask,
            max_output_tokens: 8_192,
            max_session_cost_microusd: None,
            max_provider_calls_per_input: 50,
            context: ContextConfig::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("session store error: {0}")]
    Store(#[from] std::io::Error),
    #[error("resume failed: {0}")]
    Resume(String),
}

/// What happened during one user turn, for the UI to render.
#[derive(Debug, Default, Clone)]
pub struct TurnReport {
    pub assistant_text: String,
    pub cancelled: bool,
    pub budget_exhausted: bool,
    pub provider_calls: u32,
    pub tool_calls: u32,
    pub usage: Usage,
    pub actual_microusd: u64,
    /// Cache-blind cost (all input at the miss rate) for fair comparisons.
    pub normalized_microusd: u64,
    pub compactions: u32,
    pub elisions: u32,
}

pub struct Agent {
    pub session_id: String,
    provider: Arc<dyn Provider>,
    executor: ToolExecutor,
    tools: Vec<ToolSchema>,
    messages: Vec<ChatMessage>,
    context: ContextManager,
    store: Option<SessionStore>,
    config: SessionConfig,
    total_usage: Usage,
    total_actual_microusd: u64,
    session_approvals: HashSet<String>,
}

impl Agent {
    /// Start a fresh session in `root`. The system prompt is frozen here and
    /// never rebuilt — it is the cached prefix.
    pub fn new(
        provider: Arc<dyn Provider>,
        root: PathBuf,
        config: SessionConfig,
        store: Option<SessionStore>,
    ) -> Result<Self, SessionError> {
        let workspace_name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let instructions = crate::context::prompt::load_instructions_snapshot(&root);
        let system_prompt =
            crate::context::prompt::build_system_prompt(&workspace_name, instructions.as_deref());
        let session_id = uuid::Uuid::new_v4().simple().to_string();
        if let Some(store) = &store {
            store.append(&SessionEvent::SessionStarted {
                at_epoch_ms: store::epoch_ms_now(),
                workspace: workspace_name.clone(),
                model: config.model.wire_name().to_string(),
                system_prompt: system_prompt.clone(),
            })?;
        }
        let mut context = ContextManager::new(config.context.clone());
        let messages = vec![ChatMessage::System {
            content: system_prompt,
        }];
        context.ledger.rebase(&messages);
        Ok(Self {
            session_id,
            provider,
            executor: ToolExecutor::new(root),
            tools: tool_schemas(),
            messages,
            context,
            store,
            config,
            total_usage: Usage::default(),
            total_actual_microusd: 0,
            session_approvals: HashSet::new(),
        })
    }

    /// Resume from a persisted event log. History replays byte-exactly, so a
    /// still-warm provider prefix cache keeps hitting.
    pub fn resume(
        provider: Arc<dyn Provider>,
        root: PathBuf,
        mut config: SessionConfig,
        store: SessionStore,
        session_id: String,
    ) -> Result<Self, SessionError> {
        let events = store.read_events()?;
        let replayed = replay(&events);
        if replayed.messages.is_empty() {
            return Err(SessionError::Resume("event log has no messages".into()));
        }
        if let Some(model) = replayed.model.as_deref().and_then(Model::parse) {
            config.model = model;
        }
        let mut context = ContextManager::new(config.context.clone());
        context.ledger.rebase(&replayed.messages);
        Ok(Self {
            session_id,
            provider,
            executor: ToolExecutor::new(root),
            tools: tool_schemas(),
            messages: replayed.messages,
            context,
            store: Some(store),
            config,
            total_usage: replayed.total_usage,
            total_actual_microusd: replayed.total_actual_microusd,
            session_approvals: HashSet::new(),
        })
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn total_usage(&self) -> Usage {
        self.total_usage
    }

    pub fn total_actual_microusd(&self) -> u64 {
        self.total_actual_microusd
    }

    pub fn utilization(&self) -> f64 {
        self.context.utilization()
    }

    pub fn model(&self) -> Model {
        self.config.model
    }

    pub fn permission_mode(&self) -> PermissionMode {
        self.config.permission_mode
    }

    pub fn set_permission_mode(&mut self, mode: PermissionMode) {
        self.config.permission_mode = mode;
    }

    fn record(&self, event: &SessionEvent) -> Result<(), SessionError> {
        if let Some(store) = &self.store {
            store.append(event)?;
        }
        Ok(())
    }

    fn push_user(&mut self, content: String) -> Result<(), SessionError> {
        self.record(&SessionEvent::UserMessage {
            content: content.clone(),
        })?;
        let message = ChatMessage::User { content };
        self.context.ledger.note_appended(&message);
        self.messages.push(message);
        Ok(())
    }

    fn push_tool_result(
        &mut self,
        tool_call_id: String,
        content: String,
    ) -> Result<(), SessionError> {
        self.record(&SessionEvent::ToolResult {
            tool_call_id: tool_call_id.clone(),
            content: content.clone(),
        })?;
        let message = ChatMessage::Tool {
            tool_call_id,
            content,
        };
        self.context.ledger.note_appended(&message);
        self.messages.push(message);
        Ok(())
    }

    fn budget_exhausted(&self) -> bool {
        self.config
            .max_session_cost_microusd
            .is_some_and(|cap| self.total_actual_microusd >= cap)
    }

    /// Run the two-stage context pipeline if thresholds demand it.
    async fn manage_context(
        &mut self,
        cancel: &CancellationToken,
        report: &mut TurnReport,
    ) -> Result<(), SessionError> {
        if !matches!(
            self.context.need(),
            CompactionNeed::Compact | CompactionNeed::Urgent
        ) {
            return Ok(());
        }
        if let Some(plan) = self.context.plan_elision(&self.messages) {
            let estimated = plan.estimated_savings_tokens;
            self.context.apply_elision(&mut self.messages, &plan);
            let replacements: Vec<(usize, String)> = plan
                .message_indices
                .iter()
                .filter_map(|&index| match &self.messages[index] {
                    ChatMessage::Tool { content, .. } => Some((index, content.clone())),
                    _ => None,
                })
                .collect();
            self.record(&SessionEvent::ElisionApplied {
                replacements,
                estimated_savings_tokens: estimated,
            })?;
            report.elisions += 1;
        }
        if matches!(
            self.context.need(),
            CompactionNeed::Compact | CompactionNeed::Urgent
        ) {
            self.compact_now(cancel).await?;
            report.compactions += 1;
        }
        Ok(())
    }

    /// Stage-2 compaction, also exposed for the /compact command.
    pub async fn compact_now(&mut self, cancel: &CancellationToken) -> Result<(), SessionError> {
        let outcome = self
            .context
            .summarize(
                &mut self.messages,
                &self.tools,
                self.provider.as_ref(),
                self.config.model,
                self.config.effort,
                cancel,
            )
            .await?;
        self.total_usage.add(outcome.summarizer_usage);
        self.total_actual_microusd += outcome.summarizer_cost.actual_microusd;
        self.record(&SessionEvent::UsageRecorded {
            usage: outcome.summarizer_usage,
            actual_microusd: outcome.summarizer_cost.actual_microusd,
            normalized_cache_miss_microusd: outcome.summarizer_cost.normalized_cache_miss_microusd,
        })?;
        self.record(&SessionEvent::HistoryRebased {
            messages: self.messages.clone(),
            before_tokens: outcome.before_tokens,
            after_tokens: outcome.after_tokens,
        })?;
        Ok(())
    }

    async fn complete_once(
        &mut self,
        cancel: &CancellationToken,
        sink: &mut dyn StreamSink,
    ) -> Result<CompletedTurn, SessionError> {
        let request = ProviderRequest {
            model: self.config.model,
            messages: self.messages.clone(),
            tools: self.tools.clone(),
            max_output_tokens: self.config.max_output_tokens,
            effort: self.config.effort,
        };
        match self.provider.stream(&request, cancel, sink).await {
            Ok(turn) => Ok(turn),
            Err(ProviderError::ContextLengthExceeded) => {
                // Last resort: shrink the retained tail, compact, retry once.
                self.context.shrink_keep_last_turns();
                self.compact_now(cancel).await?;
                let request = ProviderRequest {
                    model: self.config.model,
                    messages: self.messages.clone(),
                    tools: self.tools.clone(),
                    max_output_tokens: self.config.max_output_tokens,
                    effort: self.config.effort,
                };
                Ok(self.provider.stream(&request, cancel, sink).await?)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// One user turn: send input, loop provider/tool rounds until the model
    /// stops calling tools, cancellation, or a budget stop.
    pub async fn run_turn(
        &mut self,
        input: String,
        gate: &dyn ApprovalGate,
        sink: &mut dyn StreamSink,
        cancel: &CancellationToken,
    ) -> Result<TurnReport, SessionError> {
        let mut report = TurnReport::default();
        self.push_user(input)?;
        loop {
            if self.budget_exhausted() {
                report.budget_exhausted = true;
                report.assistant_text.push_str(&format!(
                    "\n[session cost cap reached: {}]",
                    format_microusd_as_usd(self.total_actual_microusd)
                ));
                return Ok(report);
            }
            if report.provider_calls >= self.config.max_provider_calls_per_input {
                report.assistant_text.push_str(
                    "\n[turn limit reached for this input; send a follow-up to continue]",
                );
                return Ok(report);
            }
            self.manage_context(cancel, &mut report).await?;

            let turn = match self.complete_once(cancel, sink).await {
                Ok(turn) => turn,
                Err(SessionError::Provider(ProviderError::Cancelled)) => {
                    report.cancelled = true;
                    return Ok(report);
                }
                Err(error) => return Err(error),
            };
            report.provider_calls += 1;
            report.usage.add(turn.usage);
            report.actual_microusd += turn.cost.actual_microusd;
            report.normalized_microusd += turn.cost.normalized_cache_miss_microusd;
            self.total_usage.add(turn.usage);
            self.total_actual_microusd += turn.cost.actual_microusd;
            // Anchor BEFORE pushing: the response usage already covers the
            // assistant message, so it is not drift.
            self.context.ledger.observe_usage(&turn.usage);
            self.record(&SessionEvent::AssistantMessage {
                message: turn.message.clone(),
            })?;
            self.record(&SessionEvent::UsageRecorded {
                usage: turn.usage,
                actual_microusd: turn.cost.actual_microusd,
                normalized_cache_miss_microusd: turn.cost.normalized_cache_miss_microusd,
            })?;
            self.messages
                .push(ChatMessage::Assistant(turn.message.clone()));

            if let Some(text) = &turn.message.content {
                if !report.assistant_text.is_empty() {
                    report.assistant_text.push('\n');
                }
                report.assistant_text.push_str(text);
            }
            if turn.message.tool_calls.is_empty() {
                return Ok(report);
            }

            let mut cancelled_mid_tools = false;
            for call in &turn.message.tool_calls {
                report.tool_calls += 1;
                let preview = describe_call(&call.name, &call.arguments);
                if cancelled_mid_tools || cancel.is_cancelled() {
                    cancelled_mid_tools = true;
                    self.push_tool_result(call.id.clone(), "cancelled by user".into())?;
                    continue;
                }
                sink.on_tool_start(&preview);
                let output = self.execute_gated(call, gate, cancel).await;
                if output.is_none() {
                    cancelled_mid_tools = true;
                    sink.on_tool_result(&preview, true);
                    self.push_tool_result(call.id.clone(), "cancelled by user".into())?;
                    continue;
                }
                let output = output.expect("checked above");
                sink.on_tool_result(&preview, output.is_error);
                let content = if output.is_error {
                    format!("ERROR: {}", output.content)
                } else {
                    output.content
                };
                self.push_tool_result(call.id.clone(), content)?;
            }
            if cancelled_mid_tools {
                report.cancelled = true;
                return Ok(report);
            }
        }
    }

    /// Apply deterministic policy, consult the gate when required, execute.
    /// Returns None only when the user cancelled mid-execution.
    async fn execute_gated(
        &mut self,
        call: &crate::domain::ToolCall,
        gate: &dyn ApprovalGate,
        cancel: &CancellationToken,
    ) -> Option<crate::tools::ToolOutput> {
        let Some(kind) = tool_kind(&call.name) else {
            return Some(crate::tools::ToolOutput::error(format!(
                "unknown tool: {}",
                call.name
            )));
        };
        // Validate command policy BEFORE the approval gate: never ask the user
        // to approve something deterministic policy will reject anyway.
        if call.name == "run_command" {
            if let Some(argv) = parse_argv(&call.arguments) {
                if let Err(error) = crate::tools::policy::validate_command(&argv) {
                    return Some(crate::tools::ToolOutput::error(error.to_string()));
                }
            }
        }
        let outcome = policy_outcome(self.config.permission_mode, kind);
        let approved = match outcome {
            PolicyOutcome::AutoApprove => true,
            PolicyOutcome::Deny => false,
            PolicyOutcome::NeedsReview => {
                let key = approval::scope_key(&call.name, &call.arguments);
                if self.session_approvals.contains(&key) {
                    true
                } else {
                    let request = ApprovalRequest {
                        tool_name: call.name.clone(),
                        preview: describe_call(&call.name, &call.arguments),
                        scope_key: key.clone(),
                    };
                    match gate.review(&request).await {
                        Decision::Approve => true,
                        Decision::ApproveForSession => {
                            self.session_approvals.insert(key);
                            true
                        }
                        Decision::Deny => false,
                    }
                }
            }
        };
        if !approved {
            let reason = match outcome {
                PolicyOutcome::Deny => "denied: session is in read-only mode",
                _ => "denied by user; adjust the approach or ask them",
            };
            return Some(crate::tools::ToolOutput::error(reason.to_string()));
        }
        if cancel.is_cancelled() {
            return None;
        }
        Some(
            self.executor
                .execute(&call.name, &call.arguments, cancel)
                .await,
        )
    }
}

/// Extract the argv vector from run_command arguments; None when the
/// arguments are malformed (the executor then reports the parse error).
fn parse_argv(arguments: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let argv = value.get("command")?.as_array()?;
    Some(
        argv.iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AssistantMessage, ToolCall};
    use crate::provider::scripted::ScriptedProvider;
    use crate::provider::NullSink;

    fn assistant_with_call(id: &str, name: &str, arguments: &str) -> AssistantMessage {
        AssistantMessage {
            content: None,
            reasoning_content: Some("thinking".into()),
            tool_calls: vec![ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: arguments.into(),
            }],
        }
    }

    fn final_text(text: &str) -> AssistantMessage {
        AssistantMessage {
            content: Some(text.into()),
            reasoning_content: None,
            tool_calls: vec![],
        }
    }

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hello world\n").unwrap();
        dir
    }

    #[tokio::test]
    async fn tool_loop_runs_reads_without_approval_and_yields() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "read_file", r#"{"path":"hello.txt"}"#),
            final_text("done"),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig::default(),
            None,
        )
        .unwrap();
        let report = agent
            .run_turn(
                "read the file".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.assistant_text, "done");
        assert_eq!(report.provider_calls, 2);
        assert_eq!(report.tool_calls, 1);
        // Tool result flowed back into history.
        assert!(agent.messages().iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("hello world")
        )));
    }

    #[tokio::test]
    async fn requests_grow_append_only_between_provider_calls() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "read_file", r#"{"path":"hello.txt"}"#),
            final_text("ok"),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig::default(),
            None,
        )
        .unwrap();
        agent
            .run_turn(
                "go".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let requests = provider.recorded_requests();
        assert_eq!(requests.len(), 2);
        // The first request's serialized wire messages are a strict byte
        // prefix of the second's — the property DeepSeek's cache keys on.
        let first: Vec<String> = crate::provider::deepseek::wire_messages(&requests[0])
            .iter()
            .map(|value| serde_json::to_string(value).unwrap())
            .collect();
        let second: Vec<String> = crate::provider::deepseek::wire_messages(&requests[1])
            .iter()
            .map(|value| serde_json::to_string(value).unwrap())
            .collect();
        assert!(second.len() > first.len());
        assert_eq!(&second[..first.len()], &first[..]);
    }

    #[tokio::test]
    async fn read_only_mode_denies_mutations_deterministically() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "write_file", r#"{"path":"x.txt","content":"x"}"#),
            final_text("understood"),
        ]));
        let config = SessionConfig {
            permission_mode: PermissionMode::ReadOnly,
            ..SessionConfig::default()
        };
        let mut agent = Agent::new(provider, dir.path().to_path_buf(), config, None).unwrap();
        agent
            .run_turn(
                "write".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(!dir.path().join("x.txt").exists());
        assert!(agent.messages().iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("read-only")
        )));
    }

    #[tokio::test]
    async fn denial_feeds_back_as_error_result() {
        struct DenyAll;
        #[async_trait::async_trait]
        impl ApprovalGate for DenyAll {
            async fn review(&self, _request: &ApprovalRequest) -> Decision {
                Decision::Deny
            }
        }
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "run_command", r#"{"command":["cargo","test"]}"#),
            final_text("stopped"),
        ]));
        let mut agent = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig::default(),
            None,
        )
        .unwrap();
        let report = agent
            .run_turn(
                "run tests".into(),
                &DenyAll,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.assistant_text, "stopped");
        assert!(agent.messages().iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("denied by user")
        )));
    }

    #[tokio::test]
    async fn policy_rejected_commands_never_reach_the_gate() {
        struct PanicGate;
        #[async_trait::async_trait]
        impl ApprovalGate for PanicGate {
            async fn review(&self, request: &ApprovalRequest) -> Decision {
                panic!(
                    "gate consulted for policy-rejected call: {}",
                    request.preview
                );
            }
        }
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "run_command", r#"{"command":["rm","-rf","x"]}"#),
            final_text("adjusted"),
        ]));
        let mut agent = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig::default(),
            None,
        )
        .unwrap();
        let report = agent
            .run_turn(
                "delete stuff".into(),
                &PanicGate,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.assistant_text, "adjusted");
        assert!(agent.messages().iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("not in allowlist")
        )));
    }

    #[tokio::test]
    async fn session_cost_cap_stops_the_loop() {
        let dir = workspace();
        let provider = Arc::new(
            ScriptedProvider::new(vec![
                assistant_with_call("c1", "read_file", r#"{"path":"hello.txt"}"#),
                final_text("never reached"),
            ])
            .with_usage(Usage {
                input_tokens: 1_000_000,
                cache_hit_input_tokens: 0,
                cache_miss_input_tokens: 1_000_000,
                output_tokens: 100_000,
            }),
        );
        let config = SessionConfig {
            max_session_cost_microusd: Some(1_000),
            ..SessionConfig::default()
        };
        let mut agent = Agent::new(provider, dir.path().to_path_buf(), config, None).unwrap();
        let report = agent
            .run_turn(
                "go".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(report.budget_exhausted);
        assert!(report.assistant_text.contains("cost cap"));
        assert_eq!(report.provider_calls, 1);
    }

    #[tokio::test]
    async fn persistence_roundtrip_resumes_identical_history() {
        let dir = workspace();
        let state = tempfile::tempdir().unwrap();
        let store = SessionStore::create_under(state.path(), "s1").unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "read_file", r#"{"path":"hello.txt"}"#),
            final_text("done"),
        ]));
        let mut agent = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig::default(),
            Some(store),
        )
        .unwrap();
        agent
            .run_turn(
                "read".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let live_messages = agent.messages().to_vec();
        let live_cost = agent.total_actual_microusd();

        let store = SessionStore::create_under(state.path(), "s1").unwrap();
        let resumed = Agent::resume(
            Arc::new(ScriptedProvider::new(vec![])),
            dir.path().to_path_buf(),
            SessionConfig::default(),
            store,
            "s1".into(),
        )
        .unwrap();
        assert_eq!(resumed.messages(), &live_messages[..]);
        assert_eq!(resumed.total_actual_microusd(), live_cost);
    }
}

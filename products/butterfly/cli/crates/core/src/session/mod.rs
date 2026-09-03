//! The interactive agent session: an append-only conversation driven by a
//! deterministic loop. The model proposes tool calls; this code owns budgets,
//! approval policy, context management, and termination.

pub mod approval;
pub mod store;
pub mod working_state;

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use crate::context::{CompactionNeed, ContextConfig, ContextManager};
use crate::domain::{format_microusd_as_usd, ChatMessage, Model, Usage};
use crate::provider::{
    CompletedTurn, Provider, ProviderError, ProviderRequest, ReasoningEffort, StreamSink,
    ToolSchema,
};
use crate::tools::{describe_call, tool_kind, tool_schemas, ToolExecutor};

/// The session's tool list; Atlas tools drop out when disabled (their
/// schemas are the tail of the frozen ordering, so the shared prefix of the
/// remaining schemas is unchanged).
fn session_tools(atlas_enabled: bool) -> Vec<ToolSchema> {
    let mut tools = tool_schemas();
    if !atlas_enabled {
        tools.retain(|tool| tool.name != "repo_map" && tool.name != "symbol_search");
    }
    tools
}
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
    /// Expose the Atlas code-graph tools (repo_map, symbol_search).
    pub atlas_enabled: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            model: Model::DeepseekV4Flash,
            effort: ReasoningEffort::High,
            permission_mode: PermissionMode::Ask,
            // A ceiling, not a spend: reasoning shares this budget, and
            // 8k starved pro-at-high-effort into content-less turns
            // (dogfood 2026-09-03). The API accepts far larger caps.
            max_output_tokens: 65_536,
            max_session_cost_microusd: None,
            max_provider_calls_per_input: 50,
            context: ContextConfig::default(),
            atlas_enabled: true,
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

/// Messages typed while a turn is running; drained into the conversation at
/// the next provider-call boundary (pi-style steering). Cloneable so the UI
/// can push while `run_turn` mutably borrows the agent.
pub type SteerQueue = Arc<Mutex<VecDeque<String>>>;

/// What happened during one user turn, for the UI to render.
#[derive(Debug, Default, Clone)]
pub struct TurnReport {
    pub assistant_text: String,
    pub cancelled: bool,
    pub budget_exhausted: bool,
    /// Some provider response in this turn stopped at the output-token
    /// limit (finish_reason "length"): the answer is truncated — possibly
    /// to nothing, when reasoning consumed the whole budget.
    pub hit_output_limit: bool,
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
    working_state: working_state::WorkingState,
    atlas: butterfly_atlas::Atlas,
    steer: SteerQueue,
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
        let system_prompt = crate::context::prompt::build_system_prompt(
            &workspace_name,
            instructions.as_deref(),
            config.atlas_enabled,
        );
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
        let atlas = butterfly_atlas::Atlas::new(root.clone());
        Ok(Self {
            session_id,
            provider,
            executor: ToolExecutor::new(root),
            tools: session_tools(config.atlas_enabled),
            messages,
            context,
            store,
            config,
            total_usage: Usage::default(),
            total_actual_microusd: 0,
            session_approvals: HashSet::new(),
            working_state: working_state::WorkingState::default(),
            atlas,
            steer: SteerQueue::default(),
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
        let atlas = butterfly_atlas::Atlas::new(root.clone());
        Ok(Self {
            session_id,
            provider,
            executor: ToolExecutor::new(root),
            tools: session_tools(config.atlas_enabled),
            messages: replayed.messages,
            context,
            store: Some(store),
            config,
            total_usage: replayed.total_usage,
            total_actual_microusd: replayed.total_actual_microusd,
            session_approvals: HashSet::new(),
            working_state: replayed.working_state,
            atlas,
            steer: SteerQueue::default(),
        })
    }

    pub fn working_state(&self) -> &working_state::WorkingState {
        &self.working_state
    }

    /// Handle for queueing user messages while a turn runs. Delivered in
    /// order at the next provider-call boundary; anything still queued when
    /// the turn ends is the caller's to drain (run as fresh turns).
    pub fn steer_handle(&self) -> SteerQueue {
        self.steer.clone()
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn atlas_enabled(&self) -> bool {
        self.config.atlas_enabled
    }

    /// The session's Atlas index (UI features like @-mention symbol
    /// completion read it; refreshing it never touches the transcript).
    pub fn atlas_mut(&mut self) -> &mut butterfly_atlas::Atlas {
        &mut self.atlas
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

    /// Switch models mid-session. History stays valid; the provider's prefix
    /// cache is per model, so the next request pays a one-time re-cache.
    pub fn set_model(&mut self, model: Model) {
        self.config.model = model;
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
            self.compact_now(cancel, None).await?;
            report.compactions += 1;
        }
        Ok(())
    }

    /// Stage-2 compaction, also exposed for the /compact command. The
    /// checkpoint's structured half is the tracked working state — data the
    /// loop recorded as it happened, immune to summarizer quality and to
    /// earlier compactions.
    pub async fn compact_now(
        &mut self,
        cancel: &CancellationToken,
        focus: Option<&str>,
    ) -> Result<(), SessionError> {
        let checkpoint = self.working_state.render();
        let outcome = self
            .context
            .summarize(
                &mut self.messages,
                &checkpoint,
                focus,
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
                self.compact_now(cancel, None).await?;
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
            // Steering: deliver queued user messages at the turn boundary.
            let queued: Vec<String> = {
                let mut steer = self.steer.lock().expect("steer lock");
                steer.drain(..).collect()
            };
            for message in queued {
                self.push_user(message)?;
            }
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
            if turn.finish == crate::provider::FinishReason::Length {
                report.hit_output_limit = true;
            }
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
                self.working_state
                    .observe(&call.name, &call.arguments, !output.is_error);
                // The agent's own writes are the likeliest index mutations.
                if !output.is_error && matches!(call.name.as_str(), "edit" | "write_file") {
                    if let Some(path) = serde_json::from_str::<serde_json::Value>(&call.arguments)
                        .ok()
                        .and_then(|args| {
                            args.get("path")
                                .and_then(serde_json::Value::as_str)
                                .map(std::path::PathBuf::from)
                        })
                    {
                        self.atlas.invalidate(&path);
                    }
                }
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
        // update_plan mutates session state, not the workspace: handled here,
        // never gated (recording a plan is not a side effect on the world).
        if call.name == "update_plan" {
            return Some(match self.working_state.apply_update(&call.arguments) {
                Ok(summary) => crate::tools::ToolOutput::text(summary),
                Err(error) => crate::tools::ToolOutput::error(error),
            });
        }
        // Atlas tools read session-owned index state: handled here, read-only,
        // never gated.
        if call.name == "repo_map" || call.name == "symbol_search" {
            if !self.config.atlas_enabled {
                return Some(crate::tools::ToolOutput::error(
                    "atlas is disabled in this session".to_string(),
                ));
            }
            return Some(self.atlas_tool(&call.name, &call.arguments));
        }
        let Some(kind) = tool_kind(&call.name) else {
            return Some(crate::tools::ToolOutput::error(format!(
                "unknown tool: {}",
                call.name
            )));
        };
        // Classify command policy BEFORE the approval gate: forbidden commands
        // never reach a prompt; commands outside the auto-run allowlist force
        // a review even in auto mode.
        let mut policy_warning: Option<String> = None;
        if call.name == "run_command" {
            if let Some(argv) = parse_argv(&call.arguments) {
                match crate::tools::policy::classify_command(&argv) {
                    crate::tools::policy::CommandClass::Forbidden(reason) => {
                        return Some(crate::tools::ToolOutput::error(reason));
                    }
                    crate::tools::policy::CommandClass::NeedsApproval(reason) => {
                        policy_warning = Some(reason);
                    }
                    crate::tools::policy::CommandClass::AutoEligible => {}
                }
            }
        }
        let outcome = policy_outcome(self.config.permission_mode, kind);
        let needs_review = match outcome {
            PolicyOutcome::Deny => {
                return Some(crate::tools::ToolOutput::error(
                    "denied: session is in read-only mode".to_string(),
                ));
            }
            PolicyOutcome::NeedsReview => true,
            // Auto-approval never extends past the allowlist: a policy
            // warning forces the gate regardless of permission mode.
            PolicyOutcome::AutoApprove => policy_warning.is_some(),
        };
        let approved = if !needs_review {
            true
        } else {
            let key = approval::scope_key(&call.name, &call.arguments);
            if self.session_approvals.contains(&key) {
                true
            } else {
                let request = ApprovalRequest {
                    tool_name: call.name.clone(),
                    preview: describe_call(&call.name, &call.arguments),
                    detail: crate::tools::approval_detail(
                        self.executor.root(),
                        &call.name,
                        &call.arguments,
                    ),
                    policy_warning: policy_warning.clone(),
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
        };
        if !approved {
            return Some(crate::tools::ToolOutput::error(
                "denied by user; adjust the approach or ask them".to_string(),
            ));
        }
        if cancel.is_cancelled() {
            return None;
        }
        let mut output = self
            .executor
            .execute(&call.name, &call.arguments, cancel)
            .await;
        // Search fusion, direction 1: a text search for a plain identifier
        // gets Atlas definitions prepended — structural answers with zero
        // extra turns, whichever tool the model picked.
        if call.name == "search_text" && !output.is_error {
            let pattern = serde_json::from_str::<serde_json::Value>(&call.arguments)
                .ok()
                .and_then(|args| {
                    args.get("pattern")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                });
            if let Some(block) = pattern.and_then(|p| self.atlas_definitions_block(&p)) {
                output.content = format!("{block}\nText matches (ripgrep):\n{}", output.content);
            }
        }
        Some(output)
    }
}

/// A bare identifier (the only shape Atlas indexes by name).
fn is_plain_identifier(text: &str) -> bool {
    text.len() >= 3
        && text
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Agent {
    /// Deterministic search fusion: the model never has to choose the right
    /// search tool. A text search for a plain identifier gets the index's
    /// definitions prepended; a symbol lookup that misses the index falls
    /// back to a text search in the same result — no extra turn either way.
    fn atlas_definitions_block(&mut self, name: &str) -> Option<String> {
        if !self.config.atlas_enabled || !is_plain_identifier(name) {
            return None;
        }
        self.atlas.refresh();
        let defs = self.atlas.definitions(name);
        if defs.is_empty() {
            return None;
        }
        let mut block = String::from("Definitions (Atlas):\n");
        for (path, def) in defs.iter().take(10) {
            block.push_str(&format!(
                "  {}:{} [{}] {}\n",
                path.display(),
                def.line,
                def.kind.label(),
                def.signature
            ));
        }
        Some(block)
    }

    /// Execute one Atlas tool call: incremental refresh, then a ranked map or
    /// symbol lookup. Working-state files seed the ranking so the map leans
    /// toward what the session is already touching.
    fn atlas_tool(&mut self, name: &str, arguments: &str) -> crate::tools::ToolOutput {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(value) => value,
            Err(error) => {
                return crate::tools::ToolOutput::error(format!("invalid arguments: {error}"))
            }
        };
        self.atlas.refresh();
        match name {
            "repo_map" => {
                let mut focus: Vec<String> = args
                    .get("focus")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                focus.extend(self.working_state.files_changed.iter().cloned());
                focus.extend(self.working_state.files_read.iter().cloned());
                let max_tokens = args
                    .get("max_tokens")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(1_500)
                    .clamp(200, 4_000) as usize;
                crate::tools::ToolOutput::text(self.atlas.render_map(&focus, max_tokens))
            }
            "symbol_search" => {
                let Some(symbol) = args.get("name").and_then(serde_json::Value::as_str) else {
                    return crate::tools::ToolOutput::error("missing required: name".to_string());
                };
                let mode = args
                    .get("mode")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("definitions");
                let mut lines: Vec<String> = match mode {
                    "references" => self
                        .atlas
                        .references(symbol)
                        .into_iter()
                        .map(|(path, line)| format!("{}:{line}", path.display()))
                        .collect(),
                    _ => self
                        .atlas
                        .definitions(symbol)
                        .into_iter()
                        .map(|(path, def)| {
                            let container = def
                                .container
                                .as_deref()
                                .map(|c| format!(" (in {c})"))
                                .unwrap_or_default();
                            format!(
                                "{}:{} [{}] {}{container}",
                                path.display(),
                                def.line,
                                def.kind.label(),
                                def.signature
                            )
                        })
                        .collect(),
                };
                if lines.is_empty() {
                    lines.push(format!(
                        "[no {mode} found in the Atlas index for '{symbol}']"
                    ));
                    // Search fusion, direction 2: an index miss falls back to
                    // ripgrep in the same result — no wasted turn.
                    if is_plain_identifier(symbol) {
                        let fallback = crate::tools::search::search_text(
                            self.executor.root(),
                            crate::tools::search::SearchTextArgs {
                                pattern: format!(r"\b{symbol}\b"),
                                path: None,
                                case_insensitive: false,
                            },
                        );
                        if let Ok(result) = fallback {
                            lines.push("Text matches (ripgrep fallback):".to_string());
                            lines.push(result.content);
                        }
                    }
                }
                if lines.len() > 100 {
                    lines.truncate(100);
                    lines.push("[truncated at 100 results]".to_string());
                }
                crate::tools::ToolOutput::text(lines.join("\n"))
            }
            _ => crate::tools::ToolOutput::error(format!("unknown atlas tool: {name}")),
        }
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
    async fn output_limit_truncation_is_reported() {
        let dir = workspace();
        // A response that stopped at the output cap with EMPTY content —
        // the exact dogfood failure: reasoning consumed the whole budget
        // and the turn would otherwise end silently.
        let provider = Arc::new(
            ScriptedProvider::new(vec![AssistantMessage {
                content: None,
                reasoning_content: Some("thinking…".into()),
                tool_calls: vec![],
            }])
            .with_finish(crate::provider::FinishReason::Length),
        );
        let mut agent = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig::default(),
            None,
        )
        .unwrap();
        let report = agent
            .run_turn(
                "explain".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(report.hit_output_limit, "length finish must be surfaced");
        assert!(report.assistant_text.is_empty());
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
    async fn forbidden_commands_never_reach_the_gate() {
        struct PanicGate;
        #[async_trait::async_trait]
        impl ApprovalGate for PanicGate {
            async fn review(&self, request: &ApprovalRequest) -> Decision {
                panic!(
                    "gate consulted for policy-forbidden call: {}",
                    request.preview
                );
            }
        }
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "run_command", r#"{"command":["sudo","ls"]}"#),
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
                "escalate".into(),
                &PanicGate,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.assistant_text, "adjusted");
        assert!(agent.messages().iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("privilege escalation")
        )));
    }

    #[tokio::test]
    async fn auto_mode_still_gates_non_allowlisted_commands() {
        use std::sync::atomic::{AtomicU32, Ordering};
        struct RecordingGate {
            reviews: AtomicU32,
        }
        #[async_trait::async_trait]
        impl ApprovalGate for RecordingGate {
            async fn review(&self, request: &ApprovalRequest) -> Decision {
                assert!(
                    request.policy_warning.is_some(),
                    "forced review must carry the policy warning"
                );
                self.reviews.fetch_add(1, Ordering::SeqCst);
                Decision::Approve
            }
        }
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "run_command", r#"{"command":["touch","made.txt"]}"#),
            final_text("done"),
        ]));
        let mut agent = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let gate = RecordingGate {
            reviews: AtomicU32::new(0),
        };
        agent
            .run_turn(
                "make a file".into(),
                &gate,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        // Auto mode did NOT auto-run the unknown binary: the gate was
        // consulted once, and after approval the command actually executed.
        assert_eq!(gate.reviews.load(Ordering::SeqCst), 1);
        assert!(dir.path().join("made.txt").exists());
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
    async fn working_state_tracks_plan_and_tools_and_feeds_checkpoints() {
        let dir = workspace();
        let state = tempfile::tempdir().unwrap();
        let store = SessionStore::create_under(state.path(), "ws").unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call(
                "c1",
                "update_plan",
                r#"{"objective":"greet the world","tasks":[{"text":"read file","done":false}]}"#,
            ),
            assistant_with_call("c2", "read_file", r#"{"path":"hello.txt"}"#),
            assistant_with_call("c3", "run_command", r#"{"command":["cat","hello.txt"]}"#),
            final_text("done"),
            // Summarizer turn for compact_now below.
            final_text("narrative"),
        ]));
        let mut agent = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            Some(store),
        )
        .unwrap();
        agent
            .run_turn(
                "do the thing".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();

        let state_now = agent.working_state();
        assert_eq!(state_now.objective.as_deref(), Some("greet the world"));
        assert_eq!(state_now.tasks.len(), 1);
        assert!(state_now.files_read.contains("hello.txt"));
        assert_eq!(state_now.commands.len(), 1);
        assert!(state_now.commands[0].ok);
        // The plan confirmation flowed back to the model as a tool result.
        assert!(agent.messages().iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("plan updated")
        )));

        // Compaction checkpoint carries the tracked state verbatim.
        agent
            .compact_now(&CancellationToken::new(), None)
            .await
            .unwrap();
        match &agent.messages()[1] {
            ChatMessage::User { content } => {
                assert!(content.contains("Objective: greet the world"), "{content}");
                assert!(content.contains("`cat hello.txt` → ok"), "{content}");
                assert!(content.contains("narrative"));
            }
            other => panic!("expected checkpoint, got {other:?}"),
        }
        // State survives compaction (it lives outside the transcript)…
        assert_eq!(
            agent.working_state().objective.as_deref(),
            Some("greet the world")
        );

        // …and survives resume via replay.
        let store = SessionStore::create_under(state.path(), "ws").unwrap();
        let resumed = Agent::resume(
            Arc::new(ScriptedProvider::new(vec![])),
            dir.path().to_path_buf(),
            SessionConfig::default(),
            store,
            "ws".into(),
        )
        .unwrap();
        assert_eq!(resumed.working_state(), agent.working_state());
    }

    #[tokio::test]
    async fn atlas_tools_map_search_and_track_own_writes() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "repo_map", r#"{}"#),
            assistant_with_call("c2", "symbol_search", r#"{"name":"alpha"}"#),
            assistant_with_call(
                "c3",
                "edit",
                r#"{"path":"lib.rs","old_string":"pub fn alpha() {}","new_string":"pub fn alpha() {}\npub fn gamma() {}"}"#,
            ),
            assistant_with_call("c4", "symbol_search", r#"{"name":"gamma"}"#),
            final_text("done"),
        ]));
        let mut agent = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        agent
            .run_turn(
                "map it".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let tool_contents: Vec<&str> = agent
            .messages()
            .iter()
            .filter_map(|m| match m {
                ChatMessage::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            tool_contents[0].contains("Atlas repo map") && tool_contents[0].contains("fn alpha"),
            "{}",
            tool_contents[0]
        );
        assert!(
            tool_contents[1].contains("lib.rs:1 [fn]"),
            "{}",
            tool_contents[1]
        );
        // The agent's own edit was invalidated and re-indexed: the NEW symbol
        // is findable in the same turn.
        assert!(
            tool_contents[3].contains("lib.rs:2 [fn] pub fn gamma"),
            "{}",
            tool_contents[3]
        );
    }

    #[tokio::test]
    async fn steering_messages_enter_history_at_the_turn_boundary() {
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
        let steer = agent.steer_handle();
        steer
            .lock()
            .unwrap()
            .push_back("also check the tests".to_string());
        agent
            .run_turn(
                "read the file".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        // The steered message was delivered as a user message and reached the
        // provider in the very first request of the turn.
        let requests = provider.recorded_requests();
        assert!(requests[0].iter().any(|m| matches!(
            m,
            ChatMessage::User { content } if content == "also check the tests"
        )));
        assert!(steer.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn search_fusion_makes_either_tool_return_the_best_of_both() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("lib.rs"),
            "// the magic_token lives here\npub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![
            // Model greps for a known symbol: Atlas defs ride along.
            assistant_with_call("c1", "search_text", r#"{"pattern":"alpha"}"#),
            // Model asks the index for something only present in a comment:
            // ripgrep fallback rides along.
            assistant_with_call("c2", "symbol_search", r#"{"name":"magic_token"}"#),
            final_text("done"),
        ]));
        let mut agent = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        agent
            .run_turn(
                "find things".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let tool_contents: Vec<&str> = agent
            .messages()
            .iter()
            .filter_map(|m| match m {
                ChatMessage::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            tool_contents[0].contains("Definitions (Atlas):")
                && tool_contents[0].contains("lib.rs:2 [fn]")
                && tool_contents[0].contains("Text matches (ripgrep):"),
            "{}",
            tool_contents[0]
        );
        assert!(
            tool_contents[1].contains("no definitions found in the Atlas index")
                && tool_contents[1].contains("ripgrep fallback")
                && tool_contents[1].contains("magic_token lives here"),
            "{}",
            tool_contents[1]
        );
    }

    #[tokio::test]
    async fn disabling_atlas_removes_only_the_tail_schemas() {
        let dir = workspace();
        let with = Agent::new(
            Arc::new(ScriptedProvider::new(vec![])),
            dir.path().to_path_buf(),
            SessionConfig::default(),
            None,
        )
        .unwrap();
        let without = Agent::new(
            Arc::new(ScriptedProvider::new(vec![])),
            dir.path().to_path_buf(),
            SessionConfig {
                atlas_enabled: false,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(with.tools.len(), without.tools.len() + 2);
        // The remaining schemas are a strict prefix of the full set: byte
        // stability for non-atlas sessions is unaffected.
        for (a, b) in without.tools.iter().zip(with.tools.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(
                serde_json::to_string(&a.parameters).unwrap(),
                serde_json::to_string(&b.parameters).unwrap()
            );
        }
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

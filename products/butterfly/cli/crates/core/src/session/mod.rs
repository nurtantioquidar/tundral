//! The interactive agent session: an append-only conversation driven by a
//! deterministic loop. The model proposes tool calls; this code owns budgets,
//! approval policy, context management, and termination.

pub mod approval;
pub mod store;
pub mod working_state;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use crate::context::{CompactionNeed, ContextConfig, ContextManager};
use crate::domain::{format_microusd_as_usd, ChatMessage, Usage};
use crate::failure::{Failure, FailureKind};
use crate::model::ModelDescriptor;
use crate::provider::{
    CompletedTurn, Provider, ProviderError, ProviderRequest, RoutingFacts, RoutingMetadata,
    StreamSink, TaskFeedback, ToolSchema, NIMBLE_CONTRACT_VERSION,
};
use crate::telemetry::{self, TelemetryEvent};
use crate::tools::policy::PolicySettings;
use crate::tools::{describe_call, tool_kind, tool_schemas, ToolExecutor};
use crate::verify::{self, VerificationOutcome, VerificationPlan, WorkspaceSnapshot};

/// The session's tool list; Atlas tools drop out when disabled and the shell
/// tool exists only in sandboxed sessions (both are the tail of the frozen
/// ordering, so the shared prefix of the remaining schemas is unchanged).
fn session_tools(
    atlas_enabled: bool,
    sandboxed: bool,
    completion_gate: bool,
    verifier: bool,
) -> Vec<ToolSchema> {
    let mut tools = tool_schemas();
    if !atlas_enabled {
        tools.retain(|tool| tool.name != "repo_map" && tool.name != "symbol_search");
    }
    if !sandboxed {
        tools.retain(|tool| tool.name != "shell");
    }
    if !completion_gate {
        tools.retain(|tool| tool.name != "complete_task");
    }
    if verifier {
        // The verifier observes and runs; it never edits or plans.
        tools.retain(|tool| {
            !matches!(
                tool.name.as_str(),
                "edit" | "write_file" | "update_plan" | "complete_task"
            )
        });
    } else {
        tools.retain(|tool| tool.name != "report_verdict");
    }
    tools
}

/// Text-only replies tolerated per turn before the gate gives up on the
/// attempt (each one is answered with `COMPLETION_NUDGE`).
const COMPLETION_NUDGE_CAP: u32 = 3;
const COMPLETION_NUDGE: &str = "[completion gate: the task is not finished until you call \
complete_task. Continue working with tools; when every deliverable exists and is verified, call \
complete_task with the evidence for each one.]";
/// Longest task statement echoed back in the completion review.
const REVIEW_STATEMENT_CAP: usize = 8_000;
/// Claims rejected for weak acceptance checks before one is accepted anyway
/// (recorded as a claim, never as verified).
const WEAK_CHECK_REJECTIONS: u32 = 2;
/// Final claims rejected for lacking the requirement checklist (once: the
/// list is the verifier's input, not a hoop).
const REQUIREMENTS_REJECTIONS: u32 = 1;
/// Provider calls the independent verifier may spend on one claim.
/// Observed 2026-09-09: GLM verifying a Muse claim built a scratch copy and a
/// harness, then spent its last calls debugging the harness and never
/// reported (20 calls, $0.036, no verdict).
const VERIFIER_CALL_CAP: u32 = 30;
/// Calls left when the verifier is told to report with what it has.
const VERIFIER_WARNING_CALLS_LEFT: u32 = 3;
/// Text-only verifier replies answered with `VERDICT_NUDGE` before giving up.
const VERDICT_NUDGE_CAP: u32 = 2;
const VERDICT_NUDGE: &str = "[verifier: nothing is recorded until you call report_verdict. \
Test the remaining requirements with commands, then call report_verdict with every requirement, \
its status, and the evidence.]";
/// Changed paths listed for the verifier.
const VERIFIER_FILES_CAP: usize = 60;
/// Longest evidence string kept per requirement in verdicts and claims.
const EVIDENCE_CAP: usize = 600;

/// An acceptance check that cannot fail on wrong behaviour: it only proves a
/// file exists or parses. Observed 2026-09-04: `python -m compileall` offered
/// as the proof for a task whose hidden tests then failed 3 of 7.
pub fn is_weak_check(argv: &[String]) -> bool {
    let joined = argv.join(" ");
    let lower = joined.to_ascii_lowercase();
    let first = argv
        .first()
        .map(|s| s.rsplit('/').next().unwrap_or(s))
        .unwrap_or("");
    const TRIVIAL_BINARIES: &[&str] = &[
        "true", "false", "echo", "ls", "cat", "stat", "test", "[", "head", "tail", "wc", "file",
        "pwd", "which", "type",
    ];
    if TRIVIAL_BINARIES.contains(&first) {
        return true;
    }
    const COMPILE_ONLY: &[&str] = &[
        "compileall",
        "py_compile",
        "--syntax-only",
        "-fsyntax-only",
        "--check",
        "--dry-run",
        "--version",
        "--help",
        "cargo check",
        "cargo build",
        "go build",
        "go vet",
        "tsc --noemit",
        "node --check",
        "bun build",
        "ruff check",
        "eslint",
        "flake8",
        "pylint",
        "mypy",
        "black --check",
        "prettier --check",
        "gofmt -l",
        "shellcheck",
        "-c import ",
        "-c \"import ",
        "-c 'import ",
    ];
    COMPILE_ONLY.iter().any(|needle| lower.contains(needle))
}

/// What the model declared when it called `complete_task` the second time.
/// A claim, never a verdict: `TaskReport::verified` is decided by checks.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompletionClaim {
    pub summary: String,
    pub deliverables: Vec<Deliverable>,
    /// Acceptance commands (argv vectors) the harness runs as model-declared checks.
    #[serde(default)]
    pub checks: Vec<Vec<String>>,
    /// Every requirement of the task statement as the model read it, met or
    /// not: the spec-anchored checklist the independent verifier grades.
    #[serde(default)]
    pub requirements: Vec<Requirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Deliverable {
    pub item: String,
    pub evidence: String,
}

/// One requirement of the task statement with the status a party assigned it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Requirement {
    pub requirement: String,
    /// met | unmet (worker) | unverifiable (verifier only).
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub evidence: String,
}

impl Requirement {
    fn is_met(&self) -> bool {
        self.status.eq_ignore_ascii_case("met")
    }
}

/// What the independent verifier concluded about a completion claim.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Verdict {
    /// pass | fail.
    pub verdict: String,
    pub requirements: Vec<Requirement>,
}

impl Verdict {
    pub fn passed(&self) -> bool {
        self.verdict.eq_ignore_ascii_case("pass") && self.unmet().is_empty()
    }
    pub fn unmet(&self) -> Vec<&Requirement> {
        self.requirements.iter().filter(|r| !r.is_met()).collect()
    }
}

/// The independent verifier's run over one attempt's claim.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct VerifierReport {
    /// None when the verifier ended without a verdict (cap, error, cancel):
    /// the claim then stands, exactly as without a verifier.
    pub verdict: Option<Verdict>,
    /// The model that verified (the worker's own unless a backend was set).
    pub model: String,
    pub provider_calls: u32,
    pub tool_calls: u32,
    pub actual_microusd: u64,
    pub error: Option<String>,
}
use approval::{
    policy_outcome, ApprovalGate, ApprovalRequest, Decision, PermissionMode, PolicyOutcome,
};
use store::{replay, SessionEvent, SessionStore};

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub model: Arc<ModelDescriptor>,
    /// Reasoning effort, validated against the model (`resolve_effort`).
    pub effort: Option<String>,
    pub permission_mode: PermissionMode,
    pub max_output_tokens: u32,
    /// Hard cap on total actual session cost; None = uncapped.
    pub max_session_cost_microusd: Option<u64>,
    /// Provider round-trips allowed for a single user input.
    pub max_provider_calls_per_input: u32,
    pub context: ContextConfig,
    /// Expose the Atlas code-graph tools (repo_map, symbol_search).
    pub atlas_enabled: bool,
    /// The workspace is a disposable container the operator owns: relax path
    /// confinement and the command allowlist, and expose the `shell` tool.
    pub sandboxed: bool,
    /// Coarse task class forwarded to a router (never inferred from prompts).
    pub task_type: Option<String>,
    /// Autonomous runs must earn their ending: advertise `complete_task`, nudge
    /// text-only replies, review the task statement before accepting the
    /// claim. Appends to the prefix only when on.
    pub completion_gate: bool,
    /// Review every accepted claim in a fresh-context verifier session before
    /// the loop treats it as an ending; unmet requirements become the next
    /// attempt's evidence. Requires the completion gate.
    pub independent_verifier: bool,
    /// This session IS the verifier: read-only tools plus `report_verdict`,
    /// verifier instructions in its own (separate) prefix. Internal; set by
    /// `run_task`, never by operators.
    pub verifier: bool,
}

impl SessionConfig {
    pub fn policy(&self) -> PolicySettings {
        if self.sandboxed {
            PolicySettings::SANDBOXED
        } else {
            PolicySettings::default()
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        let model = crate::model::reference_model();
        let effort = model.resolve_effort(None).unwrap_or(None);
        Self {
            model,
            effort,
            permission_mode: PermissionMode::Ask,
            // A ceiling, not a spend: reasoning shares this budget, and
            // 8k starved pro-at-high-effort into content-less turns
            // (dogfood 2026-09-03). The API accepts far larger caps.
            max_output_tokens: 65_536,
            max_session_cost_microusd: None,
            max_provider_calls_per_input: 50,
            context: ContextConfig::default(),
            atlas_enabled: true,
            sandboxed: false,
            task_type: None,
            completion_gate: false,
            independent_verifier: false,
            verifier: false,
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
    /// The turn ended because it used its provider-call budget, not because
    /// the model finished.
    pub turn_limit_reached: bool,
    pub provider_calls: u32,
    pub tool_calls: u32,
    pub usage: Usage,
    pub actual_microusd: u64,
    /// Cache-blind cost (all input at the miss rate) for fair comparisons.
    pub normalized_microusd: u64,
    pub compactions: u32,
    pub elisions: u32,
    /// Provider wall-clock, summed over the turn's requests.
    pub latency_ms: u64,
    /// HTTP retries the provider needed, summed.
    pub retries: u32,
    /// Classified tool failures observed this turn (bounded).
    pub failures: Vec<Failure>,
    /// The model called `complete_task` (after the review) — a claim.
    pub completed: bool,
    pub completion: Option<CompletionClaim>,
    /// Text-only replies the gate answered with a nudge this turn.
    pub completion_nudges: u32,
    /// The gate gave up: the model kept replying without tools or a claim.
    pub gate_unfinished: bool,
    /// Declared checks that run a file this session created or changed: the
    /// model grading its own homework, counted as a claim, never as proof.
    pub self_authored_checks: u32,
    /// Verifier sessions only: the verdict `report_verdict` recorded.
    pub verdict: Option<Verdict>,
}

const TURN_FAILURES_CAP: usize = 50;
/// Mid-stream transport failures (and upstream 5xx that survived the HTTP
/// retries) re-issued per provider call, with a doubling backoff capped at
/// `STREAM_TRANSPORT_MAX_BACKOFF_MS`: 1.5, 3, 6, 12, 24, 48s ≈ 95s of patience
/// on top of the HTTP layer's own minute. Observed 2026-09-06: a network drop
/// of a few minutes ended three trials that two short retries could not span.
const STREAM_TRANSPORT_RETRIES: u32 = 6;
const STREAM_TRANSPORT_BACKOFF_MS: u64 = 1_500;
const STREAM_TRANSPORT_MAX_BACKOFF_MS: u64 = 60_000;
/// Rate limits relayed inside a stream get more patience: the upstream names
/// the wait ("try again in 5.3s"); honour it, capped, up to this many times.
const STREAM_RATE_LIMIT_RETRIES: u32 = 4;
const STREAM_RATE_LIMIT_MAX_WAIT_MS: u64 = 45_000;

/// Autonomous-mode options for `run_task`.
#[derive(Debug, Clone)]
pub struct TaskOptions {
    /// Attempts (each = one turn + verification). 1 = a plain turn.
    pub max_attempts: u32,
    pub verification: VerificationPlan,
}

impl Default for TaskOptions {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            verification: VerificationPlan::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AttemptReport {
    pub attempt: u32,
    pub turn: TurnReport,
    pub verification: Option<VerificationOutcome>,
    /// The independent verifier's review of this attempt's claim, when run.
    pub verifier: Option<VerifierReport>,
}

/// The outcome of `run_task`: every attempt plus aggregate counters.
#[derive(Debug, Clone, Default)]
pub struct TaskReport {
    pub attempts: Vec<AttemptReport>,
    /// The loop's decision: Some(true) every check passed; Some(false) still
    /// failing after the last attempt; None when nothing decided (no plan, or
    /// the model's claim stood).
    pub success: Option<bool>,
    /// Truthful verdict: Some(true) only when an explicit or detected check
    /// passed (an oracle the model did not write); Some(false) when a check
    /// failed; None when only a diff test, model-declared checks, or a bare
    /// claim backed the ending. Reported as `verified` and fed to routers.
    pub verified: Option<bool>,
    /// What backed the ending: explicit | detected | model_declared | diff | claim | none.
    pub verification_source: &'static str,
    /// The final attempt ended with a `complete_task` claim.
    pub completed: bool,
    pub completion: Option<CompletionClaim>,
    /// The final attempt's assistant text.
    pub assistant_text: String,
    pub cancelled: bool,
    pub budget_exhausted: bool,
    pub hit_output_limit: bool,
    pub provider_calls: u32,
    pub tool_calls: u32,
    pub usage: Usage,
    pub actual_microusd: u64,
    pub normalized_microusd: u64,
    pub compactions: u32,
    pub elisions: u32,
    pub latency_ms: u64,
    pub retries: u32,
}

impl TaskReport {
    fn absorb(&mut self, turn: &TurnReport) {
        self.assistant_text = turn.assistant_text.clone();
        self.completed = turn.completed;
        self.completion = turn.completion.clone();
        self.cancelled |= turn.cancelled;
        self.budget_exhausted |= turn.budget_exhausted;
        self.hit_output_limit |= turn.hit_output_limit;
        self.provider_calls += turn.provider_calls;
        self.tool_calls += turn.tool_calls;
        self.usage.add(turn.usage);
        self.actual_microusd += turn.actual_microusd;
        self.normalized_microusd += turn.normalized_microusd;
        self.compactions += turn.compactions;
        self.elisions += turn.elisions;
        self.latency_ms += turn.latency_ms;
        self.retries += turn.retries;
    }
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
    telemetry: telemetry::Recorder,
    /// Failed `edit` signatures (path + old_string hash) → failure count,
    /// for repeated-patch detection. Cleared when the edit succeeds.
    failed_edits: HashMap<u64, u32>,
    /// A history rewrite (elision/compaction) preceded the next request, so
    /// its cache-miss ratio is expected, not a regression.
    rewrote_history: bool,
    /// Increments on every history rewrite; tells a router the prefix changed.
    context_version: u32,
    /// Current attempt within `run_task` (1 outside autonomous runs).
    attempt: u32,
    /// The previous attempt failed deterministic verification.
    last_verification_failed: bool,
    /// Routing facts from the most recent provider response, if it is a router.
    last_routing: Option<RoutingFacts>,
    /// Pre-task workspace fingerprint for `--require-diff` outside git.
    diff_baseline: Option<WorkspaceSnapshot>,
    /// The task statement of the current `run_task`, echoed in the completion review.
    task_statement: Option<String>,
    /// The completion review was issued this attempt; the next claim is final.
    completion_reviewed: bool,
    /// The final claim of this attempt, once made.
    completion_claim: Option<CompletionClaim>,
    /// Claims rejected this attempt for lacking a behavioural check.
    weak_check_rejections: u32,
    /// Final claims rejected this attempt for lacking the requirement list.
    requirements_rejections: u32,
    /// Workspace fingerprint taken when the gated task started, so a declared
    /// check can be recognised as running a file the model wrote.
    gate_baseline: Option<WorkspaceSnapshot>,
    /// Declared checks recognised as self-authored at claim time.
    self_authored_checks: Vec<Vec<String>>,
    /// Verifier sessions: the verdict once `report_verdict` was called.
    verdict: Option<Verdict>,
    verdict_nudges: u32,
    /// A different provider and model for the independent verifier, so it
    /// does not share the worker's blind spots. None = the worker's own.
    verifier_backend: Option<(Arc<dyn Provider>, Arc<ModelDescriptor>)>,
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
            config.sandboxed,
            config.completion_gate,
            config.verifier,
        );
        let session_id = uuid::Uuid::new_v4().simple().to_string();
        if let Some(store) = &store {
            store.append(&SessionEvent::SessionStarted {
                at_epoch_ms: store::epoch_ms_now(),
                workspace: workspace_name.clone(),
                model: config.model.id.clone(),
                provider: Some(config.model.provider.clone()),
                system_prompt: system_prompt.clone(),
            })?;
        }
        let mut context = ContextManager::new(config.context.clone());
        let messages = vec![ChatMessage::System {
            content: system_prompt,
        }];
        context.ledger.rebase(&messages);
        let atlas = butterfly_atlas::Atlas::new(root.clone());
        let telemetry = telemetry::Recorder::new(
            store
                .as_ref()
                .map(|store| store.directory().join("telemetry.jsonl")),
        );
        Ok(Self {
            session_id,
            provider,
            executor: ToolExecutor::new(root).with_policy(config.policy()),
            tools: session_tools(
                config.atlas_enabled,
                config.sandboxed,
                config.completion_gate,
                config.verifier,
            ),
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
            telemetry,
            failed_edits: HashMap::new(),
            rewrote_history: false,
            context_version: 0,
            attempt: 1,
            last_verification_failed: false,
            last_routing: None,
            diff_baseline: None,
            task_statement: None,
            completion_reviewed: false,
            completion_claim: None,
            weak_check_rejections: 0,
            requirements_rejections: 0,
            gate_baseline: None,
            self_authored_checks: Vec::new(),
            verdict: None,
            verdict_nudges: 0,
            verifier_backend: None,
        })
    }

    /// Resume from a persisted event log. History replays byte-exactly, so a
    /// still-warm provider prefix cache keeps hitting. The caller resolves the
    /// model recorded in the log (see `store::replay`) into `config.model`.
    pub fn resume(
        provider: Arc<dyn Provider>,
        root: PathBuf,
        config: SessionConfig,
        store: SessionStore,
        session_id: String,
    ) -> Result<Self, SessionError> {
        let events = store.read_events()?;
        let replayed = replay(&events);
        if replayed.messages.is_empty() {
            return Err(SessionError::Resume("event log has no messages".into()));
        }
        let mut context = ContextManager::new(config.context.clone());
        context.ledger.rebase(&replayed.messages);
        let atlas = butterfly_atlas::Atlas::new(root.clone());
        let telemetry = telemetry::Recorder::new(Some(store.directory().join("telemetry.jsonl")));
        Ok(Self {
            session_id,
            provider,
            executor: ToolExecutor::new(root).with_policy(config.policy()),
            tools: session_tools(
                config.atlas_enabled,
                config.sandboxed,
                config.completion_gate,
                config.verifier,
            ),
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
            telemetry,
            failed_edits: HashMap::new(),
            rewrote_history: false,
            context_version: 0,
            attempt: 1,
            last_verification_failed: false,
            last_routing: None,
            diff_baseline: None,
            task_statement: None,
            completion_reviewed: false,
            completion_claim: None,
            weak_check_rejections: 0,
            requirements_rejections: 0,
            gate_baseline: None,
            self_authored_checks: Vec::new(),
            verdict: None,
            verdict_nudges: 0,
            verifier_backend: None,
        })
    }

    /// Run the independent verifier on another provider and model
    /// (cross-model verification). Observed 2026-09-09: a same-model verifier
    /// tested 14 and 19 probes on two Muse claims and passed both; the hidden
    /// tests failed the same requirements the worker had misread.
    pub fn set_verifier_backend(
        &mut self,
        provider: Arc<dyn Provider>,
        model: Arc<ModelDescriptor>,
    ) {
        self.verifier_backend = Some((provider, model));
    }

    pub fn working_state(&self) -> &working_state::WorkingState {
        &self.working_state
    }

    pub fn telemetry_summary(&self) -> &telemetry::Summary {
        &self.telemetry.summary
    }

    pub fn telemetry_path(&self) -> Option<&std::path::Path> {
        self.telemetry.path()
    }

    /// Redirect telemetry (e.g. `--telemetry`); takes effect for later events.
    pub fn set_telemetry_path(&mut self, path: PathBuf) {
        self.telemetry.set_path(path);
    }

    pub fn sandboxed(&self) -> bool {
        self.config.sandboxed
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

    pub fn model(&self) -> Arc<ModelDescriptor> {
        self.config.model.clone()
    }

    pub fn effort(&self) -> Option<&str> {
        self.config.effort.as_deref()
    }

    pub fn permission_mode(&self) -> PermissionMode {
        self.config.permission_mode
    }

    pub fn set_permission_mode(&mut self, mode: PermissionMode) {
        self.config.permission_mode = mode;
    }

    /// Switch models mid-session. History stays valid; the provider's prefix
    /// cache is per model, so the next request pays a one-time re-cache. The
    /// effort is re-resolved against the new model (an explicit value the new
    /// model rejects falls back to its default).
    pub fn set_model(&mut self, model: Arc<ModelDescriptor>, provider: Arc<dyn Provider>) {
        self.config.effort = model
            .resolve_effort(self.config.effort.as_deref())
            .or_else(|_| model.resolve_effort(None))
            .unwrap_or(None);
        self.config.model = model;
        self.provider = provider;
        self.context
            .set_window_tokens(self.config.model.limits.context_tokens);
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
            let before = self.context.ledger.estimated_next_prompt_tokens();
            self.context.apply_elision(&mut self.messages, &plan);
            self.telemetry.record(TelemetryEvent::Compaction {
                at_epoch_ms: telemetry::epoch_ms_now(),
                stage: "elision".into(),
                before_tokens: before,
                after_tokens: self.context.ledger.estimated_next_prompt_tokens(),
            });
            self.rewrote_history = true;
            self.context_version += 1;
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
        let routing = Some(self.routing_metadata("summarization"));
        let outcome = self
            .context
            .summarize(
                &mut self.messages,
                &checkpoint,
                focus,
                &self.tools,
                self.provider.as_ref(),
                self.config.model.clone(),
                self.config.effort.clone(),
                routing,
                cancel,
            )
            .await?;
        self.total_usage.add(outcome.summarizer_usage);
        self.total_actual_microusd += outcome.summarizer_cost.actual_microusd;
        self.telemetry.record(TelemetryEvent::Compaction {
            at_epoch_ms: telemetry::epoch_ms_now(),
            stage: "summarize".into(),
            before_tokens: outcome.before_tokens,
            after_tokens: outcome.after_tokens,
        });
        self.rewrote_history = true;
        self.context_version += 1;
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

    fn request(&self) -> ProviderRequest {
        ProviderRequest {
            model: self.config.model.clone(),
            messages: self.messages.clone(),
            tools: self.tools.clone(),
            max_output_tokens: self
                .config
                .max_output_tokens
                .min(self.config.model.limits.max_output_tokens),
            effort: self.config.effort.clone(),
            routing: Some(self.routing_metadata(if self.attempt > 1 {
                "recovery"
            } else {
                "implementation"
            })),
        }
    }

    async fn complete_once(
        &mut self,
        cancel: &CancellationToken,
        sink: &mut dyn StreamSink,
    ) -> Result<CompletedTurn, SessionError> {
        // A stream that dies mid-response (router restart, proxy hiccup,
        // upstream reset) is a transport error the HTTP-level retry cannot
        // see: nothing of the partial turn was recorded, so re-issuing the
        // same request is safe. Bounded, backed off, cancel-aware. Observed
        // 2026-09-04: two Terminal-Bench trials lost to one router restart.
        let mut transport_retries: u32 = 0;
        loop {
            let request = self.request();
            match self.provider.stream(&request, cancel, sink).await {
                Ok(mut turn) => {
                    turn.retries += transport_retries;
                    return Ok(turn);
                }
                Err(ProviderError::ContextLengthExceeded) => {
                    // Last resort: shrink the retained tail, compact, retry once.
                    self.context.shrink_keep_last_turns();
                    self.compact_now(cancel, None).await?;
                    let request = self.request();
                    let mut turn = self.provider.stream(&request, cancel, sink).await?;
                    turn.retries += transport_retries;
                    return Ok(turn);
                }
                Err(ProviderError::RateLimited { body, .. })
                    if transport_retries < STREAM_RATE_LIMIT_RETRIES && !cancel.is_cancelled() =>
                {
                    // Observed 2026-09-05: a 500k tokens-per-minute cap on
                    // 100k-token requests, relayed by the router as an
                    // in-stream error; one wait would have saved the task.
                    transport_retries += 1;
                    let wait = crate::provider::openai_compat::retry_after_hint(&body)
                        .unwrap_or(std::time::Duration::from_millis(
                            STREAM_TRANSPORT_BACKOFF_MS * (1u64 << transport_retries),
                        ))
                        .min(std::time::Duration::from_millis(
                            STREAM_RATE_LIMIT_MAX_WAIT_MS,
                        ))
                        + std::time::Duration::from_millis(500);
                    self.telemetry.record(TelemetryEvent::ProviderError {
                        at_epoch_ms: telemetry::epoch_ms_now(),
                        model: self.config.model.id.clone(),
                        provider: self.config.model.provider.clone(),
                        kind: FailureKind::EnvironmentFailure,
                        summary: format!(
                            "rate limited, retry {transport_retries}/{STREAM_RATE_LIMIT_RETRIES} after {}s: {}",
                            wait.as_secs(),
                            crate::failure::first_line(&body)
                        ),
                    });
                    sink.on_notice(&format!(
                        "[rate limited; retrying {transport_retries}/{STREAM_RATE_LIMIT_RETRIES} in {}s]",
                        wait.as_secs()
                    ));
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = cancel.cancelled() => return Err(ProviderError::Cancelled.into()),
                    }
                }
                Err(error @ (ProviderError::Transport(_) | ProviderError::Status { .. }))
                    if is_transient(&error)
                        && transport_retries < STREAM_TRANSPORT_RETRIES
                        && !cancel.is_cancelled() =>
                {
                    let text = match &error {
                        ProviderError::Transport(text) => text.clone(),
                        ProviderError::Status { status, body } => {
                            format!("status {status}: {body}")
                        }
                        _ => unreachable!("guarded by the pattern"),
                    };
                    transport_retries += 1;
                    self.telemetry.record(TelemetryEvent::ProviderError {
                        at_epoch_ms: telemetry::epoch_ms_now(),
                        model: self.config.model.id.clone(),
                        provider: self.config.model.provider.clone(),
                        kind: crate::failure::classify_provider_error(&ProviderError::Transport(
                            text.clone(),
                        )),
                        summary: format!(
                            "stream interrupted, retry {transport_retries}/{STREAM_TRANSPORT_RETRIES}: {}",
                            crate::failure::first_line(&text)
                        ),
                    });
                    sink.on_notice(&format!(
                        "[stream interrupted ({}); retrying {transport_retries}/{STREAM_TRANSPORT_RETRIES}]",
                        crate::failure::first_line(&text)
                    ));
                    let delay = std::time::Duration::from_millis(
                        (STREAM_TRANSPORT_BACKOFF_MS * (1u64 << (transport_retries - 1)))
                            .min(STREAM_TRANSPORT_MAX_BACKOFF_MS),
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = cancel.cancelled() => return Err(ProviderError::Cancelled.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
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
            if self.config.verifier
                && self.verdict.is_none()
                && self
                    .config
                    .max_provider_calls_per_input
                    .saturating_sub(report.provider_calls)
                    == VERIFIER_WARNING_CALLS_LEFT
            {
                self.push_user(format!(
                    "[verifier: {VERIFIER_WARNING_CALLS_LEFT} calls remain. Stop building tooling \
                     and call report_verdict now with the evidence you have; mark anything you \
                     could not test as unverifiable.]"
                ))?;
            }
            if report.provider_calls >= self.config.max_provider_calls_per_input {
                report.turn_limit_reached = true;
                report.assistant_text.push_str(
                    "\n[turn limit reached for this input; send a follow-up to continue]",
                );
                return Ok(report);
            }
            self.manage_context(cancel, &mut report).await?;

            let turn = match self.complete_once(cancel, sink).await {
                Ok(mut turn) => {
                    sanitize_tool_arguments(&mut turn.message);
                    turn
                }
                Err(SessionError::Provider(ProviderError::Cancelled)) => {
                    report.cancelled = true;
                    return Ok(report);
                }
                Err(error) => {
                    if let SessionError::Provider(provider_error) = &error {
                        let kind = crate::failure::classify_provider_error(provider_error);
                        self.working_state
                            .record_failure(kind, &provider_error.to_string());
                        self.telemetry.record(TelemetryEvent::ProviderError {
                            at_epoch_ms: telemetry::epoch_ms_now(),
                            model: self.config.model.id.clone(),
                            provider: self.config.model.provider.clone(),
                            kind,
                            summary: crate::failure::first_line(&provider_error.to_string()),
                        });
                    }
                    return Err(error);
                }
            };
            report.provider_calls += 1;
            report.latency_ms += turn.latency_ms;
            report.retries += turn.retries;
            self.telemetry.record(TelemetryEvent::ProviderCall {
                at_epoch_ms: telemetry::epoch_ms_now(),
                model: self.config.model.id.clone(),
                provider: self.config.model.provider.clone(),
                latency_ms: turn.latency_ms,
                retries: turn.retries,
                usage: turn.usage,
                actual_microusd: turn.cost.actual_microusd,
                normalized_microusd: turn.cost.normalized_cache_miss_microusd,
                price_verified: turn.cost.price_verified,
                finish: telemetry::finish_name(turn.finish).into(),
                after_rewrite: self.rewrote_history,
            });
            self.rewrote_history = false;
            if let Some(facts) = &turn.routing {
                if facts.model_changed {
                    // Appended at the next provider-call boundary: the prefix
                    // stays byte-stable, the model learns why earlier turns
                    // read differently. Nothing here instructs a solution.
                    self.steer.lock().expect("steer lock").push_back(format!(
                        "[routing note: requests are now served by {} ({}). Earlier turns came \
                         from a different model; continue the task from the current state.]",
                        facts.selected_model, facts.routing_reason
                    ));
                }
                if facts.context_limit > 0 {
                    self.context.set_window_tokens(facts.context_limit);
                }
                self.last_routing = Some(facts.clone());
            }
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
            // An assistant message with neither content nor tool calls (the
            // output budget ran out mid-reasoning) is not replayable: DeepSeek
            // rejects it ("content or tool_calls must be set"). Keep its usage,
            // drop the message; the retry evidence explains what happened.
            let replayable = turn
                .message
                .content
                .as_deref()
                .is_some_and(|c| !c.is_empty())
                || !turn.message.tool_calls.is_empty();
            if replayable {
                self.record(&SessionEvent::AssistantMessage {
                    message: turn.message.clone(),
                })?;
            }
            self.record(&SessionEvent::UsageRecorded {
                usage: turn.usage,
                actual_microusd: turn.cost.actual_microusd,
                normalized_cache_miss_microusd: turn.cost.normalized_cache_miss_microusd,
            })?;
            if replayable {
                self.messages
                    .push(ChatMessage::Assistant(turn.message.clone()));
            }

            if let Some(text) = &turn.message.content {
                if !report.assistant_text.is_empty() {
                    report.assistant_text.push('\n');
                }
                report.assistant_text.push_str(text);
            }
            if turn.message.tool_calls.is_empty() {
                // The verifier's ending is a verdict, not prose (bounded nudge).
                if self.config.verifier && self.verdict.is_none() {
                    if self.verdict_nudges < VERDICT_NUDGE_CAP {
                        self.verdict_nudges += 1;
                        self.push_user(VERDICT_NUDGE.to_string())?;
                        continue;
                    }
                    return Ok(report);
                }
                // The gate: a reply without tools is not an ending. Answer it
                // with the same nudge (bounded), never a new instruction.
                if self.config.completion_gate && !report.completed {
                    if report.completion_nudges < COMPLETION_NUDGE_CAP {
                        report.completion_nudges += 1;
                        self.push_user(COMPLETION_NUDGE.to_string())?;
                        continue;
                    }
                    report.gate_unfinished = true;
                }
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
                let started = std::time::Instant::now();
                let output = self.execute_gated(call, gate, cancel).await;
                if output.is_none() {
                    cancelled_mid_tools = true;
                    sink.on_tool_result(&preview, true, None);
                    self.push_tool_result(call.id.clone(), "cancelled by user".into())?;
                    continue;
                }
                let mut output = output.expect("checked above");
                let failure = self.classify_tool_failure(call, &mut output);
                self.telemetry.record(TelemetryEvent::ToolCall {
                    at_epoch_ms: telemetry::epoch_ms_now(),
                    name: call.name.clone(),
                    preview: preview.clone(),
                    ok: !output.is_error,
                    duration_ms: started.elapsed().as_millis() as u64,
                    failure: failure.as_ref().map(|f| f.kind),
                });
                if let Some(failure) = failure {
                    self.working_state
                        .record_failure(failure.kind, &failure.summary);
                    if report.failures.len() < TURN_FAILURES_CAP {
                        report.failures.push(failure);
                    }
                }
                let error = output.is_error.then(|| {
                    output
                        .content
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or("")
                });
                sink.on_tool_result(&preview, output.is_error, error);
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
                if call.name == "complete_task" && !output.is_error {
                    if let Some(claim) = &self.completion_claim {
                        report.completed = true;
                        report.completion = Some(claim.clone());
                        report.self_authored_checks = self.self_authored_checks.len() as u32;
                    }
                }
            }
            if cancelled_mid_tools {
                report.cancelled = true;
                return Ok(report);
            }
            if self.config.verifier && self.verdict.is_some() {
                report.verdict = self.verdict.clone();
                return Ok(report);
            }
            if report.completed {
                // Every tool result of the batch is pushed; the transcript
                // stays valid for the next attempt or a resume.
                return Ok(report);
            }
        }
    }

    /// `complete_task`: the first call answers with the task statement to
    /// review; the second records the claim. Session state only, never gated.
    fn complete_task(&mut self, arguments: &str) -> crate::tools::ToolOutput {
        if !self.config.completion_gate {
            return crate::tools::ToolOutput::error(
                "complete_task is disabled in this session; finish with a summary".to_string(),
            );
        }
        let claim: CompletionClaim = match serde_json::from_str(arguments) {
            Ok(claim) => claim,
            Err(error) => {
                return crate::tools::ToolOutput::error(format!(
                    "invalid complete_task arguments: {error}"
                ))
            }
        };
        if !self.completion_reviewed {
            self.completion_reviewed = true;
            let mut statement = self.task_statement.clone().unwrap_or_default();
            if statement.len() > REVIEW_STATEMENT_CAP {
                let mut end = REVIEW_STATEMENT_CAP;
                while !statement.is_char_boundary(end) {
                    end -= 1;
                }
                statement.truncate(end);
                statement.push_str("\n[statement truncated]");
            }
            return crate::tools::ToolOutput::text(format!(
                "REVIEW BEFORE COMPLETION — not finished yet.\nTask statement:\n{statement}\n\n\
                 You listed {} deliverable(s). For each requirement in the statement above, name \
                 the tool output that proves it. Anything unproven, untested, or not yet done: \
                 continue working now. Acceptance checks must exercise behaviour — run the program \
                 or its tests on real input and compare the output to the requirement. Checks that \
                 only compile, lint, or confirm a file exists are not accepted. When every \
                 requirement is proven, call complete_task again with the final deliverables and \
                 behavioural checks to finish.",
                claim.deliverables.len()
            ));
        }
        let checks: Vec<Vec<String>> = claim
            .checks
            .iter()
            .filter(|argv| !argv.is_empty())
            .cloned()
            .collect();
        let strong = checks.iter().filter(|argv| !is_weak_check(argv)).count();
        if strong == 0 && self.weak_check_rejections < WEAK_CHECK_REJECTIONS {
            self.weak_check_rejections += 1;
            self.working_state.record_failure(
                FailureKind::UncertainResult,
                "completion claimed without a behavioural acceptance check",
            );
            let listed = if checks.is_empty() {
                "no checks".to_string()
            } else {
                checks
                    .iter()
                    .map(|argv| argv.join(" "))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            return crate::tools::ToolOutput::error(format!(
                "completion not accepted ({}/{WEAK_CHECK_REJECTIONS}): the acceptance checks prove \
                 nothing about behaviour ({listed}). Add at least one check that runs the program \
                 or its tests on real input and fails when the requirement is unmet (a test \
                 runner, a script that asserts on output, a command whose result is compared). \
                 Run it yourself first, then call complete_task again with it in `checks`.",
                self.weak_check_rejections
            ));
        }
        if claim.requirements.is_empty() && self.requirements_rejections < REQUIREMENTS_REJECTIONS {
            self.requirements_rejections += 1;
            return crate::tools::ToolOutput::error(
                "completion not accepted: `requirements` is missing. List every requirement the \
                 task statement makes, in its own terms, each with status met or unmet and the \
                 command output that decided it. Include requirements you have not met. Then call \
                 complete_task again."
                    .to_string(),
            );
        }
        let self_authored: Vec<Vec<String>> = checks
            .iter()
            .filter(|argv| self.is_self_authored(argv))
            .cloned()
            .collect();
        let self_authored_count = self_authored.len();
        self.self_authored_checks = self_authored;
        if !checks.is_empty() {
            self.working_state.verification_commands = checks.clone();
        }
        let mut summary = format!(
            "Task marked complete: {} deliverable(s), {} requirement(s) listed, {} acceptance \
             check(s) queued.",
            claim.deliverables.len(),
            claim.requirements.len(),
            checks.len()
        );
        if self_authored_count > 0 {
            summary.push_str(&format!(
                " {self_authored_count} of {} check(s) run a file written in this session; they \
                 count as your claim, not as independent proof.",
                checks.len()
            ));
        }
        self.completion_claim = Some(claim);
        crate::tools::ToolOutput::text(summary)
    }

    /// A declared check is self-authored when the program it executes — the
    /// binary itself, the script handed to an interpreter, or the test files
    /// handed to a runner — is a file this task created or changed (fs-tool
    /// writes, or the pre-task fingerprint). Reading a changed deliverable
    /// (`grep x out.txt`) is not self-authored. Observed 2026-09-08
    /// (Muse Spark, 3 of 3 failures): `python3 /app/check_acceptance.py`,
    /// `bun scripts/verify-release.ts`, `bun dist/client-entry.js` — all
    /// written by the model, all passing, hidden tests 4/7, 27/36, 27/36.
    fn is_self_authored(&self, argv: &[String]) -> bool {
        let Some(program) = argv.first() else {
            return false;
        };
        let base = program.rsplit('/').next().unwrap_or(program);
        let mut candidates: Vec<&str> = vec![program.as_str()];
        let interpreter = base.starts_with("python")
            || matches!(
                base,
                "bun"
                    | "node"
                    | "deno"
                    | "sh"
                    | "bash"
                    | "zsh"
                    | "dash"
                    | "ruby"
                    | "perl"
                    | "php"
                    | "ts-node"
                    | "tsx"
                    | "uv"
                    | "uvx"
                    | "npx"
            );
        let runner = matches!(base, "pytest" | "py.test")
            || (matches!(base, "bun" | "deno" | "node" | "npx" | "uv")
                && argv.iter().skip(1).take(2).any(|a| a == "test"));
        if interpreter || runner {
            for token in argv.iter().skip(1) {
                if token == "-m" || token == "-c" || token == "-e" {
                    // A module or inline program, not a file on disk.
                    return false;
                }
                if token.starts_with('-') || matches!(token.as_str(), "run" | "test" | "exec") {
                    continue;
                }
                candidates.push(token.as_str());
                if !runner {
                    break;
                }
            }
        }
        candidates
            .into_iter()
            .any(|token| self.is_changed_file(token))
    }

    /// `token` names a regular file under the workspace that this task
    /// created or modified.
    fn is_changed_file(&self, token: &str) -> bool {
        let root = self.executor.root();
        let token = token.split("::").next().unwrap_or(token);
        let token = token.strip_prefix("./").unwrap_or(token);
        if token.is_empty() {
            return false;
        }
        let path = std::path::Path::new(token);
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        if !absolute.is_file() {
            return false;
        }
        let relative = absolute
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| token.to_string());
        self.working_state.files_changed.contains(&relative)
            || self.working_state.files_changed.contains(token)
            || self
                .gate_baseline
                .as_ref()
                .is_some_and(|baseline| baseline.path_changed(&absolute))
    }

    /// `report_verdict` (verifier sessions only): record the verdict; the
    /// turn ends once it exists.
    fn report_verdict(&mut self, arguments: &str) -> crate::tools::ToolOutput {
        if !self.config.verifier {
            return crate::tools::ToolOutput::error(
                "report_verdict is only available to the verifier".to_string(),
            );
        }
        let mut verdict: Verdict = match serde_json::from_str(arguments) {
            Ok(verdict) => verdict,
            Err(error) => {
                return crate::tools::ToolOutput::error(format!(
                    "invalid report_verdict arguments: {error}"
                ))
            }
        };
        if !matches!(
            verdict.verdict.to_ascii_lowercase().as_str(),
            "pass" | "fail"
        ) {
            return crate::tools::ToolOutput::error(
                "verdict must be \"pass\" or \"fail\"".to_string(),
            );
        }
        if verdict.requirements.is_empty() {
            return crate::tools::ToolOutput::error(
                "list every requirement of the task statement with its status and evidence"
                    .to_string(),
            );
        }
        for requirement in &mut verdict.requirements {
            cap_string(&mut requirement.evidence, EVIDENCE_CAP);
            cap_string(&mut requirement.requirement, EVIDENCE_CAP);
        }
        let unmet = verdict.unmet().len();
        self.verdict = Some(verdict);
        crate::tools::ToolOutput::text(format!("Verdict recorded ({unmet} unmet)."))
    }

    /// Apply deterministic policy, consult the gate when required, execute.
    /// Returns None only when the user cancelled mid-execution.
    async fn execute_gated(
        &mut self,
        call: &crate::domain::ToolCall,
        gate: &dyn ApprovalGate,
        cancel: &CancellationToken,
    ) -> Option<crate::tools::ToolOutput> {
        if let Some(reason) = invalid_json_reason(&call.arguments) {
            return Some(crate::tools::ToolOutput::error(format!(
                "invalid arguments: the tool call's arguments were not valid JSON ({reason}). \
                 Call {} again with a well-formed JSON object.",
                call.name
            )));
        }
        // update_plan mutates session state, not the workspace: handled here,
        // never gated (recording a plan is not a side effect on the world).
        if call.name == "update_plan" {
            return Some(match self.working_state.apply_update(&call.arguments) {
                Ok(summary) => crate::tools::ToolOutput::text(summary),
                Err(error) => crate::tools::ToolOutput::error(error),
            });
        }
        if call.name == "complete_task" {
            return Some(self.complete_task(&call.arguments));
        }
        if call.name == "report_verdict" {
            return Some(self.report_verdict(&call.arguments));
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
        if call.name == "shell" && !self.config.sandboxed {
            return Some(crate::tools::ToolOutput::error(
                "shell is disabled outside sandboxed sessions; use run_command with an argv array"
                    .to_string(),
            ));
        }
        // Classify command policy BEFORE the approval gate: forbidden commands
        // never reach a prompt; commands outside the auto-run allowlist force
        // a review even in auto mode. Malformed arguments fail fast here —
        // a human must never be asked to approve a command that can't be shown.
        let mut policy_warning: Option<String> = None;
        if call.name == "run_command" {
            let Some(argv) = parse_argv(&call.arguments) else {
                return Some(crate::tools::ToolOutput::error(
                    "invalid arguments: command must be a JSON array of strings (argv), \
                     e.g. {\"command\": [\"git\", \"status\"]}"
                        .to_string(),
                ));
            };
            match crate::tools::policy::classify_command_with(&argv, &self.config.policy()) {
                crate::tools::policy::CommandClass::Forbidden(reason) => {
                    return Some(crate::tools::ToolOutput::error(reason));
                }
                crate::tools::policy::CommandClass::NeedsApproval(reason) => {
                    policy_warning = Some(reason);
                }
                crate::tools::policy::CommandClass::AutoEligible => {}
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
                        self.executor.policy(),
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

impl Agent {
    /// Facts the most recent provider response carried, when it is a router.
    pub fn last_routing(&self) -> Option<&RoutingFacts> {
        self.last_routing.as_ref()
    }

    /// Routing metadata for a router (contract `nimble/v1`): what the harness
    /// knows about its own state — never the task's content. The stable-prefix
    /// hash covers the system message and tool schemas, the two things every
    /// request of a session shares byte-for-byte.
    fn routing_metadata(&self, phase: &str) -> RoutingMetadata {
        let mut hasher = blake3::Hasher::new();
        let mut stable_bytes: u64 = 0;
        if let Some(ChatMessage::System { content }) = self.messages.first() {
            hasher.update(content.as_bytes());
            stable_bytes += content.len() as u64;
        }
        for tool in &self.tools {
            hasher.update(tool.name.as_bytes());
            hasher.update(tool.description.as_bytes());
            let parameters = serde_json::to_string(&tool.parameters).unwrap_or_default();
            hasher.update(parameters.as_bytes());
            stable_bytes += (tool.name.len() + tool.description.len() + parameters.len()) as u64;
        }
        let stable_tokens = self.context.ledger.tokens_for_bytes(stable_bytes);
        let total_tokens = self.context.ledger.estimated_next_prompt_tokens();
        RoutingMetadata {
            version: NIMBLE_CONTRACT_VERSION.into(),
            session_id: Some(self.session_id.clone()),
            task_type: self.config.task_type.clone(),
            task_complexity: None,
            agent_phase: Some(phase.to_string()),
            attempt: Some(self.attempt.max(1)),
            previous_failure: self
                .working_state
                .last_failure()
                .map(|f| f.kind.as_str().to_string()),
            verification_failed: Some(self.last_verification_failed),
            stable_prefix_hash: Some(format!("blake3:{}", hasher.finalize().to_hex())),
            estimated_stable_tokens: Some(stable_tokens),
            estimated_dynamic_tokens: Some(total_tokens.saturating_sub(stable_tokens)),
            context_version: Some(self.context_version),
        }
    }

    /// Classify a failed tool output and detect repeated failing edits. On the
    /// second identical failing `edit`, the result is upgraded to
    /// `repeated_patch_failure` and corrective guidance is appended so the
    /// model changes approach.
    fn classify_tool_failure(
        &mut self,
        call: &crate::domain::ToolCall,
        output: &mut crate::tools::ToolOutput,
    ) -> Option<Failure> {
        let signature = (call.name == "edit").then(|| edit_signature(&call.arguments));
        if !output.is_error {
            if let Some(key) = signature {
                self.failed_edits.remove(&key);
            }
            return None;
        }
        let mut failure = Failure::from_tool(&call.name, &output.content);
        if let Some(key) = signature {
            let count = self.failed_edits.entry(key).or_insert(0);
            *count += 1;
            if *count >= 2 {
                failure = failure.with_kind(FailureKind::RepeatedPatchFailure);
                output.content.push_str(&format!(
                    "\n[this exact edit has now failed {count} times: {}]",
                    FailureKind::RepeatedPatchFailure.guidance()
                ));
            }
        }
        Some(failure)
    }

    /// Autonomous loop: attempt → verify → classify → gather evidence → retry,
    /// bounded by `options.max_attempts`, the cost cap, and cancellation. The
    /// model never judges completion: with a non-empty verification plan the
    /// task is done only when every check passes. Each retry appends new
    /// evidence as a user message — never the same prompt twice — so the
    /// prefix stays warm.
    pub async fn run_task(
        &mut self,
        input: String,
        options: &TaskOptions,
        gate: &dyn ApprovalGate,
        sink: &mut dyn StreamSink,
        cancel: &CancellationToken,
    ) -> Result<TaskReport, SessionError> {
        let max_attempts = options.max_attempts.max(1);
        let mut task = TaskReport::default();
        let mut next_input = input;
        if options.verification.require_diff
            && verify::git_changed_paths(self.executor.root()).is_none()
        {
            self.diff_baseline = Some(WorkspaceSnapshot::capture(self.executor.root()));
        }
        self.task_statement = Some(next_input.clone());
        if self.config.completion_gate {
            self.gate_baseline = Some(WorkspaceSnapshot::capture(self.executor.root()));
        }
        for attempt in 1..=max_attempts {
            self.attempt = attempt;
            self.completion_reviewed = false;
            self.completion_claim = None;
            self.weak_check_rejections = 0;
            self.requirements_rejections = 0;
            self.self_authored_checks.clear();
            let turn = self.run_turn(next_input, gate, sink, cancel).await?;
            task.absorb(&turn);
            let stop = turn.cancelled || turn.budget_exhausted;
            // Model-declared checks (update_plan.verification or
            // complete_task.checks) run even without an operator plan: a
            // failing one is real evidence for the next attempt.
            let declared = self.working_state.verification_commands.clone();
            let verification = if options.verification.is_empty() && declared.is_empty() {
                None
            } else {
                let outcome = verify::run_verification(
                    self.executor.root(),
                    &options.verification,
                    &self.working_state.files_changed,
                    &declared,
                    self.executor.policy(),
                    self.diff_baseline.as_ref(),
                    cancel,
                )
                .await;
                let failure = outcome.failure();
                self.working_state.verification = Some(outcome.summary_line());
                if let Some(failure) = &failure {
                    self.working_state
                        .record_failure(failure.kind, &failure.summary);
                }
                self.telemetry.record(TelemetryEvent::Verification {
                    at_epoch_ms: telemetry::epoch_ms_now(),
                    attempt,
                    passed: outcome.passed(),
                    checks: outcome.checks.len(),
                    failure: failure.map(|f| f.kind),
                    summary: outcome.summary_line(),
                });
                Some(outcome)
            };
            let mut passed = verification.as_ref().map(VerificationOutcome::passed);
            let mut evidence = verification
                .as_ref()
                .filter(|outcome| !outcome.passed())
                .map(|outcome| outcome.render_evidence(attempt, max_attempts));
            // A turn that ended because the output budget ran out before any
            // visible answer is not a completion, whatever the verification
            // plan says: retry with that fact as evidence (observed on
            // Terminal-Bench 2026-09-04: 66k reasoning tokens, no edits).
            let starved = turn.hit_output_limit && turn.assistant_text.trim().is_empty();
            // Likewise a turn cut off by the provider-call budget: the model did
            // not declare completion, so only a real passing check (not a bare
            // "the workspace changed") may end the task. The next attempt
            // continues the same session (observed on Terminal-Bench
            // 2026-09-04: 50 calls of exploration, deliverables never written).
            let real_checks_passed = verification
                .as_ref()
                .is_some_and(|o| !o.checks.is_empty() && o.checks.iter().all(|c| c.ok));
            let oracle_passed = verification.as_ref().is_some_and(|o| {
                o.passed()
                    && o.checks.iter().any(|c| {
                        matches!(
                            c.check.source,
                            verify::CheckSource::Explicit | verify::CheckSource::Detected
                        )
                    })
            });
            // Independent verification: a claim that no oracle backs is
            // reviewed by a fresh session before the loop accepts it. Unmet
            // requirements are evidence for the next attempt; a verdict of
            // pass leaves the claim a claim (verified stays None).
            let mut verifier_report = None;
            if self.config.independent_verifier
                && self.config.completion_gate
                && turn.completed
                && !stop
                && !oracle_passed
                && passed != Some(false)
            {
                let claim = turn.completion.clone().unwrap_or_else(|| CompletionClaim {
                    summary: String::new(),
                    deliverables: Vec::new(),
                    checks: Vec::new(),
                    requirements: Vec::new(),
                });
                let outcome = self
                    .verify_claim(&claim, attempt, max_attempts, gate, sink, cancel)
                    .await;
                task.provider_calls += outcome.provider_calls;
                task.tool_calls += outcome.tool_calls;
                task.actual_microusd += outcome.actual_microusd;
                self.telemetry.record(TelemetryEvent::Verification {
                    at_epoch_ms: telemetry::epoch_ms_now(),
                    attempt,
                    passed: outcome.verdict.as_ref().is_some_and(Verdict::passed),
                    checks: outcome.verdict.as_ref().map_or(0, |v| v.requirements.len()),
                    failure: outcome
                        .verdict
                        .as_ref()
                        .filter(|v| !v.passed())
                        .map(|_| FailureKind::IncorrectAssumption),
                    summary: match &outcome.verdict {
                        Some(v) => format!(
                            "independent verifier: {} ({} unmet of {})",
                            if v.passed() { "pass" } else { "fail" },
                            v.unmet().len(),
                            v.requirements.len()
                        ),
                        None => format!(
                            "independent verifier: no verdict ({})",
                            outcome.error.as_deref().unwrap_or("gave up")
                        ),
                    },
                });
                if let Some(verdict) = outcome.verdict.as_ref().filter(|v| !v.passed()) {
                    passed = Some(false);
                    self.working_state.record_failure(
                        FailureKind::IncorrectAssumption,
                        "independent verifier found unmet requirements",
                    );
                    let extra = evidence
                        .take()
                        .map(|e| format!("\n\n{e}"))
                        .unwrap_or_default();
                    evidence = Some(format!(
                        "{}{extra}",
                        render_verdict_evidence(verdict, attempt, max_attempts)
                    ));
                }
                verifier_report = Some(outcome);
            }
            // The gate gave up on an attempt that never claimed completion:
            // retry with that fact, unless an operator check already passed.
            if self.config.completion_gate
                && !turn.completed
                && !turn.turn_limit_reached
                && !stop
                && !real_checks_passed
                && attempt < max_attempts
            {
                passed = Some(false);
                self.working_state.record_failure(
                    FailureKind::UncertainResult,
                    "attempt ended without a completion claim",
                );
                let extra = evidence
                    .take()
                    .map(|e| format!("\n\n{e}"))
                    .unwrap_or_default();
                evidence = Some(format!(
                    "COMPLETION GATE (attempt {attempt} of {max_attempts}): the previous attempt \
                     ended without calling complete_task, so the task is not finished. Continue \
                     from the current state: produce the remaining deliverables, verify them, and \
                     call complete_task with the evidence.{extra}"
                ));
            }
            if turn.turn_limit_reached && !real_checks_passed && attempt < max_attempts {
                passed = Some(false);
                self.working_state.record_failure(
                    FailureKind::UncertainResult,
                    "attempt used its provider-call budget before finishing",
                );
                let extra = evidence
                    .take()
                    .map(|e| format!("\n\n{e}"))
                    .unwrap_or_default();
                evidence = Some(format!(
                    "CALL BUDGET (attempt {attempt} of {max_attempts}): the previous attempt used its \
                     provider-call budget before finishing. Continue from the current state — do not \
                     re-explore; produce the remaining deliverables now and stop when they exist.{extra}"
                ));
            }
            if starved && passed != Some(true) {
                passed = Some(false);
                self.working_state.record_failure(
                    FailureKind::UncertainResult,
                    "response truncated by the output limit with no visible answer",
                );
                let extra = evidence.map(|e| format!("\n\n{e}")).unwrap_or_default();
                evidence = Some(format!(
                    "OUTPUT LIMIT (attempt {attempt} of {max_attempts}): your previous response was cut \
                     off by the output-token limit before it produced a visible answer or a tool call. \
                     Continue from the current state: act with tools now, keep reasoning brief, and \
                     finish with a short summary of what changed.{extra}"
                ));
            }
            self.last_verification_failed = passed == Some(false);
            task.attempts.push(AttemptReport {
                attempt,
                turn,
                verification,
                verifier: verifier_report,
            });
            match passed {
                None => {
                    task.success = None;
                    break;
                }
                Some(true) => {
                    task.success = Some(true);
                    break;
                }
                Some(false) => {
                    task.success = Some(false);
                    if stop || attempt == max_attempts {
                        break;
                    }
                    let mut input = evidence.expect("failed verification renders evidence");
                    input.push_str(retry_strategy(attempt + 1));
                    next_input = input;
                }
            }
        }
        // Truthful verdict. Success needs an oracle the model did not write;
        // failure needs only one failing check (a diff-only or model-declared
        // pass is a claim, reported as such and never fed to a router as
        // success — observed 2026-09-04: 23 of 24 "verified" Terminal-Bench
        // trials had zero checks and failed the real verifier).
        let last = task.attempts.last().and_then(|a| a.verification.as_ref());
        let oracle_checks = |o: &VerificationOutcome| {
            o.checks
                .iter()
                .filter(|c| {
                    matches!(
                        c.check.source,
                        verify::CheckSource::Explicit | verify::CheckSource::Detected
                    )
                })
                .count()
        };
        task.verification_source = match last {
            Some(o) if oracle_checks(o) > 0 => {
                if o.checks
                    .iter()
                    .any(|c| c.check.source == verify::CheckSource::Explicit)
                {
                    "explicit"
                } else {
                    "detected"
                }
            }
            Some(o) if !o.checks.is_empty() => {
                let last_attempt = task.attempts.last();
                let verifier_passed = last_attempt
                    .and_then(|a| a.verifier.as_ref())
                    .and_then(|v| v.verdict.as_ref())
                    .is_some_and(Verdict::passed);
                let all_self_authored = last_attempt.is_some_and(|a| {
                    a.turn.self_authored_checks > 0
                        && a.turn.self_authored_checks as usize == o.checks.len()
                });
                if verifier_passed {
                    "verifier"
                } else if all_self_authored {
                    "self_authored"
                } else {
                    "model_declared"
                }
            }
            Some(o) if o.require_diff => "diff",
            _ if task.completed => {
                let verifier_passed = task
                    .attempts
                    .last()
                    .and_then(|a| a.verifier.as_ref())
                    .and_then(|v| v.verdict.as_ref())
                    .is_some_and(Verdict::passed);
                if verifier_passed {
                    "verifier"
                } else {
                    "claim"
                }
            }
            _ => "none",
        };
        task.verified = match (task.success, last) {
            (Some(false), _) => Some(false),
            (_, Some(o)) if oracle_checks(o) > 0 && o.passed() => Some(true),
            _ => None,
        };
        // Routers learn from outcomes; everyone else ignores this. Never fatal.
        let feedback = TaskFeedback {
            version: NIMBLE_CONTRACT_VERSION.into(),
            session_id: self.session_id.clone(),
            request_id: self
                .last_routing
                .as_ref()
                .map(|f| f.request_id.clone())
                .filter(|id| !id.is_empty()),
            task_type: self.config.task_type.clone(),
            task_success: task.verified,
            verification_result: Some(
                match task.verified {
                    Some(true) => "passed",
                    Some(false) => "failed",
                    None => "not_run",
                }
                .into(),
            ),
            failure_type: if task.success == Some(false) {
                self.working_state
                    .last_failure()
                    .map(|f| f.kind.as_str().to_string())
            } else {
                None
            },
            attempt_count: Some(task.attempts.len() as u32),
            // Behind a router the local card is a placeholder; let the router
            // use its own per-request pricing instead of a misleading zero.
            cost_microusd: self
                .config
                .model
                .pricing
                .verified
                .then_some(task.actual_microusd),
        };
        let _ = self.provider.report_feedback(&feedback).await;
        self.attempt = 1;
        self.telemetry.record(TelemetryEvent::TaskFinished {
            at_epoch_ms: telemetry::epoch_ms_now(),
            attempts: task.attempts.len() as u32,
            success: task.verified,
        });
        self.task_statement = None;
        Ok(task)
    }
}

impl Agent {
    /// Review a completion claim in a fresh session: same provider, same
    /// workspace, read-only tools, no memory of the work. Never fatal: any
    /// failure to reach a verdict leaves the claim standing as it would
    /// without a verifier.
    async fn verify_claim(
        &mut self,
        claim: &CompletionClaim,
        attempt: u32,
        max_attempts: u32,
        gate: &dyn ApprovalGate,
        sink: &mut dyn StreamSink,
        cancel: &CancellationToken,
    ) -> VerifierReport {
        let mut config = self.config.clone();
        config.completion_gate = false;
        config.independent_verifier = false;
        config.verifier = true;
        config.max_provider_calls_per_input = VERIFIER_CALL_CAP;
        let provider = match &self.verifier_backend {
            Some((provider, model)) => {
                config.model = model.clone();
                config.effort = model.resolve_effort(None).unwrap_or(None);
                config.context.window_tokens = model.limits.context_tokens;
                provider.clone()
            }
            None => self.provider.clone(),
        };
        config.max_session_cost_microusd = self
            .config
            .max_session_cost_microusd
            .map(|cap| cap.saturating_sub(self.total_actual_microusd));
        let mut verifier =
            match Agent::new(provider, self.executor.root().to_path_buf(), config, None) {
                Ok(agent) => agent,
                Err(error) => {
                    return VerifierReport {
                        error: Some(error.to_string()),
                        ..VerifierReport::default()
                    }
                }
            };
        if let Some(path) = self.telemetry.path() {
            verifier.set_telemetry_path(path.to_path_buf());
        }
        let input = render_verifier_brief(
            self.task_statement.as_deref().unwrap_or(""),
            claim,
            &self.working_state.files_changed,
            &self.self_authored_checks,
            attempt,
            max_attempts,
        );
        sink.on_notice(&format!(
            "independent verifier reviewing the claim (attempt {attempt} of {max_attempts})"
        ));
        let turn = verifier.run_turn(input, gate, sink, cancel).await;
        self.total_actual_microusd += verifier.total_actual_microusd;
        self.total_usage.add(verifier.total_usage);
        self.telemetry.summary.absorb(&verifier.telemetry.summary);
        match turn {
            Ok(turn) => VerifierReport {
                model: verifier.config.model.id.clone(),
                verdict: turn.verdict.clone(),
                provider_calls: turn.provider_calls,
                tool_calls: turn.tool_calls,
                actual_microusd: turn.actual_microusd,
                error: turn
                    .verdict
                    .is_none()
                    .then(|| "verifier ended without a verdict".to_string()),
            },
            Err(error) => VerifierReport {
                error: Some(error.to_string()),
                ..VerifierReport::default()
            },
        }
    }
}

/// Marker key under which a malformed tool-call argument string is kept.
const INVALID_JSON_KEY: &str = "__invalid_json__";

/// Tool-call arguments that fail to parse are wrapped in a valid JSON object
/// before the message is recorded or replayed: strict providers reject a
/// transcript containing them outright (observed 2026-09-09, Meta: HTTP 400
/// "`arguments` must be valid JSON" after 87 good calls, ending the task).
/// The tool then answers with a format error the model can act on.
fn sanitize_tool_arguments(message: &mut crate::domain::AssistantMessage) {
    for call in &mut message.tool_calls {
        if let Err(error) = serde_json::from_str::<serde_json::Value>(&call.arguments) {
            let mut raw = call.arguments.clone();
            cap_string(&mut raw, 4_000);
            call.arguments = serde_json::json!({
                INVALID_JSON_KEY: raw,
                "error": error.to_string(),
            })
            .to_string();
        }
    }
}

fn invalid_json_reason(arguments: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let object = value.as_object()?;
    object.contains_key(INVALID_JSON_KEY).then(|| {
        object
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("parse error")
            .to_string()
    })
}

fn cap_string(value: &mut String, cap: usize) {
    if value.len() > cap {
        let mut end = cap;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        value.push('…');
    }
}

/// The verifier's brief: the statement, what the worker claims, what it
/// changed, and which of its checks it wrote itself. Deterministic layout.
fn render_verifier_brief(
    statement: &str,
    claim: &CompletionClaim,
    files_changed: &std::collections::BTreeSet<String>,
    self_authored: &[Vec<String>],
    attempt: u32,
    max_attempts: u32,
) -> String {
    let mut statement = statement.to_string();
    cap_string(&mut statement, REVIEW_STATEMENT_CAP);
    let mut out = format!(
        "INDEPENDENT VERIFICATION of a completion claim (attempt {attempt} of {max_attempts}).\n\
         Another session did the work below and says it is finished. Decide whether every \
         requirement of the task statement is met in the workspace as it is now. Do not modify \
         anything.\n\nTASK STATEMENT:\n{statement}\n\nWORKER'S CLAIM:\n{}\n",
        claim.summary.trim()
    );
    if !claim.requirements.is_empty() {
        out.push_str("\nWorker's requirement checklist (grade it; add anything it missed):\n");
        for (i, r) in claim.requirements.iter().enumerate() {
            let mut evidence = r.evidence.clone();
            cap_string(&mut evidence, EVIDENCE_CAP);
            out.push_str(&format!(
                "{}. [{}] {} — {}\n",
                i + 1,
                if r.status.is_empty() { "?" } else { &r.status },
                r.requirement.trim(),
                evidence.trim()
            ));
        }
    }
    if !claim.deliverables.is_empty() {
        out.push_str("\nDeliverables claimed:\n");
        for d in &claim.deliverables {
            let mut evidence = d.evidence.clone();
            cap_string(&mut evidence, EVIDENCE_CAP);
            out.push_str(&format!("- {} — {}\n", d.item.trim(), evidence.trim()));
        }
    }
    if !claim.checks.is_empty() {
        out.push_str("\nChecks the worker declared:\n");
        for argv in &claim.checks {
            let authored = self_authored.contains(argv);
            out.push_str(&format!(
                "- {}{}\n",
                argv.join(" "),
                if authored {
                    "  (written by the worker this session: not independent evidence)"
                } else {
                    ""
                }
            ));
        }
    }
    if !files_changed.is_empty() {
        out.push_str("\nFiles the worker changed:\n");
        for path in files_changed.iter().take(VERIFIER_FILES_CAP) {
            out.push_str(&format!("- {path}\n"));
        }
        if files_changed.len() > VERIFIER_FILES_CAP {
            out.push_str(&format!(
                "- … {} more\n",
                files_changed.len() - VERIFIER_FILES_CAP
            ));
        }
    }
    out.push_str(
        "\nPROCEDURE: enumerate every requirement in the statement, including negative ones \
         (what must not appear or happen) and every edge case it names. For each, run a command \
         that would fail if the requirement were unmet — the program on real input, existing \
         tests, direct inspection of outputs — and judge from the output you saw, not from the \
         worker's description. Then call report_verdict with every requirement, its status, and \
         the evidence.",
    );
    out
}

/// Deterministic evidence for the next attempt: what an independent reader
/// of the statement found unmet, and what it confirmed.
fn render_verdict_evidence(verdict: &Verdict, attempt: u32, max_attempts: u32) -> String {
    let mut out = format!(
        "INDEPENDENT VERIFICATION (attempt {attempt} of {max_attempts}): a fresh reviewer re-read \
         the task statement and tested your claim against the workspace. It is not finished.\n\
         Unmet requirements:\n"
    );
    for (i, r) in verdict.unmet().iter().enumerate() {
        out.push_str(&format!(
            "{}. {} — {}\n",
            i + 1,
            r.requirement.trim(),
            r.evidence.trim()
        ));
    }
    let met: Vec<&Requirement> = verdict.requirements.iter().filter(|r| r.is_met()).collect();
    if !met.is_empty() {
        out.push_str("Confirmed met:\n");
        for r in met {
            out.push_str(&format!("- {}\n", r.requirement.trim()));
        }
    }
    out.push_str(
        "Fix each unmet requirement: write a check that fails now for it, change the code until \
         the check passes, keep the confirmed requirements intact, then call complete_task again \
         with the full requirement list.",
    );
    out
}

/// Transport failures and upstream 5xx are transient by nature; 4xx are not.
fn is_transient(error: &ProviderError) -> bool {
    match error {
        ProviderError::Transport(_) => true,
        ProviderError::Status { status, .. } => *status >= 500,
        _ => false,
    }
}

/// A different plan for each retry: repeating the previous attempt's approach
/// with the same failure evidence is the retry that never converges (observed
/// 2026-09-04: attempts 2 and 3 re-ran attempt 1's steps on 5 of 8 failures).
/// Deterministic text, never the model's own words.
fn retry_strategy(attempt: u32) -> &'static str {
    match attempt {
        2 => {
            "\n\nSTRATEGY FOR THIS ATTEMPT: re-read the task statement and list every \
             requirement as a checklist. For each unmet requirement write a check that fails \
             now (a test, or a script asserting on real output), then change the code until it \
             passes. Do not repeat the previous attempt's steps."
        }
        _ => {
            "\n\nSTRATEGY FOR THIS ATTEMPT: work from the failing evidence only. Make the \
             smallest change that addresses each failure, verify each requirement individually \
             with a behavioural check, and inspect edge cases the statement names (empty input, \
             ordering, late or duplicate events, exact output format) before completing."
        }
    }
}

/// Stable hash of an edit's target and anchor text.
fn edit_signature(arguments: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let args: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    args.get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .hash(&mut hasher);
    args.get("old_string")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .hash(&mut hasher);
    hasher.finish()
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
/// arguments are malformed (the caller then fails the call fast).
fn parse_argv(arguments: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    crate::tools::extract_argv(value.get("command")?)
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
            replay_state: None,
        }
    }

    fn final_text(text: &str) -> AssistantMessage {
        AssistantMessage {
            content: Some(text.into()),
            reasoning_content: None,
            tool_calls: vec![],
            replay_state: None,
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
                replay_state: None,
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
        let replay = crate::model::ReasoningReplay::Required;
        let first: Vec<String> =
            crate::provider::openai_compat::wire_messages(&requests[0], replay)
                .iter()
                .map(|value| serde_json::to_string(value).unwrap())
                .collect();
        let second: Vec<String> =
            crate::provider::openai_compat::wire_messages(&requests[1], replay)
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
    async fn malformed_command_arguments_fail_fast_without_prompting() {
        struct PanicGate;
        #[async_trait::async_trait]
        impl ApprovalGate for PanicGate {
            async fn review(&self, request: &ApprovalRequest) -> Decision {
                panic!("gate consulted for a malformed call: {}", request.preview);
            }
        }
        let dir = workspace();
        // A bare shell string is not argv: the call must error back to the
        // model with corrective guidance, never reach a blank approval prompt.
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "run_command", r#"{"command":"cat hello.txt"}"#),
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
                "read it".into(),
                &PanicGate,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.assistant_text, "adjusted");
        assert!(agent.messages().iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("array of strings")
        )));
    }

    #[tokio::test]
    async fn double_encoded_argv_executes_like_the_schema_shape() {
        struct PanicGate;
        #[async_trait::async_trait]
        impl ApprovalGate for PanicGate {
            async fn review(&self, request: &ApprovalRequest) -> Decision {
                panic!("allowlisted command prompted: {}", request.preview);
            }
        }
        let dir = workspace();
        // Models sometimes emit argv as a JSON-encoded string (dogfood
        // 2026-09-03); it must classify, scope, and run as the real argv —
        // in auto mode an allowlisted double-encoded call never prompts.
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call(
                "c1",
                "run_command",
                r#"{"command":"[\"cat\", \"hello.txt\"]"}"#,
            ),
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
        let report = agent
            .run_turn(
                "read it".into(),
                &PanicGate,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.assistant_text, "done");
        assert!(agent.messages().iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("hello world")
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

    #[tokio::test]
    async fn run_task_retries_with_verification_evidence_until_checks_pass() {
        let dir = workspace();
        // Attempt 1: the model claims completion without doing anything.
        // Attempt 2: it creates the file the acceptance check needs.
        let provider = Arc::new(ScriptedProvider::new(vec![
            final_text("done, I think"),
            assistant_with_call(
                "c1",
                "write_file",
                r#"{"path":"made.txt","content":"ok\n"}"#,
            ),
            final_text("fixed"),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let options = TaskOptions {
            max_attempts: 3,
            verification: VerificationPlan {
                explicit: vec![vec!["cat".into(), "made.txt".into()]],
                detect: false,
                require_diff: false,
                check_timeout_seconds: 10,
            },
        };
        let report = agent
            .run_task(
                "make the file".into(),
                &options,
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.success, Some(true));
        assert_eq!(report.attempts.len(), 2);
        assert!(!report.attempts[0].verification.as_ref().unwrap().passed());
        assert!(report.attempts[1].verification.as_ref().unwrap().passed());
        assert_eq!(report.assistant_text, "fixed");
        assert_eq!(report.provider_calls, 3);
        // The retry was driven by evidence, not a repeated prompt: the second
        // attempt's first request carries the verification failure.
        let requests = provider.recorded_requests();
        assert!(requests[1].iter().any(|m| matches!(
            m,
            ChatMessage::User { content }
                if content.starts_with("VERIFICATION FAILED (attempt 1 of 3)")
                    && content.contains("$ cat made.txt")
        )));
        assert!(agent
            .working_state()
            .verification
            .as_deref()
            .unwrap()
            .contains("verified"));
        // The last recorded failure is attempt 1's verification (the file the
        // check needed did not exist yet).
        assert_eq!(
            agent.working_state().last_failure().unwrap().kind,
            FailureKind::MissingContext
        );
        let summary = agent.telemetry_summary();
        assert_eq!(summary.attempts, 2);
        assert_eq!(summary.task_success, Some(true));
        assert_eq!(summary.verifications, 2);
        assert_eq!(summary.provider_calls, 3);
    }

    #[tokio::test]
    async fn run_task_without_a_plan_is_a_single_unverified_turn() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![final_text("done")]));
        let mut agent = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig::default(),
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "hi".into(),
                &TaskOptions {
                    max_attempts: 5,
                    ..TaskOptions::default()
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.success, None);
        assert_eq!(report.attempts.len(), 1);
        assert!(report.attempts[0].verification.is_none());
    }

    #[tokio::test]
    async fn repeated_failing_edits_are_reclassified_with_guidance() {
        let dir = workspace();
        let bad_edit = r#"{"path":"hello.txt","old_string":"absent text","new_string":"x"}"#;
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "edit", bad_edit),
            assistant_with_call("c2", "edit", bad_edit),
            final_text("giving up"),
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
        let report = agent
            .run_turn(
                "edit it".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.failures.len(), 2);
        assert_eq!(report.failures[0].kind, FailureKind::IncorrectAssumption);
        assert_eq!(report.failures[1].kind, FailureKind::RepeatedPatchFailure);
        let tool_results: Vec<&str> = agent
            .messages()
            .iter()
            .filter_map(|m| match m {
                ChatMessage::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert!(!tool_results[0].contains("failed 2 times"));
        assert!(
            tool_results[1].contains("failed 2 times"),
            "{}",
            tool_results[1]
        );
        assert_eq!(
            agent.telemetry_summary().failures_by_kind["repeated_patch_failure"],
            1
        );
    }

    #[tokio::test]
    async fn shell_tool_exists_only_in_sandboxed_sessions() {
        let dir = workspace();
        // Default session: no shell schema, and a stray call is refused.
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "shell", r#"{"command":"echo hi"}"#),
            final_text("ok"),
        ]));
        let mut plain = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        assert!(plain.tools.iter().all(|t| t.name != "shell"));
        plain
            .run_turn(
                "go".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(plain.messages().iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("disabled outside sandboxed")
        )));

        // Sandboxed session: shell is the LAST schema (prefix of the default
        // set unchanged), pipelines run, absolute paths resolve.
        let absolute = dir.path().join("hello.txt").display().to_string();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "shell", r#"{"command":"echo hi | tr a-z A-Z"}"#),
            assistant_with_call("c2", "read_file", &format!(r#"{{"path":"{absolute}"}}"#)),
            final_text("ok"),
        ]));
        let mut sandboxed = Agent::new(
            provider,
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                sandboxed: true,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(sandboxed.tools.last().unwrap().name, "shell");
        assert_eq!(sandboxed.tools.len(), plain.tools.len() + 1);
        for (a, b) in plain.tools.iter().zip(sandboxed.tools.iter()) {
            assert_eq!(a.name, b.name);
        }
        struct PanicGate;
        #[async_trait::async_trait]
        impl ApprovalGate for PanicGate {
            async fn review(&self, request: &ApprovalRequest) -> Decision {
                panic!("sandboxed sessions never prompt: {}", request.preview);
            }
        }
        sandboxed
            .run_turn(
                "go".into(),
                &PanicGate,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let tool_results: Vec<&str> = sandboxed
            .messages()
            .iter()
            .filter_map(|m| match m {
                ChatMessage::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert!(tool_results[0].contains("HI"), "{}", tool_results[0]);
        assert!(
            tool_results[1].contains("hello world"),
            "{}",
            tool_results[1]
        );
        assert!(
            sandboxed.messages()[0].approximate_bytes() > plain.messages()[0].approximate_bytes()
        );
    }

    #[tokio::test]
    async fn telemetry_lands_next_to_the_session_log() {
        let dir = workspace();
        let state = tempfile::tempdir().unwrap();
        let store = SessionStore::create_under(state.path(), "t1").unwrap();
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
        let path = state.path().join("t1/telemetry.jsonl");
        let text = std::fs::read_to_string(&path).unwrap();
        let events: Vec<&str> = text.lines().collect();
        assert_eq!(events.len(), 3, "{text}");
        assert!(events[0].contains("\"event\":\"provider_call\""));
        assert!(events[1].contains("\"event\":\"tool_call\""));
        assert!(events[1].contains("\"name\":\"read_file\""));
        assert!(
            !text.contains("hello world"),
            "telemetry must not carry file contents"
        );
        assert_eq!(agent.telemetry_summary().tool_calls, 1);
    }

    #[tokio::test]
    async fn routing_metadata_and_facts_flow_through_the_loop() {
        let dir = workspace();
        let facts = RoutingFacts {
            version: "nimble/v1".into(),
            selected_model: "deepseek-v4-flash".into(),
            provider: "deepseek".into(),
            model_changed: true,
            cache_affinity: 0.5,
            estimated_cached_tokens: 10,
            context_limit: 200_000,
            routing_reason: "escalated".into(),
            decision_id: Some("d1".into()),
            request_id: "r1".into(),
        };
        let provider = Arc::new(
            ScriptedProvider::new(vec![
                assistant_with_call("c1", "read_file", r#"{"path":"hello.txt"}"#),
                final_text("done"),
            ])
            .with_routing_facts(facts),
        );
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                task_type: Some("debugging".into()),
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "go".into(),
                &TaskOptions {
                    max_attempts: 1,
                    verification: VerificationPlan {
                        explicit: vec![vec!["cat".into(), "hello.txt".into()]],
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.success, Some(true));

        // Metadata went out on every request and describes harness state only.
        let routing = provider.recorded_routing();
        let first = routing[0].as_ref().expect("metadata on the first request");
        assert_eq!(first.version, "nimble/v1");
        assert_eq!(first.session_id.as_deref(), Some(agent.session_id.as_str()));
        assert_eq!(first.agent_phase.as_deref(), Some("implementation"));
        assert_eq!(first.attempt, Some(1));
        assert_eq!(first.task_type.as_deref(), Some("debugging"));
        assert_eq!(first.context_version, Some(0));
        assert_eq!(first.verification_failed, Some(false));
        assert!(first
            .stable_prefix_hash
            .as_deref()
            .unwrap()
            .starts_with("blake3:"));
        assert!(first.estimated_stable_tokens.unwrap() > 0);
        // Same prefix → same hash across the session.
        assert_eq!(
            routing[1].as_ref().unwrap().stable_prefix_hash,
            first.stable_prefix_hash
        );

        // A model change surfaced as an appended note at the next boundary, and
        // the router's context limit resized the window.
        let requests = provider.recorded_requests();
        assert!(requests[1].iter().any(|m| matches!(
            m,
            ChatMessage::User { content } if content.starts_with("[routing note")
        )));
        assert_eq!(agent.last_routing().unwrap().request_id, "r1");

        // Task outcome was reported once, joined to the last routed request.
        let feedback = provider.recorded_feedback();
        assert_eq!(feedback.len(), 1);
        assert_eq!(feedback[0].task_success, Some(true));
        assert_eq!(feedback[0].request_id.as_deref(), Some("r1"));
        assert_eq!(feedback[0].attempt_count, Some(1));
        assert_eq!(feedback[0].verification_result.as_deref(), Some("passed"));
        assert_eq!(feedback[0].failure_type, None);
    }

    #[tokio::test]
    async fn output_starved_turns_retry_with_evidence_even_without_a_plan() {
        let dir = workspace();
        // Attempt 1 ends at the output cap with no visible answer; attempt 2
        // answers. No verification plan at all.
        let provider = Arc::new(
            ScriptedProvider::new(vec![
                AssistantMessage {
                    content: None,
                    reasoning_content: Some("thinking…".into()),
                    tool_calls: vec![],
                    replay_state: None,
                },
                final_text("done properly"),
            ])
            .with_finish(crate::provider::FinishReason::Length),
        );
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig::default(),
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "do it".into(),
                &TaskOptions {
                    max_attempts: 3,
                    ..TaskOptions::default()
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.attempts.len(), 2);
        // Attempt 2 also carries finish=Length (scripted), but it produced text.
        assert_eq!(report.assistant_text, "done properly");
        let requests = provider.recorded_requests();
        // The empty (unreplayable) assistant message never entered history.
        assert!(!requests[1].iter().any(|m| matches!(
            m,
            ChatMessage::Assistant(a) if a.content.is_none() && a.tool_calls.is_empty()
        )));
        assert!(requests[1].iter().any(|m| matches!(
            m,
            ChatMessage::User { content } if content.starts_with("OUTPUT LIMIT (attempt 1 of 3)")
        )));
        assert_eq!(
            agent.working_state().last_failure().unwrap().kind,
            FailureKind::UncertainResult
        );
    }

    #[tokio::test]
    async fn require_diff_uses_a_snapshot_outside_git() {
        let dir = workspace(); // not a git repository
        let provider = Arc::new(ScriptedProvider::new(vec![
            final_text("nothing to do"),
            assistant_with_call("c1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            final_text("wrote it"),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "produce out.txt".into(),
                &TaskOptions {
                    max_attempts: 2,
                    verification: VerificationPlan {
                        require_diff: true,
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.attempts.len(), 2);
        assert_eq!(
            report.attempts[0]
                .verification
                .as_ref()
                .unwrap()
                .changed_paths,
            Some(0)
        );
        assert_eq!(
            report.attempts[1]
                .verification
                .as_ref()
                .unwrap()
                .changed_paths,
            Some(1)
        );
        assert_eq!(report.success, Some(true));
    }

    #[tokio::test]
    async fn call_budget_cut_turns_continue_on_the_next_attempt() {
        let dir = workspace();
        // Attempt 1 burns its 2-call budget reading; attempt 2 finishes.
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "read_file", r#"{"path":"hello.txt"}"#),
            assistant_with_call("c2", "read_file", r#"{"path":"hello.txt"}"#),
            assistant_with_call("c3", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            final_text("done"),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                max_provider_calls_per_input: 2,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "make out.txt".into(),
                &TaskOptions {
                    max_attempts: 3,
                    verification: VerificationPlan {
                        require_diff: true,
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.attempts.len(), 2);
        assert!(report.attempts[0].turn.turn_limit_reached);
        assert!(!report.attempts[1].turn.turn_limit_reached);
        assert_eq!(report.success, Some(true));
        assert_eq!(report.assistant_text, "done");
        let requests = provider.recorded_requests();
        assert!(requests[2].iter().any(|m| matches!(
            m,
            ChatMessage::User { content } if content.starts_with("CALL BUDGET (attempt 1 of 3)")
        )));
    }

    #[tokio::test]
    async fn completion_gate_nudges_text_only_replies_and_reviews_the_claim() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            // Intent without action: the classic "Now I'll implement…" ending.
            final_text("Now I'll implement the fix."),
            assistant_with_call("c1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            assistant_with_call(
                "c2",
                "complete_task",
                r#"{"summary":"wrote out.txt","deliverables":[{"item":"out.txt exists","evidence":"write_file ok"}]}"#,
            ),
            // Compile-only proof: rejected once with guidance…
            assistant_with_call(
                "c3",
                "complete_task",
                r#"{"summary":"wrote out.txt","deliverables":[{"item":"out.txt exists","evidence":"write_file ok"}],"checks":[["python3","-m","compileall","-q","."]]}"#,
            ),
            // …then a behavioural check is accepted and run.
            assistant_with_call(
                "c4",
                "complete_task",
                r#"{"summary":"wrote out.txt","deliverables":[{"item":"out.txt exists","evidence":"write_file ok"}],"checks":[["grep","-q","x","out.txt"]],"requirements":[{"requirement":"out.txt contains x","status":"met","evidence":"grep"}]}"#,
            ),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                sandboxed: true,
                completion_gate: true,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "Create out.txt containing x".into(),
                &TaskOptions {
                    max_attempts: 3,
                    verification: VerificationPlan {
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            report.attempts.len(),
            1,
            "one attempt: the gate nudged, never retried"
        );
        let turn = &report.attempts[0].turn;
        assert_eq!(turn.completion_nudges, 1);
        assert!(turn.completed);
        assert!(!turn.gate_unfinished);
        assert!(report.completed);
        assert_eq!(
            report.completion.as_ref().unwrap().checks,
            vec![vec![
                "grep".to_string(),
                "-q".to_string(),
                "x".to_string(),
                "out.txt".to_string()
            ]]
        );
        // The model-declared check ran and passed, so the loop is satisfied…
        let verification = report.attempts[0].verification.as_ref().unwrap();
        assert_eq!(verification.checks.len(), 1);
        assert_eq!(
            verification.checks[0].check.source,
            verify::CheckSource::ModelDeclared
        );
        assert!(verification.checks[0].ok);
        assert_eq!(report.success, Some(true));
        // …but a check the model wrote is not an oracle: no verified verdict.
        assert_eq!(report.verified, None);
        assert_eq!(report.verification_source, "model_declared");
        let feedback = provider.recorded_feedback();
        assert_eq!(feedback[0].task_success, None);
        assert_eq!(feedback[0].verification_result.as_deref(), Some("not_run"));

        let requests = provider.recorded_requests();
        // The text-only reply was answered with the fixed nudge.
        assert!(requests[1].iter().any(|m| matches!(
            m,
            ChatMessage::User { content } if content.starts_with("[completion gate:")
        )));
        // The first claim came back as a review carrying the task statement.
        assert!(requests[3].iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. }
                if content.starts_with("REVIEW BEFORE COMPLETION")
                    && content.contains("Create out.txt containing x")
        )));
        // The compile-only claim was refused with guidance, not recorded.
        assert!(requests[4].iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. }
                if content.starts_with("ERROR: completion not accepted (1/2)")
                    && content.contains("compileall")
        )));
    }

    #[test]
    fn weak_checks_are_recognised() {
        let argv = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert!(is_weak_check(&argv("python -m compileall -q app")));
        assert!(is_weak_check(&argv("python3 -m py_compile app/x.py")));
        assert!(is_weak_check(&argv("test -f out.txt")));
        assert!(is_weak_check(&argv("cargo build --release")));
        assert!(is_weak_check(&argv("ls -la")));
        assert!(is_weak_check(&argv("true")));
        assert!(!is_weak_check(&argv("cargo test")));
        assert!(!is_weak_check(&argv("pytest -q tests")));
        assert!(!is_weak_check(&argv(
            "python3 verify.py --input sample.csv"
        )));
        assert!(!is_weak_check(&argv("bun test")));
        assert!(!is_weak_check(&argv("grep -q x out.txt")));
    }

    #[tokio::test]
    async fn completion_gate_gives_up_after_the_nudge_cap_and_retries_with_evidence() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            final_text("Let me explore."),
            final_text("Let me explore more."),
            final_text("Still exploring."),
            final_text("Done exploring."),
            // Attempt 2 finishes properly.
            assistant_with_call(
                "c1",
                "complete_task",
                r#"{"summary":"nothing to do","deliverables":[{"item":"n/a","evidence":"n/a"}]}"#,
            ),
            assistant_with_call(
                "c2",
                "complete_task",
                r#"{"summary":"nothing to do","deliverables":[{"item":"n/a","evidence":"n/a"}],"checks":[["sh","-c","exit 0"]],"requirements":[{"requirement":"n/a","status":"met","evidence":"n/a"}]}"#,
            ),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                completion_gate: true,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "do the thing".into(),
                &TaskOptions {
                    max_attempts: 2,
                    verification: VerificationPlan {
                        // Generous: the model's `sh -c` check must not time out
                        // under parallel test load.
                        check_timeout_seconds: 30,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.attempts.len(), 2);
        assert_eq!(
            report.attempts[0].turn.completion_nudges,
            COMPLETION_NUDGE_CAP
        );
        assert!(report.attempts[0].turn.gate_unfinished);
        assert!(!report.attempts[0].turn.completed);
        assert!(report.attempts[1].turn.completed);
        assert!(report.completed);
        // No operator plan: the model's own check ran and passed, but nothing
        // the model did not write backs it, so nothing is "verified".
        assert_eq!(report.success, Some(true));
        assert_eq!(report.verified, None);
        assert_eq!(report.verification_source, "model_declared");
        let requests = provider.recorded_requests();
        assert!(requests[4].iter().any(|m| matches!(
            m,
            ChatMessage::User { content }
                if content.starts_with("COMPLETION GATE (attempt 1 of 2)")
                    && content.contains("STRATEGY FOR THIS ATTEMPT: re-read the task statement")
        )));
    }

    #[tokio::test]
    async fn diff_only_pass_is_a_claim_not_a_verified_success() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            final_text("done"),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "make out.txt".into(),
                &TaskOptions {
                    max_attempts: 2,
                    verification: VerificationPlan {
                        require_diff: true,
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        // The loop is satisfied (the workspace changed)…
        assert_eq!(report.success, Some(true));
        // …but "files changed" proves nothing about the task.
        assert_eq!(report.verified, None);
        assert_eq!(report.verification_source, "diff");
        let feedback = provider.recorded_feedback();
        assert_eq!(feedback.len(), 1);
        assert_eq!(feedback[0].task_success, None);
        assert_eq!(feedback[0].verification_result.as_deref(), Some("not_run"));
    }

    #[tokio::test]
    async fn explicit_check_pass_is_a_verified_success() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            final_text("done"),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                sandboxed: true,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "make out.txt".into(),
                &TaskOptions {
                    max_attempts: 1,
                    verification: VerificationPlan {
                        explicit: vec![vec!["test".into(), "-f".into(), "out.txt".into()]],
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.success, Some(true));
        assert_eq!(report.verified, Some(true));
        assert_eq!(report.verification_source, "explicit");
        let feedback = provider.recorded_feedback();
        assert_eq!(feedback[0].task_success, Some(true));
        assert_eq!(feedback[0].verification_result.as_deref(), Some("passed"));
    }

    #[test]
    fn complete_task_is_advertised_only_behind_the_gate() {
        let default_names: Vec<String> = session_tools(false, false, false, false)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(!default_names.iter().any(|n| n == "complete_task"));
        assert!(!default_names.iter().any(|n| n == "shell"));
        assert!(!default_names.iter().any(|n| n == "report_verdict"));
        let gated: Vec<String> = session_tools(false, false, true, false)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(gated.last().map(String::as_str), Some("complete_task"));
        assert_eq!(&gated[..gated.len() - 1], &default_names[..]);
        // The verifier: observe and run, never edit; its ending is a verdict.
        let verifier: Vec<String> = session_tools(false, true, false, true)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(verifier.last().map(String::as_str), Some("report_verdict"));
        assert!(verifier.iter().any(|n| n == "shell"));
        assert!(verifier.iter().any(|n| n == "run_command"));
        for forbidden in ["edit", "write_file", "update_plan", "complete_task"] {
            assert!(!verifier.iter().any(|n| n == forbidden), "{forbidden}");
        }
    }

    const CLAIM_WITH_REQUIREMENTS: &str = r#"{"summary":"wrote out.txt","deliverables":[{"item":"out.txt","evidence":"write_file ok"}],"checks":[["grep","-q","x","out.txt"]],"requirements":[{"requirement":"out.txt contains x","status":"met","evidence":"grep -q x out.txt exit 0"}]}"#;

    #[tokio::test]
    async fn final_claim_without_requirements_is_rejected_once() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            assistant_with_call(
                "c2",
                "complete_task",
                r#"{"summary":"s","deliverables":[]}"#,
            ),
            // Final claim without the checklist: rejected with guidance…
            assistant_with_call(
                "c3",
                "complete_task",
                r#"{"summary":"s","deliverables":[{"item":"out.txt","evidence":"ok"}],"checks":[["grep","-q","x","out.txt"]]}"#,
            ),
            // …then accepted with it.
            assistant_with_call("c4", "complete_task", CLAIM_WITH_REQUIREMENTS),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                sandboxed: true,
                completion_gate: true,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "Create out.txt containing x".into(),
                &TaskOptions {
                    max_attempts: 2,
                    verification: VerificationPlan {
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.attempts.len(), 1);
        assert!(report.completed);
        let claim = report.completion.unwrap();
        assert_eq!(claim.requirements.len(), 1);
        assert!(claim.requirements[0].is_met());
        let requests = provider.recorded_requests();
        let rejection = requests[3]
            .iter()
            .filter_map(|m| match m {
                ChatMessage::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .find(|c| c.contains("`requirements` is missing"));
        assert!(
            rejection.is_some(),
            "the checklist-less claim is rejected once"
        );
        assert_eq!(report.verification_source, "model_declared");
        assert_eq!(report.attempts[0].turn.self_authored_checks, 0);
    }

    #[tokio::test]
    async fn checks_that_run_files_the_model_wrote_are_self_authored() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            assistant_with_call(
                "c2",
                "write_file",
                r#"{"path":"check.sh","content":"grep -q x out.txt\n"}"#,
            ),
            assistant_with_call(
                "c3",
                "complete_task",
                r#"{"summary":"s","deliverables":[]}"#,
            ),
            assistant_with_call(
                "c4",
                "complete_task",
                r#"{"summary":"s","deliverables":[{"item":"out.txt","evidence":"ok"}],"checks":[["sh","./check.sh"]],"requirements":[{"requirement":"out.txt contains x","status":"met","evidence":"check.sh passes"}]}"#,
            ),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                sandboxed: true,
                completion_gate: true,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "Create out.txt containing x".into(),
                &TaskOptions {
                    max_attempts: 1,
                    verification: VerificationPlan {
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(report.completed);
        // The check ran and passed, but it is the model's own script.
        assert_eq!(
            report.success,
            Some(true),
            "{}",
            report.attempts[0]
                .verification
                .as_ref()
                .map(|v| format!(
                    "{} {:?}",
                    v.summary_line(),
                    v.checks
                        .iter()
                        .map(|c| (c.check.label(), c.output_tail.clone()))
                        .collect::<Vec<_>>()
                ))
                .unwrap_or_default()
        );
        assert_eq!(report.attempts[0].turn.self_authored_checks, 1);
        assert_eq!(report.verification_source, "self_authored");
        assert_eq!(report.verified, None);
        // The deliverable itself is not a check: reading a changed file with
        // an unchanged program stays model-declared (see the gate test above).
        assert!(agent.is_self_authored(&["sh".into(), "./check.sh".into()]));
        assert!(agent.is_self_authored(&["./check.sh".into()]));
        assert!(agent.is_self_authored(&["python3".into(), "-u".into(), "check.sh".into()]));
        assert!(!agent.is_self_authored(&[
            "grep".into(),
            "-q".into(),
            "x".into(),
            "out.txt".into()
        ]));
        assert!(!agent.is_self_authored(&[
            "python3".into(),
            "-m".into(),
            "pytest".into(),
            "check.sh".into()
        ]));
        assert!(!agent.is_self_authored(&["pytest".into(), "tests/".into()]));
    }

    #[tokio::test]
    async fn independent_verifier_turns_a_false_completion_into_a_retry() {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            // Attempt 1 (worker): writes x, claims done with a passing check.
            assistant_with_call("w1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            assistant_with_call(
                "w2",
                "complete_task",
                r#"{"summary":"s","deliverables":[]}"#,
            ),
            assistant_with_call("w3", "complete_task", CLAIM_WITH_REQUIREMENTS),
            // Verifier (fresh session): reads the file, finds y missing.
            assistant_with_call("v1", "read_file", r#"{"path":"out.txt"}"#),
            assistant_with_call(
                "v2",
                "report_verdict",
                r#"{"verdict":"fail","requirements":[{"requirement":"out.txt contains x","status":"met","evidence":"read_file shows x"},{"requirement":"out.txt contains y","status":"unmet","evidence":"read_file shows only x"}]}"#,
            ),
            // Attempt 2 (worker): fixes it and claims again.
            assistant_with_call("w4", "write_file", r#"{"path":"out.txt","content":"xy"}"#),
            assistant_with_call(
                "w5",
                "complete_task",
                r#"{"summary":"s","deliverables":[]}"#,
            ),
            assistant_with_call(
                "w6",
                "complete_task",
                r#"{"summary":"wrote xy","deliverables":[{"item":"out.txt","evidence":"ok"}],"checks":[["grep","-q","xy","out.txt"]],"requirements":[{"requirement":"out.txt contains x and y","status":"met","evidence":"grep"}]}"#,
            ),
            // Verifier again: a text-only reply is nudged, then the verdict.
            final_text("Looks fine to me."),
            assistant_with_call(
                "v3",
                "report_verdict",
                r#"{"verdict":"pass","requirements":[{"requirement":"out.txt contains x and y","status":"met","evidence":"grep -q xy out.txt exit 0"}]}"#,
            ),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                sandboxed: true,
                completion_gate: true,
                independent_verifier: true,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "Create out.txt containing x and y".into(),
                &TaskOptions {
                    max_attempts: 3,
                    verification: VerificationPlan {
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            report.attempts.len(),
            2,
            "the failed verdict forced a retry"
        );
        let first = report.attempts[0].verifier.as_ref().expect("verifier ran");
        let verdict = first.verdict.as_ref().unwrap();
        assert!(!verdict.passed());
        assert_eq!(verdict.unmet().len(), 1);
        assert_eq!(first.provider_calls, 2);
        assert_eq!(first.tool_calls, 2);
        let second = report.attempts[1]
            .verifier
            .as_ref()
            .expect("verifier ran again");
        assert!(second.verdict.as_ref().unwrap().passed());
        assert_eq!(
            second.provider_calls, 2,
            "the text-only reply was nudged, not counted"
        );
        // A verifier pass is still a claim.
        assert_eq!(report.verification_source, "verifier");
        assert_eq!(report.verified, None);
        assert!(report.completed);
        // Every call, worker and verifier, is on the task's bill.
        assert_eq!(report.provider_calls, 3 + 2 + 3 + 2);
        assert_eq!(agent.telemetry_summary().provider_calls, 3 + 2 + 3 + 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
            "xy"
        );
        let requests = provider.recorded_requests();
        // The verifier session starts fresh: its first request holds only its
        // own system prompt and the brief, none of the worker's history.
        let verifier_first = &requests[3];
        assert_eq!(verifier_first.len(), 2, "{verifier_first:?}");
        match &verifier_first[0] {
            ChatMessage::System { content } => {
                assert!(content.contains("Independent verifier"));
                assert!(!content.contains("complete_task"));
            }
            other => panic!("expected system prompt, got {other:?}"),
        }
        match &verifier_first[1] {
            ChatMessage::User { content } => {
                assert!(content.starts_with("INDEPENDENT VERIFICATION"));
                assert!(content.contains("Create out.txt containing x and y"));
                assert!(content.contains("out.txt contains x"));
                assert!(content.contains("grep -q x out.txt"));
            }
            other => panic!("expected the brief, got {other:?}"),
        }
        // Attempt 2 opens with the verdict as evidence, in the worker's session.
        let retry = requests[5]
            .iter()
            .rev()
            .find_map(|m| match m {
                ChatMessage::User { content } => Some(content.as_str()),
                _ => None,
            })
            .unwrap();
        assert!(
            retry.starts_with("INDEPENDENT VERIFICATION (attempt 1 of 3)"),
            "{retry}"
        );
        assert!(retry.contains("out.txt contains y — read_file shows only x"));
        assert!(retry.contains("Confirmed met:\n- out.txt contains x"));
        assert!(retry.contains("STRATEGY FOR THIS ATTEMPT"));
        let feedback = provider.recorded_feedback();
        assert_eq!(feedback.len(), 1);
        assert_eq!(feedback[0].task_success, None);
        assert_eq!(feedback[0].attempt_count, Some(2));
    }

    #[tokio::test]
    async fn the_verifier_can_run_on_another_provider_and_model() {
        let dir = workspace();
        let worker = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("w1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            assistant_with_call(
                "w2",
                "complete_task",
                r#"{"summary":"s","deliverables":[]}"#,
            ),
            assistant_with_call("w3", "complete_task", CLAIM_WITH_REQUIREMENTS),
        ]));
        let reviewer = Arc::new(ScriptedProvider::new(vec![assistant_with_call(
            "v1",
            "report_verdict",
            r#"{"verdict":"pass","requirements":[{"requirement":"out.txt contains x","status":"met","evidence":"cat"}]}"#,
        )]));
        let mut agent = Agent::new(
            worker.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                sandboxed: true,
                completion_gate: true,
                independent_verifier: true,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let other = crate::model::Registry::builtin()
            .resolve("glm-5.3-flash")
            .expect("built-in model");
        agent.set_verifier_backend(reviewer.clone(), other.clone());
        let report = agent
            .run_task(
                "Create out.txt containing x".into(),
                &TaskOptions {
                    max_attempts: 2,
                    verification: VerificationPlan {
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.attempts.len(), 1);
        let verifier = report.attempts[0].verifier.as_ref().unwrap();
        assert_eq!(verifier.model, other.id);
        assert!(verifier.verdict.as_ref().unwrap().passed());
        assert_eq!(report.verification_source, "verifier");
        // The worker's provider never saw the verifier's traffic, and the
        // reviewer's first request is a fresh verifier session.
        assert_eq!(worker.recorded_requests().len(), 3);
        let reviewed = reviewer.recorded_requests();
        assert_eq!(reviewed.len(), 1);
        assert!(
            matches!(&reviewed[0][0], ChatMessage::System { content } if content.contains("Independent verifier"))
        );
        assert_eq!(report.provider_calls, 4);
    }

    #[tokio::test]
    async fn the_verifier_is_told_to_report_before_its_budget_runs_out() {
        let dir = workspace();
        let mut turns = vec![
            assistant_with_call("w1", "write_file", r#"{"path":"out.txt","content":"x"}"#),
            assistant_with_call(
                "w2",
                "complete_task",
                r#"{"summary":"s","deliverables":[]}"#,
            ),
            assistant_with_call("w3", "complete_task", CLAIM_WITH_REQUIREMENTS),
        ];
        // The verifier reads and reads; it only reports once warned.
        let reads = VERIFIER_CALL_CAP - VERIFIER_WARNING_CALLS_LEFT;
        for i in 0..reads {
            turns.push(assistant_with_call(
                &format!("v{i}"),
                "read_file",
                r#"{"path":"out.txt"}"#,
            ));
        }
        turns.push(assistant_with_call(
            "vend",
            "report_verdict",
            r#"{"verdict":"pass","requirements":[{"requirement":"out.txt contains x","status":"met","evidence":"read"}]}"#,
        ));
        let provider = Arc::new(ScriptedProvider::new(turns));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                sandboxed: true,
                completion_gate: true,
                independent_verifier: true,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_task(
                "Create out.txt containing x".into(),
                &TaskOptions {
                    max_attempts: 1,
                    verification: VerificationPlan {
                        check_timeout_seconds: 10,
                        ..VerificationPlan::default()
                    },
                },
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        let verifier = report.attempts[0].verifier.as_ref().unwrap();
        assert!(verifier.verdict.as_ref().unwrap().passed());
        assert_eq!(verifier.provider_calls, reads + 1);
        let requests = provider.recorded_requests();
        let warned: Vec<usize> = requests
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.iter().any(|m| matches!(m, ChatMessage::User { content } if content.contains("calls remain")))
            })
            .map(|(i, _)| i)
            .collect();
        // Exactly one warning, first present in the request that has three calls left.
        let first_warned = *warned.first().expect("warning issued");
        assert_eq!(first_warned, 3 + reads as usize, "{warned:?}");
        let warnings_in_last = requests
            .last()
            .unwrap()
            .iter()
            .filter(
                |m| matches!(m, ChatMessage::User { content } if content.contains("calls remain")),
            )
            .count();
        assert_eq!(warnings_in_last, 1);
    }

    #[tokio::test]
    async fn malformed_tool_arguments_are_replayed_as_valid_json_and_answered_with_a_format_error()
    {
        let dir = workspace();
        let provider = Arc::new(ScriptedProvider::new(vec![
            assistant_with_call("c1", "read_file", r#"{"path": "hello.txt"#),
            final_text("done"),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_turn(
                "read hello".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.tool_calls, 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].kind, FailureKind::ModelFormatFailure);
        let requests = provider.recorded_requests();
        let second = &requests[1];
        let assistant = second
            .iter()
            .find_map(|m| match m {
                ChatMessage::Assistant(message) => Some(message),
                _ => None,
            })
            .unwrap();
        // What goes back to the provider parses: the raw text is kept inside.
        let replayed: serde_json::Value =
            serde_json::from_str(&assistant.tool_calls[0].arguments).unwrap();
        assert_eq!(replayed[INVALID_JSON_KEY], r#"{"path": "hello.txt"#);
        let tool_result = second
            .iter()
            .find_map(|m| match m {
                ChatMessage::Tool { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .unwrap();
        assert!(tool_result.contains("were not valid JSON"), "{tool_result}");
        assert!(tool_result.contains("Call read_file again"));
    }

    #[tokio::test]
    async fn interrupted_streams_are_retried_after_a_backoff() {
        let dir = workspace();
        let provider =
            Arc::new(ScriptedProvider::new(vec![final_text("done")]).with_transport_failures(1));
        let mut agent = Agent::new(
            provider.clone(),
            dir.path().to_path_buf(),
            SessionConfig {
                permission_mode: PermissionMode::Auto,
                ..SessionConfig::default()
            },
            None,
        )
        .unwrap();
        let report = agent
            .run_turn(
                "hi".into(),
                &approval::AutoApprove,
                &mut NullSink,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.assistant_text, "done");
        assert_eq!(report.retries, 1);
        assert_eq!(
            report.provider_calls, 1,
            "a retried call is still one provider round"
        );
        // The same request went out twice, byte for byte.
        let requests = provider.recorded_requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
        assert_eq!(agent.telemetry_summary().provider_errors, 1);
    }
}

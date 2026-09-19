//! Optional System One triage: a typed, non-generative second opinion asked
//! ONLY where the deterministic rules have already given up.
//!
//! Governing law, and the reason this module is safe to have at all:
//!
//! 1. **Triage may tighten, never open.** It is consulted only when a
//!    rule-based classifier returned `UncertainResult` — the answer that
//!    carries no recovery guidance. A kind the rules decided is never
//!    overwritten, so no triage answer can turn a failure into a pass, relax
//!    a policy, or end a task. Invariant 4 (deterministic code owns budgets,
//!    approval, termination) is untouched: this only chooses which sentence
//!    of advice the next attempt reads.
//! 2. **Absent means unchanged.** With no client configured every entry point
//!    is a no-op returning `None`, and the caller keeps exactly the behaviour
//!    it had before this module existed. Butterfly stays a local, offline,
//!    model-agnostic CLI by default.
//! 3. **Best effort, never blocking.** One request, a short timeout, no
//!    retries. Any error, any timeout, any answer under the confidence floor
//!    leaves the rule verdict standing. A triage outage can slow one
//!    classification by `timeout`; it can never fail a task.
//!
//! Privacy: enabling triage sends the failing command line and a bounded tail
//! of its output to a third party (TypeSafe). That is a real egress of build
//! output and it is why this is opt-in and off by default. Nothing else is
//! sent: no prompts, no file contents, no transcript, no credentials.
//!
//! The model behind it (`jev-latest`) does not generate text. It answers typed
//! questions — `choice`, `score`, `noul` — against a state blob and returns a
//! value plus a probability distribution, so a malformed or out-of-vocabulary
//! answer is not a failure mode the caller has to parse around.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::failure::FailureKind;

pub const DEFAULT_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
pub const DEFAULT_MODEL: &str = "jev-latest";
/// Answers below this confidence are discarded and the rule verdict stands.
/// TypeSafe's own guidance calls <0.5 "genuinely unsure" and >0.9 safe to act
/// on automatically; a misclassification here costs a wasted attempt, so we
/// start at the top of that band and let the operator lower it.
pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.9;
/// One request, this long, no retries.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Bytes of command output handed to the classifier. The rules already work
/// from `EVIDENCE_CAP_BYTES` of tail; matching it keeps the two looking at the
/// same evidence.
const STATE_CAP_BYTES: usize = 3_000;

/// The provider id this module's key is stored under in the credential store
/// (`~/.config/butterfly/credentials.toml`), and the stem of its environment
/// variable. Not a model-registry provider: System One is not an
/// OpenAI-compatible chat endpoint and never serves a turn.
pub const CREDENTIAL_ID: &str = "typesafe";
pub const API_KEY_ENV: &str = "TYPESAFE_API_KEY";

/// A typed question. Serializes to the System One wire shape.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    /// Truthfulness, answered as a probability in 0..=1.
    Noul {
        instructions: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<BTreeMap<String, Option<String>>>,
    },
    /// One option from a closed set; `criteria` maps option to rubric
    /// description (null when the label speaks for itself).
    Choice {
        instructions: String,
        criteria: BTreeMap<String, Option<String>>,
    },
    /// A rating along an ordered rubric of at least two levels.
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct TriageUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Answer {
    #[serde(default)]
    pub noul: Option<f64>,
    #[serde(default)]
    pub choice: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub probabilities: BTreeMap<String, f64>,
    /// Absent on `noul`, which carries its own probability instead.
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Answers {
    #[serde(default)]
    pub answers: BTreeMap<String, Answer>,
    #[serde(default)]
    pub usage: TriageUsage,
}

#[derive(Debug, thiserror::Error)]
pub enum TriageError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("status {status}: {message}")]
    Status { status: u16, message: String },
    #[error("malformed response: {0}")]
    Malformed(String),
}

/// A System One endpoint. Implemented by `JevClient` in production and by a
/// scripted fake in tests, so the deterministic suite never touches the
/// network (invariant 8).
#[async_trait]
pub trait SystemOne: Send + Sync {
    async fn ask(
        &self,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Answers, TriageError>;
}

#[derive(Debug, Clone, Serialize)]
struct SystemOneRequest<'a> {
    state: &'a str,
    model: &'a str,
    questions: &'a BTreeMap<String, Question>,
}

/// TypeSafe's hosted System One endpoint.
pub struct JevClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

// The key must never reach a log line, a session file, or telemetry
// (invariant 6), and `{:?}` on a config struct is how that usually happens.
impl std::fmt::Debug for JevClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevClient")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl JevClient {
    pub fn new(
        api_key: String,
        endpoint: Option<String>,
        model: Option<String>,
        timeout: Duration,
    ) -> Result<Self, TriageError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| TriageError::Transport(error.to_string()))?;
        Ok(Self {
            http,
            endpoint: endpoint.unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
            api_key,
            model: model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        })
    }
}

#[async_trait]
impl SystemOne for JevClient {
    async fn ask(
        &self,
        state: &str,
        questions: &BTreeMap<String, Question>,
    ) -> Result<Answers, TriageError> {
        let body = SystemOneRequest {
            state,
            model: &self.model,
            questions,
        };
        // No retry ladder on purpose: a 429 or 529 means the rule verdict
        // stands this once. Backing off here would put the agent loop to
        // sleep for a classification it can already live without.
        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| TriageError::Transport(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            return Err(TriageError::Status {
                status: status.as_u16(),
                message,
            });
        }
        response
            .json::<Answers>()
            .await
            .map_err(|error| TriageError::Malformed(error.to_string()))
    }
}

/// The handle the session holds. `Triage::disabled()` is the default and makes
/// every entry point a no-op.
#[derive(Clone)]
pub struct Triage {
    client: Option<Arc<dyn SystemOne>>,
    min_confidence: f64,
}

impl std::fmt::Debug for Triage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Triage")
            .field("enabled", &self.client.is_some())
            .field("min_confidence", &self.min_confidence)
            .finish()
    }
}

impl Default for Triage {
    fn default() -> Self {
        Self::disabled()
    }
}

/// What one triage attempt did, for telemetry. Every variant but `Refined`
/// leaves the rule verdict in place.
#[derive(Debug, Clone, PartialEq)]
pub enum TriageOutcome {
    /// A confident answer the caller applied.
    Refined { kind: FailureKind, confidence: f64 },
    /// The classifier declined: it picked `uncertain_result` itself.
    Declined { confidence: f64 },
    /// An answer arrived but sat under the confidence floor.
    BelowFloor { kind: FailureKind, confidence: f64 },
    /// The endpoint failed, timed out, or answered in a shape we do not read.
    Failed { error: String },
}

impl Triage {
    pub fn disabled() -> Self {
        Self {
            client: None,
            min_confidence: DEFAULT_MIN_CONFIDENCE,
        }
    }

    pub fn new(client: Arc<dyn SystemOne>, min_confidence: f64) -> Self {
        Self {
            client: Some(client),
            min_confidence: min_confidence.clamp(0.0, 1.0),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.client.is_some()
    }

    pub fn min_confidence(&self) -> f64 {
        self.min_confidence
    }

    /// Ask what kind of failure a command's output describes.
    ///
    /// Returns `None` — meaning "keep what the rules said" — when triage is
    /// disabled or when `rules` is anything other than `UncertainResult`. The
    /// second condition is the tighten-only rule in code: a confident
    /// rule-based verdict is never sent for a second opinion, so triage can
    /// only ever fill a gap, never contradict.
    pub async fn refine_command_failure(
        &self,
        rules: FailureKind,
        command: &str,
        output: &str,
    ) -> Option<TriageOutcome> {
        let client = self.client.as_ref()?;
        if rules != FailureKind::UncertainResult {
            return None;
        }
        let questions = BTreeMap::from([("failure_kind".to_string(), failure_kind_question())]);
        let state = command_state(command, output);
        let outcome = match client.ask(&state, &questions).await {
            Err(error) => TriageOutcome::Failed {
                error: error.to_string(),
            },
            Ok(answers) => match answers.answers.get("failure_kind") {
                None => TriageOutcome::Failed {
                    error: "no answer for failure_kind".to_string(),
                },
                Some(answer) => self.read_kind(answer),
            },
        };
        Some(outcome)
    }

    fn read_kind(&self, answer: &Answer) -> TriageOutcome {
        let confidence = answer.confidence.unwrap_or(0.0);
        let Some(chosen) = answer.choice.as_deref() else {
            return TriageOutcome::Failed {
                error: "answer carried no choice".to_string(),
            };
        };
        let Some(kind) = kind_from_wire(chosen) else {
            return TriageOutcome::Failed {
                error: format!("unknown failure kind: {chosen}"),
            };
        };
        if kind == FailureKind::UncertainResult {
            return TriageOutcome::Declined { confidence };
        }
        if confidence < self.min_confidence {
            return TriageOutcome::BelowFloor { kind, confidence };
        }
        TriageOutcome::Refined { kind, confidence }
    }
}

/// The closed option set offered for a command failure, with the rubric each
/// option is judged against.
///
/// Two `FailureKind`s are deliberately absent. `ModelFormatFailure` describes
/// malformed tool arguments, not command output, and `RepeatedPatchFailure` is
/// loop state (the same edit failing twice) that no single output can show.
/// `UncertainResult` IS offered so the classifier can decline instead of being
/// forced into a guess.
const COMMAND_FAILURE_OPTIONS: &[(FailureKind, &str)] = &[
    (
        FailureKind::SyntaxFailure,
        "The source could not be parsed at all: a syntax, indentation, or unterminated-literal error.",
    ),
    (
        FailureKind::CompileFailure,
        "The source parsed but failed compilation, type checking, or name and import resolution.",
    ),
    (
        FailureKind::TestFailure,
        "The code built and a test or assertion actually ran, and it reported a wrong result.",
    ),
    (
        FailureKind::MissingContext,
        "A file, directory, or artifact the command expected to read does not exist.",
    ),
    (
        FailureKind::IncorrectAssumption,
        "The command ran and completed, but the output contradicts an assumption about how the code or data behaves.",
    ),
    (
        FailureKind::ToolFailure,
        "The command itself was rejected: bad arguments, an unsupported flag, or a refusal by policy or permissions.",
    ),
    (
        FailureKind::EnvironmentFailure,
        "The environment is at fault: a missing binary or dependency, no network access, or exhausted memory or disk.",
    ),
    (
        FailureKind::Timeout,
        "The command exceeded its time budget and was killed before finishing.",
    ),
    (
        FailureKind::UncertainResult,
        "The output does not clearly indicate any of the other categories. Choose this rather than guessing.",
    ),
];

fn failure_kind_question() -> Question {
    Question::Choice {
        instructions: "A command run by a coding agent failed. Which category does its output \
             describe? Judge only what the output shows, not what the command was meant to do."
            .to_string(),
        criteria: COMMAND_FAILURE_OPTIONS
            .iter()
            .map(|(kind, rubric)| (kind.as_str().to_string(), Some(rubric.to_string())))
            .collect(),
    }
}

/// `FailureKind` from its wire spelling, via the same snake_case mapping the
/// enum already serializes with — so the option keys we send and the answer we
/// read can never drift apart.
fn kind_from_wire(text: &str) -> Option<FailureKind> {
    serde_json::from_value(serde_json::Value::String(text.to_string())).ok()
}

/// The state blob: the command line, then a bounded tail of its output.
fn command_state(command: &str, output: &str) -> String {
    format!(
        "$ {command}\n\n{}",
        crate::failure::evidence_tail(output, STATE_CAP_BYTES)
    )
}

/// Scripted System One endpoints for tests. The deterministic suite must never
/// touch the network (invariant 8), so nothing outside a live test may
/// construct a `JevClient`.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub(crate) struct Scripted(pub(crate) Result<Answers, &'static str>);

    #[async_trait]
    impl SystemOne for Scripted {
        async fn ask(
            &self,
            _state: &str,
            _questions: &BTreeMap<String, Question>,
        ) -> Result<Answers, TriageError> {
            match &self.0 {
                Ok(answers) => Ok(answers.clone()),
                Err(message) => Err(TriageError::Transport(message.to_string())),
            }
        }
    }

    /// A triage whose endpoint always answers `choice` at `confidence`.
    pub(crate) fn answering(choice: &str, confidence: f64) -> Triage {
        let answers = Answers {
            answers: BTreeMap::from([(
                "failure_kind".to_string(),
                Answer {
                    choice: Some(choice.to_string()),
                    confidence: Some(confidence),
                    ..Answer::default()
                },
            )]),
            usage: TriageUsage::default(),
        };
        Triage::new(Arc::new(Scripted(Ok(answers))), DEFAULT_MIN_CONFIDENCE)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{answering, Scripted};
    use super::*;

    #[tokio::test]
    async fn disabled_triage_never_answers() {
        let outcome = Triage::disabled()
            .refine_command_failure(FailureKind::UncertainResult, "cargo test", "???")
            .await;
        assert!(outcome.is_none());
    }

    /// The tighten-only rule: a kind the rules decided is never second-guessed.
    #[tokio::test]
    async fn a_decided_kind_is_never_sent_for_review() {
        for decided in [
            FailureKind::TestFailure,
            FailureKind::CompileFailure,
            FailureKind::EnvironmentFailure,
        ] {
            let outcome = answering("environment_failure", 1.0)
                .refine_command_failure(decided, "cargo test", "boom")
                .await;
            assert!(outcome.is_none(), "{decided:?} was sent for review");
        }
    }

    #[tokio::test]
    async fn a_confident_answer_refines_an_uncertain_verdict() {
        let outcome = answering("test_failure", 0.97)
            .refine_command_failure(FailureKind::UncertainResult, "bun test", "1 of 3 wrong")
            .await;
        assert_eq!(
            outcome,
            Some(TriageOutcome::Refined {
                kind: FailureKind::TestFailure,
                confidence: 0.97,
            })
        );
    }

    #[tokio::test]
    async fn an_answer_under_the_floor_leaves_the_verdict_alone() {
        let outcome = answering("test_failure", 0.62)
            .refine_command_failure(FailureKind::UncertainResult, "bun test", "?")
            .await;
        assert_eq!(
            outcome,
            Some(TriageOutcome::BelowFloor {
                kind: FailureKind::TestFailure,
                confidence: 0.62,
            })
        );
    }

    #[tokio::test]
    async fn the_classifier_may_decline() {
        let outcome = answering("uncertain_result", 0.99)
            .refine_command_failure(FailureKind::UncertainResult, "make", "...")
            .await;
        assert_eq!(outcome, Some(TriageOutcome::Declined { confidence: 0.99 }));
    }

    #[tokio::test]
    async fn an_endpoint_failure_is_survivable() {
        let triage = Triage::new(
            Arc::new(Scripted(Err("connection refused"))),
            DEFAULT_MIN_CONFIDENCE,
        );
        let outcome = triage
            .refine_command_failure(FailureKind::UncertainResult, "make", "...")
            .await;
        assert!(matches!(outcome, Some(TriageOutcome::Failed { .. })));
    }

    #[tokio::test]
    async fn an_out_of_vocabulary_answer_is_rejected_not_panicked_on() {
        let outcome = answering("catastrophic_vibes", 1.0)
            .refine_command_failure(FailureKind::UncertainResult, "make", "...")
            .await;
        assert!(matches!(outcome, Some(TriageOutcome::Failed { .. })));
    }

    /// Every option we advertise must round-trip back to the enum, or a
    /// correct answer would be discarded as out-of-vocabulary.
    #[test]
    fn every_offered_option_parses_back_to_its_kind() {
        for (kind, _) in COMMAND_FAILURE_OPTIONS {
            assert_eq!(kind_from_wire(kind.as_str()), Some(*kind));
        }
        let Question::Choice { criteria, .. } = failure_kind_question() else {
            panic!("the failure-kind question must be a choice");
        };
        assert_eq!(criteria.len(), COMMAND_FAILURE_OPTIONS.len());
        assert!(criteria.contains_key("uncertain_result"));
        // Loop state and tool-argument shapes are not answerable from output.
        assert!(!criteria.contains_key("repeated_patch_failure"));
        assert!(!criteria.contains_key("model_format_failure"));
        // Nor is an unchanged workspace: that verdict comes from the diff, not
        // from a command, and the loop decides it without asking anyone.
        assert!(!criteria.contains_key("workspace_unchanged"));
    }

    #[test]
    fn the_state_blob_carries_the_command_and_a_bounded_tail() {
        let state = command_state("cargo test", &"x".repeat(10_000));
        assert!(state.starts_with("$ cargo test\n\n"));
        assert!(state.contains("[…]"));
        assert!(state.len() < STATE_CAP_BYTES + 100);
    }

    #[test]
    fn the_key_never_reaches_a_debug_line() {
        let client = JevClient::new("sk-secret-value".into(), None, None, DEFAULT_TIMEOUT).unwrap();
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("sk-secret-value"));
        assert!(rendered.contains("<redacted>"));
        let triage = Triage::new(Arc::new(client), 0.9);
        assert!(!format!("{triage:?}").contains("sk-secret-value"));
    }

    #[test]
    fn the_request_serializes_to_the_system_one_wire_shape() {
        let questions = BTreeMap::from([("failure_kind".to_string(), failure_kind_question())]);
        let body = serde_json::to_value(SystemOneRequest {
            state: "$ cargo test\n\nboom",
            model: DEFAULT_MODEL,
            questions: &questions,
        })
        .unwrap();
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["state"], "$ cargo test\n\nboom");
        assert_eq!(body["questions"]["failure_kind"]["type"], "choice");
        assert_eq!(
            body["questions"]["failure_kind"]["criteria"]["test_failure"],
            COMMAND_FAILURE_OPTIONS
                .iter()
                .find(|(k, _)| *k == FailureKind::TestFailure)
                .unwrap()
                .1
        );
    }

    #[test]
    fn a_response_parses_from_the_documented_shape() {
        let answers: Answers = serde_json::from_str(
            r#"{"model":"jev-latest","answers":{"failure_kind":{"type":"choice",
                "choice":"test_failure","probabilities":{"test_failure":0.96},
                "confidence":0.93}},"usage":{"input_tokens":312,"output_tokens":48}}"#,
        )
        .unwrap();
        let answer = &answers.answers["failure_kind"];
        assert_eq!(answer.choice.as_deref(), Some("test_failure"));
        assert_eq!(answer.confidence, Some(0.93));
        assert_eq!(answers.usage.input_tokens, 312);
    }
}

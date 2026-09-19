//! Observability: a per-session JSONL stream of structured events (provider
//! calls, tool calls, compactions, verification, attempts) plus a running
//! summary the CLI reports. Nothing here contains credentials or file
//! contents: events carry ids, counts, durations, tool names, call previews,
//! and failure kinds; free text is limited to one-line summaries.
//!
//! Also home to key-based redaction for anything persisted about
//! configuration or errors.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::Usage;
use crate::failure::FailureKind;
use crate::provider::FinishReason;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TelemetryEvent {
    ProviderCall {
        at_epoch_ms: u64,
        model: String,
        provider: String,
        latency_ms: u64,
        retries: u32,
        usage: Usage,
        actual_microusd: u64,
        normalized_microusd: u64,
        price_verified: bool,
        finish: String,
        /// The request followed a history rewrite this turn, so a low
        /// cache-hit ratio is expected rather than a byte-stability regression.
        after_rewrite: bool,
    },
    ProviderError {
        at_epoch_ms: u64,
        model: String,
        provider: String,
        kind: FailureKind,
        summary: String,
    },
    ToolCall {
        at_epoch_ms: u64,
        name: String,
        preview: String,
        ok: bool,
        duration_ms: u64,
        failure: Option<FailureKind>,
    },
    Compaction {
        at_epoch_ms: u64,
        stage: String,
        before_tokens: u64,
        after_tokens: u64,
    },
    Verification {
        at_epoch_ms: u64,
        attempt: u32,
        passed: bool,
        checks: usize,
        failure: Option<FailureKind>,
        summary: String,
    },
    /// One System One triage attempt. Recorded whether or not it changed
    /// anything, so the refinement rate and the decline rate are both
    /// measurable: `to: null` means the rule verdict stood.
    Triage {
        at_epoch_ms: u64,
        /// Where in the loop the question was asked, e.g. `verification`.
        site: String,
        from: FailureKind,
        to: Option<FailureKind>,
        confidence: Option<f64>,
        latency_ms: u64,
        /// One line; absent when the endpoint answered.
        error: Option<String>,
    },
    TaskFinished {
        at_epoch_ms: u64,
        attempts: u32,
        success: Option<bool>,
    },
}

/// Running totals for the session, updated on every recorded event.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    pub provider_calls: u32,
    pub provider_errors: u32,
    pub retries: u32,
    pub latency_ms: u64,
    pub tool_calls: u32,
    pub tool_failures: u32,
    pub compactions: u32,
    pub elisions: u32,
    pub verifications: u32,
    pub verification_passed: Option<bool>,
    pub attempts: u32,
    pub task_success: Option<bool>,
    pub usage: Usage,
    pub actual_microusd: u64,
    pub normalized_microusd: u64,
    /// Any request priced with an unverified card taints the totals.
    pub price_verified: bool,
    pub failures_by_kind: BTreeMap<String, u32>,
    /// Requests that saw a cache-hit ratio below 50% without a preceding
    /// rewrite: a byte-stability regression signal worth alarming on.
    pub suspicious_cache_misses: u32,
    /// System One triage attempts, and how many of them actually replaced an
    /// `uncertain_result`. The ratio is the whole case for having it on.
    pub triage_calls: u32,
    pub triage_refined: u32,
    pub triage_latency_ms: u64,
}

impl Summary {
    pub fn cache_hit_rate(&self) -> f64 {
        self.usage.cache_hit_rate()
    }

    /// Fold another session's counters into this one (the independent
    /// verifier's bill belongs to the task that spawned it). Verdict-shaped
    /// fields (`verification_passed`, `task_success`, `attempts`) are the
    /// owner's alone.
    pub fn absorb(&mut self, other: &Summary) {
        self.provider_calls += other.provider_calls;
        self.provider_errors += other.provider_errors;
        self.retries += other.retries;
        self.latency_ms += other.latency_ms;
        self.tool_calls += other.tool_calls;
        self.tool_failures += other.tool_failures;
        self.compactions += other.compactions;
        self.elisions += other.elisions;
        self.usage.add(other.usage);
        self.actual_microusd += other.actual_microusd;
        self.normalized_microusd += other.normalized_microusd;
        self.price_verified &= other.price_verified;
        self.suspicious_cache_misses += other.suspicious_cache_misses;
        self.triage_calls += other.triage_calls;
        self.triage_refined += other.triage_refined;
        self.triage_latency_ms += other.triage_latency_ms;
        for (kind, count) in &other.failures_by_kind {
            *self.failures_by_kind.entry(kind.clone()).or_default() += count;
        }
    }
}

/// Appends events to `path` (when set) and folds them into `summary`.
#[derive(Debug, Default)]
pub struct Recorder {
    path: Option<PathBuf>,
    pub summary: Summary,
    write_failed: bool,
}

impl Recorder {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            summary: Summary {
                price_verified: true,
                ..Summary::default()
            },
            write_failed: false,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    pub fn record(&mut self, event: TelemetryEvent) {
        self.fold(&event);
        if let Some(path) = &self.path {
            if self.write_failed {
                return;
            }
            let result = (|| -> std::io::Result<()> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut line = serde_json::to_string(&event)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                line.push('\n');
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?;
                restrict_permissions(path);
                file.write_all(line.as_bytes())
            })();
            // Telemetry must never break a session; fail once, quietly.
            if result.is_err() {
                self.write_failed = true;
            }
        }
    }

    fn fold(&mut self, event: &TelemetryEvent) {
        let summary = &mut self.summary;
        match event {
            TelemetryEvent::ProviderCall {
                latency_ms,
                retries,
                usage,
                actual_microusd,
                normalized_microusd,
                price_verified,
                after_rewrite,
                ..
            } => {
                summary.provider_calls += 1;
                summary.retries += retries;
                summary.latency_ms += latency_ms;
                summary.usage.add(*usage);
                summary.actual_microusd += actual_microusd;
                summary.normalized_microusd += normalized_microusd;
                summary.price_verified &= *price_verified;
                // The very first request of a session is a cold read by
                // definition; only later requests can signal a regression.
                if !after_rewrite
                    && summary.provider_calls > 1
                    && usage.input_tokens >= 2_000
                    && usage.cache_hit_rate() < 0.5
                {
                    summary.suspicious_cache_misses += 1;
                }
            }
            TelemetryEvent::ProviderError { kind, .. } => {
                summary.provider_errors += 1;
                *summary
                    .failures_by_kind
                    .entry(kind.as_str().to_string())
                    .or_default() += 1;
            }
            TelemetryEvent::ToolCall { ok, failure, .. } => {
                summary.tool_calls += 1;
                if !ok {
                    summary.tool_failures += 1;
                    if let Some(kind) = failure {
                        *summary
                            .failures_by_kind
                            .entry(kind.as_str().to_string())
                            .or_default() += 1;
                    }
                }
            }
            TelemetryEvent::Compaction { stage, .. } => {
                if stage == "elision" {
                    summary.elisions += 1;
                } else {
                    summary.compactions += 1;
                }
            }
            TelemetryEvent::Verification {
                passed, failure, ..
            } => {
                summary.verifications += 1;
                summary.verification_passed = Some(*passed);
                if let Some(kind) = failure {
                    *summary
                        .failures_by_kind
                        .entry(kind.as_str().to_string())
                        .or_default() += 1;
                }
            }
            TelemetryEvent::Triage { to, latency_ms, .. } => {
                summary.triage_calls += 1;
                summary.triage_latency_ms += latency_ms;
                if to.is_some() {
                    summary.triage_refined += 1;
                }
            }
            TelemetryEvent::TaskFinished {
                attempts, success, ..
            } => {
                summary.attempts = *attempts;
                summary.task_success = *success;
            }
        }
    }
}

fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

pub fn finish_name(finish: FinishReason) -> &'static str {
    match finish {
        FinishReason::Stop => "stop",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::Length => "length",
        FinishReason::Other => "other",
    }
}

pub fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

/// Key-based redaction for anything persisted about configuration or errors.
/// Values under credential-looking keys are replaced before hitting disk.
/// Conversation content is persisted verbatim (it is required to resume a
/// session) — but credentials never enter the message history in the first
/// place: the provider key lives only in process memory and command
/// environments are credential-stripped.
pub fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if crate::tools::policy::is_credential_key(key) {
                    *entry = Value::String("[redacted]".to_string());
                } else {
                    redact_value(entry);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_credential_keys_recursively() {
        let mut value = json!({
            "model": "deepseek-v4-flash",
            "api_key": "sk-123",
            "nested": {"github_token": "ghp_x", "path": "src"},
            "list": [{"password": "p"}]
        });
        redact_value(&mut value);
        assert_eq!(value["api_key"], "[redacted]");
        assert_eq!(value["nested"]["github_token"], "[redacted]");
        assert_eq!(value["list"][0]["password"], "[redacted]");
        assert_eq!(value["nested"]["path"], "src");
        assert_eq!(value["model"], "deepseek-v4-flash");
    }

    fn call(hit: u64, total: u64, after_rewrite: bool) -> TelemetryEvent {
        TelemetryEvent::ProviderCall {
            at_epoch_ms: 1,
            model: "m".into(),
            provider: "p".into(),
            latency_ms: 100,
            retries: 1,
            usage: Usage {
                input_tokens: total,
                cache_hit_input_tokens: hit,
                cache_miss_input_tokens: total - hit,
                output_tokens: 10,
            },
            actual_microusd: 5,
            normalized_microusd: 9,
            price_verified: true,
            finish: "stop".into(),
            after_rewrite,
        }
    }

    #[test]
    fn recorder_writes_jsonl_and_folds_a_summary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t/telemetry.jsonl");
        let mut recorder = Recorder::new(Some(path.clone()));
        recorder.record(call(0, 10_000, false)); // cold first call: not suspicious
        recorder.record(call(9_000, 10_000, false));
        recorder.record(call(100, 10_000, false)); // regression signal
        recorder.record(call(100, 10_000, true)); // expected after a rewrite
        recorder.record(TelemetryEvent::ToolCall {
            at_epoch_ms: 1,
            name: "edit".into(),
            preview: "edit(a.rs)".into(),
            ok: false,
            duration_ms: 3,
            failure: Some(FailureKind::IncorrectAssumption),
        });
        recorder.record(TelemetryEvent::Compaction {
            at_epoch_ms: 1,
            stage: "elision".into(),
            before_tokens: 10,
            after_tokens: 5,
        });
        recorder.record(TelemetryEvent::Verification {
            at_epoch_ms: 1,
            attempt: 1,
            passed: false,
            checks: 2,
            failure: Some(FailureKind::TestFailure),
            summary: "1/2".into(),
        });
        recorder.record(TelemetryEvent::TaskFinished {
            at_epoch_ms: 1,
            attempts: 2,
            success: Some(true),
        });
        let s = &recorder.summary;
        assert_eq!(s.provider_calls, 4);
        assert_eq!(s.retries, 4);
        assert_eq!(s.latency_ms, 400);
        assert_eq!(s.usage.input_tokens, 40_000);
        assert_eq!(s.actual_microusd, 20);
        assert_eq!(s.suspicious_cache_misses, 1);
        assert_eq!(s.tool_calls, 1);
        assert_eq!(s.tool_failures, 1);
        assert_eq!(s.elisions, 1);
        assert_eq!(s.verification_passed, Some(false));
        assert_eq!(s.task_success, Some(true));
        assert_eq!(s.failures_by_kind["incorrect_assumption"], 1);
        assert_eq!(s.failures_by_kind["test_failure"], 1);
        let lines: Vec<String> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        assert_eq!(lines.len(), 8);
        assert!(lines[0].contains("\"event\":\"provider_call\""));
        let parsed: TelemetryEvent = serde_json::from_str(&lines[4]).unwrap();
        assert!(matches!(parsed, TelemetryEvent::ToolCall { .. }));
    }

    #[test]
    fn recorder_without_a_path_only_summarizes() {
        let mut recorder = Recorder::new(None);
        recorder.record(call(0, 10, false));
        assert_eq!(recorder.summary.provider_calls, 1);
        assert!(recorder.path().is_none());
    }
}

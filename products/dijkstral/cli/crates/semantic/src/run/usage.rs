//! Usage accounting: tokens and wall time per model request, totals reported at the end and
//! written to `dijkstral-out/semantic/usage.json` (one section per command).

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One model request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// `extract` or `label`.
    pub kind: String,
    /// The chunk (extract) or batch (label) number, from 1.
    pub request: usize,
    /// Attempt number for this piece of work, from 1.
    pub attempt: usize,
    /// Files (extract) or communities (label) in the request.
    pub items: usize,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub wall_ms: u64,
    /// `ok`, `hollow`, `invalid`, `truncated`, `error` or `pending`.
    pub outcome: String,
}

/// All requests of one command, with totals.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageReport {
    pub backend: String,
    pub model: String,
    pub requests: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Summed request time (exceeds `elapsed_ms` when requests overlap).
    pub request_ms: u64,
    /// Wall time of the whole command.
    pub elapsed_ms: u64,
    pub records: Vec<UsageRecord>,
}

impl UsageReport {
    /// `model calls: 1 · tokens 16,843 in / 3,275 out · 70.3 s` (or that nothing was sent).
    pub fn summary(&self) -> String {
        if self.requests == 0 {
            return format!("model calls: 0 ({} {})", self.backend, self.model);
        }
        format!(
            "model calls: {} ({} {}) · tokens {} in / {} out · {:.1} s",
            self.requests,
            self.backend,
            self.model,
            thousands(self.input_tokens),
            thousands(self.output_tokens),
            self.elapsed_ms as f64 / 1000.0
        )
    }

    /// Writes this report as the `section` (`extract`, `label`) of
    /// `<out_dir>/semantic/usage.json`, keeping the other sections.
    pub fn write(&self, out_dir: &Path, section: &str) -> Result<()> {
        let path = out_dir.join("semantic").join("usage.json");
        let mut doc = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Default::default()));
        doc.as_object_mut()
            .expect("object")
            .insert(section.to_string(), serde_json::to_value(self)?);
        super::cache::write_atomic(&path, &serde_json::to_vec_pretty(&doc)?)
            .with_context(|| format!("writing {}", path.display()))
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Thread-safe collector.
pub(crate) struct UsageLog {
    started: Instant,
    records: Mutex<Vec<UsageRecord>>,
}

impl UsageLog {
    pub fn new() -> Self {
        UsageLog {
            started: Instant::now(),
            records: Mutex::new(Vec::new()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &self,
        kind: &str,
        request: usize,
        attempt: usize,
        items: usize,
        tokens: (Option<u64>, Option<u64>),
        wall: Duration,
        outcome: &str,
    ) {
        self.records.lock().expect("usage lock").push(UsageRecord {
            kind: kind.to_string(),
            request,
            attempt,
            items,
            input_tokens: tokens.0,
            output_tokens: tokens.1,
            wall_ms: wall.as_millis() as u64,
            outcome: outcome.to_string(),
        });
    }

    pub fn report(self, backend: &str, model: &str) -> UsageReport {
        let mut records = self.records.into_inner().expect("usage lock");
        records.sort_by_key(|r| (r.request, r.attempt));
        let sent: Vec<&UsageRecord> = records.iter().filter(|r| r.outcome != "pending").collect();
        UsageReport {
            backend: backend.to_string(),
            model: model.to_string(),
            requests: sent.len(),
            input_tokens: sent.iter().filter_map(|r| r.input_tokens).sum(),
            output_tokens: sent.iter().filter_map(|r| r.output_tokens).sum(),
            request_ms: sent.iter().map(|r| r.wall_ms).sum(),
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            records,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_and_sections() {
        let log = UsageLog::new();
        log.add(
            "extract",
            1,
            1,
            3,
            (Some(16843), Some(3275)),
            Duration::from_millis(70300),
            "ok",
        );
        log.add("extract", 2, 1, 1, (None, None), Duration::ZERO, "pending");
        let r = log.report("codex", "gpt-6.1-sol");
        assert_eq!(r.requests, 1);
        assert!(r
            .summary()
            .starts_with("model calls: 1 (codex gpt-6.1-sol) · tokens 16,843 in / 3,275 out"));
        let dir = tempfile::tempdir().unwrap();
        r.write(dir.path(), "extract").unwrap();
        UsageReport::default().write(dir.path(), "label").unwrap();
        let doc: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("semantic/usage.json")).unwrap())
                .unwrap();
        assert_eq!(doc["extract"]["input_tokens"], 16843);
        assert_eq!(doc["label"]["requests"], 0);
    }
}

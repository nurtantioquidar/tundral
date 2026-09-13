//! Typed working state: the session's own record of what it is doing,
//! maintained as events happen rather than reconstructed from the transcript.
//!
//! Two update paths:
//! - deterministic: the loop calls `observe()` for every executed tool call
//!   (files read/changed, commands and their outcomes) — correct by
//!   construction, no model involved;
//! - declarative: the model calls the `update_plan` tool to record objective,
//!   constraints, decisions, and the task list — the fields that cannot be
//!   derived mechanically.
//!
//! The state lives OUTSIDE the message transcript, so it survives elision,
//! summarization, and resume unchanged; compaction checkpoints serialize it
//! instead of re-scanning a (possibly already compacted) history.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::failure::FailureKind;

const LIST_CAP: usize = 20;
const FAILURES_CAP: usize = 10;
const ENTRY_CAP_CHARS: usize = 200;
const FILES_SHOWN_CAP: usize = 40;
const COMMANDS_CAP: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanTask {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub command: String,
    pub ok: bool,
}

/// A classified failure the loop recorded, kept for compaction checkpoints
/// and routing metadata (`previous_failure`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedFailure {
    pub kind: FailureKind,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingState {
    pub objective: Option<String>,
    pub constraints: Vec<String>,
    pub decisions: Vec<String>,
    pub tasks: Vec<PlanTask>,
    pub files_changed: BTreeSet<String>,
    pub files_read: BTreeSet<String>,
    /// Latest outcome per distinct command, most recent last.
    pub commands: Vec<CommandOutcome>,
    /// Most recent classified failures (tool, verification, provider).
    #[serde(default)]
    pub failures: Vec<RecordedFailure>,
    /// Verification commands the model declared via `update_plan`; the loop
    /// runs them itself and never trusts a claimed result.
    #[serde(default)]
    pub verification_commands: Vec<Vec<String>>,
    /// One-line result of the last verification stage, if any ran.
    #[serde(default)]
    pub verification: Option<String>,
    /// Questions put to the user in clarify mode and what came back; they are
    /// authoritative and must survive compaction.
    #[serde(default)]
    pub clarifications: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePlanArgs {
    #[serde(default)]
    objective: Option<String>,
    /// Replaces the constraint list when present.
    #[serde(default)]
    constraints: Option<Vec<String>>,
    /// Appended (deduplicated) when present.
    #[serde(default)]
    add_decisions: Option<Vec<String>>,
    /// Replaces the task list when present.
    #[serde(default)]
    tasks: Option<Vec<PlanTask>>,
    /// Replaces the declared verification commands (argv vectors) when
    /// present. Accepted but not advertised in the schema (prefix bytes).
    #[serde(default)]
    verification: Option<Vec<Vec<String>>>,
}

fn clip(text: &str) -> String {
    let mut clipped = text.trim().to_string();
    if clipped.len() > ENTRY_CAP_CHARS {
        let mut end = ENTRY_CAP_CHARS;
        while !clipped.is_char_boundary(end) {
            end -= 1;
        }
        clipped.truncate(end);
        clipped.push('…');
    }
    clipped
}

impl WorkingState {
    /// Apply a model-issued `update_plan` call. Returns a short confirmation
    /// (fed back to the model) or an error string for invalid arguments.
    pub fn apply_update(&mut self, arguments: &str) -> Result<String, String> {
        let args: UpdatePlanArgs = serde_json::from_str(arguments)
            .map_err(|error| format!("invalid update_plan arguments: {error}"))?;
        if let Some(objective) = args.objective {
            self.objective = Some(clip(&objective));
        }
        if let Some(constraints) = args.constraints {
            self.constraints = constraints.iter().map(|c| clip(c)).take(LIST_CAP).collect();
        }
        if let Some(decisions) = args.add_decisions {
            for decision in decisions {
                let decision = clip(&decision);
                if !self.decisions.contains(&decision) {
                    self.decisions.push(decision);
                }
            }
            let excess = self.decisions.len().saturating_sub(LIST_CAP);
            self.decisions.drain(..excess);
        }
        if let Some(tasks) = args.tasks {
            self.tasks = tasks
                .into_iter()
                .map(|task| PlanTask {
                    text: clip(&task.text),
                    done: task.done,
                })
                .take(LIST_CAP)
                .collect();
        }
        if let Some(commands) = args.verification {
            self.verification_commands = commands
                .into_iter()
                .filter(|argv| !argv.is_empty())
                .take(LIST_CAP)
                .collect();
        }
        let open = self.tasks.iter().filter(|t| !t.done).count();
        Ok(format!(
            "plan updated: objective {}, {} task(s) ({open} open), {} decision(s)",
            if self.objective.is_some() {
                "set"
            } else {
                "unset"
            },
            self.tasks.len(),
            self.decisions.len(),
        ))
    }

    /// Deterministic tracking for an executed tool call.
    pub fn observe(&mut self, tool_name: &str, arguments: &str, success: bool) {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(value) => value,
            Err(_) => return,
        };
        let path = || {
            args.get("path")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        match tool_name {
            "read_file" => {
                if success {
                    self.files_read.extend(path());
                }
            }
            "edit" | "write_file" => {
                if success {
                    self.files_changed.extend(path());
                }
            }
            "run_command" => {
                let Some(argv) = args.get("command").and_then(|v| v.as_array()) else {
                    return;
                };
                let command = argv
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ");
                if command.is_empty() {
                    return;
                }
                self.commands.retain(|record| record.command != command);
                self.commands.push(CommandOutcome {
                    command,
                    ok: success,
                });
                let excess = self.commands.len().saturating_sub(COMMANDS_CAP);
                self.commands.drain(..excess);
            }
            _ => {}
        }
    }

    /// Remember a classified failure (bounded, most recent last).
    pub fn note_clarification(&mut self, questions: &str, answer: Option<&str>) {
        let mut entry = format!("Q:\n{}", questions.trim_end());
        match answer {
            Some(a) if !a.trim().is_empty() => entry.push_str(&format!("\nA: {}", a.trim())),
            _ => entry.push_str("\nA: (unanswered; proceeding on the stated assumptions)"),
        }
        self.clarifications.push(entry);
    }

    pub fn record_failure(&mut self, kind: FailureKind, summary: &str) {
        self.failures.push(RecordedFailure {
            kind,
            summary: clip(summary),
        });
        let excess = self.failures.len().saturating_sub(FAILURES_CAP);
        self.failures.drain(..excess);
    }

    pub fn last_failure(&self) -> Option<&RecordedFailure> {
        self.failures.last()
    }

    pub fn is_empty(&self) -> bool {
        *self == WorkingState::default()
    }

    /// Render for compaction checkpoints and the /plan command. Everything
    /// here is data the loop recorded — it cannot be hallucinated.
    pub fn render(&self) -> String {
        let mut out = String::from("## Working state (tracked)\n");
        out.push_str(&format!(
            "Objective: {}\n",
            self.objective.as_deref().unwrap_or("(not set)")
        ));
        if !self.constraints.is_empty() {
            out.push_str("Constraints:\n");
            for constraint in &self.constraints {
                out.push_str(&format!("- {constraint}\n"));
            }
        }
        if !self.decisions.is_empty() {
            out.push_str("Decisions:\n");
            for decision in &self.decisions {
                out.push_str(&format!("- {decision}\n"));
            }
        }
        if !self.tasks.is_empty() {
            out.push_str("Tasks:\n");
            for task in &self.tasks {
                out.push_str(&format!(
                    "- [{}] {}\n",
                    if task.done { "x" } else { " " },
                    task.text
                ));
            }
        }
        if !self.clarifications.is_empty() {
            out.push_str("Clarifications (user answers are authoritative):\n");
            for c in &self.clarifications {
                out.push_str(c);
                out.push('\n');
            }
        }
        out.push_str(&format!(
            "Files changed: {}\n",
            join_capped(self.files_changed.iter())
        ));
        out.push_str(&format!(
            "Files read: {}\n",
            join_capped(self.files_read.iter())
        ));
        if self.commands.is_empty() {
            out.push_str("Commands run: none\n");
        } else {
            out.push_str("Commands run:\n");
            for record in &self.commands {
                out.push_str(&format!(
                    "- `{}` → {}\n",
                    record.command,
                    if record.ok { "ok" } else { "failed" }
                ));
            }
        }
        if !self.failures.is_empty() {
            out.push_str("Known failures (most recent last):\n");
            for failure in &self.failures {
                out.push_str(&format!("- {}: {}\n", failure.kind, failure.summary));
            }
        }
        if let Some(verification) = &self.verification {
            out.push_str(&format!("Verification: {verification}\n"));
        }
        out
    }
}

fn join_capped<'a>(items: impl Iterator<Item = &'a String>) -> String {
    let mut rendered = String::new();
    let mut count = 0usize;
    for item in items {
        if count > 0 {
            rendered.push_str(", ");
        }
        if count >= FILES_SHOWN_CAP {
            rendered.push('…');
            break;
        }
        rendered.push_str(item);
        count += 1;
    }
    if count == 0 {
        "none".to_string()
    } else {
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_plan_replaces_and_appends_per_field() {
        let mut state = WorkingState::default();
        state
            .apply_update(
                r#"{"objective":"fix the limiter","tasks":[{"text":"find bug","done":true},{"text":"fix","done":false}]}"#,
            )
            .unwrap();
        state
            .apply_update(r#"{"add_decisions":["root cause in refill.ts"]}"#)
            .unwrap();
        state
            .apply_update(r#"{"add_decisions":["root cause in refill.ts"]}"#)
            .unwrap();
        assert_eq!(state.objective.as_deref(), Some("fix the limiter"));
        assert_eq!(state.tasks.len(), 2);
        assert_eq!(state.decisions.len(), 1, "decisions dedupe");
        assert!(state.apply_update(r#"{"bogus":1}"#).is_err());
        // Replacing tasks drops the old list.
        state.apply_update(r#"{"tasks":[]}"#).unwrap();
        assert!(state.tasks.is_empty());
    }

    #[test]
    fn observe_tracks_files_and_latest_command_outcomes() {
        let mut state = WorkingState::default();
        state.observe("read_file", r#"{"path":"a.rs"}"#, true);
        state.observe(
            "edit",
            r#"{"path":"a.rs","old_string":"x","new_string":"y"}"#,
            true,
        );
        state.observe("write_file", r#"{"path":"b.rs","content":""}"#, false);
        state.observe("run_command", r#"{"command":["bun","test"]}"#, false);
        state.observe("run_command", r#"{"command":["bun","test"]}"#, true);
        assert!(state.files_read.contains("a.rs"));
        assert!(state.files_changed.contains("a.rs"));
        assert!(
            !state.files_changed.contains("b.rs"),
            "failed writes don't count"
        );
        assert_eq!(state.commands.len(), 1, "latest outcome per command");
        assert!(state.commands[0].ok);
    }

    #[test]
    fn render_is_compact_and_capped() {
        let mut state = WorkingState::default();
        state
            .apply_update(&format!(
                r#"{{"objective":"{}"}}"#,
                "o".repeat(ENTRY_CAP_CHARS * 2)
            ))
            .unwrap();
        state.observe("run_command", r#"{"command":["cargo","test"]}"#, true);
        let rendered = state.render();
        assert!(rendered.contains("Objective: o"));
        assert!(rendered.contains("`cargo test` → ok"));
        assert!(rendered.len() < ENTRY_CAP_CHARS + 400);
    }

    #[test]
    fn failures_and_verification_are_tracked_and_rendered() {
        let mut state = WorkingState::default();
        for i in 0..(FAILURES_CAP + 3) {
            state.record_failure(FailureKind::TestFailure, &format!("boom {i}"));
        }
        assert_eq!(state.failures.len(), FAILURES_CAP);
        assert_eq!(state.last_failure().unwrap().summary, "boom 12");
        state
            .apply_update(r#"{"verification":[["cargo","test"],[]]}"#)
            .unwrap();
        assert_eq!(state.verification_commands, vec![vec!["cargo", "test"]]);
        state.verification = Some("1/1 checks passed → verified".into());
        let rendered = state.render();
        assert!(rendered.contains("Known failures"));
        assert!(rendered.contains("test_failure: boom 12"));
        assert!(rendered.contains("Verification: 1/1 checks passed"));
    }
}

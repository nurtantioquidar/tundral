//! System prompt assembly. Everything here lands in the cached prefix, so it
//! is frozen at session start: no timestamps, no git status, no dynamic state.

const INSTRUCTIONS_SNAPSHOT_CAP: usize = 12_000;

// The @-mention convention line is a sanctioned prefix change
// (operator-approved 2026-09-01, one-time re-cache per repo).
pub const CORE_INSTRUCTIONS: &str = "\
You are Butterfly, a fast coding agent working inside the user's repository.

Rules:
- Investigate before editing: read the relevant files and nearby context first.
- Prefer the smallest change that fully solves the problem.
- Use the `edit` tool for changes to existing files and `write_file` for new files.
- Commands run as argv vectors with no shell; there is no piping or expansion.
- Validate your work: run the most targeted test or check that covers the change.
- Never invent file contents or claim a check passed without tool output proving it.
- For multi-step work, call update_plan early to set the objective and task
  list, and keep it current: it is your durable memory across context
  compaction.
- The user may reference workspace files as @-mentions (e.g. @src/main.rs).
  Treat them as workspace-relative paths and read the relevant ones with
  read_file before answering.
- When the task is complete, stop calling tools and summarize what changed and
  what was verified.";

/// Appended only when the Atlas tools are enabled for the session (large
/// repos). Steering measured on the 2026-09-01 A/B: repo_map won orientation
/// (10 vs 15 turns) and sweeps (29k vs 44k tokens); ripgrep won exact-string
/// lookup — so route by task shape.
pub const ATLAS_INSTRUCTIONS: &str = "\n\
- This is a large repository. To orient yourself or sweep for a pattern
  across it, start with repo_map. Use symbol_search when you don't yet know
  where a symbol lives; when you already know the exact string, search_text
  (ripgrep) answers in one shot.";

/// Appended only in sandboxed sessions (disposable containers). Gated so the
/// default prefix bytes are untouched.
pub const SANDBOX_INSTRUCTIONS: &str = "\n\
- You are running inside a disposable sandbox you fully control. Use the `shell`
  tool for pipelines, redirects, globbing, package installation, and anything
  run_command cannot express; absolute paths are allowed. Verify results with
  real commands before finishing.";

/// Appended only when the completion gate is on (autonomous runs that must
/// earn their ending). Gated so the default prefix bytes are untouched.
pub const COMPLETION_INSTRUCTIONS: &str = "\n\
- Autonomous run: the task ends only when you call complete_task. List every
  deliverable the task statement asks for, each with the tool output that
  proves it (a file read back, a command result, a passing check). A reply
  without tool calls does not end the task. If the repository has no tests
  covering the task, write concrete acceptance checks for the stated
  requirements, run them, and pass them as complete_task.checks so the harness
  can run them too.";

/// Appended only when lean turns are requested (`--lean`). Measured on the
/// private suite 2026-09-12 (GLM, 24 trials): 2.0 re-reads of a just-edited
/// file per trial, a median 5 min of orientation before the first edit, and
/// model latency at 95% of wall clock, so every avoided round trip is the
/// whole speed lever. Gated so the default prefix bytes are untouched.
/// Extended 2026-09-16 (operator-approved prefix change for lean sessions):
/// 91 of 1,794 provider calls on the Flink suite carried only `update_plan`,
/// and 330 carried a single edit.
pub const LEAN_INSTRUCTIONS: &str = "\n\
- Lean turns: a successful edit or write_file result proves the change landed;
  do not read the file back afterwards (a failed edit returns an error). Before
  the first edit, read only the files the task names and what a search shows is
  directly involved; do not survey other services, docs, or history. Issue
  independent reads and searches together in one turn instead of one per turn.
  Run the relevant tests after your changes, not before every step. Send
  update_plan in the same reply as the tool call that starts the step, never as
  the only call in a reply. Edits that do not depend on each other's results go
  in one reply.";

/// Appended only in clarify mode (`--clarify`). Motivation, private suite
/// 2026-09-12: the same harness and model went from 0/5 to 5/5 when the task
/// statement named the interfaces the hidden tests expected; the loop cannot
/// invent those names, but it can ask. Gated so other prefixes keep their bytes.
pub const CLARIFY_INSTRUCTIONS: &str = "\n\
- Clarify before you commit: when the task statement leaves open a name or
  signature you must introduce, an exact message or format another party will
  depend on, or behaviour in a case it does not mention, call ask_user once,
  before your first edit, with every such question and the assumption you would
  otherwise make. Read the relevant code first so the questions are concrete;
  do not ask what the code or its tests already answer. An answer is
  authoritative. Without one, proceed on your assumptions and state them in
  the final summary.";

/// Appended only in the fresh-context session that reviews a worker's
/// completion claim (never in a worker session).
pub const VERIFIER_INSTRUCTIONS: &str = "\n\
- Independent verifier: you did not do the work in this workspace and you must
  not change it. Read the task statement, list every requirement it states,
  and test each one against the current workspace by running the program or
  its tests on real input. A requirement is met only when a command you ran
  proves it; a script the worker wrote proves nothing by itself. Look for what
  the worker most likely skipped: edge cases the statement names, negative
  requirements (what must NOT appear or happen), and inputs other than the
  ones the worker tried. When the statement says the result must work for
  whatever input, app, config, or data is present, or for cases beyond the
  sample, build a second, different input yourself (in a scratch copy, never
  in place) and run the solution on it: a solution that only handles the
  sample is unmet. Finish by calling report_verdict; nothing else ends the
  verification.";

/// Build the session system prompt. Byte-stable for the life of the session:
/// same inputs, same bytes.
///
/// NOTE: these bytes are the cached prefix shared across sessions of the same
/// repo. Any change to them costs a one-time prefix re-cache on the next new
/// session — improve deliberately, not incidentally. (The workspace root path
/// was considered and declined for this reason; the model can run the
/// allowlisted `pwd` instead.)
// One flag per gated block, in append order; a flags struct would read the
// same at every call site and hide the order the bytes depend on.
#[allow(clippy::too_many_arguments)]
pub fn build_system_prompt(
    workspace_name: &str,
    instructions_snapshot: Option<&str>,
    atlas_enabled: bool,
    sandboxed: bool,
    completion_gate: bool,
    lean: bool,
    clarify: bool,
    verifier: bool,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(CORE_INSTRUCTIONS);
    if atlas_enabled {
        prompt.push_str(ATLAS_INSTRUCTIONS);
    }
    if sandboxed {
        prompt.push_str(SANDBOX_INSTRUCTIONS);
    }
    if completion_gate {
        prompt.push_str(COMPLETION_INSTRUCTIONS);
    }
    if lean {
        prompt.push_str(LEAN_INSTRUCTIONS);
    }
    if clarify {
        prompt.push_str(CLARIFY_INSTRUCTIONS);
    }
    if verifier {
        prompt.push_str(VERIFIER_INSTRUCTIONS);
    }
    prompt.push_str("\n\nWorkspace: ");
    prompt.push_str(workspace_name);
    if let Some(snapshot) = instructions_snapshot {
        let mut snapshot = snapshot.trim().to_string();
        if snapshot.len() > INSTRUCTIONS_SNAPSHOT_CAP {
            let mut end = INSTRUCTIONS_SNAPSHOT_CAP;
            while !snapshot.is_char_boundary(end) {
                end -= 1;
            }
            snapshot.truncate(end);
            snapshot.push_str("\n[instructions truncated]");
        }
        if !snapshot.is_empty() {
            prompt.push_str("\n\nRepository instructions (snapshot at session start):\n");
            prompt.push_str(&snapshot);
        }
    }
    prompt
}

/// Locate repository instructions (AGENTS.md, falling back to CLAUDE.md) at
/// the workspace root. Read once at session start.
pub fn load_instructions_snapshot(root: &std::path::Path) -> Option<String> {
    for name in ["AGENTS.md", "CLAUDE.md"] {
        if let Ok(text) = std::fs::read_to_string(root.join(name)) {
            return Some(text);
        }
    }
    None
}

pub const SUMMARIZE_INSTRUCTION: &str = "\
SYSTEM: The conversation is being compacted to free context space.
Write a continuation summary of everything above that a fresh instance of
yourself needs to continue seamlessly. Do not call any tools. Cover: the
user's goal and constraints; the approach taken and why; decisions made;
files inspected or changed (exact paths); commands run and their outcomes;
what is verified working; what remains to be done; any dead ends to avoid.
Be precise with paths and identifiers. Respond with the summary only.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_is_byte_stable() {
        let a = build_system_prompt(
            "monet",
            Some("Do the thing."),
            false,
            false,
            false,
            false,
            false,
            false,
        );
        let b = build_system_prompt(
            "monet",
            Some("Do the thing."),
            false,
            false,
            false,
            false,
            false,
            false,
        );
        assert_eq!(a, b);
        assert!(a.contains("Workspace: monet"));
        assert!(a.contains("Do the thing."));
        assert!(!a.contains("repo_map"), "atlas line must be gated");
        let with_atlas =
            build_system_prompt("monet", None, true, false, false, false, false, false);
        assert!(with_atlas.contains("repo_map"));
        assert!(with_atlas.contains("search_text"));
        assert!(!a.contains("`shell`"), "sandbox line must be gated");
        let sandboxed = build_system_prompt("monet", None, false, true, false, false, false, false);
        assert!(sandboxed.contains("`shell`"));
        assert!(
            !a.contains("complete_task"),
            "completion line must be gated"
        );
        let gated = build_system_prompt("monet", None, false, false, true, false, false, false);
        assert!(gated.contains("complete_task"));
        assert!(!a.contains("report_verdict"), "verifier line must be gated");
        let verifier = build_system_prompt("monet", None, false, true, false, false, false, true);
        assert!(verifier.contains("report_verdict"));
        assert!(!verifier.contains("complete_task"));
        assert!(!a.contains("Lean turns"), "lean line must be gated");
        let lean = build_system_prompt("monet", None, false, false, true, true, false, false);
        assert!(lean.contains("Lean turns"));
        let gated_head = gated.split("\n\nWorkspace:").next().unwrap();
        assert!(
            lean.starts_with(gated_head),
            "lean appends after the completion line so gated sessions keep their prefix"
        );
        assert!(
            !a.contains("Clarify before you commit"),
            "clarify line must be gated"
        );
        let clarify = build_system_prompt("monet", None, false, false, true, false, true, false);
        assert!(clarify.contains("ask_user"));
        // Default bytes are exactly the historical prompt: the gated lines
        // only ever APPEND after CORE_INSTRUCTIONS.
        assert!(a.starts_with(CORE_INSTRUCTIONS));
    }

    #[test]
    fn oversized_instruction_snapshots_are_capped() {
        let big = "x".repeat(INSTRUCTIONS_SNAPSHOT_CAP * 2);
        let prompt = build_system_prompt("w", Some(&big), false, false, false, false, false, false);
        assert!(prompt.len() < INSTRUCTIONS_SNAPSHOT_CAP + CORE_INSTRUCTIONS.len() + 200);
        assert!(prompt.contains("[instructions truncated]"));
    }
}

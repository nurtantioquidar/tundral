//! System prompt assembly. Everything here lands in the cached prefix, so it
//! is frozen at session start: no timestamps, no git status, no dynamic state.

const INSTRUCTIONS_SNAPSHOT_CAP: usize = 12_000;

pub const CORE_INSTRUCTIONS: &str = "\
You are hx, a fast coding agent working inside the user's repository.

Rules:
- Investigate before editing: read the relevant files and nearby context first.
- Prefer the smallest change that fully solves the problem.
- Use the `edit` tool for changes to existing files and `write_file` for new files.
- Commands run as argv vectors with no shell; there is no piping or expansion.
- Validate your work: run the most targeted test or check that covers the change.
- Never invent file contents or claim a check passed without tool output proving it.
- When the task is complete, stop calling tools and summarize what changed and
  what was verified.";

/// Build the session system prompt. Byte-stable for the life of the session:
/// same inputs, same bytes.
///
/// NOTE: these bytes are the cached prefix shared across sessions of the same
/// repo. Any change to them costs a one-time prefix re-cache on the next new
/// session — improve deliberately, not incidentally. (The workspace root path
/// was considered and declined for this reason; the model can run the
/// allowlisted `pwd` instead.)
pub fn build_system_prompt(workspace_name: &str, instructions_snapshot: Option<&str>) -> String {
    let mut prompt = String::new();
    prompt.push_str(CORE_INSTRUCTIONS);
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
        let a = build_system_prompt("monet", Some("Do the thing."));
        let b = build_system_prompt("monet", Some("Do the thing."));
        assert_eq!(a, b);
        assert!(a.contains("Workspace: monet"));
        assert!(a.contains("Do the thing."));
    }

    #[test]
    fn oversized_instruction_snapshots_are_capped() {
        let big = "x".repeat(INSTRUCTIONS_SNAPSHOT_CAP * 2);
        let prompt = build_system_prompt("w", Some(&big));
        assert!(prompt.len() < INSTRUCTIONS_SNAPSHOT_CAP + CORE_INSTRUCTIONS.len() + 200);
        assert!(prompt.contains("[instructions truncated]"));
    }
}

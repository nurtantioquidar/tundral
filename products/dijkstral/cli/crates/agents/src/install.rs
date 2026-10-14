//! Installs and removes dijkstral's assistant integrations (`docs/graphify-query.md` §12).
//!
//! | Command | Writes |
//! |---|---|
//! | `claude install` | `<project>/CLAUDE.md` `## dijkstral` section; `<project>/.claude/settings.json` `PreToolUse` hooks (`Bash\|Grep` → `hook-guard search`, `Read\|Glob` → `hook-guard read`, timeout 10 s) |
//! | `codex install` | `<project>/AGENTS.md` `## dijkstral` section; `<project>/.codex/hooks.json` `PreToolUse` `Bash` → `hook-check` |
//! | `install --platform claude` | `~/.claude/skills/dijkstral/{SKILL.md,.dijkstral_version}`; `~/.claude/CLAUDE.md` `# dijkstral` section |
//! | `install --platform codex` | `~/.codex/skills/dijkstral/{SKILL.md,.dijkstral_version}` |
//! | `install --project` | the platform's skill (and `.claude/CLAUDE.md` pointer) under the project, plus its project install with a bare `dijkstral` command |
//!
//! Every write is idempotent (an unchanged file is not rewritten) and every uninstall removes
//! exactly what install added, keeping all other bytes of shared files. Settings files are checked
//! before anything is written, so an unreadable one leaves the project untouched.

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::jsonedit::{remove_entries, upsert_entries};
use crate::section;
use crate::text::{AGENTS_SECTION, CLAUDE_SECTION, PROJECT_TITLE, SKILL_MD, SKILL_POINTER_SECTION};

/// Assistants dijkstral integrates with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Claude,
    Codex,
}

/// What happened to one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Created,
    Updated,
    Unchanged,
    Removed,
    /// Nothing of dijkstral's was there.
    Absent,
}

/// One line of an install or uninstall report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub action: Action,
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.action {
            Action::Created => "created",
            Action::Updated => "updated",
            Action::Unchanged => "unchanged",
            Action::Removed => "removed",
            Action::Absent => "not found",
        };
        write!(f, "{what:<10}{}", self.path.display())
    }
}

const EVENT: &str = "PreToolUse";
const SKILL_NAME: &str = "dijkstral";
const SKILL_FILE: &str = "SKILL.md";
const VERSION_FILE: &str = ".dijkstral_version";
const CLAUDE_MATCHERS: [&str; 2] = ["Bash|Grep", "Read|Glob"];
const CODEX_MATCHER: &str = "Bash";

/// The executable as written into a hook command: as is when it needs no quoting, otherwise
/// single-quoted for the shell that runs hook commands.
pub fn hook_program(exe: &str) -> String {
    let safe = !exe.is_empty()
        && exe
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+:@%,=".contains(c));
    if safe {
        exe.to_string()
    } else {
        format!("'{}'", exe.replace('\'', "'\\''"))
    }
}

fn claude_entries(program: &str, strict: bool) -> Vec<Value> {
    let strict = if strict { " --strict" } else { "" };
    vec![
        json!({"matcher": CLAUDE_MATCHERS[0], "hooks": [
            {"type": "command", "command": format!("{program} hook-guard search"), "timeout": 10}
        ]}),
        json!({"matcher": CLAUDE_MATCHERS[1], "hooks": [
            {"type": "command", "command": format!("{program} hook-guard read{strict}"), "timeout": 10}
        ]}),
    ]
}

fn codex_entries(program: &str) -> Vec<Value> {
    vec![json!({"matcher": CODEX_MATCHER, "hooks": [
        {"type": "command", "command": format!("{program} hook-check")}
    ]})]
}

/// An entry is dijkstral's when it has one of our matchers and runs `dijkstral … <subcommand>`.
/// Unlike Graphify (§12.1), an unrelated user hook that merely mentions the tool is never ours.
fn entry_runs(entry: &Value, matchers: &[&str], subcommand: &str) -> bool {
    let matcher = entry.get("matcher").and_then(Value::as_str).unwrap_or("");
    matchers.contains(&matcher)
        && entry
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hooks| {
                hooks.iter().any(|h| {
                    h.get("command")
                        .and_then(Value::as_str)
                        .is_some_and(|c| runs_dijkstral(c, subcommand))
                })
            })
}

fn runs_dijkstral(command: &str, subcommand: &str) -> bool {
    let mut words = command.split_whitespace();
    while let Some(word) = words.next() {
        let base = word
            .trim_matches(['\'', '"'])
            .rsplit('/')
            .next()
            .unwrap_or("");
        if base == "dijkstral" || base == "dijkstral.exe" {
            return words.next() == Some(subcommand);
        }
    }
    false
}

fn is_claude_entry(entry: &Value) -> bool {
    entry_runs(entry, &CLAUDE_MATCHERS, "hook-guard")
}

fn is_codex_entry(entry: &Value) -> bool {
    entry_runs(entry, &[CODEX_MATCHER], "hook-check")
}

/// `dijkstral claude install`: the project's `CLAUDE.md` section and `PreToolUse` hooks.
pub fn claude_project_install(project: &Path, program: &str, strict: bool) -> Result<Vec<Change>> {
    let settings = project.join(".claude").join("settings.json");
    let entries = claude_entries(program, strict);
    let new_settings = edit_settings(&settings, |text| {
        upsert_entries(text, EVENT, &entries, &is_claude_entry)
    })?;
    Ok(vec![
        write_section(&project.join("CLAUDE.md"), 2, CLAUDE_SECTION)?,
        commit(&settings, new_settings)?,
    ])
}

/// `dijkstral claude uninstall`: removes the section, our hooks, and a project-level skill.
pub fn claude_project_uninstall(project: &Path) -> Result<Vec<Change>> {
    let settings = project.join(".claude").join("settings.json");
    let new_settings = edit_settings(&settings, |text| {
        remove_entries(text, EVENT, &is_claude_entry)
    })?;
    let mut changes = vec![
        remove_section(&project.join("CLAUDE.md"), 2)?,
        commit(&settings, new_settings)?,
    ];
    changes.extend(skill_uninstall(Platform::Claude, project)?);
    Ok(changes)
}

/// `dijkstral codex install`: the project's `AGENTS.md` section and `.codex/hooks.json` hook.
pub fn codex_project_install(project: &Path, program: &str) -> Result<Vec<Change>> {
    let hooks = project.join(".codex").join("hooks.json");
    let entries = codex_entries(program);
    let new_hooks = edit_settings(&hooks, |text| {
        upsert_entries(text, EVENT, &entries, &is_codex_entry)
    })?;
    Ok(vec![
        write_section(&project.join("AGENTS.md"), 2, AGENTS_SECTION)?,
        commit(&hooks, new_hooks)?,
    ])
}

/// `dijkstral codex uninstall`.
pub fn codex_project_uninstall(project: &Path) -> Result<Vec<Change>> {
    let hooks = project.join(".codex").join("hooks.json");
    let new_hooks = edit_settings(&hooks, |text| remove_entries(text, EVENT, &is_codex_entry))?;
    let mut changes = vec![
        remove_section(&project.join("AGENTS.md"), 2)?,
        commit(&hooks, new_hooks)?,
    ];
    changes.extend(skill_uninstall(Platform::Codex, project)?);
    Ok(changes)
}

fn skill_dir(platform: Platform, base: &Path) -> PathBuf {
    let home = match platform {
        Platform::Claude => ".claude",
        Platform::Codex => ".codex",
    };
    base.join(home).join("skills").join(SKILL_NAME)
}

/// The skill under `base` (the user's home, or a project for `--project`), and for Claude Code a
/// pointer section in `base/.claude/CLAUDE.md`.
pub fn skill_install(platform: Platform, base: &Path) -> Result<Vec<Change>> {
    let dir = skill_dir(platform, base);
    let mut changes = vec![
        write_if_changed(&dir.join(SKILL_FILE), SKILL_MD)?,
        write_if_changed(&dir.join(VERSION_FILE), env!("CARGO_PKG_VERSION"))?,
    ];
    if platform == Platform::Claude {
        let pointer = base.join(".claude").join("CLAUDE.md");
        changes.push(write_section(&pointer, 1, SKILL_POINTER_SECTION)?);
    }
    Ok(changes)
}

/// Removes what `skill_install` wrote; the skill directory and an emptied `skills/` go too, but
/// files dijkstral did not write are left in place.
pub fn skill_uninstall(platform: Platform, base: &Path) -> Result<Vec<Change>> {
    let dir = skill_dir(platform, base);
    let mut changes = Vec::new();
    for name in [SKILL_FILE, VERSION_FILE] {
        let path = dir.join(name);
        let action = match fs::remove_file(&path) {
            Ok(()) => Action::Removed,
            Err(e) if e.kind() == ErrorKind::NotFound => Action::Absent,
            Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
        };
        if name == SKILL_FILE || action == Action::Removed {
            changes.push(Change { path, action });
        }
    }
    let is_empty = |d: &Path| fs::read_dir(d).is_ok_and(|mut d| d.next().is_none());
    if is_empty(&dir) && fs::remove_dir(&dir).is_ok() {
        if let Some(skills) = dir.parent().filter(|s| is_empty(s)) {
            fs::remove_dir(skills).ok();
        }
    }
    if platform == Platform::Claude {
        changes.push(remove_section(&base.join(".claude").join("CLAUDE.md"), 1)?);
    }
    Ok(changes)
}

/// The current and edited content of a hook settings file, computed without writing anything.
/// An absent file is edited as empty text.
fn edit_settings(
    path: &Path,
    edit: impl Fn(&str) -> Result<String>,
) -> Result<(Option<String>, String)> {
    let old = read_optional(path)?;
    let new = edit(old.as_deref().unwrap_or("")).map_err(|e| {
        anyhow!(
            "refusing to modify {}: {e} (fix or move it and re-run)",
            path.display()
        )
    })?;
    Ok((old, new))
}

/// Writes an `edit_settings` result if it changed the file.
fn commit(path: &Path, (old, new): (Option<String>, String)) -> Result<Change> {
    let path = path.to_path_buf();
    let action = match old {
        None if new.is_empty() => Action::Absent,
        Some(old) if old == new => Action::Unchanged,
        Some(_) => {
            write_atomic(&path, &new)?;
            Action::Updated
        }
        None => {
            write_atomic(&path, &new)?;
            Action::Created
        }
    };
    Ok(Change { path, action })
}

fn write_section(path: &Path, level: usize, text: &str) -> Result<Change> {
    let old = read_optional(path)?;
    let action = match section::upsert(old.as_deref().unwrap_or(""), level, PROJECT_TITLE, text) {
        None => Action::Unchanged,
        Some(new) => {
            write_atomic(path, &new)?;
            if old.is_some() {
                Action::Updated
            } else {
                Action::Created
            }
        }
    };
    Ok(Change {
        path: path.to_path_buf(),
        action,
    })
}

/// Removes the section; a file left with only whitespace is deleted.
fn remove_section(path: &Path, level: usize) -> Result<Change> {
    let action =
        match read_optional(path)?.and_then(|old| section::remove(&old, level, PROJECT_TITLE)) {
            None => Action::Absent,
            Some(new) if new.trim().is_empty() => {
                fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
                Action::Removed
            }
            Some(new) => {
                write_atomic(path, &new)?;
                Action::Updated
            }
        };
    Ok(Change {
        path: path.to_path_buf(),
        action,
    })
}

fn write_if_changed(path: &Path, content: &str) -> Result<Change> {
    let old = read_optional(path)?;
    let action = match old {
        Some(ref o) if o == content => Action::Unchanged,
        Some(_) => Action::Updated,
        None => Action::Created,
    };
    if action != Action::Unchanged {
        write_atomic(path, content)?;
    }
    Ok(Change {
        path: path.to_path_buf(),
        action,
    })
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| anyhow!("{} is not UTF-8 text; leaving it alone", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Replaces `path` through a temporary file in the same directory, keeping the old file's
/// permissions; a symlink is written through, not replaced.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let target = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::canonicalize(path).with_context(|| format!("resolving {}", path.display()))?
        }
        _ => path.to_path_buf(),
    };
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", target.display()))?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.dijkstral-tmp"));
    fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    if let Ok(meta) = fs::metadata(&target) {
        fs::set_permissions(&tmp, meta.permissions()).ok();
    }
    fs::rename(&tmp, &target).with_context(|| format!("writing {}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = "/opt/bin/dijkstral";

    fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    out.push((
                        path.strip_prefix(root).unwrap().display().to_string() + "/",
                        vec![],
                    ));
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(root).unwrap().display().to_string();
                    out.push((rel, fs::read(&path).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    fn actions(changes: &[Change]) -> Vec<Action> {
        changes.iter().map(|c| c.action).collect()
    }

    const USER_CLAUDE_MD: &str = "# My project\n\nAlways run `make test`.\n\n## Style\n\n- tabs\n";
    const USER_SETTINGS: &str = "{\n  \"model\": \"opus\",\n  \"permissions\": {\n    \"allow\": [\"Bash(make:*)\"]\n  },\n  \"hooks\": {\n    \"PreToolUse\": [\n      {\n        \"matcher\": \"Bash\",\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"echo dijkstral is great\"\n          }\n        ]\n      }\n    ],\n    \"Stop\": [\n      {\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"notify\"\n          }\n        ]\n      }\n    ]\n  },\n  \"env\": {\"A\": \"1\"}\n}\n";

    #[test]
    fn claude_round_trip_preserves_user_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::create_dir_all(p.join(".claude")).unwrap();
        fs::write(p.join("CLAUDE.md"), USER_CLAUDE_MD).unwrap();
        fs::write(p.join(".claude/settings.json"), USER_SETTINGS).unwrap();
        let before = snapshot(p);

        let first = claude_project_install(p, PROGRAM, false).unwrap();
        assert_eq!(actions(&first), [Action::Updated, Action::Updated]);
        let md = fs::read_to_string(p.join("CLAUDE.md")).unwrap();
        assert_eq!(md, format!("{USER_CLAUDE_MD}\n{CLAUDE_SECTION}"));
        let settings = fs::read_to_string(p.join(".claude/settings.json")).unwrap();
        assert!(settings
            .starts_with(&USER_SETTINGS[..USER_SETTINGS.find("\n    ],\n    \"Stop\"").unwrap()]));
        let doc: Value = serde_json::from_str(&settings).unwrap();
        let pre = doc["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 3);
        assert_eq!(pre[0]["hooks"][0]["command"], "echo dijkstral is great");
        assert_eq!(pre[1]["matcher"], "Bash|Grep");
        assert_eq!(
            pre[1]["hooks"][0]["command"],
            "/opt/bin/dijkstral hook-guard search"
        );
        assert_eq!(pre[1]["hooks"][0]["timeout"], 10);
        assert_eq!(pre[2]["matcher"], "Read|Glob");
        assert_eq!(
            pre[2]["hooks"][0]["command"],
            "/opt/bin/dijkstral hook-guard read"
        );
        let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "permissions", "hooks", "env"]);
        let installed = snapshot(p);

        let second = claude_project_install(p, PROGRAM, false).unwrap();
        assert_eq!(actions(&second), [Action::Unchanged, Action::Unchanged]);
        assert_eq!(snapshot(p), installed);

        // Reinstalling from another path replaces our hooks instead of adding more.
        claude_project_install(p, "/elsewhere/dijkstral", true).unwrap();
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(p.join(".claude/settings.json")).unwrap())
                .unwrap();
        let pre = doc["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 3);
        assert_eq!(
            pre[2]["hooks"][0]["command"],
            "/elsewhere/dijkstral hook-guard read --strict"
        );

        let removed = claude_project_uninstall(p).unwrap();
        assert_eq!(&actions(&removed)[..2], [Action::Updated, Action::Updated]);
        assert_eq!(snapshot(p), before);
        let again = claude_project_uninstall(p).unwrap();
        assert_eq!(&actions(&again)[..2], [Action::Absent, Action::Unchanged]);
        assert_eq!(snapshot(p), before);
    }

    #[test]
    fn claude_fresh_project() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let changes = claude_project_install(p, PROGRAM, false).unwrap();
        assert_eq!(actions(&changes), [Action::Created, Action::Created]);
        let installed = snapshot(p);
        let again = claude_project_install(p, PROGRAM, false).unwrap();
        assert_eq!(actions(&again), [Action::Unchanged, Action::Unchanged]);
        assert_eq!(snapshot(p), installed);
        assert_eq!(
            fs::read_to_string(p.join("CLAUDE.md")).unwrap(),
            CLAUDE_SECTION
        );
        let settings = fs::read_to_string(p.join(".claude/settings.json")).unwrap();
        let doc: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(
            doc,
            json!({"hooks": {"PreToolUse": claude_entries(PROGRAM, false)}})
        );
        // Key order inside entries and hooks is matcher, hooks / type, command, timeout.
        assert!(settings.contains(
            "{\n        \"matcher\": \"Bash|Grep\",\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"/opt/bin/dijkstral hook-guard search\",\n            \"timeout\": 10\n"
        ));
        claude_project_uninstall(p).unwrap();
        assert!(!p.join("CLAUDE.md").exists());
        // The file install created is kept (it may have gained other settings since), emptied.
        assert_eq!(
            fs::read_to_string(p.join(".claude/settings.json")).unwrap(),
            "{}\n"
        );
    }

    #[test]
    fn invalid_settings_leave_the_project_untouched() {
        for bad in ["{bad", "[]", "{\"hooks\": 5}"] {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path();
            fs::create_dir_all(p.join(".claude")).unwrap();
            fs::write(p.join(".claude/settings.json"), bad).unwrap();
            fs::write(p.join("CLAUDE.md"), USER_CLAUDE_MD).unwrap();
            let before = snapshot(p);
            let err = claude_project_install(p, PROGRAM, false).unwrap_err();
            assert!(err.to_string().contains("refusing to modify"), "{err}");
            assert!(claude_project_uninstall(p).is_err());
            assert_eq!(snapshot(p), before);
        }
    }

    #[test]
    fn user_hooks_mentioning_dijkstral_survive() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let settings = "{\"hooks\": {\"PreToolUse\": [{\"matcher\": \"Bash|Grep\", \"hooks\": [{\"type\": \"command\", \"command\": \"dijkstral query x\"}]}, {\"matcher\": \"Read\", \"hooks\": [{\"type\": \"command\", \"command\": \"dijkstral hook-guard read\"}]}]}}";
        fs::create_dir_all(p.join(".claude")).unwrap();
        fs::write(p.join(".claude/settings.json"), settings).unwrap();
        claude_project_install(p, PROGRAM, false).unwrap();
        claude_project_uninstall(p).unwrap();
        assert_eq!(
            fs::read_to_string(p.join(".claude/settings.json")).unwrap(),
            settings
        );
    }

    #[test]
    fn codex_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let agents = "# Agents\n\nBe careful.\n";
        let hooks = "{\n  \"hooks\": {\n    \"PostToolUse\": []\n  }\n}\n";
        fs::create_dir_all(p.join(".codex")).unwrap();
        fs::write(p.join("AGENTS.md"), agents).unwrap();
        fs::write(p.join(".codex/hooks.json"), hooks).unwrap();
        let before = snapshot(p);

        codex_project_install(p, PROGRAM).unwrap();
        assert_eq!(
            fs::read_to_string(p.join("AGENTS.md")).unwrap(),
            format!("{agents}\n{AGENTS_SECTION}")
        );
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(p.join(".codex/hooks.json")).unwrap())
                .unwrap();
        assert_eq!(
            doc,
            json!({"hooks": {"PostToolUse": [], "PreToolUse": [
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "/opt/bin/dijkstral hook-check"}]}
            ]}})
        );
        let installed = snapshot(p);
        assert_eq!(
            actions(&codex_project_install(p, PROGRAM).unwrap()),
            [Action::Unchanged, Action::Unchanged]
        );
        assert_eq!(snapshot(p), installed);
        codex_project_uninstall(p).unwrap();
        assert_eq!(snapshot(p), before);
    }

    #[test]
    fn skills_global_and_project() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fs::create_dir_all(h.join(".claude")).unwrap();
        let user_md = "# Me\n\nPrefer short answers.\n";
        fs::write(h.join(".claude/CLAUDE.md"), user_md).unwrap();
        let before = snapshot(h);

        let c = skill_install(Platform::Claude, h).unwrap();
        assert_eq!(
            actions(&c),
            [Action::Created, Action::Created, Action::Updated]
        );
        let skill = fs::read_to_string(h.join(".claude/skills/dijkstral/SKILL.md")).unwrap();
        assert!(skill.starts_with("---\nname: dijkstral\ndescription: "));
        assert_eq!(
            fs::read_to_string(h.join(".claude/skills/dijkstral/.dijkstral_version")).unwrap(),
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(
            fs::read_to_string(h.join(".claude/CLAUDE.md")).unwrap(),
            format!("{user_md}\n{SKILL_POINTER_SECTION}")
        );
        let x = skill_install(Platform::Codex, h).unwrap();
        assert_eq!(actions(&x), [Action::Created, Action::Created]);
        assert!(h.join(".codex/skills/dijkstral/SKILL.md").is_file());
        let installed = snapshot(h);
        skill_install(Platform::Claude, h).unwrap();
        skill_install(Platform::Codex, h).unwrap();
        assert_eq!(snapshot(h), installed);

        skill_uninstall(Platform::Claude, h).unwrap();
        skill_uninstall(Platform::Codex, h).unwrap();
        // The skills directories go; `.codex/` itself was created by install and stays (empty).
        fs::remove_dir(h.join(".codex")).unwrap();
        assert_eq!(snapshot(h), before);
    }

    #[test]
    fn skill_uninstall_keeps_foreign_files() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        skill_install(Platform::Claude, h).unwrap();
        fs::write(h.join(".claude/skills/dijkstral/notes.md"), "mine").unwrap();
        fs::create_dir_all(h.join(".claude/skills/other")).unwrap();
        skill_uninstall(Platform::Claude, h).unwrap();
        assert!(h.join(".claude/skills/dijkstral/notes.md").is_file());
        assert!(!h.join(".claude/skills/dijkstral/SKILL.md").exists());
        assert!(h.join(".claude/skills/other").is_dir());
        // The pointer file held only our section, so it is gone.
        assert!(!h.join(".claude/CLAUDE.md").exists());
    }

    #[test]
    fn symlinked_instruction_file_is_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::write(p.join("AGENTS.md"), "# A\n").unwrap();
        std::os::unix::fs::symlink("AGENTS.md", p.join("CLAUDE.md")).unwrap();
        claude_project_install(p, PROGRAM, false).unwrap();
        assert!(fs::symlink_metadata(p.join("CLAUDE.md"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_to_string(p.join("AGENTS.md")).unwrap(),
            format!("# A\n\n{CLAUDE_SECTION}")
        );
        claude_project_uninstall(p).unwrap();
        assert_eq!(fs::read_to_string(p.join("AGENTS.md")).unwrap(), "# A\n");
    }

    #[test]
    fn program_quoting_and_ownership() {
        assert_eq!(
            hook_program("/usr/local/bin/dijkstral"),
            "/usr/local/bin/dijkstral"
        );
        assert_eq!(
            hook_program("/Users/a b/dijkstral"),
            "'/Users/a b/dijkstral'"
        );
        assert_eq!(hook_program("/it's/dijkstral"), "'/it'\\''s/dijkstral'");
        assert!(runs_dijkstral(
            "'/Users/a b/dijkstral' hook-guard search",
            "hook-guard"
        ));
        assert!(runs_dijkstral(
            "dijkstral hook-guard read --strict",
            "hook-guard"
        ));
        assert!(!runs_dijkstral("dijkstral query hook-guard", "hook-guard"));
        assert!(!runs_dijkstral("graphify hook-guard search", "hook-guard"));
        assert!(!runs_dijkstral("echo dijkstral", "hook-guard"));
    }
}

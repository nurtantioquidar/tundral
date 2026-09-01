//! @-mention and slash-command completion: an ignore-aware file/dir list,
//! session-recency boosts, Atlas symbols, all fuzzy-ranked with nucleo.
//! Completion only shapes the text the user submits — it never touches the
//! transcript or the cached prefix.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

/// How the accepted choice behaves in the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceKind {
    File,
    /// Inserts with a trailing `/` and keeps the dropdown open to drill down.
    Dir,
    Symbol,
    Command,
}

#[derive(Debug, Clone)]
pub struct Choice {
    /// Replacement for the whole trigger token (includes the `@` / `/`).
    pub insert: String,
    /// What the dropdown row shows.
    pub label: String,
    /// Dim annotation (symbol location, command help).
    pub hint: String,
    pub kind: ChoiceKind,
}

/// The active completion context at the cursor, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// `@quer|y` — `start` is the byte offset of the `@`.
    Mention { start: usize, query: String },
    /// `/comm|and` at the very start of the buffer.
    Command { start: usize, query: String },
}

impl Trigger {
    pub fn start(&self) -> usize {
        match self {
            Trigger::Mention { start, .. } | Trigger::Command { start, .. } => *start,
        }
    }
}

/// Find the trigger token containing the cursor: the run of non-whitespace
/// immediately before it, starting with `@` (anywhere) or `/` (first token).
pub fn active_trigger(text: &str, cursor: usize) -> Option<Trigger> {
    let before = &text[..cursor];
    let start = before
        .rfind(|c: char| c.is_whitespace())
        .map(|i| i + before[i..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(0);
    let token = &before[start..];
    if let Some(query) = token.strip_prefix('@') {
        // Emails and mid-word @s are not triggers.
        return Some(Trigger::Mention {
            start,
            query: query.to_string(),
        });
    }
    if start == 0 {
        if let Some(query) = token.strip_prefix('/') {
            return Some(Trigger::Command {
                start,
                query: query.to_string(),
            });
        }
    }
    None
}

const WALK_CAP: usize = 30_000;
const REBUILD_AFTER: Duration = Duration::from_secs(10);
const SYMBOL_CAP: usize = 8_000;
/// A recency hit outranks any pure fuzzy score difference.
const RECENCY_BONUS: u32 = 4_096;

pub struct CompletionIndex {
    root: PathBuf,
    files: Vec<String>,
    dirs: Vec<String>,
    /// Most recently touched first.
    recency: Vec<String>,
    /// (symbol name, "path:line kind")
    symbols: Vec<(String, String, String)>,
    commands: Vec<(&'static str, &'static str)>,
    built_at: Option<Instant>,
    matcher: Matcher,
}

impl CompletionIndex {
    pub fn new(root: PathBuf, commands: Vec<(&'static str, &'static str)>) -> Self {
        Self {
            root,
            files: Vec::new(),
            dirs: Vec::new(),
            recency: Vec::new(),
            symbols: Vec::new(),
            commands,
            built_at: None,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
        }
    }

    #[cfg(test)]
    pub fn with_entries(files: Vec<String>, dirs: Vec<String>) -> Self {
        let mut index = Self::new(PathBuf::from("."), Vec::new());
        index.files = files;
        index.dirs = dirs;
        index.built_at = Some(Instant::now());
        index
    }

    /// Refresh session-derived sources: files the agent touched (most recent
    /// preference is approximated by changed-then-read order) and Atlas
    /// symbol definitions.
    pub fn set_session_data(
        &mut self,
        recency: Vec<String>,
        symbols: Vec<(String, String, String)>,
    ) {
        self.recency = recency;
        self.symbols = symbols;
        self.symbols.truncate(SYMBOL_CAP);
    }

    fn ensure_fresh(&mut self) {
        if self.built_at.is_some_and(|at| at.elapsed() < REBUILD_AFTER) {
            return;
        }
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        let walker = ignore::WalkBuilder::new(&self.root)
            .git_ignore(true)
            .git_exclude(true)
            .require_git(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();
        for entry in walker.flatten() {
            let Ok(relative) = entry.path().strip_prefix(&self.root) else {
                continue;
            };
            if relative.as_os_str().is_empty() {
                continue;
            }
            let path = relative.to_string_lossy().replace('\\', "/");
            match entry.file_type() {
                Some(t) if t.is_dir() => dirs.push(format!("{path}/")),
                Some(t) if t.is_file() => files.push(path),
                _ => {}
            }
            if files.len() + dirs.len() >= WALK_CAP {
                break;
            }
        }
        files.sort();
        dirs.sort();
        self.files = files;
        self.dirs = dirs;
        self.built_at = Some(Instant::now());
    }

    pub fn suggest(&mut self, trigger: &Trigger, limit: usize) -> Vec<Choice> {
        match trigger {
            Trigger::Command { query, .. } => self.suggest_commands(query, limit),
            Trigger::Mention { query, .. } => self.suggest_mentions(query, limit),
        }
    }

    fn suggest_commands(&self, query: &str, limit: usize) -> Vec<Choice> {
        self.commands
            .iter()
            .filter(|(name, _)| name.starts_with(query))
            .take(limit)
            .map(|(name, hint)| Choice {
                insert: format!("/{name}"),
                label: format!("/{name}"),
                hint: hint.to_string(),
                kind: ChoiceKind::Command,
            })
            .collect()
    }

    fn recency_rank(&self, path: &str) -> Option<usize> {
        self.recency.iter().position(|p| p == path)
    }

    fn suggest_mentions(&mut self, query: &str, limit: usize) -> Vec<Choice> {
        self.ensure_fresh();
        if query.is_empty() {
            return self.default_mentions(limit);
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut scored: Vec<(u32, Choice)> = Vec::new();
        for path in &self.files {
            let haystack = Utf32String::from(path.as_str());
            if let Some(score) = pattern.score(haystack.slice(..), &mut self.matcher) {
                let bonus = match self.recency_rank(path) {
                    Some(rank) => RECENCY_BONUS + (64u32.saturating_sub(rank as u32)),
                    None => 0,
                };
                scored.push((
                    score + bonus,
                    Choice {
                        insert: format!("@{path}"),
                        label: path.clone(),
                        hint: String::new(),
                        kind: ChoiceKind::File,
                    },
                ));
            }
        }
        for dir in &self.dirs {
            let haystack = Utf32String::from(dir.as_str());
            if let Some(score) = pattern.score(haystack.slice(..), &mut self.matcher) {
                scored.push((
                    score,
                    Choice {
                        insert: format!("@{dir}"),
                        label: dir.clone(),
                        hint: String::new(),
                        kind: ChoiceKind::Dir,
                    },
                ));
            }
        }
        for (name, location, kind) in &self.symbols {
            let haystack = Utf32String::from(name.as_str());
            if let Some(score) = pattern.score(haystack.slice(..), &mut self.matcher) {
                let path = location.split(':').next().unwrap_or(location).to_string();
                scored.push((
                    // Symbols rank below an equally-good path match.
                    score.saturating_sub(1),
                    Choice {
                        insert: format!("@{path}"),
                        label: name.clone(),
                        hint: format!("{kind} · {location}"),
                        kind: ChoiceKind::Symbol,
                    },
                ));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
        scored.truncate(limit);
        scored.into_iter().map(|(_, choice)| choice).collect()
    }

    /// Bare `@`: recent session files first, then shallow workspace paths.
    fn default_mentions(&self, limit: usize) -> Vec<Choice> {
        let mut choices: Vec<Choice> = self
            .recency
            .iter()
            .take(limit)
            .map(|path| Choice {
                insert: format!("@{path}"),
                label: path.clone(),
                hint: "recent".to_string(),
                kind: ChoiceKind::File,
            })
            .collect();
        let mut shallow: Vec<&String> = self
            .files
            .iter()
            .filter(|path| !self.recency.contains(path))
            .collect();
        shallow.sort_by_key(|path| (path.matches('/').count(), path.len()));
        for path in shallow {
            if choices.len() >= limit {
                break;
            }
            choices.push(Choice {
                insert: format!("@{path}"),
                label: path.clone(),
                hint: String::new(),
                kind: ChoiceKind::File,
            });
        }
        choices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_detection() {
        assert_eq!(
            active_trigger("look at @src/ma", 15),
            Some(Trigger::Mention {
                start: 8,
                query: "src/ma".into()
            })
        );
        // Cursor inside an earlier word: no trigger.
        assert_eq!(active_trigger("plain words", 5), None);
        // Email-like text is not a mention (@ not at token start).
        assert_eq!(active_trigger("mail me a@b.co", 14), None);
        // Slash command only as the first token.
        assert_eq!(
            active_trigger("/comp", 5),
            Some(Trigger::Command {
                start: 0,
                query: "comp".into()
            })
        );
        assert_eq!(active_trigger("do /comp", 8), None);
        // Bare @ opens the default list.
        assert_eq!(
            active_trigger("@", 1),
            Some(Trigger::Mention {
                start: 0,
                query: String::new()
            })
        );
    }

    #[test]
    fn fuzzy_ranks_files_and_marks_dirs() {
        let mut index = CompletionIndex::with_entries(
            vec![
                "src/session/mod.rs".into(),
                "src/main.rs".into(),
                "docs/notes.md".into(),
            ],
            vec!["src/session/".into()],
        );
        let trigger = Trigger::Mention {
            start: 0,
            query: "sesmod".into(),
        };
        let choices = index.suggest(&trigger, 8);
        assert_eq!(choices[0].insert, "@src/session/mod.rs");
        let trigger = Trigger::Mention {
            start: 0,
            query: "session".into(),
        };
        let choices = index.suggest(&trigger, 8);
        assert!(choices
            .iter()
            .any(|c| c.kind == ChoiceKind::Dir && c.insert.ends_with('/')));
    }

    #[test]
    fn recency_outranks_better_fuzzy_scores() {
        let mut index = CompletionIndex::with_entries(
            vec!["src/parse.rs".into(), "tests/parse_helpers.rs".into()],
            vec![],
        );
        index.set_session_data(vec!["tests/parse_helpers.rs".into()], vec![]);
        let trigger = Trigger::Mention {
            start: 0,
            query: "parse".into(),
        };
        let choices = index.suggest(&trigger, 8);
        assert_eq!(choices[0].label, "tests/parse_helpers.rs");
    }

    #[test]
    fn symbols_insert_their_defining_path() {
        let mut index = CompletionIndex::with_entries(vec!["src/limiter.rs".into()], vec![]);
        index.set_session_data(
            vec![],
            vec![(
                "RateLimiter".into(),
                "src/limiter.rs:14".into(),
                "struct".into(),
            )],
        );
        let trigger = Trigger::Mention {
            start: 0,
            query: "RateLim".into(),
        };
        let choices = index.suggest(&trigger, 8);
        assert_eq!(choices[0].kind, ChoiceKind::Symbol);
        assert_eq!(choices[0].insert, "@src/limiter.rs");
        assert!(choices[0].hint.contains("struct"));
    }

    #[test]
    fn bare_at_lists_recent_then_shallow() {
        let mut index = CompletionIndex::with_entries(
            vec!["deep/nested/file.rs".into(), "top.rs".into()],
            vec![],
        );
        index.set_session_data(vec!["deep/nested/file.rs".into()], vec![]);
        let trigger = Trigger::Mention {
            start: 0,
            query: String::new(),
        };
        let choices = index.suggest(&trigger, 8);
        assert_eq!(choices[0].label, "deep/nested/file.rs");
        assert_eq!(choices[0].hint, "recent");
        assert_eq!(choices[1].label, "top.rs");
    }

    #[test]
    fn command_completion_prefix_matches() {
        let mut index = CompletionIndex::new(
            PathBuf::from("."),
            vec![("compact", "force a compaction"), ("usage", "token stats")],
        );
        let trigger = Trigger::Command {
            start: 0,
            query: "co".into(),
        };
        let choices = index.suggest(&trigger, 8);
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].insert, "/compact");
    }
}

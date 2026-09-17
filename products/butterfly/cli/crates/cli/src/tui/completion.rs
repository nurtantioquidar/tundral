//! @-mention and slash-command completion: an ignore-aware file/dir list,
//! session-recency boosts, Atlas symbols, all fuzzy-ranked with nucleo.
//! Completion only shapes the text the user submits ‒ it never touches the
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
    /// `@quer|y` ‒ `start` is the byte offset of the `@`.
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
/// Score docked per path segment so a shallow directory outranks a deep one
/// that matches as well.
const DIR_DEPTH_PENALTY: u32 = 6;

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

    /// Ranked choices plus how many candidates matched in total, so the
    /// dropdown can say what it is not showing instead of appearing to be
    /// the whole answer.
    pub fn suggest(&mut self, trigger: &Trigger, limit: usize) -> (Vec<Choice>, usize) {
        match trigger {
            Trigger::Command { query, .. } => self.suggest_commands(query, limit),
            Trigger::Mention { query, .. } => self.suggest_mentions(query, limit),
        }
    }

    fn suggest_commands(&self, query: &str, limit: usize) -> (Vec<Choice>, usize) {
        let matched = self
            .commands
            .iter()
            .filter(|(name, _)| name.starts_with(query));
        let total = matched.clone().count();
        (
            matched
                .take(limit)
                .map(|(name, hint)| Choice {
                    insert: format!("/{name}"),
                    label: format!("/{name}"),
                    hint: hint.to_string(),
                    kind: ChoiceKind::Command,
                })
                .collect(),
            total,
        )
    }

    /// Split a workspace path into the part worth reading first (the file
    /// name) and the part that only disambiguates (its directory). Six
    /// candidates under one deep directory are otherwise six near-identical
    /// strings that all have to be read to the end.
    fn path_choice(path: &str, kind: ChoiceKind, hint: &str) -> Choice {
        let trimmed = path.strip_suffix('/').unwrap_or(path);
        let (parent, name) = match trimmed.rfind('/') {
            Some(at) => (&trimmed[..at], &trimmed[at + 1..]),
            None => ("", trimmed),
        };
        let label = match kind {
            ChoiceKind::Dir => format!("{name}/"),
            _ => name.to_string(),
        };
        let hint = if hint.is_empty() {
            parent.to_string()
        } else {
            hint.to_string()
        };
        Choice {
            insert: format!("@{path}"),
            label,
            hint,
            kind,
        }
    }

    fn recency_rank(&self, path: &str) -> Option<usize> {
        self.recency.iter().position(|p| p == path)
    }

    fn suggest_mentions(&mut self, query: &str, limit: usize) -> (Vec<Choice>, usize) {
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
                scored.push((score + bonus, Self::path_choice(path, ChoiceKind::File, "")));
            }
        }
        for dir in &self.dirs {
            let haystack = Utf32String::from(dir.as_str());
            if let Some(score) = pattern.score(haystack.slice(..), &mut self.matcher) {
                // Prefer the shallower directory when two match comparably:
                // you navigate downward, so `bench/` is a better answer than
                // `bench/private/__pycache__/` for the same prefix.
                let depth = dir.trim_end_matches('/').matches('/').count() as u32;
                scored.push((
                    score.saturating_sub(depth * DIR_DEPTH_PENALTY),
                    Self::path_choice(dir, ChoiceKind::Dir, ""),
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
        let total = scored.len();
        (Self::reserve_dir_slots(scored, limit), total)
    }

    /// Keep directories in the list instead of letting files crowd them out.
    ///
    /// Any real directory has more files under it than siblings beside it, and
    /// files also carry the recency bonus, so a purely score-ordered list is
    /// all files: typing `@products/butterfly` offered five files and not one
    /// of the directories under it, which made drilling down impossible. Up to
    /// half the list is therefore reserved for directories — they are the
    /// navigational choice, so they also come first — and any slot they do not
    /// use goes back to files.
    fn reserve_dir_slots(scored: Vec<(u32, Choice)>, limit: usize) -> Vec<Choice> {
        let (dirs, rest): (Vec<_>, Vec<_>) = scored
            .into_iter()
            .partition(|(_, choice)| choice.kind == ChoiceKind::Dir);
        let dir_slots = (limit / 2).min(dirs.len());
        let mut out: Vec<Choice> = dirs
            .into_iter()
            .take(dir_slots)
            .map(|(_, choice)| choice)
            .collect();
        out.extend(
            rest.into_iter()
                .take(limit.saturating_sub(out.len()))
                .map(|(_, choice)| choice),
        );
        out
    }

    /// Bare `@`: recent session files first, then shallow workspace paths.
    fn default_mentions(&self, limit: usize) -> (Vec<Choice>, usize) {
        let mut choices: Vec<Choice> = self
            .recency
            .iter()
            .take(limit)
            .map(|path| Self::path_choice(path, ChoiceKind::File, "touched this session"))
            .collect();
        // Shallow directories come before shallow files: a bare `@` is the
        // start of navigating somewhere, and offering only files made the
        // top level of the workspace unreachable.
        let mut shallow_dirs: Vec<&String> = self.dirs.iter().collect();
        shallow_dirs.sort_by_key(|path| (path.matches('/').count(), path.len()));
        for dir in shallow_dirs.into_iter().take(limit / 2) {
            if choices.len() >= limit {
                break;
            }
            choices.push(Self::path_choice(dir, ChoiceKind::Dir, ""));
        }
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
            choices.push(Self::path_choice(path, ChoiceKind::File, ""));
        }
        let total = self.recency.len() + self.files.len() + self.dirs.len();
        (choices, total)
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
        let (choices, _) = index.suggest(&trigger, 8);
        assert_eq!(choices[0].insert, "@src/session/mod.rs");
        let trigger = Trigger::Mention {
            start: 0,
            query: "session".into(),
        };
        let (choices, _) = index.suggest(&trigger, 8);
        assert!(choices.iter().any(|c| c.kind == ChoiceKind::Dir
            && c.insert.ends_with('/')
            && c.label == "session/"));
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
        let (choices, _) = index.suggest(&trigger, 8);
        // Labels are file names now; the path lives in the hint column.
        assert_eq!(choices[0].label, "parse_helpers.rs");
        assert_eq!(choices[0].insert, "@tests/parse_helpers.rs");
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
        let (choices, _) = index.suggest(&trigger, 8);
        assert_eq!(choices[0].kind, ChoiceKind::Symbol);
        assert_eq!(choices[0].insert, "@src/limiter.rs");
        assert!(choices[0].hint.contains("struct"));
    }

    /// Files outnumber their own directory and carry the recency bonus, so a
    /// purely score-ordered list is all files — typing a directory prefix
    /// offered five files under it and not one of the directories, which made
    /// drilling down impossible.
    #[test]
    fn directories_keep_their_slots_against_a_crowd_of_files() {
        let files: Vec<String> = (0..40)
            .map(|i| format!("products/butterfly/cli/file{i}.rs"))
            .collect();
        let dirs = vec![
            "products/butterfly/cli".to_string(),
            "products/butterfly/bench".to_string(),
        ];
        let mut index = CompletionIndex::with_entries(files, dirs);
        let trigger = Trigger::Mention {
            start: 0,
            query: "products/butterfly".into(),
        };
        let (choices, _) = index.suggest(&trigger, 6);
        let dirs_offered: Vec<&Choice> = choices
            .iter()
            .filter(|c| c.kind == ChoiceKind::Dir)
            .collect();
        assert_eq!(
            dirs_offered.len(),
            2,
            "both directories must survive: {choices:#?}"
        );
        // Navigational choices come first, and a directory reads as one.
        assert_eq!(choices[0].kind, ChoiceKind::Dir);
        assert!(choices[0].label.ends_with('/'), "{:?}", choices[0].label);
        // The rest of the list is still files.
        assert_eq!(choices.len(), 6);
        assert!(choices[2..].iter().all(|c| c.kind == ChoiceKind::File));
    }

    /// A bare `@` must offer somewhere to go, not only files.
    #[test]
    fn bare_at_offers_directories_too() {
        let mut index = CompletionIndex::with_entries(
            vec!["top.rs".into(), "deep/nested/file.rs".into()],
            vec!["deep".into(), "deep/nested".into()],
        );
        let trigger = Trigger::Mention {
            start: 0,
            query: String::new(),
        };
        let (choices, _) = index.suggest(&trigger, 6);
        assert!(
            choices.iter().any(|c| c.kind == ChoiceKind::Dir),
            "a bare @ offered no directories: {choices:#?}"
        );
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
        let (choices, _) = index.suggest(&trigger, 8);
        assert_eq!(choices[0].label, "file.rs");
        assert_eq!(choices[0].insert, "@deep/nested/file.rs");
        assert_eq!(choices[0].hint, "touched this session");
        assert_eq!(choices[1].label, "top.rs");
        // A file at the root has no directory to disambiguate it.
        assert_eq!(choices[1].hint, "");
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
        let (choices, total) = index.suggest(&trigger, 8);
        assert_eq!(choices.len(), 1);
        assert_eq!(total, 1);
        assert_eq!(choices[0].insert, "/compact");
    }
}

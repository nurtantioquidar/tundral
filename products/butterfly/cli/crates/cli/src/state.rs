//! Small persisted UI state (~/.local/state/butterfly/state.toml): the
//! last-used model and the effort chosen for each model — new sessions start
//! where the user left off, regardless of which repo they are in. Explicit
//! pins (a --model flag or a `model =` line in config files) still win over
//! the remembered value.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    /// Wire name of the last model the user ran with.
    model: Option<String>,
    /// Effort last chosen, keyed by model id. Per model on purpose: the
    /// vocabularies are not comparable, so `max` on DeepSeek (top of two)
    /// must not follow the user onto a model where it means something else,
    /// and a model with no entry starts at its own default.
    #[serde(default)]
    efforts: BTreeMap<String, String>,
}

fn state_path() -> Option<PathBuf> {
    butterfly_core::session::store::SessionStore::state_root()
        .parent()
        .map(|dir| dir.join("state.toml"))
}

fn load_at(path: &Path) -> StateFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// The last-used model's wire name, if recorded.
pub fn last_model() -> Option<String> {
    state_path()
        .map(|path| load_at(&path))
        .and_then(|s| s.model)
}

/// The effort last chosen for `model_id`, if recorded.
pub fn effort_for(model_id: &str) -> Option<String> {
    state_path()
        .map(|path| load_at(&path))
        .and_then(|s| s.efforts.get(model_id).cloned())
}

/// Rewrite the state file with `edit` applied. Best-effort throughout:
/// losing stickiness must never break a session.
fn update_at(path: &Path, edit: impl FnOnce(&mut StateFile)) {
    let mut state = load_at(path);
    edit(&mut state);
    let Ok(text) = toml::to_string_pretty(&state) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, text);
}

fn save_model_at(path: &Path, wire_name: &str) {
    update_at(path, |state| state.model = Some(wire_name.to_string()));
}

/// `None` forgets the entry, so that model goes back to starting at its own
/// default rather than at a value the user has since abandoned.
fn save_effort_at(path: &Path, model_id: &str, effort: Option<&str>) {
    update_at(path, |state| match effort {
        Some(value) => {
            state
                .efforts
                .insert(model_id.to_string(), value.to_string());
        }
        None => {
            state.efforts.remove(model_id);
        }
    });
}

/// Remember `wire_name` as the model to start future sessions with.
pub fn remember_model(wire_name: &str) {
    if let Some(path) = state_path() {
        save_model_at(&path, wire_name);
    }
}

/// Remember the effort to start future sessions on `model_id` with.
pub fn remember_effort(model_id: &str, effort: Option<&str>) {
    if let Some(path) = state_path() {
        save_effort_at(&path, model_id, effort);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_and_replaces_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        assert_eq!(load_at(&path).model, None);
        save_model_at(&path, "deepseek-v4-pro");
        assert_eq!(load_at(&path).model.as_deref(), Some("deepseek-v4-pro"));
        save_model_at(&path, "deepseek-v4-flash");
        assert_eq!(load_at(&path).model.as_deref(), Some("deepseek-v4-flash"));
    }

    /// Effort is remembered per model and the two keys do not disturb each
    /// other: switching models and back must restore the effort that model
    /// was left on, not the one in force elsewhere.
    #[test]
    fn remembers_effort_per_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        save_model_at(&path, "deepseek-v4-flash");
        save_effort_at(&path, "deepseek-v4-flash", Some("max"));
        save_effort_at(&path, "gpt-5.6-luna", Some("low"));
        let state = load_at(&path);
        assert_eq!(state.model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(state.efforts.get("deepseek-v4-flash").unwrap(), "max");
        assert_eq!(state.efforts.get("gpt-5.6-luna").unwrap(), "low");

        // Back to the provider's own default: the entry goes away rather
        // than persisting a value the user has just cleared.
        save_effort_at(&path, "gpt-5.6-luna", None);
        let state = load_at(&path);
        assert_eq!(state.efforts.get("gpt-5.6-luna"), None);
        assert_eq!(state.efforts.get("deepseek-v4-flash").unwrap(), "max");
    }

    #[test]
    fn corrupt_state_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, "]]junk").unwrap();
        assert_eq!(load_at(&path).model, None);
        assert!(load_at(&path).efforts.is_empty());
        save_model_at(&path, "deepseek-v4-flash");
        assert_eq!(load_at(&path).model.as_deref(), Some("deepseek-v4-flash"));
    }
}

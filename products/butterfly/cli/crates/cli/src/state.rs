//! Small persisted UI state (~/.local/state/butterfly/state.toml): today,
//! the last-used model — new sessions start where the user left off,
//! regardless of which repo they are in. Explicit pins (a --model flag or a
//! `model =` line in config files) still win over the remembered value.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    /// Wire name of the last model the user ran with.
    model: Option<String>,
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

fn save_model_at(path: &Path, wire_name: &str) {
    let mut state = load_at(path);
    state.model = Some(wire_name.to_string());
    let Ok(text) = toml::to_string_pretty(&state) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Best-effort: losing model stickiness must never break a session.
    let _ = std::fs::write(path, text);
}

/// Remember `wire_name` as the model to start future sessions with.
pub fn remember_model(wire_name: &str) {
    if let Some(path) = state_path() {
        save_model_at(&path, wire_name);
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

    #[test]
    fn corrupt_state_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, "]]junk").unwrap();
        assert_eq!(load_at(&path).model, None);
        save_model_at(&path, "deepseek-v4-flash");
        assert_eq!(load_at(&path).model.as_deref(), Some("deepseek-v4-flash"));
    }
}

//! The credential store: ~/.config/butterfly/credentials.toml, 0600 in a
//! 0700 dir. The ONE sanctioned place a provider key may persist (AGENTS.md
//! invariant 6); the environment variable always wins when set. Keys never
//! enter session logs, transcripts, or telemetry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct CredentialsFile {
    #[serde(default)]
    providers: BTreeMap<String, Provider>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Provider {
    api_key: String,
}

/// ~/.config/butterfly/credentials.toml
pub fn credentials_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("butterfly/credentials.toml"))
}

fn load_file(path: &Path) -> Result<CredentialsFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map_err(|error| anyhow::anyhow!("invalid credentials {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(CredentialsFile::default())
        }
        Err(error) => Err(anyhow::anyhow!("cannot read {}: {error}", path.display())),
    }
}

fn load_key_at(path: &Path, provider: &str) -> Option<String> {
    load_file(path)
        .ok()?
        .providers
        .remove(provider)
        .map(|p| p.api_key)
}

/// The saved key for `provider`, if any. Unreadable/invalid files read as
/// absent — the caller falls back to the connect flow.
pub fn load_key(provider: &str) -> Option<String> {
    load_key_at(&credentials_path()?, provider)
}

fn save_key_at(path: &Path, provider: &str, api_key: &str) -> Result<()> {
    let mut file = load_file(path).unwrap_or_default();
    file.providers.insert(
        provider.to_string(),
        Provider {
            api_key: api_key.to_string(),
        },
    );
    let text = toml::to_string_pretty(&file).context("serialize credentials")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    // Write via a same-directory temp file + rename so a crash never leaves
    // a half-written store, then pin 0600.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &text).with_context(|| format!("cannot write {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .context("cannot restrict credential file permissions")?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("cannot move into {}", path.display()))?;
    Ok(())
}

/// Save (or replace) `provider`'s key in the credential store.
pub fn save_key(provider: &str, api_key: &str) -> Result<PathBuf> {
    let path = credentials_path().context("no config directory on this system")?;
    save_key_at(&path, provider, api_key)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_replaces_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        assert_eq!(load_key_at(&path, "deepseek"), None);
        save_key_at(&path, "deepseek", "sk-first").unwrap();
        assert_eq!(load_key_at(&path, "deepseek"), Some("sk-first".into()));
        save_key_at(&path, "deepseek", "sk-second").unwrap();
        assert_eq!(load_key_at(&path, "deepseek"), Some("sk-second".into()));
        assert_eq!(load_key_at(&path, "other"), None);
    }

    #[cfg(unix)]
    #[test]
    fn credential_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        save_key_at(&path, "deepseek", "sk-secret").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "credentials must be 0600");
    }

    #[test]
    fn corrupt_store_reads_as_absent_but_never_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        std::fs::write(&path, "not = [valid").unwrap();
        assert_eq!(load_key_at(&path, "deepseek"), None);
    }
}

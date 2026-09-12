use std::{path::Path, process::Stdio};
use tokio::process::Command;
use zeroize::Zeroizing;

pub struct SecretValue(Zeroizing<String>);

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl SecretValue {
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }

    #[cfg(test)]
    pub(crate) fn synthetic(value: &str) -> Self {
        Self(Zeroizing::new(value.to_owned()))
    }
}

pub async fn read_one_password(reference: &str) -> Result<SecretValue, String> {
    read_one_password_with(Path::new("op"), reference).await
}

async fn read_one_password_with(executable: &Path, reference: &str) -> Result<SecretValue, String> {
    if !reference.starts_with("op://") {
        return Err("refusing to resolve a non-1Password secret reference".to_owned());
    }

    let output = Command::new(executable)
        .arg("read")
        .arg("--no-newline")
        .arg(reference)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("could not execute 1Password CLI: {error}"))?;

    if !output.status.success() {
        return Err("1Password could not resolve the configured secret reference".to_owned());
    }

    let value = String::from_utf8(output.stdout)
        .map_err(|_| "1Password returned a non-UTF-8 secret".to_owned())?;
    if value.is_empty() {
        return Err("1Password returned an empty secret".to_owned());
    }
    Ok(SecretValue(Zeroizing::new(value)))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn script(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fake-op");
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        (directory, path)
    }

    #[tokio::test]
    async fn captures_a_secret_without_logging_it() {
        let (_directory, executable) = script("#!/bin/sh\nprintf 'synthetic-secret'");
        let secret = read_one_password_with(&executable, "op://Agents/DeepSeek/key")
            .await
            .unwrap();
        assert_eq!(secret.expose(), "synthetic-secret");
        assert!(!format!("{secret:?}").contains("synthetic-secret"));
    }

    #[tokio::test]
    async fn suppresses_provider_error_output() {
        let (_directory, executable) =
            script("#!/bin/sh\nprintf 'sensitive diagnostic' >&2\nexit 1");
        let error = read_one_password_with(&executable, "op://Agents/DeepSeek/key")
            .await
            .unwrap_err();
        assert!(!error.contains("sensitive diagnostic"));
    }

    #[tokio::test]
    async fn refuses_non_reference_inputs_before_execution() {
        let (_directory, executable) = script("#!/bin/sh\nexit 0");
        let error = read_one_password_with(&executable, "literal-secret")
            .await
            .unwrap_err();
        assert!(error.contains("non-1Password"));
    }
}

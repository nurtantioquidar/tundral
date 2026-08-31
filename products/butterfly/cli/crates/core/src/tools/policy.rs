//! Security policy for tool execution, ported rule-for-rule from the original
//! TypeScript harness executor:
//! - paths are confined to the workspace root: no absolute paths, no `..`,
//!   no `.git` internals, no known secret-bearing files, and a realpath
//!   re-check after resolution defeats symlink escapes;
//! - commands run as an argv vector (never a shell) from an allowlist, with
//!   inline-eval flags and package-manager network verbs rejected and
//!   credential-like environment variables stripped.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("path is not allowed: {0}")]
    Path(String),
    #[error("command is not allowed: {0}")]
    Command(String),
}

/// Validate a model-supplied relative path without touching the filesystem.
pub fn validate_relative_path(path: &str) -> Result<PathBuf, PolicyError> {
    if path.is_empty() {
        return Err(PolicyError::Path("empty path".into()));
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err(PolicyError::Path(format!("absolute path: {path}")));
    }
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            _ => {
                return Err(PolicyError::Path(format!(
                    "path escapes the workspace root: {path}"
                )))
            }
        }
    }
    let first = normalized
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or_default();
    if first == ".git" {
        return Err(PolicyError::Path(format!(".git internals: {path}")));
    }
    for part in normalized.components() {
        let name = part.as_os_str().to_string_lossy();
        if is_secret_file_name(&name) {
            return Err(PolicyError::Path(format!(
                "potential secret-bearing file: {path}"
            )));
        }
    }
    Ok(normalized)
}

fn is_secret_file_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    if lowered.ends_with(".example") {
        return false;
    }
    if lowered.starts_with(".env") {
        return true;
    }
    if lowered == "auth.json" || lowered.starts_with("credentials") {
        return true;
    }
    if lowered.contains("id_rsa") || lowered.contains("id_ecdsa") || lowered.contains("id_ed25519")
    {
        return true;
    }
    matches!(
        lowered.rsplit_once('.').map(|(_, ext)| ext),
        Some("pem" | "key" | "p12")
    )
}

/// Resolve a validated relative path against the root and defend against
/// symlink escapes: the closest existing ancestor must canonicalize inside the
/// canonical root.
pub fn confine_to_root(root: &Path, relative: &Path) -> Result<PathBuf, PolicyError> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| PolicyError::Path(format!("workspace root unavailable: {error}")))?;
    let joined = canonical_root.join(relative);
    let mut probe = joined.clone();
    let resolved = loop {
        match probe.canonicalize() {
            Ok(real) => break real,
            Err(_) => match probe.parent() {
                Some(parent) => probe = parent.to_path_buf(),
                None => return Err(PolicyError::Path("path resolution failed".into())),
            },
        }
    };
    if !resolved.starts_with(&canonical_root) {
        return Err(PolicyError::Path(
            "path resolves outside the workspace root".into(),
        ));
    }
    Ok(joined)
}

const ALLOWED_COMMANDS: &[&str] = &[
    "bun", "cargo", "cat", "deno", "gcc", "git", "go", "gofmt", "gradle", "head", "ls", "make",
    "mvn", "node", "npm", "pnpm", "pwd", "pytest", "python", "python3", "ruff", "rustc", "sort",
    "tail", "tsc", "uniq", "uv", "uvx", "wc", "yarn",
];

const ALLOWED_GIT_SUBCOMMANDS: &[&str] = &[
    "blame",
    "branch",
    "describe",
    "diff",
    "grep",
    "log",
    "ls-files",
    "rev-parse",
    "show",
    "status",
];

const INTERPRETERS: &[&str] = &["bun", "deno", "node", "python", "python3"];
const INLINE_EVAL_FLAGS: &[&str] = &["-c", "-e", "-p", "--eval", "--print"];

const PACKAGE_MANAGERS: &[&str] = &["bun", "cargo", "npm", "pnpm", "uv", "yarn"];
const PACKAGE_NETWORK_VERBS: &[&str] = &[
    "add",
    "install",
    "publish",
    "remove",
    "uninstall",
    "update",
    "upgrade",
];

/// Privilege escalation is never runnable, approved or not.
const FORBIDDEN_COMMANDS: &[&str] = &["doas", "su", "sudo"];

/// How a model-issued argv vector may run. There is no shell anywhere — the
/// vector is spawned directly, so metacharacters are inert; the policy
/// controls which binaries may run WITHOUT a human in the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandClass {
    /// Allowlisted binary with strict argument shape: eligible for auto-run
    /// when the permission mode allows it.
    AutoEligible,
    /// Runnable, but only with explicit human approval — even in auto mode.
    /// The string is the reason shown in the approval prompt.
    NeedsApproval(String),
    /// Never runs.
    Forbidden(String),
}

pub fn classify_command(argv: &[String]) -> CommandClass {
    let Some(program) = argv.first() else {
        return CommandClass::Forbidden("empty command".into());
    };
    if FORBIDDEN_COMMANDS.contains(&program.as_str()) {
        return CommandClass::Forbidden(format!("privilege escalation is never run: {program}"));
    }
    if !ALLOWED_COMMANDS.contains(&program.as_str()) {
        return CommandClass::NeedsApproval(format!(
            "'{program}' is not in the auto-run allowlist"
        ));
    }
    for argument in &argv[1..] {
        if Path::new(argument).is_absolute() {
            return CommandClass::NeedsApproval(format!(
                "absolute path argument escapes the workspace: {argument}"
            ));
        }
        if argument == ".." || argument.contains("../") || argument.contains("/..") {
            return CommandClass::NeedsApproval(format!(
                "parent-directory argument escapes the workspace: {argument}"
            ));
        }
    }
    if program == "git" {
        let subcommand = argv.get(1).map(String::as_str).unwrap_or_default();
        if !ALLOWED_GIT_SUBCOMMANDS.contains(&subcommand) {
            return CommandClass::NeedsApproval(format!(
                "git '{subcommand}' can modify repository state"
            ));
        }
    }
    if INTERPRETERS.contains(&program.as_str()) {
        for argument in &argv[1..] {
            if INLINE_EVAL_FLAGS.contains(&argument.as_str()) {
                return CommandClass::NeedsApproval(format!(
                    "inline code evaluation ({argument}) bypasses file review"
                ));
            }
        }
    }
    if PACKAGE_MANAGERS.contains(&program.as_str()) {
        let subcommand = argv.get(1).map(String::as_str).unwrap_or_default();
        if PACKAGE_NETWORK_VERBS.contains(&subcommand) {
            return CommandClass::NeedsApproval(format!(
                "package manager network action: {program} {subcommand}"
            ));
        }
    }
    CommandClass::AutoEligible
}

/// Drop credential-like variables from the inherited environment.
pub fn sanitized_environment() -> HashMap<String, String> {
    std::env::vars()
        .filter(|(key, _)| !is_credential_key(key))
        .collect()
}

pub fn is_credential_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    ["AUTH", "CREDENTIAL", "KEY", "PASSWORD", "SECRET", "TOKEN"]
        .iter()
        .any(|needle| upper.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escaping_and_secret_paths() {
        assert!(validate_relative_path("src/main.rs").is_ok());
        assert!(validate_relative_path("./src/main.rs").is_ok());
        assert!(validate_relative_path("/etc/passwd").is_err());
        assert!(validate_relative_path("../outside").is_err());
        assert!(validate_relative_path("a/../../outside").is_err());
        assert!(validate_relative_path(".git/config").is_err());
        assert!(validate_relative_path(".env").is_err());
        assert!(validate_relative_path("config/.env.local").is_err());
        assert!(validate_relative_path(".env.example").is_ok());
        assert!(validate_relative_path("keys/id_rsa").is_err());
        assert!(validate_relative_path("tls/server.pem").is_err());
        assert!(validate_relative_path("credentials.json").is_err());
    }

    #[test]
    fn symlink_escape_is_caught_by_realpath_recheck() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "s").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
            let relative = validate_relative_path("link/secret.txt").unwrap();
            assert!(confine_to_root(root.path(), &relative).is_err());
        }
        let relative = validate_relative_path("inside.txt").unwrap();
        assert!(confine_to_root(root.path(), &relative).is_ok());
    }

    #[test]
    fn command_policy_classifies_three_ways() {
        let classify = |argv: &[&str]| {
            classify_command(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        let auto = CommandClass::AutoEligible;
        // Strict allowlisted commands stay auto-eligible.
        assert_eq!(classify(&["cargo", "test"]), auto);
        assert_eq!(classify(&["pwd"]), auto);
        assert_eq!(classify(&["git", "status"]), auto);
        assert_eq!(classify(&["cargo", "build", "--release"]), auto);
        // Everything that used to be a hard reject is now approvable…
        let needs = |argv: &[&str]| matches!(classify(argv), CommandClass::NeedsApproval(_));
        assert!(needs(&["git", "push"]));
        assert!(needs(&["rm", "-rf", "x"]));
        assert!(needs(&["bash", "-c", "echo hi"]));
        assert!(needs(&["python3", "-c", "print(1)"]));
        assert!(needs(&["node", "--eval", "1"]));
        assert!(needs(&["npm", "install"]));
        assert!(needs(&["cargo", "publish"]));
        assert!(needs(&["cat", "/etc/passwd"]));
        assert!(needs(&["ls", "../.."]));
        assert!(needs(&["mv", "a", "b"]));
        // …except privilege escalation, which never runs.
        assert!(matches!(
            classify(&["sudo", "ls"]),
            CommandClass::Forbidden(_)
        ));
        assert!(matches!(classify(&["su", "-"]), CommandClass::Forbidden(_)));
        assert!(matches!(classify(&[]), CommandClass::Forbidden(_)));
    }

    #[test]
    fn credential_environment_variables_are_stripped() {
        assert!(is_credential_key("DEEPSEEK_API_KEY"));
        assert!(is_credential_key("aws_secret_access_key"));
        assert!(is_credential_key("GITHUB_TOKEN"));
        assert!(is_credential_key("MY_PASSWORD"));
        assert!(!is_credential_key("PATH"));
        assert!(!is_credential_key("HOME"));
    }
}

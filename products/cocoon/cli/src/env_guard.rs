use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Eq, PartialEq)]
pub struct Finding {
    pub path: PathBuf,
    pub line: usize,
    pub variable: String,
}

pub fn scan_workspace(root: &Path) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    scan_directory(root, &mut findings)?;
    Ok(findings)
}

fn scan_directory(directory: &Path, findings: &mut Vec<Finding>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("could not inspect {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not inspect workspace: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("could not inspect {}: {error}", entry.path().display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_symlink() {
            if is_env_file(&name) {
                return Err(format!(
                    "refusing to inspect symlinked environment file {}",
                    entry.path().display()
                ));
            }
            continue;
        }
        if file_type.is_dir() {
            if !is_ignored_directory(&name) {
                scan_directory(&entry.path(), findings)?;
            }
        } else if file_type.is_file() && is_env_file(&name) {
            scan_file(&entry.path(), findings)?;
        }
    }
    Ok(())
}

fn is_ignored_directory(name: &str) -> bool {
    matches!(name, ".git" | ".jj" | "node_modules" | "target" | ".next")
}

fn is_env_file(name: &str) -> bool {
    (name == ".env" || name.starts_with(".env."))
        && !matches!(name, ".env.example" | ".env.sample" | ".env.template")
}

fn scan_file(path: &Path, findings: &mut Vec<Finding>) -> Result<(), String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    for (index, raw_line) in contents.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = unquote(value.trim());
        if looks_sensitive_name(name) && looks_like_plaintext(value) {
            findings.push(Finding {
                path: path.to_owned(),
                line: index + 1,
                variable: name.to_owned(),
            });
        }
    }
    Ok(())
}

fn unquote(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn looks_sensitive_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"]
        .iter()
        .any(|part| upper.contains(part))
}

fn looks_like_plaintext(value: &str) -> bool {
    if value.is_empty()
        || value.starts_with("op://")
        || value.starts_with("cocoon://")
        || value.starts_with("${")
        || value == "changeme"
        || value == "placeholder"
    {
        return false;
    }
    true
}

pub fn scrubbed_environment() -> Vec<(OsString, OsString)> {
    let exact: BTreeSet<&str> = [
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "GOOGLE_APPLICATION_CREDENTIALS",
    ]
    .into_iter()
    .collect();

    std::env::vars_os()
        .filter(|(name, _)| {
            let name = name.to_string_lossy().to_ascii_uppercase();
            !exact.contains(name.as_str()) && !is_sensitive_environment_name(&name)
        })
        .collect()
}

fn is_sensitive_environment_name(name: &str) -> bool {
    name.starts_with("OP_")
        || [
            "_API_KEY",
            "_TOKEN",
            "_SECRET",
            "_PASSWORD",
            "_CREDENTIAL",
            "_PRIVATE_KEY",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_names_and_locations_without_values() {
        let directory = tempfile::tempdir().unwrap();
        let env_path = directory.path().join(".env.local");
        fs::write(
            &env_path,
            "SAFE=value\nDEEPSEEK_API_KEY=sk-should-never-appear\nGLM_API_KEY=op://Agents/GLM/key\n",
        )
        .unwrap();

        let findings = scan_workspace(directory.path()).unwrap();
        assert_eq!(
            findings,
            vec![Finding {
                path: env_path,
                line: 2,
                variable: "DEEPSEEK_API_KEY".to_owned(),
            }]
        );
        assert!(!format!("{findings:?}").contains("should-never-appear"));
    }

    #[test]
    fn ignores_non_env_files() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("notes.txt"), "API_KEY=visible").unwrap();
        assert!(scan_workspace(directory.path()).unwrap().is_empty());
    }

    #[test]
    fn ignores_documentation_templates() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join(".env.example"),
            "DEEPSEEK_API_KEY=example-value",
        )
        .unwrap();
        assert!(scan_workspace(directory.path()).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_environment_files() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), directory.path().join(".env")).unwrap();
        assert!(scan_workspace(directory.path())
            .unwrap_err()
            .contains("symlinked environment file"));
    }

    #[test]
    fn scans_nested_workspaces_but_not_dependency_trees() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("apps/web")).unwrap();
        fs::create_dir_all(directory.path().join("node_modules/package")).unwrap();
        fs::write(
            directory.path().join("apps/web/.env"),
            "GLM_API_KEY=a-real-value",
        )
        .unwrap();
        fs::write(
            directory.path().join("node_modules/package/.env"),
            "IGNORED_API_KEY=dependency-value",
        )
        .unwrap();

        let findings = scan_workspace(directory.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].variable, "GLM_API_KEY");
    }

    #[test]
    fn recognizes_sensitive_process_environment_names() {
        for name in [
            "OP_SERVICE_ACCOUNT_TOKEN",
            "DEEPSEEK_API_KEY",
            "GITHUB_TOKEN",
            "DATABASE_PASSWORD",
            "SIGNING_PRIVATE_KEY",
        ] {
            assert!(is_sensitive_environment_name(name), "{name}");
        }
        assert!(!is_sensitive_environment_name("PATH"));
        assert!(!is_sensitive_environment_name("OPENAI_BASE_URL"));
    }
}

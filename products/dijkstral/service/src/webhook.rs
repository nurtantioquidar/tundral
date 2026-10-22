//! GitHub App webhooks: signature check, then the few events the service acts on.

use anyhow::{Context, Result};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;

/// `X-Hub-Signature-256` (`sha256=<hex>`) of `body` under `secret`, compared in constant time.
pub fn verify(secret: &[u8], header: Option<&str>, body: &[u8]) -> bool {
    let Some(sig) = header
        .and_then(|h| h.strip_prefix("sha256="))
        .and_then(|h| hex::decode(h).ok())
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

/// A repository as events name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub id: i64,
    pub full_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The app was installed, or repositories were added to an installation.
    Added {
        installation: i64,
        owner: String,
        repos: Vec<RepoRef>,
    },
    /// Repositories were removed from an installation.
    Removed { installation: i64, repos: Vec<i64> },
    /// The app was uninstalled (or suspended): everything of the installation goes.
    Uninstalled { installation: i64 },
    /// A push to a repository's default branch.
    Push {
        installation: i64,
        repo: i64,
        default_branch: String,
        commit: String,
    },
    /// Anything else (`ping`, pushes to other branches, unsupported events).
    Ignored,
}

fn repos(v: &Value) -> Vec<RepoRef> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(RepoRef {
                        id: r.get("id")?.as_i64()?,
                        full_name: r.get("full_name")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The event named by `X-GitHub-Event` with its JSON `body`.
pub fn parse(event: &str, body: &[u8]) -> Result<Event> {
    let v: Value = serde_json::from_slice(body).context("webhook body is not JSON")?;
    let action = v.get("action").and_then(Value::as_str).unwrap_or("");
    let installation = v
        .pointer("/installation/id")
        .and_then(Value::as_i64)
        .context("webhook without installation id")?;
    Ok(match (event, action) {
        ("installation", "created" | "unsuspend") => Event::Added {
            installation,
            owner: v
                .pointer("/installation/account/login")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            repos: repos(&v["repositories"]),
        },
        ("installation", "deleted" | "suspend") => Event::Uninstalled { installation },
        ("installation_repositories", "added" | "removed") => {
            let added = repos(&v["repositories_added"]);
            let removed: Vec<i64> = repos(&v["repositories_removed"])
                .into_iter()
                .map(|r| r.id)
                .collect();
            if action == "added" {
                Event::Added {
                    installation,
                    owner: v
                        .pointer("/installation/account/login")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    repos: added,
                }
            } else {
                Event::Removed {
                    installation,
                    repos: removed,
                }
            }
        }
        ("push", _) => {
            let branch = v
                .pointer("/repository/default_branch")
                .and_then(Value::as_str)
                .unwrap_or("");
            let reference = v.get("ref").and_then(Value::as_str).unwrap_or("");
            let deleted = v.get("deleted").and_then(Value::as_bool).unwrap_or(false);
            match (
                v.pointer("/repository/id").and_then(Value::as_i64),
                v.get("after").and_then(Value::as_str),
            ) {
                (Some(repo), Some(after))
                    if !deleted && reference == format!("refs/heads/{branch}") =>
                {
                    Event::Push {
                        installation,
                        repo,
                        default_branch: branch.to_string(),
                        commit: after.to_string(),
                    }
                }
                _ => Event::Ignored,
            }
        }
        _ => Event::Ignored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn signatures() {
        let body = br#"{"zen":"x"}"#;
        let good = sign(b"secret", body);
        assert!(verify(b"secret", Some(&good), body));
        assert!(!verify(b"other", Some(&good), body));
        assert!(!verify(b"secret", Some(&good), b"{}"));
        assert!(!verify(b"secret", None, body));
        assert!(!verify(b"secret", Some("sha1=abcd"), body));
        assert!(!verify(b"secret", Some("sha256=zz"), body));
    }

    #[test]
    fn events() {
        let install = br#"{"action":"created","installation":{"id":7,"account":{"login":"acme"}},
            "repositories":[{"id":100,"full_name":"acme/demo"}]}"#;
        assert_eq!(
            parse("installation", install).unwrap(),
            Event::Added {
                installation: 7,
                owner: "acme".into(),
                repos: vec![RepoRef {
                    id: 100,
                    full_name: "acme/demo".into()
                }]
            }
        );
        let push = |r: &str, deleted: bool| {
            format!(
                r#"{{"ref":"{r}","after":"abc","deleted":{deleted},"installation":{{"id":7}},
                "repository":{{"id":100,"default_branch":"main"}}}}"#
            )
        };
        assert_eq!(
            parse("push", push("refs/heads/main", false).as_bytes()).unwrap(),
            Event::Push {
                installation: 7,
                repo: 100,
                default_branch: "main".into(),
                commit: "abc".into()
            }
        );
        assert_eq!(
            parse("push", push("refs/heads/feature", false).as_bytes()).unwrap(),
            Event::Ignored
        );
        assert_eq!(
            parse("push", push("refs/heads/main", true).as_bytes()).unwrap(),
            Event::Ignored
        );
        let removed = br#"{"action":"removed","installation":{"id":7},
            "repositories_removed":[{"id":100,"full_name":"acme/demo"}]}"#;
        assert_eq!(
            parse("installation_repositories", removed).unwrap(),
            Event::Removed {
                installation: 7,
                repos: vec![100]
            }
        );
        assert_eq!(
            parse(
                "installation",
                br#"{"action":"deleted","installation":{"id":7}}"#
            )
            .unwrap(),
            Event::Uninstalled { installation: 7 }
        );
        assert!(parse("push", b"{}").is_err());
    }
}

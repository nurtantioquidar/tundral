//! Custom LLM providers (`dijkstral provider list|show|add|remove`; `docs/graphify-inputs.md`
//! §6): named OpenAI-compatible endpoints that `--backend <name>` selects.
//!
//! dijkstral keeps its own registry in `~/.dijkstral/providers.json`, in Graphify's file format,
//! and also reads Graphify's `~/.graphify/providers.json` (never writing it), so providers
//! registered with either tool work with both; dijkstral's entry wins a shared name. A provider
//! names the environment variable that holds its key (`env_key`); keys are never stored, printed
//! or accepted as arguments.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

/// Names `--backend` already means: Graphify's built-in providers and dijkstral's backends and
/// their aliases (rule R5).
pub const RESERVED: &[&str] = &[
    "gemini",
    "kimi",
    "claude",
    "openai",
    "deepseek",
    "ollama",
    "bedrock",
    "claude-cli",
    "azure",
    "codex",
    "google",
    "moonshot",
    "anthropic",
    "replay",
    "handoff",
    "skill",
];

/// Where a provider is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `~/.dijkstral/providers.json`.
    Dijkstral,
    /// `~/.graphify/providers.json` (read only).
    Graphify,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    /// The environment variable holding the API key.
    pub env_key: String,
    /// USD per million input and output tokens (stored; reported by nothing yet).
    pub pricing: (f64, f64),
    pub origin: Origin,
}

impl Provider {
    /// The entry as Graphify writes it (`provider show`).
    pub fn to_json(&self) -> Value {
        json!({
            "base_url": self.base_url,
            "default_model": self.default_model,
            "env_key": self.env_key,
            "pricing": {"input": self.pricing.0, "output": self.pricing.1},
            "temperature": 0,
        })
    }

    fn from_json(name: &str, v: &Value, origin: Origin) -> Option<Provider> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let price = |k: &str| {
            v.get("pricing")
                .and_then(|p| p.get(k))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        Some(Provider {
            name: name.to_string(),
            base_url: s("base_url")?,
            default_model: s("default_model").unwrap_or_default(),
            // An entry without one cannot authenticate; selection says so.
            env_key: s("env_key").unwrap_or_default(),
            pricing: (price("input"), price("output")),
            origin,
        })
    }
}

/// The registry files under a home directory.
#[derive(Debug, Clone)]
pub struct Registry {
    pub path: PathBuf,
    pub graphify_path: PathBuf,
    /// dijkstral's file as read, entries in order (unknown keys kept on rewrite).
    own: Map<String, Value>,
    graphify: Vec<Provider>,
}

impl Registry {
    /// `~/.dijkstral/providers.json` and `~/.graphify/providers.json` under `home`. A malformed
    /// dijkstral file is an error (it is never overwritten); a malformed Graphify file is skipped.
    pub fn load(home: &Path) -> Result<Registry> {
        let path = home.join(".dijkstral").join("providers.json");
        let graphify_path = home.join(".graphify").join("providers.json");
        let own = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(Value::Object(m)) => m,
                _ => bail!(
                    "{} is not a JSON object of providers; fix or remove it",
                    path.display()
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let graphify = fs::read(&graphify_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_object().cloned())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Provider::from_json(k, v, Origin::Graphify))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Registry {
            path,
            graphify_path,
            own,
            graphify,
        })
    }

    /// The registry of `$HOME` (an empty one without `HOME`).
    pub fn system() -> Result<Registry> {
        match std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            Some(home) => Registry::load(Path::new(&home)),
            None => Ok(Registry {
                path: PathBuf::from(".dijkstral/providers.json"),
                graphify_path: PathBuf::from(".graphify/providers.json"),
                own: Map::new(),
                graphify: Vec::new(),
            }),
        }
    }

    /// Every provider: dijkstral's in file order, then Graphify's it does not shadow.
    pub fn all(&self) -> Vec<Provider> {
        let mut out: Vec<Provider> = self
            .own
            .iter()
            .filter_map(|(k, v)| Provider::from_json(k, v, Origin::Dijkstral))
            .collect();
        for p in &self.graphify {
            if !self.own.contains_key(&p.name) {
                out.push(p.clone());
            }
        }
        out
    }

    pub fn get(&self, name: &str) -> Option<Provider> {
        self.all().into_iter().find(|p| p.name == name)
    }

    /// Register `p`; returns a warning to show (plaintext http to another host).
    pub fn add(&mut self, p: &Provider) -> Result<Option<String>> {
        check_name(&p.name)?;
        if self.own.contains_key(&p.name) {
            bail!(
                "provider '{}' is already registered; `dijkstral provider remove {}` first",
                p.name,
                p.name
            );
        }
        check_env_key(&p.env_key)?;
        if p.default_model.trim().is_empty() {
            bail!("--default-model must not be empty");
        }
        let warning = check_base_url(&p.base_url)?;
        self.own.insert(p.name.clone(), p.to_json());
        self.save()?;
        Ok(warning.map(|host| plaintext_warning(&p.name, &host)))
    }

    /// Remove `name` from dijkstral's registry.
    pub fn remove(&mut self, name: &str) -> Result<()> {
        if self.own.remove(name).is_none() {
            if self.graphify.iter().any(|p| p.name == name) {
                bail!(
                    "provider '{name}' is registered in {}, which dijkstral does not edit; use \
                     `graphify provider remove {name}`",
                    self.graphify_path.display()
                );
            }
            bail!("provider '{name}' is not registered");
        }
        self.save()
    }

    fn save(&self) -> Result<()> {
        let dir = self
            .path
            .parent()
            .context("providers path has no directory")?;
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let mut text = serde_json::to_string_pretty(&Value::Object(self.own.clone()))?;
        text.push('\n');
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &self.path).with_context(|| format!("writing {}", self.path.display()))
    }
}

/// Provider names are `--backend` values: letters, digits, `.`, `_` and `-`, not a built-in.
pub fn check_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        bail!("provider names use letters, digits, '.', '_' and '-' only");
    }
    if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        bail!("'{name}' is a built-in backend name; choose another provider name");
    }
    Ok(())
}

/// `--env-key` names a variable; a value that is not a variable name (a pasted key) is refused
/// without being repeated (quirk R3b).
pub fn check_env_key(env_key: &str) -> Result<()> {
    let mut chars = env_key.chars();
    let ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && env_key.len() <= 128;
    if !ok {
        bail!(
            "--env-key takes the name of the environment variable that holds the key (for \
             example MY_LLM_API_KEY), not the key itself"
        );
    }
    Ok(())
}

/// An `http`/`https` URL with a host. Returns the host when the URL sends files in plaintext to
/// a machine other than this one.
pub fn check_base_url(url: &str) -> Result<Option<String>> {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = hostport.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    if !matches!(scheme.as_str(), "http" | "https") || host.is_empty() {
        bail!("--base-url must be an http:// or https:// URL with a host");
    }
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback());
    Ok((scheme == "http" && !loopback).then(|| host.to_string()))
}

pub fn plaintext_warning(name: &str, host: &str) -> String {
    format!(
        "warning: provider '{name}' sends your files to {host} over plain http; use https unless \
         that host is on a network you trust"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str, url: &str, key: &str) -> Provider {
        Provider {
            name: name.into(),
            base_url: url.into(),
            default_model: "m".into(),
            env_key: key.into(),
            pricing: (2.5, 10.0),
            origin: Origin::Dijkstral,
        }
    }

    #[test]
    fn registry_round_trip_and_graphify_file() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".graphify")).unwrap();
        fs::write(
            home.path().join(".graphify/providers.json"),
            r#"{"shared": {"base_url": "https://g/v1", "default_model": "gm", "env_key": "G_KEY"},
                "gonly": {"base_url": "https://o/v1", "default_model": "om", "env_key": "O_KEY"}}"#,
        )
        .unwrap();
        let mut r = Registry::load(home.path()).unwrap();
        assert!(r
            .add(&provider("shared", "https://d/v1", "D_KEY"))
            .unwrap()
            .is_none());
        let names: Vec<(String, Origin)> =
            r.all().into_iter().map(|p| (p.name, p.origin)).collect();
        assert_eq!(
            names,
            [
                ("shared".to_string(), Origin::Dijkstral),
                ("gonly".to_string(), Origin::Graphify)
            ]
        );
        let text = fs::read_to_string(home.path().join(".dijkstral/providers.json")).unwrap();
        assert_eq!(
            text,
            "{\n  \"shared\": {\n    \"base_url\": \"https://d/v1\",\n    \"default_model\": \"m\",\n    \"env_key\": \"D_KEY\",\n    \"pricing\": {\n      \"input\": 2.5,\n      \"output\": 10.0\n    },\n    \"temperature\": 0\n  }\n}\n"
        );
        assert!(r.add(&provider("shared", "https://x/v1", "D_KEY")).is_err());
        assert!(r
            .remove("gonly")
            .unwrap_err()
            .to_string()
            .contains("graphify"));
        r.remove("shared").unwrap();
        assert!(r.remove("shared").is_err());
        assert_eq!(
            Registry::load(home.path())
                .unwrap()
                .get("shared")
                .unwrap()
                .origin,
            Origin::Graphify
        );
    }

    #[test]
    fn validation() {
        assert!(check_name("local-llm.v2").is_ok());
        assert!(check_name("bad name").is_err());
        assert!(check_name("OpenAI").is_err());
        assert!(check_name("anthropic").is_err());
        let err = check_env_key("sk-abcdef1234567890")
            .unwrap_err()
            .to_string();
        assert!(!err.contains("sk-abcdef"), "{err}");
        assert!(check_env_key("MY_KEY_2").is_ok());
        assert_eq!(check_base_url("https://h/v1").unwrap(), None);
        assert_eq!(check_base_url("http://localhost:8080/v1").unwrap(), None);
        assert_eq!(check_base_url("http://127.0.0.2/v1").unwrap(), None);
        assert_eq!(check_base_url("http://[::1]:8/v1").unwrap(), None);
        assert_eq!(
            check_base_url("http://192.168.1.2/v1").unwrap().as_deref(),
            Some("192.168.1.2")
        );
        assert!(check_base_url("ftp://host").is_err());
        assert!(check_base_url("nohost").is_err());
    }

    #[test]
    fn malformed_own_file_is_never_overwritten() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".dijkstral")).unwrap();
        fs::write(home.path().join(".dijkstral/providers.json"), "{not json").unwrap();
        assert!(Registry::load(home.path()).is_err());
    }
}

use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path};
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u8,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub profiles: BTreeMap<String, ProfileConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub upstream: String,
    pub secret: String,
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub allowed_models: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    pub provider: String,
    #[serde(default = "default_base_url_env")]
    pub base_url_env: String,
    #[serde(default = "default_api_key_env")]
    pub api_key_env: String,
}

fn default_base_url_env() -> String {
    "OPENAI_BASE_URL".to_owned()
}

fn default_api_key_env() -> String {
    "OPENAI_API_KEY".to_owned()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let config: Self = toml::from_str(&contents)
            .map_err(|error| format!("invalid {}: {error}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("unsupported config version {}", self.version));
        }
        if self.providers.is_empty() {
            return Err("at least one provider is required".to_owned());
        }
        if self.profiles.is_empty() {
            return Err("at least one profile is required".to_owned());
        }

        for (name, provider) in &self.providers {
            validate_name("provider", name)?;
            validate_provider(name, provider)?;
        }
        for (name, profile) in &self.profiles {
            validate_name("profile", name)?;
            if !self.providers.contains_key(&profile.provider) {
                return Err(format!(
                    "profile {name} references unknown provider {}",
                    profile.provider
                ));
            }
            validate_env_name(&profile.base_url_env)?;
            validate_env_name(&profile.api_key_env)?;
        }
        Ok(())
    }
}

fn validate_provider(name: &str, provider: &ProviderConfig) -> Result<(), String> {
    let upstream = Url::parse(&provider.upstream)
        .map_err(|error| format!("provider {name} has invalid upstream: {error}"))?;
    if upstream.scheme() != "https" {
        return Err(format!("provider {name} upstream must use https"));
    }
    if upstream.host_str().is_none() {
        return Err(format!("provider {name} upstream must have a host"));
    }
    if !upstream.username().is_empty() || upstream.password().is_some() {
        return Err(format!(
            "provider {name} upstream may not contain credentials"
        ));
    }
    if upstream.query().is_some() || upstream.fragment().is_some() {
        return Err(format!(
            "provider {name} upstream may not contain a query or fragment"
        ));
    }
    if !provider.secret.starts_with("op://") {
        return Err(format!(
            "provider {name} secret must be a 1Password op:// reference"
        ));
    }
    if provider.allowed_paths.is_empty() {
        return Err(format!("provider {name} must allow at least one path"));
    }
    for path in &provider.allowed_paths {
        if !path.starts_with('/') || path.contains('?') || path.contains('#') {
            return Err(format!("provider {name} has invalid allowed path {path}"));
        }
    }
    Ok(())
}

fn validate_name(kind: &str, value: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "{kind} name {value:?} must be lowercase kebab-case"
        ))
    }
}

fn validate_env_name(value: &str) -> Result<(), String> {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err("environment variable name may not be empty".to_owned());
    };
    if !(first.is_ascii_uppercase() || first == b'_')
        || !bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(format!("invalid environment variable name {value:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> Config {
        toml::from_str(
            r#"
                version = 1
                [providers.deepseek]
                upstream = "https://api.deepseek.com"
                secret = "op://Agents/DeepSeek/api-key"
                allowed_paths = ["/v1/chat/completions"]
                allowed_models = ["deepseek-chat"]
                [profiles.coding]
                provider = "deepseek"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn accepts_a_valid_config() {
        valid_config().validate().unwrap();
    }

    #[test]
    fn rejects_plaintext_secrets() {
        let mut config = valid_config();
        config.providers.get_mut("deepseek").unwrap().secret = "sk-real-secret".to_owned();
        assert!(config.validate().unwrap_err().contains("op://"));
    }

    #[test]
    fn rejects_non_tls_upstreams() {
        let mut config = valid_config();
        config.providers.get_mut("deepseek").unwrap().upstream =
            "http://api.deepseek.com".to_owned();
        assert!(config.validate().unwrap_err().contains("https"));
    }

    #[test]
    fn rejects_credentials_embedded_in_upstream_urls() {
        let mut config = valid_config();
        config.providers.get_mut("deepseek").unwrap().upstream =
            "https://user:password@api.deepseek.com".to_owned();
        assert!(config.validate().unwrap_err().contains("credentials"));
    }

    #[test]
    fn rejects_unknown_profile_provider() {
        let mut config = valid_config();
        config.profiles.get_mut("coding").unwrap().provider = "missing".to_owned();
        assert!(config.validate().unwrap_err().contains("unknown provider"));
    }
}

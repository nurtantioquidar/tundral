//! Key-based redaction for anything persisted about configuration or errors.
//! Values under credential-looking keys are replaced before hitting disk.
//! Conversation content is persisted verbatim (it is required to resume a
//! session) — but credentials never enter the message history in the first
//! place: the provider key lives only in process memory and command
//! environments are credential-stripped.

use serde_json::Value;

pub fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if crate::tools::policy::is_credential_key(key) {
                    *entry = Value::String("[redacted]".to_string());
                } else {
                    redact_value(entry);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_credential_keys_recursively() {
        let mut value = json!({
            "model": "deepseek-v4-flash",
            "api_key": "sk-123",
            "nested": {"github_token": "ghp_x", "path": "src"},
            "list": [{"password": "p"}]
        });
        redact_value(&mut value);
        assert_eq!(value["api_key"], "[redacted]");
        assert_eq!(value["nested"]["github_token"], "[redacted]");
        assert_eq!(value["list"][0]["password"], "[redacted]");
        assert_eq!(value["nested"]["path"], "src");
        assert_eq!(value["model"], "deepseek-v4-flash");
    }
}

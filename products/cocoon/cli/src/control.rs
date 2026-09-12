use crate::{broker::BrokerState, config::ProfileConfig};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path, sync::Arc, time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};
use zeroize::Zeroizing;

const MAX_CONTROL_BYTES: u64 = 16 * 1024;
const SESSION_LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);

#[derive(Clone)]
pub struct ControlState {
    broker: BrokerState,
    profiles: Arc<BTreeMap<String, ProfileConfig>>,
    broker_origin: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum ControlRequest {
    Issue { profile: String },
    Revoke { token: String },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ControlResponse {
    Granted {
        token: String,
        base_url: String,
        base_url_env: String,
        api_key_env: String,
    },
    Revoked,
    Error {
        message: String,
    },
}

pub struct SessionGrant {
    pub token: Zeroizing<String>,
    pub base_url: String,
    pub base_url_env: String,
    pub api_key_env: String,
}

impl ControlState {
    pub fn new(
        broker: BrokerState,
        profiles: BTreeMap<String, ProfileConfig>,
        broker_origin: String,
    ) -> Self {
        Self {
            broker,
            profiles: Arc::new(profiles),
            broker_origin,
        }
    }
}

pub fn bind(socket_path: &Path) -> Result<UnixListener, String> {
    if socket_path.exists() {
        return Err(format!(
            "control socket {} already exists; stop the existing daemon or remove a confirmed stale socket",
            socket_path.display()
        ));
    }
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let listener = UnixListener::bind(socket_path)
        .map_err(|error| format!("could not bind {}: {error}", socket_path.display()))?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not protect {}: {error}", socket_path.display()))?;
    Ok(listener)
}

pub async fn serve(listener: UnixListener, state: ControlState) -> Result<(), String> {
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|error| format!("control socket failed: {error}"))?;
        let state = state.clone();
        tokio::spawn(async move {
            let _ = handle_connection(stream, state).await;
        });
    }
}

async fn handle_connection(stream: UnixStream, state: ControlState) -> Result<(), String> {
    let (read_half, mut write_half) = stream.into_split();
    let mut payload = Vec::new();
    read_half
        .take(MAX_CONTROL_BYTES)
        .read_to_end(&mut payload)
        .await
        .map_err(|error| format!("could not read control request: {error}"))?;
    let request: ControlRequest =
        serde_json::from_slice(&payload).map_err(|_| "invalid control request".to_owned())?;
    let response = process(request, &state).await;
    let payload = serde_json::to_vec(&response)
        .map_err(|error| format!("could not encode control response: {error}"))?;
    write_half
        .write_all(&payload)
        .await
        .map_err(|error| format!("could not write control response: {error}"))?;
    write_half
        .shutdown()
        .await
        .map_err(|error| format!("could not close control response: {error}"))?;
    Ok(())
}

async fn process(request: ControlRequest, state: &ControlState) -> ControlResponse {
    match request {
        ControlRequest::Issue { profile } => {
            let Some(profile) = state.profiles.get(&profile) else {
                return ControlResponse::Error {
                    message: "unknown profile".to_owned(),
                };
            };
            match state
                .broker
                .issue_session(&profile.provider, SESSION_LIFETIME)
                .await
            {
                Ok(token) => ControlResponse::Granted {
                    token: token.to_string(),
                    base_url: format!("{}/{}", state.broker_origin, profile.provider),
                    base_url_env: profile.base_url_env.clone(),
                    api_key_env: profile.api_key_env.clone(),
                },
                Err(message) => ControlResponse::Error { message },
            }
        }
        ControlRequest::Revoke { token } => {
            state.broker.revoke_session(&token).await;
            ControlResponse::Revoked
        }
    }
}

pub async fn issue_session(socket_path: &Path, profile: &str) -> Result<SessionGrant, String> {
    match request(
        socket_path,
        &ControlRequest::Issue {
            profile: profile.to_owned(),
        },
    )
    .await?
    {
        ControlResponse::Granted {
            token,
            base_url,
            base_url_env,
            api_key_env,
        } => Ok(SessionGrant {
            token: Zeroizing::new(token),
            base_url,
            base_url_env,
            api_key_env,
        }),
        ControlResponse::Error { message } => Err(message),
        ControlResponse::Revoked => Err("invalid control response".to_owned()),
    }
}

pub async fn revoke_session(socket_path: &Path, token: &str) -> Result<(), String> {
    match request(
        socket_path,
        &ControlRequest::Revoke {
            token: token.to_owned(),
        },
    )
    .await?
    {
        ControlResponse::Revoked => Ok(()),
        ControlResponse::Error { message } => Err(message),
        ControlResponse::Granted { .. } => Err("invalid control response".to_owned()),
    }
}

async fn request(socket_path: &Path, request: &ControlRequest) -> Result<ControlResponse, String> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|error| format!("could not connect to Cocoon daemon: {error}"))?;
    let payload = serde_json::to_vec(request)
        .map_err(|error| format!("could not encode control request: {error}"))?;
    stream
        .write_all(&payload)
        .await
        .map_err(|error| format!("could not send control request: {error}"))?;
    stream
        .shutdown()
        .await
        .map_err(|error| format!("could not finish control request: {error}"))?;
    let mut response = Vec::new();
    stream
        .take(MAX_CONTROL_BYTES)
        .read_to_end(&mut response)
        .await
        .map_err(|error| format!("could not read control response: {error}"))?;
    serde_json::from_slice(&response).map_err(|_| "invalid response from Cocoon daemon".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{broker::ProviderRuntime, config::ProviderConfig, secret::SecretValue};
    use std::collections::BTreeMap;

    fn state(origin: &str) -> ControlState {
        let provider_config = ProviderConfig {
            upstream: "https://api.deepseek.com".to_owned(),
            secret: "op://Agents/DeepSeek/key".to_owned(),
            allowed_paths: vec!["/v1/chat/completions".to_owned()],
            allowed_models: vec!["deepseek-chat".to_owned()],
        };
        let provider =
            ProviderRuntime::new(&provider_config, SecretValue::synthetic("upstream-secret"))
                .unwrap();
        let broker = BrokerState::new(BTreeMap::from([("deepseek".to_owned(), provider)])).unwrap();
        let profile = ProfileConfig {
            provider: "deepseek".to_owned(),
            base_url_env: "OPENAI_BASE_URL".to_owned(),
            api_key_env: "OPENAI_API_KEY".to_owned(),
        };
        ControlState::new(
            broker,
            BTreeMap::from([("coding".to_owned(), profile)]),
            origin.to_owned(),
        )
    }

    #[tokio::test]
    async fn issues_and_revokes_a_session_over_a_protected_socket() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("cocoon.sock");
        let server = tokio::spawn({
            let socket = socket.clone();
            async move {
                let listener = bind(&socket).unwrap();
                serve(listener, state("http://127.0.0.1:4646")).await
            }
        });
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            tokio::task::yield_now().await;
        }

        let grant = issue_session(&socket, "coding").await.unwrap();
        assert_eq!(grant.base_url, "http://127.0.0.1:4646/deepseek".to_owned());
        assert_eq!(grant.base_url_env, "OPENAI_BASE_URL");
        assert_eq!(grant.api_key_env, "OPENAI_API_KEY");
        assert_eq!(grant.token.len(), 64);
        revoke_session(&socket, &grant.token).await.unwrap();

        server.abort();
    }

    #[tokio::test]
    async fn rejects_unknown_profiles() {
        let response = process(
            ControlRequest::Issue {
                profile: "unknown".to_owned(),
            },
            &state("http://127.0.0.1:4646"),
        )
        .await;
        assert!(matches!(response, ControlResponse::Error { .. }));
    }
}

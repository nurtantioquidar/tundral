use crate::{config::ProviderConfig, secret::SecretValue};
use axum::{
    body::{to_bytes, Body},
    extract::{OriginalUri, Path, State},
    http::{header, HeaderMap, HeaderValue, Method, Request, Response, StatusCode},
    response::IntoResponse,
    routing::any,
    Router,
};
use rand::RngCore;
use reqwest::redirect::Policy;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use url::Url;
use zeroize::Zeroizing;

const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;

pub struct ProviderRuntime {
    upstream: Url,
    secret: SecretValue,
    allowed_paths: Vec<String>,
    allowed_models: Vec<String>,
}

impl ProviderRuntime {
    pub fn new(config: &ProviderConfig, secret: SecretValue) -> Result<Self, String> {
        let upstream = Url::parse(&config.upstream)
            .map_err(|error| format!("invalid provider upstream: {error}"))?;
        Ok(Self {
            upstream,
            secret,
            allowed_paths: config.allowed_paths.clone(),
            allowed_models: config.allowed_models.clone(),
        })
    }

    #[cfg(test)]
    fn synthetic(
        upstream: Url,
        secret: &str,
        allowed_paths: &[&str],
        allowed_models: &[&str],
    ) -> Self {
        Self {
            upstream,
            secret: SecretValue::synthetic(secret),
            allowed_paths: allowed_paths
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            allowed_models: allowed_models
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }
}

#[derive(Clone)]
pub struct BrokerState(Arc<BrokerInner>);

struct BrokerInner {
    client: reqwest::Client,
    providers: BTreeMap<String, ProviderRuntime>,
    sessions: RwLock<HashMap<[u8; 32], Session>>,
}

struct Session {
    provider: String,
    expires_at: Instant,
}

impl BrokerState {
    pub fn new(providers: BTreeMap<String, ProviderRuntime>) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .map_err(|error| format!("could not create broker HTTP client: {error}"))?;
        Ok(Self(Arc::new(BrokerInner {
            client,
            providers,
            sessions: RwLock::new(HashMap::new()),
        })))
    }

    pub async fn issue_session(
        &self,
        provider: &str,
        lifetime: Duration,
    ) -> Result<Zeroizing<String>, String> {
        if !self.0.providers.contains_key(provider) {
            return Err(format!("unknown provider {provider:?}"));
        }
        let mut random = [0_u8; 32];
        rand::rng().fill_bytes(&mut random);
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        self.0.sessions.write().await.insert(
            token_digest(&token),
            Session {
                provider: provider.to_owned(),
                expires_at: Instant::now() + lifetime,
            },
        );
        Ok(Zeroizing::new(token))
    }

    pub async fn revoke_session(&self, token: &str) {
        self.0.sessions.write().await.remove(&token_digest(token));
    }

    async fn session_allows(&self, token: &str, provider: &str) -> bool {
        let sessions = self.0.sessions.read().await;
        sessions.get(&token_digest(token)).is_some_and(|session| {
            session.provider == provider && session.expires_at > Instant::now()
        })
    }
}

fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

pub fn router(state: BrokerState) -> Router {
    Router::new()
        .route("/{provider}/{*path}", any(proxy))
        .with_state(state)
}

async fn proxy(
    State(state): State<BrokerState>,
    Path((provider_name, path)): Path<(String, String)>,
    OriginalUri(original_uri): OriginalUri,
    request: Request<Body>,
) -> Response<Body> {
    if request.method() != Method::POST {
        return error_response(StatusCode::METHOD_NOT_ALLOWED, "method is not allowed");
    }
    let Some(session_token) = bearer_token(request.headers()) else {
        return error_response(StatusCode::UNAUTHORIZED, "invalid Cocoon session");
    };
    if !state.session_allows(session_token, &provider_name).await {
        return error_response(StatusCode::UNAUTHORIZED, "invalid Cocoon session");
    }

    let Some(provider) = state.0.providers.get(&provider_name) else {
        return error_response(StatusCode::NOT_FOUND, "unknown provider");
    };
    let request_path = format!("/{path}");
    if !provider
        .allowed_paths
        .iter()
        .any(|allowed| allowed == &request_path)
    {
        return error_response(StatusCode::FORBIDDEN, "path is not allowed");
    }

    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_REQUEST_BYTES).await {
        Ok(body) => body,
        Err(_) => return error_response(StatusCode::PAYLOAD_TOO_LARGE, "request is too large"),
    };
    if let Err(message) = authorize_model(provider, &body) {
        return error_response(StatusCode::FORBIDDEN, message);
    }

    let upstream_url = match upstream_url(provider, &request_path, original_uri.query()) {
        Ok(url) => url,
        Err(message) => return error_response(StatusCode::BAD_GATEWAY, message),
    };
    let authorization = match HeaderValue::from_str(&format!("Bearer {}", provider.secret.expose()))
    {
        Ok(value) => value,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "configured credential cannot be used as an HTTP header",
            )
        }
    };

    let mut outbound = state
        .0
        .client
        .post(upstream_url)
        .header(header::AUTHORIZATION, authorization)
        .body(body);
    for name in [header::ACCEPT, header::CONTENT_TYPE, header::USER_AGENT] {
        if let Some(value) = parts.headers.get(&name) {
            outbound = outbound.header(name, value);
        }
    }

    let upstream_response = match outbound.send().await {
        Ok(response) => response,
        Err(_) => return error_response(StatusCode::BAD_GATEWAY, "upstream request failed"),
    };
    let status = upstream_response.status();
    let mut response = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CACHE_CONTROL,
        header::RETRY_AFTER,
    ] {
        if let Some(value) = upstream_response.headers().get(&name) {
            response = response.header(name, value);
        }
    }
    match response.body(Body::from_stream(upstream_response.bytes_stream())) {
        Ok(response) => response,
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not construct upstream response",
        ),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?;
    let value = value.to_str().ok()?;
    value.strip_prefix("Bearer ")
}

fn authorize_model(provider: &ProviderRuntime, body: &[u8]) -> Result<(), &'static str> {
    if provider.allowed_models.is_empty() {
        return Ok(());
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| "request body must be JSON")?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .ok_or("request body must contain a model")?;
    if provider
        .allowed_models
        .iter()
        .any(|allowed| allowed == model)
    {
        Ok(())
    } else {
        Err("model is not allowed")
    }
}

fn upstream_url(
    provider: &ProviderRuntime,
    path: &str,
    query: Option<&str>,
) -> Result<Url, &'static str> {
    let mut url = provider.upstream.clone();
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}{path}"));
    url.set_query(query);
    Ok(url)
}

fn error_response(status: StatusCode, message: &'static str) -> Response<Body> {
    (status, message).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, routing::post};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Option<String>>>);

    async fn fake_upstream(
        State(capture): State<Capture>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let value = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *capture.0.lock().unwrap() = value;
        (StatusCode::OK, r#"{"ok":true}"#)
    }

    async fn start_fake_upstream(capture: Capture) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/v1/chat/completions", post(fake_upstream))
                    .with_state(capture),
            )
            .await
            .unwrap();
        });
        address
    }

    async fn start_broker(
        upstream_address: std::net::SocketAddr,
    ) -> (std::net::SocketAddr, String) {
        let provider = ProviderRuntime::synthetic(
            Url::parse(&format!("http://{upstream_address}")).unwrap(),
            "upstream-secret",
            &["/v1/chat/completions"],
            &["deepseek-chat"],
        );
        let state = BrokerState::new(BTreeMap::from([("deepseek".to_owned(), provider)])).unwrap();
        let token = state
            .issue_session("deepseek", Duration::from_secs(60))
            .await
            .unwrap()
            .to_string();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
        (address, token)
    }

    #[tokio::test]
    async fn replaces_the_session_token_with_the_upstream_secret() {
        let capture = Capture::default();
        let upstream = start_fake_upstream(capture.clone()).await;
        let (broker, token) = start_broker(upstream).await;

        let response = reqwest::Client::new()
            .post(format!("http://{broker}/deepseek/v1/chat/completions"))
            .bearer_auth(&token)
            .json(&serde_json::json!({"model": "deepseek-chat", "messages": []}))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            capture.0.lock().unwrap().as_deref(),
            Some("Bearer upstream-secret")
        );
        assert_ne!(token, "upstream-secret");
    }

    #[tokio::test]
    async fn rejects_an_unknown_session_without_contacting_upstream() {
        let capture = Capture::default();
        let upstream = start_fake_upstream(capture.clone()).await;
        let (broker, _) = start_broker(upstream).await;

        let response = reqwest::Client::new()
            .post(format!("http://{broker}/deepseek/v1/chat/completions"))
            .bearer_auth("wrong")
            .json(&serde_json::json!({"model": "deepseek-chat"}))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(capture.0.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_disallowed_models_without_contacting_upstream() {
        let capture = Capture::default();
        let upstream = start_fake_upstream(capture.clone()).await;
        let (broker, token) = start_broker(upstream).await;

        let response = reqwest::Client::new()
            .post(format!("http://{broker}/deepseek/v1/chat/completions"))
            .bearer_auth(token)
            .json(&serde_json::json!({"model": "forbidden-model"}))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(capture.0.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_disallowed_paths_without_contacting_upstream() {
        let capture = Capture::default();
        let upstream = start_fake_upstream(capture.clone()).await;
        let (broker, token) = start_broker(upstream).await;

        let response = reqwest::Client::new()
            .post(format!("http://{broker}/deepseek/v1/files"))
            .bearer_auth(token)
            .json(&serde_json::json!({"model": "deepseek-chat"}))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(capture.0.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn revoked_and_expired_sessions_are_not_authorized() {
        let provider = ProviderRuntime::synthetic(
            Url::parse("http://127.0.0.1:1").unwrap(),
            "upstream-secret",
            &["/v1/chat/completions"],
            &["deepseek-chat"],
        );
        let state = BrokerState::new(BTreeMap::from([("deepseek".to_owned(), provider)])).unwrap();

        let revoked = state
            .issue_session("deepseek", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(state.session_allows(&revoked, "deepseek").await);
        state.revoke_session(&revoked).await;
        assert!(!state.session_allows(&revoked, "deepseek").await);

        let expired = state
            .issue_session("deepseek", Duration::ZERO)
            .await
            .unwrap();
        assert!(!state.session_allows(&expired, "deepseek").await);
    }
}

//! HTTP surface: the GitHub webhook, per-repository MCP, the web app's private API and health.
//!
//! Public: `POST /github/webhook`, `/mcp/{owner}/{repo}`, `GET /healthz`. Private (never routed
//! publicly, bearer internal token): `/internal/*`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use dijkstral_serve::mcp;
use hmac::digest::Digest;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;

use crate::builder::{self, Builder};
use crate::config::Config;
use crate::db::{self, Db, Repo};
use crate::git::Source;
use crate::webhook::{self, Event};

/// Largest request body accepted (a webhook for a big push stays well below it).
const MAX_BODY: usize = 5 << 20;

pub struct AppState {
    pub config: Arc<Config>,
    pub db: Arc<Db>,
    pub builder: Arc<Builder>,
    /// One MCP server per repository, dropped when its graph is swapped.
    servers: Arc<Mutex<HashMap<i64, Arc<Mutex<mcp::Server>>>>>,
    /// Per token: (minute, requests in it).
    rate: Mutex<HashMap<i64, (i64, u32)>>,
}

/// Opens the database, starts the build worker and returns the router.
pub fn app(config: Config) -> anyhow::Result<(Router, Arc<AppState>)> {
    std::fs::create_dir_all(config.data.join("db"))?;
    std::fs::create_dir_all(config.data.join("repos"))?;
    let config = Arc::new(config);
    let db = Arc::new(Db::open(&config.data.join("db/service.sqlite"))?);
    let source = Arc::new(Source::new(config.source.clone()));
    let servers: Arc<Mutex<HashMap<i64, Arc<Mutex<mcp::Server>>>>> = Arc::default();
    let dropped = servers.clone();
    let builder = Builder::start(
        config.clone(),
        db.clone(),
        source,
        Arc::new(move |repo| {
            dropped
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&repo);
        }),
    );
    let state = Arc::new(AppState {
        config,
        db,
        builder,
        servers,
        rate: Mutex::new(HashMap::new()),
    });
    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/github/webhook", post(github_webhook))
        .route(
            "/mcp/{owner}/{repo}",
            post(mcp_post).get(mcp_not_allowed).delete(mcp_not_allowed),
        )
        .route("/internal/accounts", post(internal_account))
        .route(
            "/internal/accounts/{id}/tokens",
            post(internal_create_token),
        )
        .route(
            "/internal/accounts/{id}/tokens/{token}",
            delete(internal_revoke_token),
        )
        .route("/internal/accounts/{id}/repos", get(internal_repos))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY))
        .with_state(state.clone());
    Ok((router, state))
}

fn status(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({"error": msg}))).into_response()
}

fn internal_error(e: anyhow::Error) -> Response {
    tracing::error!(error = %e, "internal error");
    status(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

// ---------------------------------------------------------------------------------------------
// GitHub webhook

async fn github_webhook(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|h| h.to_str().ok());
    if !webhook::verify(&s.config.webhook_secret, sig, &body) {
        return status(StatusCode::UNAUTHORIZED, "bad signature");
    }
    let event = headers
        .get("x-github-event")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let parsed = match webhook::parse(event, &body) {
        Ok(e) => e,
        Err(_) if event == "ping" => Event::Ignored,
        Err(e) => return status(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    match apply(&s, parsed) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(e) => internal_error(e),
    }
}

fn forget(s: &AppState, repo: i64) -> anyhow::Result<()> {
    s.db.remove_repo(repo)?;
    builder::remove_repo_files(&s.config, repo)?;
    s.servers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&repo);
    Ok(())
}

fn apply(s: &AppState, event: Event) -> anyhow::Result<()> {
    match event {
        Event::Added {
            installation,
            owner,
            repos,
        } => {
            s.db.add_installation(installation, &owner)?;
            for r in repos {
                let repo = s.db.add_repo(
                    installation,
                    r.id,
                    &r.full_name,
                    s.config.limits.repos_per_installation,
                )?;
                if repo.state != "over_limit" {
                    s.builder.enqueue(repo.id);
                }
            }
        }
        Event::Removed {
            installation,
            repos,
        } => {
            for id in repos {
                if s.db
                    .repo(id)?
                    .is_some_and(|r| r.installation_id == installation)
                {
                    forget(s, id)?;
                }
            }
        }
        Event::Uninstalled { installation } => {
            for id in s.db.repos_of_installation(installation)? {
                forget(s, id)?;
            }
            s.db.remove_installation(installation)?;
        }
        Event::Push {
            installation,
            repo,
            default_branch,
            ..
        } => {
            if let Some(r) = s.db.repo(repo)? {
                if r.installation_id == installation && r.state != "over_limit" {
                    s.db.set_default_branch(repo, &default_branch)?;
                    s.builder.enqueue(repo);
                }
            }
        }
        Event::Ignored => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// MCP over Streamable HTTP (JSON responses; no server-initiated stream)

async fn mcp_not_allowed() -> Response {
    let mut r = StatusCode::METHOD_NOT_ALLOWED.into_response();
    r.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST"));
    r
}

fn unauthorized() -> Response {
    let mut r = status(StatusCode::UNAUTHORIZED, "missing or invalid token");
    r.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Bearer realm=\"dijkstral\""),
    );
    r
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

impl AppState {
    fn allow(&self, token: i64) -> bool {
        let minute = db::now() / 60;
        let mut rate = self.rate.lock().unwrap_or_else(|e| e.into_inner());
        let slot = rate.entry(token).or_insert((minute, 0));
        if slot.0 != minute {
            *slot = (minute, 0);
        }
        slot.1 += 1;
        slot.1 <= self.config.limits.mcp_requests_per_minute
    }

    fn server(&self, repo: &Repo) -> Arc<Mutex<mcp::Server>> {
        let mut servers = self.servers.lock().unwrap_or_else(|e| e.into_inner());
        servers
            .entry(repo.id)
            .or_insert_with(|| {
                let graph = builder::current_graph(&self.config, repo.id);
                let mut server = mcp::Server::new(mcp::Options {
                    cwd: graph
                        .parent()
                        .and_then(|p| p.parent())
                        .map(PathBuf::from)
                        .unwrap_or_default(),
                    graph: Some(graph),
                    verbose: false,
                    hosted: true,
                });
                // HTTP requests of one client may reach a server created after its handshake
                // (a restart, a swapped graph): every server starts initialised.
                server.handle(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
                    "params": {"protocolVersion": "2025-06-18"}}));
                Arc::new(Mutex::new(server))
            })
            .clone()
    }
}

async fn mcp_post(
    State(s): State<Arc<AppState>>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(token) = bearer(&headers) else {
        return unauthorized();
    };
    let (token_id, account) = match s.db.token_account(token) {
        Ok(Some(t)) => t,
        Ok(None) => return unauthorized(),
        Err(e) => return internal_error(e),
    };
    if !s.allow(token_id) {
        return status(StatusCode::TOO_MANY_REQUESTS, "rate limit");
    }
    // Unknown and inaccessible repositories look the same.
    let full_name = format!("{owner}/{repo}");
    let r = match s.db.accessible_repo(account, &full_name) {
        Ok(Some(r)) if r.built_commit.is_some() => r,
        Ok(Some(_)) => return status(StatusCode::SERVICE_UNAVAILABLE, "graph not built yet"),
        Ok(None) => return status(StatusCode::NOT_FOUND, "no such repository"),
        Err(e) => return internal_error(e),
    };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": "Parse error"}})),
            )
                .into_response()
        }
    };
    let server = s.server(&r);
    let replies = tokio::task::spawn_blocking(move || {
        let mut server = server.lock().unwrap_or_else(|e| e.into_inner());
        match &msg {
            Value::Array(batch) => {
                let out: Vec<Value> = batch.iter().filter_map(|m| server.handle(m)).collect();
                (!out.is_empty()).then_some(Value::Array(out))
            }
            m => server.handle(m),
        }
    })
    .await;
    if let Err(e) = s.db.touch(token_id, r.id) {
        tracing::warn!(error = %e, "touch failed");
    }
    match replies {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => StatusCode::ACCEPTED.into_response(),
        Err(e) => internal_error(e.into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Private API for the web app

fn internal_ok(s: &AppState, headers: &HeaderMap) -> bool {
    let Some(given) = bearer(headers) else {
        return false;
    };
    // Compare digests so the comparison time does not depend on the token's content.
    Sha256::digest(given.as_bytes()) == Sha256::digest(s.config.internal_token.as_bytes())
}

#[derive(Deserialize)]
struct AccountBody {
    github_id: i64,
    login: String,
    /// The GitHub App installations the user can access, from their sign-in.
    installation_ids: Vec<i64>,
}

async fn internal_account(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(b): Json<AccountBody>,
) -> Response {
    if !internal_ok(&s, &headers) {
        return status(StatusCode::NOT_FOUND, "not found");
    }
    match s
        .db
        .upsert_account(b.github_id, &b.login, &b.installation_ids)
    {
        Ok(id) => Json(json!({"account_id": id})).into_response(),
        Err(e) => internal_error(e),
    }
}

#[derive(Deserialize)]
struct TokenBody {
    name: String,
}

async fn internal_create_token(
    State(s): State<Arc<AppState>>,
    Path(account): Path<i64>,
    headers: HeaderMap,
    Json(b): Json<TokenBody>,
) -> Response {
    if !internal_ok(&s, &headers) {
        return status(StatusCode::NOT_FOUND, "not found");
    }
    match s.db.account_exists(account) {
        Ok(true) => {}
        Ok(false) => return status(StatusCode::NOT_FOUND, "no such account"),
        Err(e) => return internal_error(e),
    }
    match s.db.create_token(account, &b.name) {
        // Shown once; only its hash is kept.
        Ok((id, token)) => Json(json!({"id": id, "token": token})).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn internal_revoke_token(
    State(s): State<Arc<AppState>>,
    Path((account, token)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> Response {
    if !internal_ok(&s, &headers) {
        return status(StatusCode::NOT_FOUND, "not found");
    }
    match s.db.revoke_token(account, token) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => status(StatusCode::NOT_FOUND, "no such token"),
        Err(e) => internal_error(e),
    }
}

async fn internal_repos(
    State(s): State<Arc<AppState>>,
    Path(account): Path<i64>,
    headers: HeaderMap,
) -> Response {
    if !internal_ok(&s, &headers) {
        return status(StatusCode::NOT_FOUND, "not found");
    }
    match s.db.repos_of_account(account) {
        Ok(repos) => Json(json!({"repos": repos.iter().map(|r| json!({
            "id": r.id,
            "full_name": r.full_name,
            "state": r.state,
            "built_commit": r.built_commit,
            "nodes": r.nodes,
            "edges": r.edges,
            "error": r.error,
        })).collect::<Vec<_>>()}))
        .into_response(),
        Err(e) => internal_error(e),
    }
}

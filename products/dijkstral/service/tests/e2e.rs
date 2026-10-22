//! The service end to end, offline: a local bare repository stands in for GitHub, signed
//! webhooks drive builds, and MCP answers come back over HTTP.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use dijkstral_service::app::app;
use dijkstral_service::config::{Config, Limits, SourceConfig};
use hmac::{Hmac, KeyInit, Mac};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::Sha256;
use tower::ServiceExt;

const SECRET: &[u8] = b"webhook-secret-0123456789";
const INTERNAL: &str = "internal-token-0123456789abcdef0123456789";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

struct World {
    _tmp: tempfile::TempDir,
    work: PathBuf,
    data: PathBuf,
    router: Router,
}

/// `acme/demo`: the CLI's Python fixture, pushed to a bare repository with a committed
/// `dijkstral-out/` that the service must ignore.
fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("github");
    let bare = source.join("acme/demo.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare", "-b", "main"]);
    let work = tmp.path().join("work");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../cli/tests/fixtures/python"),
        &work,
    );
    std::fs::create_dir_all(work.join("dijkstral-out")).unwrap();
    std::fs::write(work.join("dijkstral-out/graph.dij"), b"not an index").unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "one"]);
    git(&work, &["push", "-q", bare.to_str().unwrap(), "main"]);
    let data = tmp.path().join("data");
    let (router, _) = app(Config {
        data: data.clone(),
        listen: "127.0.0.1:0".parse().unwrap(),
        webhook_secret: SECRET.to_vec(),
        internal_token: INTERNAL.into(),
        source: SourceConfig::Local(source),
        exe: PathBuf::from(env!("CARGO_BIN_EXE_dijkstral-service")),
        build_threads: 2,
        limits: Limits {
            build_timeout: Duration::from_secs(120),
            ..Limits::default()
        },
    })
    .unwrap();
    World {
        _tmp: tmp,
        work,
        data,
        router,
    }
}

async fn send(router: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn signed(event: &str, body: &Value, secret: &[u8]) -> Request<Body> {
    let body = serde_json::to_vec(body).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
    mac.update(&body);
    Request::post("/github/webhook")
        .header("x-github-event", event)
        .header(
            "x-hub-signature-256",
            format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
        )
        .body(Body::from(body))
        .unwrap()
}

fn internal(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {INTERNAL}"))
        .header("content-type", "application/json")
        .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
        .unwrap()
}

fn mcp(token: Option<&str>, repo: &str, msg: Value) -> Request<Body> {
    let mut r = Request::post(format!("/mcp/{repo}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(t) = token {
        r = r.header("authorization", format!("Bearer {t}"));
    }
    r.body(Body::from(msg.to_string())).unwrap()
}

/// Waits until `account`'s repository `id` has a built commit other than `not`.
async fn built(router: &Router, account: i64, id: i64, not: Option<&str>) -> Value {
    let started = Instant::now();
    loop {
        let (_, v) = send(
            router,
            internal("GET", &format!("/internal/accounts/{account}/repos"), None),
        )
        .await;
        if let Some(r) = v["repos"]
            .as_array()
            .and_then(|a| a.iter().find(|r| r["id"] == id))
        {
            if r["state"] == "failed" {
                panic!("build failed: {r}");
            }
            if r["state"] == "ready" && r["built_commit"].as_str() != not {
                return r.clone();
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "no build: {v}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn tool(name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": name, "arguments": args}})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn install_build_query_push_uninstall() {
    let w = world();
    let r = &w.router;

    // Unsigned or wrongly signed webhooks are refused before parsing.
    let install = json!({"action": "created",
        "installation": {"id": 7, "account": {"login": "acme"}},
        "repositories": [{"id": 100, "full_name": "acme/demo"}]});
    assert_eq!(
        send(r, signed("installation", &install, b"wrong-secret"))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(r, signed("installation", &install, SECRET)).await.0,
        StatusCode::ACCEPTED
    );

    // The web app registers two users: one can see installation 7, one cannot.
    let (_, a) = send(
        r,
        internal(
            "POST",
            "/internal/accounts",
            Some(json!({"github_id": 1, "login": "alice", "installation_ids": [7]})),
        ),
    )
    .await;
    let alice = a["account_id"].as_i64().unwrap();
    let (_, b) = send(
        r,
        internal(
            "POST",
            "/internal/accounts",
            Some(json!({"github_id": 2, "login": "bob", "installation_ids": []})),
        ),
    )
    .await;
    let bob = b["account_id"].as_i64().unwrap();
    let first = built(r, alice, 100, None).await;
    assert!(first["nodes"].as_i64().unwrap() > 10, "{first}");

    let (_, t) = send(
        r,
        internal(
            "POST",
            &format!("/internal/accounts/{alice}/tokens"),
            Some(json!({"name": "laptop"})),
        ),
    )
    .await;
    let token = t["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("dij_"));
    let (_, t) = send(
        r,
        internal(
            "POST",
            &format!("/internal/accounts/{bob}/tokens"),
            Some(json!({"name": "x"})),
        ),
    )
    .await;
    let bob_token = t["token"].as_str().unwrap().to_string();

    // The private API is invisible without the internal token.
    let (code, _) = send(
        r,
        Request::get(format!("/internal/accounts/{alice}/repos"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(code, StatusCode::NOT_FOUND);

    // MCP: no token, a bad token, another user's token, an unknown repository.
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18"}});
    assert_eq!(
        send(r, mcp(None, "acme/demo", init.clone())).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(r, mcp(Some("dij_nope"), "acme/demo", init.clone()))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(r, mcp(Some(&bob_token), "acme/demo", init.clone()))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(r, mcp(Some(&token), "acme/other", init.clone()))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (code, v) = send(r, mcp(Some(&token), "acme/demo", init)).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
    let (_, v) = send(
        r,
        mcp(
            Some(&token),
            "ACME/Demo",
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        ),
    )
    .await;
    assert_eq!(v["result"]["tools"][0]["name"], "context", "{v}");
    let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert_eq!(
        send(r, mcp(Some(&token), "acme/demo", notification))
            .await
            .0,
        StatusCode::ACCEPTED
    );

    let (_, v) = send(
        r,
        mcp(
            Some(&token),
            "acme/demo",
            tool(
                "context",
                json!({"question": "Who calls slugify and what does it call?"}),
            ),
        ),
    )
    .await;
    let text = v["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.starts_with("## slugify()  pkg/util.py:14\n"), "{text}");
    assert!(
        text.contains("     14| def slugify(text: str) -> str:"),
        "{text}"
    );

    // A push to the default branch rebuilds; the new function is queryable.
    std::fs::write(
        w.work.join("pkg/extra.py"),
        "from pkg.util import slugify\n\n\ndef shout_title(text):\n    return slugify(text).upper()\n",
    )
    .unwrap();
    git(&w.work, &["add", "-A"]);
    git(&w.work, &["commit", "-q", "-m", "two"]);
    let bare = w.data.parent().unwrap().join("github/acme/demo.git");
    git(&w.work, &["push", "-q", bare.to_str().unwrap(), "main"]);
    let push = json!({"ref": "refs/heads/feature", "after": "x", "installation": {"id": 7},
        "repository": {"id": 100, "default_branch": "main"}});
    assert_eq!(
        send(r, signed("push", &push, SECRET)).await.0,
        StatusCode::ACCEPTED
    );
    let push = json!({"ref": "refs/heads/main", "after": "x", "installation": {"id": 7},
        "repository": {"id": 100, "default_branch": "main"}});
    assert_eq!(
        send(r, signed("push", &push, SECRET)).await.0,
        StatusCode::ACCEPTED
    );
    let second = built(r, alice, 100, first["built_commit"].as_str()).await;
    let (_, v) = send(
        r,
        mcp(
            Some(&token),
            "acme/demo",
            tool("context", json!({"question": "Who calls `shout_title`?"})),
        ),
    )
    .await;
    let text = v["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("shout_title"), "{text}");
    // Only the served build and the one it replaced are kept.
    let repo_dir = w.data.join("repos/100");
    let dirs: Vec<_> = std::fs::read_dir(&repo_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(dirs.len() <= 3, "{dirs:?}");
    assert!(repo_dir
        .join(second["built_commit"].as_str().unwrap())
        .is_dir());

    // A revoked token stops working.
    let (_, t) = send(
        r,
        internal(
            "POST",
            &format!("/internal/accounts/{alice}/tokens"),
            Some(json!({"name": "old"})),
        ),
    )
    .await;
    let (old_id, old) = (
        t["id"].as_i64().unwrap(),
        t["token"].as_str().unwrap().to_string(),
    );
    let (code, _) = send(
        r,
        internal(
            "DELETE",
            &format!("/internal/accounts/{alice}/tokens/{old_id}"),
            None,
        ),
    )
    .await;
    assert_eq!(code, StatusCode::NO_CONTENT);
    assert_eq!(
        send(
            r,
            mcp(Some(&old), "acme/demo", tool("graph_stats", json!({})))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );

    // Uninstalling deletes the repository's code and graph at once.
    let gone = json!({"action": "deleted", "installation": {"id": 7}});
    assert_eq!(
        send(r, signed("installation", &gone, SECRET)).await.0,
        StatusCode::ACCEPTED
    );
    assert!(!repo_dir.exists());
    assert_eq!(
        send(
            r,
            mcp(Some(&token), "acme/demo", tool("graph_stats", json!({})))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

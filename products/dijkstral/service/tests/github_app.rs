//! The GitHub App source against a fake GitHub API: the app JWT is RS256 under the app's key,
//! installation tokens are minted once and reused, and reach `git` only through its environment.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use dijkstral_service::config::SourceConfig;
use dijkstral_service::git::Source;
use dijkstral_service::github::{Claims, GitHubApp};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::{json, Value};

fn run(dir: &Path, cmd: &str, args: &[&str]) {
    let out = Command::new(cmd)
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
        "{cmd} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Fake {
    public_pem: Vec<u8>,
    calls: AtomicUsize,
}

async fn mint(
    State(f): State<Arc<Fake>>,
    UrlPath(installation): UrlPath<i64>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let jwt = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    let mut v = Validation::new(Algorithm::RS256);
    v.set_required_spec_claims(&["exp", "iat", "iss"]);
    let key = DecodingKey::from_rsa_pem(&f.public_pem).unwrap();
    match jsonwebtoken::decode::<Claims>(jwt, &key, &v) {
        Ok(c) if c.claims.iss == "4242" && c.claims.exp - c.claims.iat <= 600 => {
            f.calls.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::CREATED,
                Json(
                    json!({"token": format!("ghs_install{installation}"), "expires_at": "2099-01-01T00:00:00Z"}),
                ),
            )
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "Bad credentials"})),
        ),
    }
}

#[tokio::test]
async fn installation_tokens_and_fetch() {
    let tmp = tempfile::tempdir().unwrap();
    let key = tmp.path().join("app.pem");
    let public = tmp.path().join("app.pub.pem");
    run(
        tmp.path(),
        "openssl",
        &[
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
            "-out",
            key.to_str().unwrap(),
        ],
    );
    run(
        tmp.path(),
        "openssl",
        &[
            "pkey",
            "-in",
            key.to_str().unwrap(),
            "-pubout",
            "-out",
            public.to_str().unwrap(),
        ],
    );

    let fake = Arc::new(Fake {
        public_pem: std::fs::read(&public).unwrap(),
        calls: AtomicUsize::new(0),
    });
    let api = Router::new()
        .route("/app/installations/{id}/access_tokens", post(mint))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });

    // The "git host" is a directory of bare repositories reached over file://.
    let hosting = tmp.path().join("hosting");
    let bare = hosting.join("acme/demo.git");
    std::fs::create_dir_all(&bare).unwrap();
    run(&bare, "git", &["init", "-q", "--bare", "-b", "trunk"]);
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("a.py"), "def f():\n    return 1\n").unwrap();
    run(&work, "git", &["init", "-q", "-b", "trunk"]);
    run(&work, "git", &["add", "-A"]);
    run(&work, "git", &["commit", "-q", "-m", "one"]);
    run(
        &work,
        "git",
        &["push", "-q", bare.to_str().unwrap(), "trunk"],
    );

    let web = format!("file://{}", hosting.display());
    let source = Source::new(SourceConfig::GitHub(GitHubApp {
        app_id: "4242".into(),
        private_key_pem: std::fs::read(&key).unwrap(),
        api: format!("http://{addr}"),
        web: web.clone(),
    }))
    .unwrap();

    // The credential is a header for the git host only, carried in the environment.
    let env = source.auth_env(7).await.unwrap();
    let basic = base64::engine::general_purpose::STANDARD.encode("x-access-token:ghs_install7");
    assert_eq!(
        env,
        vec![
            ("GIT_CONFIG_COUNT".to_string(), "1".to_string()),
            (
                "GIT_CONFIG_KEY_0".to_string(),
                format!("http.{web}/.extraheader")
            ),
            (
                "GIT_CONFIG_VALUE_0".to_string(),
                format!("AUTHORIZATION: basic {basic}")
            ),
        ]
    );

    assert_eq!(
        source.default_branch(7, "acme/demo").await.unwrap(),
        "trunk"
    );
    let dir = tmp.path().join("checkout");
    let commit = source
        .checkout(7, "acme/demo", "trunk", &dir)
        .await
        .unwrap();
    assert_eq!(commit.len(), 40);
    assert!(dir.join("a.py").is_file());
    // One token for installation 7, reused by every call; a second installation mints its own.
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    source.auth_env(8).await.unwrap();
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);

    // A key the fake GitHub does not know is refused, and the error carries no token.
    let other = tmp.path().join("other.pem");
    run(
        tmp.path(),
        "openssl",
        &[
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
            "-out",
            other.to_str().unwrap(),
        ],
    );
    let wrong = Source::new(SourceConfig::GitHub(GitHubApp {
        app_id: "4242".into(),
        private_key_pem: std::fs::read(&other).unwrap(),
        api: format!("http://{addr}"),
        web,
    }))
    .unwrap();
    let err = wrong
        .default_branch(7, "acme/demo")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("token refused"), "{err}");
}

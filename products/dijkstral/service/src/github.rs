//! GitHub App authentication: the app's JWT, and short-lived installation tokens minted with it
//! and cached until shortly before they expire.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// A GitHub App: its id, its private key (PEM), and where GitHub's API and git hosting live.
#[derive(Clone)]
pub struct GitHubApp {
    pub app_id: String,
    pub private_key_pem: Vec<u8>,
    /// `https://api.github.com`, or a test server.
    pub api: String,
    /// `https://github.com`; repositories are fetched from `<web>/<owner>/<repo>.git`.
    pub web: String,
}

impl std::fmt::Debug for GitHubApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the key.
        f.debug_struct("GitHubApp")
            .field("app_id", &self.app_id)
            .field("api", &self.api)
            .field("web", &self.web)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub iat: i64,
    pub exp: i64,
    pub iss: String,
}

/// Installation tokens live an hour; one is reused for 50 minutes.
const TOKEN_REUSE: Duration = Duration::from_secs(50 * 60);

pub struct Tokens {
    app: GitHubApp,
    key: EncodingKey,
    http: reqwest::Client,
    cache: Mutex<HashMap<i64, (String, Instant)>>,
}

impl Tokens {
    pub fn new(app: GitHubApp) -> Result<Tokens> {
        let key = EncodingKey::from_rsa_pem(&app.private_key_pem)
            .context("the GitHub App private key is not an RSA PEM key")?;
        let http = reqwest::Client::builder()
            .user_agent("dijkstral-service")
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Tokens {
            app,
            key,
            http,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn app(&self) -> &GitHubApp {
        &self.app
    }

    /// The app's own JWT (RS256, ten minutes, issued a minute in the past for clock drift).
    fn jwt(&self) -> Result<String> {
        let now = crate::db::now();
        Ok(jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &Claims {
                iat: now - 60,
                exp: now + 540,
                iss: self.app.app_id.clone(),
            },
            &self.key,
        )?)
    }

    /// An installation token for `installation`, minted when none is cached.
    pub async fn installation(&self, installation: i64) -> Result<String> {
        let mut cache = self.cache.lock().await;
        if let Some((token, at)) = cache.get(&installation) {
            if at.elapsed() < TOKEN_REUSE {
                return Ok(token.clone());
            }
        }
        let resp = self
            .http
            .post(format!(
                "{}/app/installations/{installation}/access_tokens",
                self.app.api
            ))
            .bearer_auth(self.jwt()?)
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
            .send()
            .await
            .context("GitHub: installation token request failed")?;
        if !resp.status().is_success() {
            bail!(
                "GitHub: installation {installation} token refused ({})",
                resp.status()
            );
        }
        #[derive(Deserialize)]
        struct Minted {
            token: String,
        }
        let minted: Minted = resp.json().await.context("GitHub: bad token reply")?;
        cache.insert(installation, (minted.token.clone(), Instant::now()));
        Ok(minted.token)
    }

    /// Forgets an installation's token (after it is uninstalled).
    pub async fn forget(&self, installation: i64) {
        self.cache.lock().await.remove(&installation);
    }
}

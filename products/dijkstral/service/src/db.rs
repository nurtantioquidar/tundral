//! The service's SQLite database; the service is its only writer.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    github_id INTEGER NOT NULL UNIQUE,
    login TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS installations (
    id INTEGER PRIMARY KEY,          -- GitHub's installation id
    owner TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS account_installations (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    installation_id INTEGER NOT NULL,
    PRIMARY KEY (account_id, installation_id)
);
CREATE TABLE IF NOT EXISTS repos (
    id INTEGER PRIMARY KEY,          -- GitHub's repository id
    installation_id INTEGER NOT NULL REFERENCES installations(id) ON DELETE CASCADE,
    full_name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    default_branch TEXT,
    state TEXT NOT NULL,             -- queued | building | ready | failed | over_limit
    built_commit TEXT,
    nodes INTEGER,
    edges INTEGER,
    error TEXT,
    updated_at INTEGER NOT NULL,
    last_query_at INTEGER
);
CREATE TABLE IF NOT EXISTS builds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id INTEGER NOT NULL,
    commit_sha TEXT,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    state TEXT NOT NULL,             -- running | ready | failed
    nodes INTEGER,
    edges INTEGER,
    error TEXT
);
CREATE INDEX IF NOT EXISTS builds_repo ON builds (repo_id, started_at);
CREATE TABLE IF NOT EXISTS tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    hash BLOB NOT NULL UNIQUE,       -- SHA-256 of the token; the token itself is never stored
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER
);
";

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The prefix of every MCP token, so leaked tokens are recognisable.
pub const TOKEN_PREFIX: &str = "dij_";

pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub id: i64,
    pub installation_id: i64,
    pub full_name: String,
    pub default_branch: Option<String>,
    pub state: String,
    pub built_commit: Option<String>,
    pub nodes: Option<i64>,
    pub edges: Option<i64>,
    pub error: Option<String>,
}

pub struct Db {
    conn: Mutex<Connection>,
}

fn repo_row(r: &rusqlite::Row) -> rusqlite::Result<Repo> {
    Ok(Repo {
        id: r.get(0)?,
        installation_id: r.get(1)?,
        full_name: r.get(2)?,
        default_branch: r.get(3)?,
        state: r.get(4)?,
        built_commit: r.get(5)?,
        nodes: r.get(6)?,
        edges: r.get(7)?,
        error: r.get(8)?,
    })
}

const REPO_COLUMNS: &str =
    "id, installation_id, full_name, default_branch, state, built_commit, nodes, edges, error";

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Ok(f(&conn)?)
    }

    pub fn add_installation(&self, id: i64, owner: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO installations (id, owner, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET owner = excluded.owner",
                params![id, owner, now()],
            )
            .map(drop)
        })
    }

    pub fn installation_exists(&self, id: i64) -> Result<bool> {
        self.with(|c| {
            c.query_row(
                "SELECT 1 FROM installations WHERE id = ?1",
                [id],
                |_| Ok(()),
            )
            .optional()
            .map(|r| r.is_some())
        })
    }

    /// Adds a repository; `over_limit` when the installation already has `cap` others.
    pub fn add_repo(
        &self,
        installation: i64,
        id: i64,
        full_name: &str,
        cap: usize,
    ) -> Result<Repo> {
        self.with(|c| {
            let others: i64 = c.query_row(
                "SELECT count(*) FROM repos WHERE installation_id = ?1 AND id != ?2
                 AND state != 'over_limit'",
                params![installation, id],
                |r| r.get(0),
            )?;
            let state = if others as usize >= cap {
                "over_limit"
            } else {
                "queued"
            };
            c.execute(
                "INSERT INTO repos (id, installation_id, full_name, state, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (id) DO UPDATE SET full_name = excluded.full_name,
                     installation_id = excluded.installation_id",
                params![id, installation, full_name, state, now()],
            )?;
            c.query_row(
                &format!("SELECT {REPO_COLUMNS} FROM repos WHERE id = ?1"),
                [id],
                repo_row,
            )
        })
    }

    pub fn repo(&self, id: i64) -> Result<Option<Repo>> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT {REPO_COLUMNS} FROM repos WHERE id = ?1"),
                [id],
                repo_row,
            )
            .optional()
        })
    }

    pub fn repos(&self) -> Result<Vec<Repo>> {
        self.with(|c| {
            let mut s = c.prepare(&format!("SELECT {REPO_COLUMNS} FROM repos"))?;
            let repos = s
                .query_map([], repo_row)?
                .collect::<rusqlite::Result<Vec<Repo>>>();
            repos
        })
    }

    pub fn repos_of_installation(&self, installation: i64) -> Result<Vec<i64>> {
        self.with(|c| {
            let mut s = c.prepare("SELECT id FROM repos WHERE installation_id = ?1")?;
            let ids = s
                .query_map([installation], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>();
            ids
        })
    }

    pub fn remove_repo(&self, id: i64) -> Result<()> {
        self.with(|c| c.execute("DELETE FROM repos WHERE id = ?1", [id]).map(drop))
    }

    pub fn remove_installation(&self, id: i64) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM repos WHERE installation_id = ?1", [id])?;
            c.execute(
                "DELETE FROM account_installations WHERE installation_id = ?1",
                [id],
            )?;
            c.execute("DELETE FROM installations WHERE id = ?1", [id])
                .map(drop)
        })
    }

    pub fn set_default_branch(&self, id: i64, branch: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE repos SET default_branch = ?2 WHERE id = ?1",
                params![id, branch],
            )
            .map(drop)
        })
    }

    pub fn set_state(&self, id: i64, state: &str, error: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE repos SET state = ?2, error = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, state, error, now()],
            )
            .map(drop)
        })
    }

    /// Builds of `repo` started in the last 24 hours.
    pub fn builds_today(&self, repo: i64) -> Result<usize> {
        self.with(|c| {
            c.query_row(
                "SELECT count(*) FROM builds WHERE repo_id = ?1 AND started_at > ?2",
                params![repo, now() - 86_400],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as usize)
        })
    }

    /// Fails the builds a stopped service left running.
    pub fn interrupt_running_builds(&self) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE builds SET finished_at = ?1, state = 'failed', error = 'interrupted'
                 WHERE state = 'running'",
                [now()],
            )
            .map(drop)
        })
    }

    pub fn start_build(&self, repo: i64) -> Result<i64> {
        self.with(|c| {
            c.execute(
                "INSERT INTO builds (repo_id, started_at, state) VALUES (?1, ?2, 'running')",
                params![repo, now()],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn finish_build(
        &self,
        build: i64,
        repo: i64,
        commit: Option<&str>,
        result: std::result::Result<(i64, i64), &str>,
    ) -> Result<()> {
        self.with(|c| match result {
            Ok((nodes, edges)) => {
                c.execute(
                    "UPDATE builds SET finished_at = ?2, state = 'ready', commit_sha = ?3,
                     nodes = ?4, edges = ?5 WHERE id = ?1",
                    params![build, now(), commit, nodes, edges],
                )?;
                c.execute(
                    "UPDATE repos SET state = 'ready', error = NULL, built_commit = ?2,
                     nodes = ?3, edges = ?4, updated_at = ?5 WHERE id = ?1",
                    params![repo, commit, nodes, edges, now()],
                )
                .map(drop)
            }
            Err(e) => {
                c.execute(
                    "UPDATE builds SET finished_at = ?2, state = 'failed', commit_sha = ?3,
                     error = ?4 WHERE id = ?1",
                    params![build, now(), commit, e],
                )?;
                // A repository keeps serving its last good graph; its state says the newest
                // build failed.
                c.execute(
                    "UPDATE repos SET state = 'failed', error = ?2, updated_at = ?3 WHERE id = ?1",
                    params![repo, e, now()],
                )
                .map(drop)
            }
        })
    }

    pub fn upsert_account(
        &self,
        github_id: i64,
        login: &str,
        installations: &[i64],
    ) -> Result<i64> {
        self.with(|c| {
            c.execute(
                "INSERT INTO accounts (github_id, login, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (github_id) DO UPDATE SET login = excluded.login",
                params![github_id, login, now()],
            )?;
            let id: i64 = c.query_row(
                "SELECT id FROM accounts WHERE github_id = ?1",
                [github_id],
                |r| r.get(0),
            )?;
            c.execute(
                "DELETE FROM account_installations WHERE account_id = ?1",
                [id],
            )?;
            for i in installations {
                c.execute(
                    "INSERT OR IGNORE INTO account_installations VALUES (?1, ?2)",
                    params![id, i],
                )?;
            }
            Ok(id)
        })
    }

    pub fn account_exists(&self, id: i64) -> Result<bool> {
        self.with(|c| {
            c.query_row("SELECT 1 FROM accounts WHERE id = ?1", [id], |_| Ok(()))
                .optional()
                .map(|r| r.is_some())
        })
    }

    /// A new token for `account`; returns `(id, token)`. Only the hash is stored.
    pub fn create_token(&self, account: i64, name: &str) -> Result<(i64, String)> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("random: {e}"))?;
        let token = format!("{TOKEN_PREFIX}{}", hex::encode(bytes));
        let hash = token_hash(&token);
        let id = self.with(|c| {
            c.execute(
                "INSERT INTO tokens (account_id, hash, name, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![account, hash, name, now()],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        Ok((id, token))
    }

    pub fn revoke_token(&self, account: i64, id: i64) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE tokens SET revoked_at = ?3 WHERE id = ?1 AND account_id = ?2
                 AND revoked_at IS NULL",
                params![id, account, now()],
            )
            .map(|n| n > 0)
        })
    }

    /// The `(token id, account id)` of a live token.
    pub fn token_account(&self, token: &str) -> Result<Option<(i64, i64)>> {
        let hash = token_hash(token);
        self.with(|c| {
            c.query_row(
                "SELECT id, account_id FROM tokens WHERE hash = ?1 AND revoked_at IS NULL",
                [hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })
    }

    pub fn touch(&self, token: i64, repo: i64) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE tokens SET last_used_at = ?2 WHERE id = ?1",
                params![token, now()],
            )?;
            c.execute(
                "UPDATE repos SET last_query_at = ?2 WHERE id = ?1",
                params![repo, now()],
            )
            .map(drop)
        })
    }

    /// The repository `full_name` if `account` may query it: one of its installations covers it.
    pub fn accessible_repo(&self, account: i64, full_name: &str) -> Result<Option<Repo>> {
        self.with(|c| {
            c.query_row(
                &format!(
                    "SELECT {} FROM repos r JOIN account_installations a
                     ON a.installation_id = r.installation_id
                     WHERE a.account_id = ?1 AND r.full_name = ?2",
                    REPO_COLUMNS
                        .split(", ")
                        .map(|col| format!("r.{col}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                params![account, full_name],
                repo_row,
            )
            .optional()
        })
    }

    pub fn repos_of_account(&self, account: i64) -> Result<Vec<Repo>> {
        self.with(|c| {
            let cols = REPO_COLUMNS
                .split(", ")
                .map(|col| format!("r.{col}"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut s = c.prepare(&format!(
                "SELECT {cols} FROM repos r JOIN account_installations a
                 ON a.installation_id = r.installation_id
                 WHERE a.account_id = ?1 ORDER BY r.full_name"
            ))?;
            let rows = s
                .query_map([account], repo_row)?
                .collect::<rusqlite::Result<Vec<_>>>();
            rows
        })
    }
}

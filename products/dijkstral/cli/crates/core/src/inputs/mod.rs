//! Graph inputs beyond the walked files, added only when requested (`docs/graphify-inputs.md`):
//! the crate graph of a Cargo workspace (`extract --cargo`, [`cargo`]) and the schema of a live
//! PostgreSQL database (`extract --postgres`, [`postgres`]; the connection itself is the CLI's,
//! this module maps catalog rows to nodes and edges). Both join the resolved graph before
//! clustering and are part of the build digest.

pub mod cargo;
pub mod postgres;

use crate::graph::Graph;

/// What a build adds besides its files, and how it walks them.
#[derive(Debug, Clone)]
pub struct Inputs {
    /// Read `.gitignore` files and `.git/info/exclude` (off with `--no-gitignore`).
    pub gitignore: bool,
    /// Add the Cargo workspace's crate graph.
    pub cargo: bool,
    /// A PostgreSQL schema read before the build.
    pub postgres: Option<postgres::Snapshot>,
}

impl Default for Inputs {
    fn default() -> Self {
        Inputs {
            gitignore: true,
            cargo: false,
            postgres: None,
        }
    }
}

/// The inputs as read for one build: hashed into the digest, then applied to the graph.
#[derive(Debug, Default)]
pub struct Loaded {
    pub cargo: Option<cargo::Workspace>,
    pub postgres: Option<postgres::Snapshot>,
}

impl Inputs {
    /// Read what the build needs from disk (the Cargo manifests).
    pub fn load(&self, root: &std::path::Path) -> Loaded {
        Loaded {
            cargo: self.cargo.then(|| cargo::scan(root)),
            postgres: self.postgres.clone(),
        }
    }
}

impl Loaded {
    /// Feeds everything that changes the graph into the build digest.
    pub fn digest(&self, h: &mut blake3::Hasher) {
        if let Some(ws) = &self.cargo {
            h.update(b"cargo\0");
            h.update(ws.fingerprint().as_bytes());
        }
        if let Some(pg) = &self.postgres {
            h.update(b"postgres\0");
            h.update(serde_json::to_string(pg).unwrap_or_default().as_bytes());
        }
    }

    /// Add the inputs to the resolved graph; returns a line per input for the user.
    pub fn apply(&self, graph: &mut Graph) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some(ws) = &self.cargo {
            let (crates, deps) = cargo::apply(graph, ws);
            notes.push(match &ws.manifest {
                Some(m) => format!("cargo: {crates} crates, {deps} dependencies (from {m})"),
                None => "cargo: no Cargo.toml at the root or one or two levels below".to_string(),
            });
        }
        if let Some(pg) = &self.postgres {
            let (nodes, edges) = postgres::apply(graph, pg);
            notes.push(format!(
                "postgres: {nodes} nodes, {edges} edges from {}",
                pg.source_file()
            ));
        }
        notes
    }
}

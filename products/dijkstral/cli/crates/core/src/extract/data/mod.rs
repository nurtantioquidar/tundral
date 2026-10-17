//! JSON manifests, `Cargo.toml`/`pyproject.toml` and shell scripts (`docs/graphify-format.md` §14,
//! §11.6).
//!
//! JSON and shell are parsed with tree-sitter (its error recovery decides what Graphify keeps from
//! a damaged file), TOML with a strict TOML 1.0 parser (an invalid file yields nothing). Everything
//! that depends on other files — package dependencies, `source`d files, calls into them — is
//! recorded in [`DataFacts`] and resolved by `crate::resolve::data`.

mod json;
mod shell;
mod toml;

use std::cell::RefCell;

use serde::{Deserialize, Serialize};
use tree_sitter::{Language, Parser, Tree};

use super::Ctx;
use crate::lang::Lang;

/// Facts the cross-file pass (`crate::resolve::data`) needs from one file. Ids are local
/// (before collision renaming); translate them with `View::id`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DataFacts {
    /// `Cargo.toml` / `pyproject.toml`: the package node this file emitted, if any.
    pub package: Option<Package>,
    /// Shell: literal `source`/`.` paths in source order.
    pub sources: Vec<Sourced>,
    /// Shell: every function defined in the file, `(name, local id)`, in source order.
    pub functions: Vec<(String, String)>,
    /// Shell: calls to names not defined in this file, one per (caller, name) at its first
    /// site, grouped by caller (entry first, then functions in definition order).
    pub calls: Vec<Call>,
    /// Shell: scripts run as commands (`./x.sh`, `bash x.sh`, `"${DIR}/x.sh"`), `name` being the
    /// path relative to this script's directory (a leading variable stands for that directory).
    /// One per (caller, path) at its first site.
    pub scripts: Vec<Call>,
    /// `Cargo.toml`: what Rust module resolution needs to name crates (`crate::resolve::rust`).
    pub cargo: Option<CargoManifest>,
}

/// The crate names a `Cargo.toml` defines and binds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CargoManifest {
    /// `[package] name`; `None` for a workspace-only root.
    pub package: Option<String>,
    /// `[lib] name`.
    pub lib_name: Option<String>,
    /// `[lib] path`, relative to the manifest's directory.
    pub lib_path: Option<String>,
    /// Every dependency table of the package (normal, dev, build, per target), in order.
    pub deps: Vec<CargoDep>,
    /// `[workspace.dependencies]`.
    pub workspace_deps: Vec<CargoDep>,
}

/// One dependency entry: `key = { package = …, path = …, workspace = true }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CargoDep {
    /// The table key: the name the crate is bound to (with `-`, as written).
    pub key: String,
    /// `package = "…"`, when the key renames the dependency.
    pub package: Option<String>,
    /// `path = "…"`, relative to the manifest's directory.
    pub path: Option<String>,
    /// `workspace = true`: the entry is inherited from the workspace root.
    pub workspace: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    pub id: String,
    /// `slug(name)`, the key dependencies are matched on.
    pub key: String,
    /// `slug(dependency name)` in declaration order.
    pub deps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sourced {
    /// The path as written, unquoted; for `variable` sources, the expanded path (relative to
    /// the script's directory unless absolute).
    pub path: String,
    pub line: u32,
    /// The path started with a variable (`"${ROOT}/lib.sh"`): resolved INFERRED, and a missing
    /// file is dropped rather than becoming an external.
    pub variable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Call {
    pub caller: String,
    pub name: String,
    pub line: u32,
}

pub fn extract(cx: &mut Ctx) {
    match cx.lang {
        Lang::Json => json::extract(cx),
        Lang::Toml => toml::extract(cx),
        Lang::Shell => shell::extract(cx),
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum Grammar {
    Json = 0,
    Bash = 1,
}

thread_local! {
    static PARSERS: RefCell<[Option<Parser>; 2]> = const { RefCell::new([None, None]) };
}

/// Parse with a parser cached per thread and grammar.
fn parse(grammar: Grammar, source: &[u8]) -> Option<Tree> {
    PARSERS.with(|cell| {
        let mut parsers = cell.borrow_mut();
        let slot = &mut parsers[grammar as usize];
        if slot.is_none() {
            let language: Language = match grammar {
                Grammar::Json => tree_sitter_json::LANGUAGE.into(),
                Grammar::Bash => tree_sitter_bash::LANGUAGE.into(),
            };
            let mut parser = Parser::new();
            parser.set_language(&language).ok()?;
            *slot = Some(parser);
        }
        slot.as_mut()?.parse(source, None)
    })
}

/// 1-based line of a tree-sitter node's start.
fn line(node: tree_sitter::Node) -> u32 {
    node.start_position().row as u32 + 1
}

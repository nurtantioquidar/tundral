//! Source languages, how files are classified, and their tree-sitter grammars.

use std::cell::RefCell;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter::{Language, Parser, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Lang {
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Rust,
    Go,
    Java,
    /// `.json` manifests (allowlist decided at extraction, §14.1).
    Json,
    /// `Cargo.toml` and `pyproject.toml` only.
    Toml,
    /// `go.mod`.
    GoMod,
    /// `.sh`, `.bash`, and sh/bash/zsh shebang scripts.
    Shell,
}

/// Languages that may share call and type resolution with each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Python,
    Script,
    Rust,
    Go,
    Java,
    Data,
    Shell,
}

impl Lang {
    pub const COUNT: usize = 11;

    /// Classify a path. `shebang` is consulted only for extensionless files.
    pub fn classify(path: &Path) -> Option<Lang> {
        let name = path.file_name()?.to_str()?;
        match name {
            "Cargo.toml" | "pyproject.toml" => return Some(Lang::Toml),
            "go.mod" => return Some(Lang::GoMod),
            "package-lock.json" => return None,
            _ => {}
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return shebang(path);
        };
        Some(match ext.to_ascii_lowercase().as_str() {
            "py" => Lang::Python,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "js" | "jsx" | "mjs" | "cjs" => Lang::JavaScript,
            "rs" => Lang::Rust,
            "go" => Lang::Go,
            "java" => Lang::Java,
            "json" => Lang::Json,
            "sh" | "bash" => Lang::Shell,
            _ => return None,
        })
    }

    pub fn family(self) -> Family {
        match self {
            Lang::Python => Family::Python,
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => Family::Script,
            Lang::Rust => Family::Rust,
            Lang::Go | Lang::GoMod => Family::Go,
            Lang::Java => Family::Java,
            Lang::Json | Lang::Toml => Family::Data,
            Lang::Shell => Family::Shell,
        }
    }

    pub fn grammar(self) -> Option<Language> {
        Some(match self {
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
            Lang::Java => tree_sitter_java::LANGUAGE.into(),
            Lang::Json | Lang::Toml | Lang::GoMod | Lang::Shell => return None,
        })
    }
}

/// Extensionless scripts: python and sh/bash/zsh shebangs are code (§14.5).
fn shebang(path: &Path) -> Option<Lang> {
    let mut head = [0u8; 128];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let line = std::str::from_utf8(&head[..n]).ok()?.lines().next()?;
    let interp = line.strip_prefix("#!")?.trim();
    let mut words = interp.split_whitespace();
    let mut prog = words.next()?.rsplit('/').next()?;
    if prog == "env" {
        prog = words.next()?;
    }
    if prog.starts_with("python") {
        Some(Lang::Python)
    } else if matches!(prog, "sh" | "bash" | "zsh") {
        Some(Lang::Shell)
    } else {
        None
    }
}

thread_local! {
    static PARSERS: RefCell<Vec<Option<Parser>>> = RefCell::new((0..Lang::COUNT).map(|_| None).collect());
}

/// Parse `source` with a parser cached per thread and language.
pub fn parse(lang: Lang, source: &[u8]) -> Option<Tree> {
    PARSERS.with(|cell| {
        let mut parsers = cell.borrow_mut();
        let slot = &mut parsers[lang as usize];
        if slot.is_none() {
            let mut parser = Parser::new();
            parser.set_language(&lang.grammar()?).ok()?;
            *slot = Some(parser);
        }
        slot.as_mut()?.parse(source, None)
    })
}

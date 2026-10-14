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
    Swift,
    /// `.json` manifests (allowlist decided at extraction, §14.1).
    Json,
    /// `Cargo.toml` and `pyproject.toml` only.
    Toml,
    /// `go.mod`.
    GoMod,
    /// `.sh`, `.bash`, and sh/bash/zsh shebang scripts.
    Shell,
    CSharp,
    Php,
    Ruby,
    /// `.kt`, `.kts`.
    Kotlin,
    /// `.scala` (`.sc` and `.sbt` are not classified).
    Scala,
    /// `.c`, and `.h` without C++ markers (§18).
    C,
    /// `.cpp .cc .cxx .hpp .cu`, and `.h` with C++ markers (§19).
    Cpp,
    /// `.groovy`, `.gradle` (§26).
    Groovy,
    /// `.dart` (§27).
    Dart,
    /// `.lua`, `.luau`, and `lua` shebang scripts (§28).
    Lua,
}

/// Languages that may share call and type resolution with each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Python,
    Script,
    Rust,
    Go,
    Java,
    Swift,
    Data,
    Shell,
    CSharp,
    Php,
    Ruby,
    Kotlin,
    Scala,
    C,
    Groovy,
    Dart,
    Lua,
}

impl Lang {
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
            "swift" => Lang::Swift,
            "json" => Lang::Json,
            "sh" | "bash" => Lang::Shell,
            "cs" => Lang::CSharp,
            "php" => Lang::Php,
            "rb" | "rake" => Lang::Ruby,
            "kt" | "kts" => Lang::Kotlin,
            "scala" => Lang::Scala,
            "c" => Lang::C,
            "cpp" | "cc" | "cxx" | "hpp" | "cu" => Lang::Cpp,
            "h" => return crate::extract::c::header_lang(path),
            "groovy" | "gradle" => Lang::Groovy,
            "dart" => Lang::Dart,
            "lua" | "luau" => Lang::Lua,
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
            Lang::Swift => Family::Swift,
            Lang::Json | Lang::Toml => Family::Data,
            Lang::Shell => Family::Shell,
            Lang::CSharp => Family::CSharp,
            Lang::Php => Family::Php,
            Lang::Ruby => Family::Ruby,
            Lang::Kotlin => Family::Kotlin,
            Lang::Scala => Family::Scala,
            Lang::C | Lang::Cpp => Family::C,
            Lang::Groovy => Family::Groovy,
            Lang::Dart => Family::Dart,
            Lang::Lua => Family::Lua,
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
            Lang::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Lang::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Lang::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Lang::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Lang::Scala => tree_sitter_scala::LANGUAGE.into(),
            Lang::Swift => tree_sitter_swift::LANGUAGE.into(),
            Lang::C => tree_sitter_c::LANGUAGE.into(),
            Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Lang::Groovy => tree_sitter_groovy::LANGUAGE.into(),
            Lang::Lua => tree_sitter_lua::LANGUAGE.into(),
            Lang::Dart => return None,
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
    } else if prog.starts_with("ruby") {
        Some(Lang::Ruby)
    } else if prog == "lua" {
        Some(Lang::Lua)
    } else {
        None
    }
}

thread_local! {
    static PARSERS: RefCell<Vec<Option<Parser>>> = const { RefCell::new(Vec::new()) };
}

/// Parse `source` with a parser cached per thread and language.
pub fn parse(lang: Lang, source: &[u8]) -> Option<Tree> {
    PARSERS.with(|cell| {
        let mut parsers = cell.borrow_mut();
        // Grown on demand so adding a language never needs a count kept in sync.
        if parsers.len() <= lang as usize {
            parsers.resize_with(lang as usize + 1, || None);
        }
        let slot = &mut parsers[lang as usize];
        if slot.is_none() {
            let mut parser = Parser::new();
            parser.set_language(&lang.grammar()?).ok()?;
            *slot = Some(parser);
        }
        slot.as_mut()?.parse(source, None)
    })
}

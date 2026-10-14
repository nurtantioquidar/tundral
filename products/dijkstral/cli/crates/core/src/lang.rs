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
    /// `.csproj`, `.fsproj`, `.vbproj` (§43.1).
    MsBuild,
    /// `.sln` (§43.2).
    Sln,
    /// `.slnx` (§43.2).
    Slnx,
    /// `.razor`, `.cshtml` (§43.3).
    Razor,
    /// `.xaml` (§43.4).
    Xaml,
    /// Code Graphify counts but extracts nothing from (§44.8): `.r`, `.sql`, `.tf`, `.ejs`, and
    /// perl, fish and tcsh shebang scripts.
    Inert,
    /// `.pas .dpr .pp .inc .lpr .dpk` (§44.1).
    Pascal,
    /// `.dfm .lfm` (§44.2).
    DelphiForm,
    /// `.lpk` (§44.3).
    Lpk,
    /// `.cls .trigger` (§44.4).
    Apex,
    /// `.cob .cbl .cpy` (§44.5).
    Cobol,
    /// `.dmf .dmi .dmm` (§44.6).
    Byond,
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
    /// .NET project and markup files (§43).
    Dotnet,
    /// Counted as code, never extracted (§44).
    Inert,
    /// Grammar-less scanners (§44).
    Legacy,
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
            "csproj" | "fsproj" | "vbproj" => Lang::MsBuild,
            "sln" => Lang::Sln,
            "slnx" => Lang::Slnx,
            "razor" | "cshtml" => Lang::Razor,
            "xaml" => Lang::Xaml,
            "r" | "sql" | "tf" | "ejs" => Lang::Inert,
            "metal" => Lang::Cpp,
            "pas" | "dpr" | "pp" | "inc" | "lpr" | "dpk" => Lang::Pascal,
            "dfm" | "lfm" => Lang::DelphiForm,
            "lpk" => Lang::Lpk,
            "cls" | "trigger" => Lang::Apex,
            "cob" | "cbl" | "cpy" => Lang::Cobol,
            "dmf" | "dmi" | "dmm" => Lang::Byond,
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
            Lang::MsBuild | Lang::Sln | Lang::Slnx | Lang::Razor | Lang::Xaml => Family::Dotnet,
            Lang::Inert => Family::Inert,
            Lang::Pascal
            | Lang::DelphiForm
            | Lang::Lpk
            | Lang::Apex
            | Lang::Cobol
            | Lang::Byond => Family::Legacy,
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
            Lang::Json | Lang::Toml | Lang::GoMod | Lang::Shell => return None,
            Lang::MsBuild
            | Lang::Sln
            | Lang::Slnx
            | Lang::Razor
            | Lang::Xaml
            | Lang::Inert
            | Lang::Pascal
            | Lang::DelphiForm
            | Lang::Lpk
            | Lang::Apex
            | Lang::Cobol
            | Lang::Byond => return None,
        })
    }
}

/// Extensionless scripts: the interpreter named by a shebang (after `env` and its flags) decides
/// the language, by exact name (§14.5, §44.8): `python3.11` or `Python3` are not code.
fn shebang(path: &Path) -> Option<Lang> {
    let mut head = [0u8; 128];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let line = std::str::from_utf8(&head[..n]).ok()?.lines().next()?;
    let interp = line.strip_prefix("#!")?.trim();
    let mut words = interp.split_whitespace();
    let mut prog = words.next()?.rsplit('/').next()?;
    if prog == "env" {
        prog = words.find(|w| !w.starts_with('-'))?;
    }
    Some(match prog {
        "python" | "python2" | "python3" => Lang::Python,
        "sh" | "bash" | "zsh" | "ksh" | "dash" => Lang::Shell,
        "ruby" => Lang::Ruby,
        "node" | "nodejs" => Lang::JavaScript,
        "php" => Lang::Php,
        "perl" | "fish" | "tcsh" => Lang::Inert,
        _ => return None,
    })
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

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
    /// `.yaml`, `.yml` (§46).
    Yaml,
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
    /// `.c`, and `.h` without C++ markers (§24).
    C,
    /// `.cpp .cc .cxx .hpp .cu`, and `.h` with C++ markers (§25).
    Cpp,
    /// `.ex`, `.exs` (§29).
    Elixir,
    /// `.erl`, `.hrl`: counted as code, no nodes (§30).
    Erlang,
    /// `.ml`, `.mli`: counted as code, no nodes (§31).
    OCaml,
    /// `.jl` (§32).
    Julia,
    /// `.vue` single-file components (script blocks only, §38).
    Vue,
    /// `.svelte` components (script blocks only, §39).
    Svelte,
    /// `.astro` components (frontmatter and script blocks, §40).
    Astro,
    /// `.ps1`, `.psm1`, `.psd1` (§41).
    PowerShell,
    /// `.vb`: counted as code, contributes nothing (the reference lacks its grammar, §42).
    VisualBasic,
    /// `.zig` (§33).
    Zig,
    /// `.m`, `.mm`, and `.h` with Objective-C markers (§34).
    ObjC,
    /// `.f .f90 .f95 .f03 .f08` (§35).
    Fortran,
    /// `.v .sv .svh .vh`: Verilog, not V (§36).
    Verilog,
    /// `.sol`: counted, never extracted (Graphify lacks the grammar, §37).
    Solidity,
    /// `.sql`: tables, views, functions, procedures, triggers and indexes (§45).
    Sql,
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
    /// Code Graphify counts but extracts nothing from (§44.8): `.r`, `.tf`, and perl, fish and
    /// tcsh shebang scripts.
    Inert,
    /// `.ejs` templates: the file node (§44.7).
    Ejs,
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
    /// `.groovy`, `.gradle` (§26).
    Groovy,
    /// `.dart` (§27).
    Dart,
    /// `.lua`, `.luau`, and `lua` shebang scripts (§28).
    Lua,
    /// `.md`/`.mdx` pages of `update`'s markdown structure (`docs/graphify-semantic.md` §3). Never
    /// returned by `classify`: such files are built by a `pipeline::Documents` pass, not here.
    Markdown,
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
    Elixir,
    Julia,
    /// Code files Graphify counts but extracts nothing from (Erlang, OCaml).
    Opaque,
    PowerShell,
    VisualBasic,
    Zig,
    ObjC,
    Fortran,
    Verilog,
    Solidity,
    Sql,
    /// .NET project and markup files (§43).
    Dotnet,
    /// Counted as code, never extracted (§44).
    Inert,
    /// Grammar-less scanners (§44).
    Legacy,
    Groovy,
    Dart,
    Lua,
    /// Markdown pages and headings (`Lang::Markdown`).
    Document,
}

impl Lang {
    /// Classify a path. `shebang` is consulted only for extensionless files.
    pub fn classify(path: &Path) -> Option<Lang> {
        let name = path.file_name()?.to_str()?;
        match name {
            "Cargo.toml" | "pyproject.toml" => return Some(Lang::Toml),
            "go.mod" => return Some(Lang::GoMod),
            "package-lock.json" | "pnpm-lock.yaml" => return None,
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
            "yaml" | "yml" => Lang::Yaml,
            "sh" | "bash" => Lang::Shell,
            "cs" => Lang::CSharp,
            "php" => Lang::Php,
            "rb" | "rake" => Lang::Ruby,
            "kt" | "kts" => Lang::Kotlin,
            "scala" => Lang::Scala,
            "c" => Lang::C,
            "cpp" | "cc" | "cxx" | "hpp" | "cu" => Lang::Cpp,
            "ex" | "exs" => Lang::Elixir,
            "erl" | "hrl" => Lang::Erlang,
            "ml" | "mli" => Lang::OCaml,
            "jl" => Lang::Julia,
            "vue" => Lang::Vue,
            "svelte" => Lang::Svelte,
            "astro" => Lang::Astro,
            "ps1" | "psm1" | "psd1" => Lang::PowerShell,
            "vb" => Lang::VisualBasic,
            "zig" => Lang::Zig,
            "m" | "mm" => Lang::ObjC,
            "f" | "f90" | "f95" | "f03" | "f08" => Lang::Fortran,
            "v" | "sv" | "svh" | "vh" => Lang::Verilog,
            "sol" => Lang::Solidity,
            "sql" => Lang::Sql,
            "h" => return crate::extract::objc::header_lang(path),
            "csproj" | "fsproj" | "vbproj" => Lang::MsBuild,
            "sln" => Lang::Sln,
            "slnx" => Lang::Slnx,
            "razor" | "cshtml" => Lang::Razor,
            "xaml" => Lang::Xaml,
            "r" | "tf" => Lang::Inert,
            "ejs" => Lang::Ejs,
            "metal" => Lang::Cpp,
            "pas" | "dpr" | "pp" | "inc" | "lpr" | "dpk" => Lang::Pascal,
            "dfm" | "lfm" => Lang::DelphiForm,
            "lpk" => Lang::Lpk,
            "cls" | "trigger" => Lang::Apex,
            "cob" | "cbl" | "cpy" => Lang::Cobol,
            "dmf" | "dmi" | "dmm" => Lang::Byond,
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
            Lang::Json | Lang::Toml | Lang::Yaml => Family::Data,
            Lang::Shell => Family::Shell,
            Lang::CSharp => Family::CSharp,
            Lang::Php => Family::Php,
            Lang::Ruby => Family::Ruby,
            Lang::Kotlin => Family::Kotlin,
            Lang::Scala => Family::Scala,
            Lang::C | Lang::Cpp => Family::C,
            Lang::Elixir => Family::Elixir,
            Lang::Erlang | Lang::OCaml => Family::Opaque,
            Lang::Julia => Family::Julia,
            Lang::Vue | Lang::Svelte | Lang::Astro => Family::Script,
            Lang::PowerShell => Family::PowerShell,
            Lang::VisualBasic => Family::VisualBasic,
            Lang::Zig => Family::Zig,
            Lang::ObjC => Family::ObjC,
            Lang::Fortran => Family::Fortran,
            Lang::Verilog => Family::Verilog,
            Lang::Solidity => Family::Solidity,
            Lang::Sql => Family::Sql,
            Lang::MsBuild | Lang::Sln | Lang::Slnx | Lang::Razor | Lang::Xaml => Family::Dotnet,
            Lang::Inert | Lang::Ejs => Family::Inert,
            Lang::Pascal
            | Lang::DelphiForm
            | Lang::Lpk
            | Lang::Apex
            | Lang::Cobol
            | Lang::Byond => Family::Legacy,
            Lang::Groovy => Family::Groovy,
            Lang::Dart => Family::Dart,
            Lang::Lua => Family::Lua,
            Lang::Markdown => Family::Document,
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
            Lang::Elixir => tree_sitter_elixir::LANGUAGE.into(),
            Lang::Julia => tree_sitter_julia::LANGUAGE.into(),
            Lang::Erlang | Lang::OCaml => return None,
            Lang::PowerShell => tree_sitter_powershell::LANGUAGE.into(),
            // Single-file components are masked and parsed as TSX by `extract::sfc`.
            Lang::Vue | Lang::Svelte | Lang::Astro | Lang::VisualBasic => return None,
            Lang::Zig => tree_sitter_zig::LANGUAGE.into(),
            Lang::ObjC => tree_sitter_objc::LANGUAGE.into(),
            Lang::Fortran => tree_sitter_fortran::LANGUAGE.into(),
            Lang::Verilog => tree_sitter_verilog::LANGUAGE.into(),
            Lang::Sql => tree_sitter_sequel::LANGUAGE.into(),
            Lang::Groovy => tree_sitter_groovy::LANGUAGE.into(),
            Lang::Lua => tree_sitter_lua::LANGUAGE.into(),
            Lang::Dart | Lang::Markdown => return None,
            Lang::Json | Lang::Toml | Lang::Yaml | Lang::GoMod | Lang::Shell => return None,
            Lang::Solidity => return None,
            Lang::MsBuild
            | Lang::Sln
            | Lang::Slnx
            | Lang::Razor
            | Lang::Xaml
            | Lang::Inert
            | Lang::Ejs
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
/// the language, by name with an optional version (§14.5, §44.8): `python3.11`, `node18` and
/// `ksh93` count, `Python3`, `pypy3`, `python3-dbg` and `node-18` do not.
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
    Some(match unversioned(prog) {
        "python" => Lang::Python,
        "sh" | "bash" | "zsh" | "ksh" | "dash" => Lang::Shell,
        "ruby" => Lang::Ruby,
        "node" | "nodejs" => Lang::JavaScript,
        "php" => Lang::Php,
        "lua" => Lang::Lua,
        "julia" => Lang::Julia,
        "perl" | "fish" | "tcsh" => Lang::Inert,
        _ => return None,
    })
}

/// `prog` without a trailing version `\d+(\.\d+)*` (`python3.11` → `python`).
fn unversioned(prog: &str) -> &str {
    let base = prog.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    let version = &prog[base.len()..];
    let valid = version.is_empty()
        || (!version.starts_with('.')
            && !version.ends_with('.')
            && !version.contains("..")
            && !base.is_empty());
    if valid {
        base
    } else {
        prog
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

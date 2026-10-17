//! Parity with Graphify's clustered golden for `tests/fixtures/rust`.

mod common;

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Cargo.toml `[package]` node (§11.6): emitted by the TOML extractor, owned elsewhere.
    "pkg_shapes",
    // Q14: generic type parameters (`T` of `Wrapper<T>`/`Pair<T>`, `F` of `apply<F>`) are not
    // type references, so neither their stubs nor the edges to them are emitted.
    "t",
    "f",
    "src_model_pair -references-> t",
    "src_model_pair_t_first -references-> t",
    "src_model_wrapper -references-> t",
    "src_model_wrapper_t_new -references-> t",
    "src_util_mod_apply -references-> f",
    // Q6: the clustered build's file-stem remap is not reproduced, so `use std::fmt` stays on the
    // external `fmt` instead of the unrelated `src/util/fmt.rs`, and `use super::model::*` keeps
    // its external `model` (the file edge to `src/model.rs` comes from module resolution).
    "fmt",
    "model",
    "src_model -imports_from-> fmt",
    "src_model -imports_from-> src_util_fmt",
    "src_util_mod -imports_from-> model",
    // Q7 not reproduced: `use` paths resolve through the module tree to the defining file
    // (`shapes::Round` → the `pub use` in `lib.rs` → `src/model.rs`; `self::fmt::render`).
    "src_lib -imports_from-> src_model",
    "src_lib -imports_from-> src_util_fmt",
    "src_main -imports_from-> src_lib",
    "src_main -imports_from-> src_model",
    "src_main -imports_from-> src_util_fmt",
    "src_util_fmt -imports_from-> src_model",
    "src_util_mod -imports_from-> src_util_fmt",
    "tests_it -imports_from-> src_model",
    "tests_it -imports_from-> src_util_mod",
    // Q23 not reproduced: calls resolve through the module tree. Path-qualified calls
    // (`model::Circle::new`, `shapes::model::classify`, `shapes::util::run_all`), a `use … as`
    // alias (`render_shape` → `util::fmt::render`) and an imported denylisted name (`build`).
    "src_lib_build -calls-> src_model_circle_new",
    "src_main_main -calls-> src_lib_build",
    "src_main_main -calls-> src_model_circle_new",
    "src_main_main -calls-> src_model_classify",
    "src_main_main -calls-> src_util_mod_run_all",
    "src_util_mod_render -calls-> src_util_fmt_render",
];

#[test]
fn rust_fixture_matches_graphify() {
    // Community ids move with the ALLOWED differences (graphify-format.md §17.7). On this 77-node
    // fixture the six Q23 call edges alone move Leiden's partition (NMI 0.72 with them), so the
    // agreement is measured without the ALLOWED links.
    common::assert_fixture(
        "rust",
        ALLOWED,
        common::Communities::MinNmiWithoutAllowedLinks(0.80),
        false,
    );
}

/// Build a probe repository and return its links as `source -relation-> target`.
fn links(files: &[(&str, &str)]) -> BTreeSet<String> {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("probe");
    for (path, text) in files {
        let p = root.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    build(&BuildOptions {
        root: root.clone(),
        out: None,
        threads: 1,
        cluster: false,
    })
    .unwrap();
    let g: Value =
        serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap();
    g["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            format!(
                "{} -{}-> {}",
                e["source"].as_str().unwrap(),
                e["relation"].as_str().unwrap(),
                e["target"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn use_paths_resolve_through_the_module_tree() {
    let l = links(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"a\", \"b\"]\n"),
        ("a/Cargo.toml", "[package]\nname = \"crate-a\"\n"),
        ("a/src/lib.rs", "mod paint;\npub use paint::Brush;\n"),
        ("a/src/paint.rs", "pub struct Brush;\n"),
        (
            "b/Cargo.toml",
            "[package]\nname = \"b\"\n\n[dependencies]\ncrate-a = { path = \"../a\" }\n",
        ),
        (
            "b/src/main.rs",
            "mod flags;\nuse crate_a::Brush;\nuse crate::flags::Flag;\nuse regex::Regex;\n\
             fn regex() {}\nfn main() { use crate::flags::defs::Other; }\n",
        ),
        (
            "b/src/flags/mod.rs",
            "mod defs;\npub use self::defs::Flag;\n#[cfg(test)]\nmod tests {\n    use super::*;\n}\n",
        ),
        (
            "b/src/flags/defs.rs",
            "pub struct Flag;\npub struct Other;\npub struct Color;\n",
        ),
        // `tests/feature.rs` is a module of `tests/tests.rs`, not a test crate of its own.
        ("b/tests/tests.rs", "mod hay;\nmod feature;\n"),
        ("b/tests/hay.rs", "pub const SHERLOCK: &str = \"\";\n"),
        ("b/tests/feature.rs", "use crate::hay::SHERLOCK;\n"),
    ]);
    for want in [
        // Through the `pub use` re-export in `a/src/lib.rs`, across crates.
        "b_src_main -imports_from-> a_src_paint",
        "b_src_main -imports_from-> b_src_flags_defs",
        "b_src_flags_mod -imports_from-> b_src_flags_defs",
        "b_tests_feature -imports_from-> b_tests_hay",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    // `use regex::Regex` is the external crate, not the file's own `fn regex`; `use super::*` in
    // an inline module is the file itself.
    assert!(
        !l.contains("b_src_main -imports_from-> b_src_main"),
        "{l:#?}"
    );
    assert!(
        !l.iter()
            .any(|x| x.starts_with("b_src_flags_mod -imports_from-> b_src_main")),
        "{l:#?}"
    );
}

#[test]
fn imported_types_resolve_through_their_import() {
    let l = links(&[
        ("Cargo.toml", "[package]\nname = \"p\"\n"),
        ("src/lib.rs", "mod defs;\nmod printer;\nmod model;\n"),
        ("src/defs.rs", "pub struct Color;\npub struct Match;\n"),
        // Imported from outside the repository: never the repository's only `Color`.
        (
            "src/printer.rs",
            "use termcolor::Color;\npub fn paint(c: Color) {}\n",
        ),
        // `Match` is declared twice; the import says which one.
        ("src/other.rs", "pub struct Match;\n"),
        (
            "src/model.rs",
            "use crate::defs::Match;\npub fn first(m: Match) {}\n",
        ),
    ]);
    assert!(
        l.contains("src_model_first -references-> src_defs_match"),
        "{l:#?}"
    );
    assert!(
        !l.iter()
            .any(|x| x.starts_with("src_printer") && x.contains("src_defs")),
        "{l:#?}"
    );
}

/// A workspace whose crates call each other through a facade crate (`pub extern crate … as`,
/// `pub use … as`), a renamed dependency (`package = …`), a `workspace = true` dependency, a
/// `[lib] name`, `use … as` aliases and struct field types (graphify-format.md Q23, §11.5).
#[test]
fn calls_resolve_through_crates_aliases_and_field_types() {
    let l = links(&[
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"app\", \"facade\", \"cli\", \"text\", \"regex\"]\n\n\
             [workspace.dependencies]\nacme-text = { path = \"text\" }\n",
        ),
        // A library whose crate name is not its package name.
        (
            "text/Cargo.toml",
            "[package]\nname = \"acme-text\"\n\n[lib]\nname = \"text_lib\"\n",
        ),
        ("text/src/lib.rs", "pub mod glob;\n"),
        (
            "text/src/glob.rs",
            "pub struct Override;\n\
             impl Override {\n    pub fn empty() -> Override { Override }\n    \
             pub fn matched(&self) -> bool { true }\n}\n\
             pub struct Glob;\nimpl Glob { pub fn new() -> Glob { Glob } }\n\
             pub trait Matcher { fn find(&self); }\n",
        ),
        ("cli/Cargo.toml", "[package]\nname = \"acme-cli\"\n"),
        (
            "cli/src/lib.rs",
            "mod decompress;\nmod hostname;\npub use crate::decompress::resolve_binary;\n\
             pub use hostname::hostname;\n",
        ),
        ("cli/src/decompress.rs", "pub fn resolve_binary() {}\n"),
        ("cli/src/hostname.rs", "pub fn hostname() {}\n"),
        // The facade re-exports both crates under short names.
        (
            "facade/Cargo.toml",
            "[package]\nname = \"acme-facade\"\n\n[dependencies]\n\
             acme-cli = { path = \"../cli\" }\nacme-text.workspace = true\n",
        ),
        (
            "facade/src/lib.rs",
            "pub extern crate acme_cli as cli;\npub use text_lib as text;\n",
        ),
        // A workspace package that shares its name with the registry crate `app` uses.
        ("regex/Cargo.toml", "[package]\nname = \"regex\"\n"),
        (
            "regex/src/lib.rs",
            "pub struct Regex;\nimpl Regex { pub fn new() -> Regex { Regex } }\n",
        ),
        (
            "app/Cargo.toml",
            "[package]\nname = \"app\"\n\n[dependencies]\n\
             fac = { package = \"acme-facade\", path = \"../facade\" }\nregex = \"1\"\n",
        ),
        ("app/src/main.rs", "mod search;\nfn main() {}\n"),
        (
            "app/src/search.rs",
            "use fac::cli::resolve_binary as rb;\nuse fac::text::glob::{Glob as G, Matcher};\n\n\
             pub struct Config {\n    globs: fac::text::glob::Override,\n}\n\
             pub struct Worker {\n    config: Config,\n}\n\n\
             impl Worker {\n    fn start() -> Config {\n        \
             fac::cli::resolve_binary();\n        fac::cli::hostname();\n        rb();\n        \
             G::new();\n        regex::Regex::new();\n        \
             Config { globs: fac::text::glob::Override::empty() }\n    }\n    \
             fn should(&self, m: &dyn Matcher) -> bool {\n        Matcher::find(m);\n        \
             self.config.globs.matched()\n    }\n}\n",
        ),
    ]);
    let edge = |from: &str, to: &str| format!("app_src_search_worker_{from} -calls-> {to}");
    for want in [
        // Through the renamed dependency, the facade's `extern crate … as` and `pub use`.
        edge("start", "cli_src_decompress_resolve_binary"),
        // `hostname` is both a module and the function re-exported beside it.
        edge("start", "cli_src_hostname_hostname"),
        // A `[lib] name` reached through `workspace = true`, and a `use … as` alias.
        edge("start", "text_src_glob_glob_new"),
        edge("start", "text_src_glob_override_empty"),
        // `self.config` is a `Config`, whose `globs` is an `Override`.
        edge("should", "text_src_glob_override_matched"),
    ] {
        assert!(l.contains(&want), "missing {want}: {l:#?}");
    }
    // `regex` is the registry crate, not the workspace package of that name; a trait-qualified
    // call dispatches on its argument's type, which is not known here.
    for unwanted in ["regex_src_lib_regex_new", "text_src_glob_matcher_find"] {
        assert!(
            !l.iter()
                .any(|x| x.starts_with("app_") && x.ends_with(&format!("-calls-> {unwanted}"))),
            "unexpected call to {unwanted}: {l:#?}"
        );
    }
}

/// `recv.m()` through the receiver's declared or inferred type (graphify-format.md Q23,
/// §11.5): constructors, struct literals, `let x: T`, parameters, builder chains through declared
/// return types, `?`, `unwrap()`, `Some(x)`, `.await`, `Arc::new`, `T::default()` and fields.
/// Trait objects, generic parameters, shadowed names and closure parameters resolve nothing.
#[test]
fn method_calls_resolve_through_local_types() {
    let l = links(&[
        ("Cargo.toml", "[package]\nname = \"p\"\n"),
        ("src/lib.rs", "pub mod model;\npub mod app;\n"),
        (
            "src/model.rs",
            "pub struct Engine { pub cfg: Config }\n\
             #[derive(Default)]\npub struct Config;\n\
             pub struct Builder;\n\
             pub trait Shape { fn area(&self) -> u32; }\n\
             pub struct Square;\n\
             impl Shape for Square { fn area(&self) -> u32 { 4 } }\n\
             impl Engine {\n    pub fn new() -> Engine { Engine { cfg: Config } }\n    \
             pub fn open() -> std::io::Result<Engine> { Ok(Engine::new()) }\n    \
             pub fn find() -> Option<Self> { None }\n    \
             pub async fn connect() -> Engine { Engine::new() }\n    \
             pub fn start(&self) {}\n    pub fn stop(&self) {}\n    pub fn drain(&self) {}\n    \
             pub fn spin(&self) {}\n    pub fn tick(&self) {}\n    pub fn halt(&self) {}\n    \
             pub fn pause(&self) {}\n    pub fn rest(&self) {}\n}\n\
             impl Config {\n    pub fn check(&self) {}\n    pub fn load(&self) {}\n}\n\
             impl Builder {\n    pub fn new() -> Self { Builder }\n    \
             pub fn color(&mut self, _on: bool) -> &mut Builder { self }\n    \
             pub fn build(&self) -> Engine { Engine::new() }\n}\n",
        ),
        (
            "src/app.rs",
            "use crate::model::{Builder, Config, Engine, Shape, Square};\nuse std::sync::Arc;\n\n\
             pub fn literal() { let e = Engine { cfg: Config }; e.start(); e.cfg.check(); }\n\
             pub fn typed() { let e: Engine = make(); e.stop(); }\n\
             pub fn param(e: &Engine, c: Config) { e.drain(); c.load(); }\n\
             pub fn chain() { Builder::new().color(true).color(false).build().spin(); }\n\
             pub fn fallible() -> std::io::Result<()> { let e = Engine::open()?; e.tick(); Ok(()) }\n\
             pub fn optional() { Engine::find().unwrap().halt(); \
             if let Some(e) = Engine::find() { e.pause(); } }\n\
             pub async fn later() { let e = Engine::connect().await; e.rest(); }\n\
             pub fn shared() { let e = Arc::new(Engine::new()); e.start(); e.clone(); }\n\
             pub fn defaulted() { let c = Config::default(); c.check(); }\n\
             fn make() -> Engine { Engine::new() }\n\
             pub fn dynamic(s: &dyn Shape, t: Box<dyn Shape>) { s.area(); t.area(); }\n\
             pub fn generic<S: Shape>(s: S) { s.area(); }\n\
             pub fn concrete(s: Square) { s.area(); }\n\
             pub fn shadowed() { let e = Engine::new(); let e = 5u8; e.stop(); }\n\
             pub fn closure() { let e = Engine::new(); let f = |e: u8| e.halt(); f(1); }\n\
             pub fn inner() { let e = Engine::new(); { let e = 1u8; e.drain(); } e.spin(); }\n\
             pub fn local_fn() { fn make() -> u8 { 1 } let e = make(); e.tick(); }\n",
        ),
    ]);
    let edge = |from: &str, to: &str| format!("src_app_{from} -calls-> src_model_{to}");
    for want in [
        edge("literal", "engine_start"),
        edge("literal", "config_check"),
        edge("typed", "engine_stop"),
        edge("param", "engine_drain"),
        edge("param", "config_load"),
        edge("chain", "builder_color"),
        edge("chain", "builder_build"),
        edge("chain", "engine_spin"),
        edge("fallible", "engine_tick"),
        edge("optional", "engine_halt"),
        edge("optional", "engine_pause"),
        edge("later", "engine_rest"),
        edge("shared", "engine_start"),
        edge("defaulted", "config_check"),
        // A concrete type's trait impl is its method.
        edge("concrete", "square_area"),
        // The inner block's `e` is a `u8`; the outer one is still an `Engine` after it.
        edge("inner", "engine_spin"),
    ] {
        assert!(l.contains(&want), "missing {want}: {l:#?}");
    }
    for unwanted in [
        // Trait objects and generic parameters dispatch at run time or per instantiation.
        edge("dynamic", "square_area"),
        edge("generic", "square_area"),
        // Shadowed by a `u8`, a closure parameter, an inner block's binding and a nested fn.
        edge("shadowed", "engine_stop"),
        edge("closure", "engine_halt"),
        edge("inner", "engine_drain"),
        edge("local_fn", "engine_tick"),
        // `Arc::clone`, not a method of `Engine`.
        edge("shared", "engine_clone"),
    ] {
        assert!(!l.contains(&unwanted), "unexpected {unwanted}: {l:#?}");
    }
}

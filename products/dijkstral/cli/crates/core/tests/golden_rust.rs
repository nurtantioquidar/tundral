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
];

#[test]
fn rust_fixture_matches_graphify() {
    // Community ids move with the ALLOWED differences (graphify-format.md §17.7).
    common::assert_fixture(
        "rust",
        ALLOWED,
        common::Communities::MinNmi(0.80),
        false,
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

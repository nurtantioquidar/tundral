//! Parity with Graphify's clustered golden for `tests/fixtures/php`, plus focused checks of the
//! PHP rules (docs/graphify-format.md §19) on small probe repositories.

mod common;

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn php_fixture_matches_graphify() {
    common::assert_fixture("php", ALLOWED, common::Communities::Exact, false, false);
}

/// Build a probe repository and return its links as `source -relation-> target [context]`.
fn probe_links(files: &[(&str, &str)]) -> BTreeSet<String> {
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
            let ctx = e["context"]
                .as_str()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            format!(
                "{} -{}-> {}{}",
                e["source"].as_str().unwrap(),
                e["relation"].as_str().unwrap(),
                e["target"].as_str().unwrap(),
                ctx
            )
        })
        .collect()
}

fn has(l: &BTreeSet<String>, want: &str) -> bool {
    l.contains(want)
}

#[test]
fn constants_link_only_names_the_file_types() {
    let l = probe_links(&[
        ("b.php", "<?php\nnamespace B;\nclass Foo {}\nclass Bar {}\n"),
        (
            "a.php",
            "<?php\nuse B\\Foo;\nuse B\\Bar;\nclass K {\n  function f(Foo $x) { Foo::X; Bar::Y; Bar::$p; }\n}\n",
        ),
    ]);
    assert!(
        has(&l, "a_k_f -references_constant-> b_foo [parameter_type]"),
        "{l:#?}"
    );
    assert!(
        !l.iter()
            .any(|x| x.starts_with("a_k_f -") && x.contains("-> b_bar")),
        "{l:#?}"
    );
}

#[test]
fn class_calls_follow_imports_names_and_denylist() {
    let l = probe_links(&[
        (
            "lib/a/caller.php",
            "<?php\nnamespace A;\nuse B\\Bar as Baz;\nuse G\\Proxy;\nclass K {\n  function a() { new Sub\\Thing(); }\n  function b() { new Baz(); }\n  function c() { Proxy::wrap(); }\n  function d() { \\Thing::m(); }\n}\n",
        ),
        (
            "lib/a/typed.php",
            "<?php\nnamespace A;\nclass T {\n  function e(Thing $t) {}\n  function f() { new Thing(); }\n}\n",
        ),
        ("lib/a/thing.php", "<?php\nnamespace A\\Sub;\nclass Thing {}\n"),
        ("lib/b/thing.php", "<?php\nnamespace C;\nclass Thing {}\n"),
        ("lib/b/bar.php", "<?php\nnamespace B;\nclass Bar {}\n"),
        ("lib/g/proxy.php", "<?php\nnamespace G;\nclass Proxy {}\n"),
    ]);
    // Qualified `new` binds by its last segment to the nearest class (INFERRED).
    assert!(
        has(&l, "lib_a_caller_k_a -calls-> lib_a_thing_thing [call]"),
        "{l:#?}"
    );
    // Aliased names, `Proxy`, qualified static calls and names used as types bind nothing.
    for caller in ["b", "c", "d"] {
        assert!(
            !l.iter()
                .any(|x| x.starts_with(&format!("lib_a_caller_k_{caller} -calls->"))),
            "{l:#?}"
        );
    }
    assert!(
        !l.iter().any(|x| x.starts_with("lib_a_typed_t_f -calls->")),
        "{l:#?}"
    );
}

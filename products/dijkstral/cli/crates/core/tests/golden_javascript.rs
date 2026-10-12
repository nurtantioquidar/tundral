//! Parity with Graphify's clustered golden for `tests/fixtures/javascript`, plus probes of the
//! JavaScript-specific shapes (CommonJS, frappe-style class expressions, test callbacks).

mod common;

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn javascript_fixture_matches_graphify() {
    common::assert_fixture(
        "javascript",
        ALLOWED,
        common::Communities::Exact,
        true,
        false,
    );
}

fn build_project(files: &[(&str, &str)]) -> (BTreeSet<String>, BTreeSet<String>) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("p");
    for (rel, body) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    build(&BuildOptions {
        root: root.clone(),
        out: None,
        threads: 2,
        cluster: false,
    })
    .unwrap();
    let g: Value =
        serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap();
    let nodes = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect();
    let links = g["links"]
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
        .collect();
    (nodes, links)
}

#[test]
fn class_expressions_and_handler_objects_become_module_functions() {
    let (nodes, links) = build_project(&[(
        "ctl.js",
        "function helper() {}\n\
         ns.Controller = class Controller extends Base {\n  refresh() { const me = this; me.load(); }\n  load() { helper(); }\n};\n\
         frappe.ui.form.on(\"Doc\", { onload(frm) { helper(); } });\n\
         const settings = { hidden() {} };\n\
         function outer() { return { inner() {} }; }\n",
    )]);
    for id in ["ctl_refresh", "ctl_load", "ctl_onload", "ctl_settings"] {
        assert!(nodes.contains(id), "missing node {id}");
    }
    // A literal initialising a module-level value and objects inside functions stay opaque.
    assert!(!nodes.contains("ctl_hidden") && !nodes.contains("ctl_inner"));
    for e in [
        "ctl_refresh -calls-> ctl_load",
        "ctl_load -calls-> ctl_helper",
        "ctl_onload -calls-> ctl_helper",
    ] {
        assert!(links.contains(e), "missing `{e}` in:\n{links:#?}");
    }
}

#[test]
fn test_callbacks_hoist_declarations_and_attribute_calls_to_the_file() {
    let (nodes, links) = build_project(&[(
        "t.test.js",
        "describe(\"x\", () => {\n  function setup() {}\n  it(\"y\", () => { setup(); });\n  run(later);\n  function later() {}\n});\n",
    )]);
    assert!(nodes.contains("t_test_setup") && nodes.contains("t_test_later"));
    // The call comes first, so `setup` keeps `contains` (with `context: call` after the fold);
    // `later` is only passed, so the file's `indirect_call` wins over `contains`.
    assert!(links.contains("t_test -contains-> t_test_setup"));
    assert!(links.contains("t_test -indirect_call-> t_test_later"));
}

#[test]
fn commonjs_requires_bind_names_and_stub_missing_ones() {
    let (nodes, links) = build_project(&[
        ("lib.js", "function a() {}\nmodule.exports = { a };\nexports.b = function () {};\n"),
        (
            "main.js",
            "const { a, b, zz } = require(\"./lib\");\nconst lib = require(\"./lib\");\nfunction go() { a(); }\n",
        ),
    ]);
    for e in [
        "main -imports_from-> lib",
        "main -imports-> lib_a",
        "main -imports-> lib_b",
        "main -imports-> lib_zz",
        "main -contains-> main_lib",
        "main_go -calls-> lib_a",
        "lib -indirect_call-> lib_a",
    ] {
        assert!(links.contains(e), "missing `{e}` in:\n{links:#?}");
    }
    assert!(nodes.contains("lib_zz"));
}

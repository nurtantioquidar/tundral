//! Parity with Graphify's clustered golden for `tests/fixtures/typescript`, plus probes of the
//! cross-file rules (module resolution, re-export chains, typed receivers) on small projects.

mod common;

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Q9: names imported through the `src/models/index.ts` barrel leave a dangling stub next to
    // the real edge in Graphify. Dijkstral resolves `export *` chains and emits only the real one.
    "src_models_index_admin",
    "src_models_index_helper",
    "src_models_index_user",
    "src_service -imports-> src_models_index_admin",
    "src_service -imports-> src_models_index_helper",
    "src_service -imports-> src_models_index_user",
    "src_tests_service_test -imports-> src_models_index_user",
    "src_ui_app -imports-> src_models_index_user",
    // Q8: `console.log(…)` and `admin.promote(…)` bind to the last same-file method of that name
    // (self-loops). Dijkstral binds member calls by receiver: `admin: Administrator` is the
    // imported `Admin`, so `admin.promote()` is an INFERRED call to `Admin.promote`.
    "src_service_userservice_log -calls-> src_service_userservice_log",
    "src_service_userservice_promote -calls-> src_service_userservice_promote",
    "src_service_userservice_promote -calls-> src_models_user_admin_promote",
    // Q9 consequence: Graphify cannot see `User` through the barrel from a method, so
    // `new User(name)` in `register` is dropped and its `references (return_type)` survives the
    // one-link-per-pair fold; dijkstral resolves the call, which outranks the reference (§8.2).
    "src_service_userservice_register -references-> src_models_user_user",
    "src_service_userservice_register -calls-> src_models_user_user",
    // Q9 consequence: `.map(modelHelper)` passes the barrel-imported `helper` (aliased).
    "src_service_userservice_export -indirect_call-> src_models_user_helper",
];

#[test]
fn typescript_fixture_matches_graphify() {
    // Community ids move with the ALLOWED differences (NMI 0.95, graphify-format.md §17.7).
    common::assert_fixture("typescript", ALLOWED, 0.2, false, false);
}

/// Build a throwaway project and return its node ids and `source -relation-> target` links.
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

fn assert_links(links: &BTreeSet<String>, expected: &[&str]) {
    for e in expected {
        assert!(links.contains(*e), "missing `{e}` in:\n{links:#?}");
    }
}

#[test]
fn specifiers_resolve_through_tsconfig_paths_workspaces_and_extensions() {
    let (nodes, links) = build_project(&[
        (
            "tsconfig.json",
            "{ // comments and trailing commas are fine\n \"compilerOptions\": { \"baseUrl\": \".\", \"paths\": { \"~/*\": [\"lib/*\"], } } }",
        ),
        ("lib/svc.ts", "export function svc() {}\n"),
        ("lib/dir/index.ts", "export const VALUE = 1;\n"),
        ("pkg/package.json", "{ \"name\": \"@acme/pkg\", \"exports\": { \".\": \"./src/main.ts\", \"./sub\": \"./src/sub.ts\" } }"),
        ("pkg/src/main.ts", "export function main() {}\n"),
        ("pkg/src/sub.ts", "export function sub() {}\n"),
        ("app/util.ts", "export function slug() {}\n"),
        (
            "app/main.ts",
            "import { svc } from \"~/svc\";\nimport { VALUE } from \"../lib/dir\";\nimport { main } from \"@acme/pkg\";\nimport { sub } from \"@acme/pkg/sub\";\nimport { slug } from \"./util.js\";\nimport \"./missing\";\nimport \"./style.css\";\nimport React from \"react\";\nexport function run() { svc(); main(); sub(); slug(); }\n",
        ),
        ("app/style.css", ".a {}\n"),
    ]);
    assert_links(
        &links,
        &[
            "app_main -imports_from-> lib_svc",
            "app_main -imports-> lib_svc_svc",
            "app_main -imports_from-> lib_dir_index",
            "app_main -imports-> lib_dir_index_value",
            "app_main -imports_from-> pkg_src_main",
            "app_main -imports_from-> pkg_src_sub",
            "app_main -imports_from-> app_util",
            "app_main -imports_from-> ref_missing",
            "app_main -imports_from-> app_style",
            "app_main -imports_from-> ref_react",
            "app_main_run -calls-> lib_svc_svc",
            "app_main_run -calls-> pkg_src_main_main",
            "app_main_run -calls-> pkg_src_sub_sub",
            "app_main_run -calls-> app_util_slug",
        ],
    );
    assert!(nodes.contains("ref_react") && nodes.contains("app_style"));
}

#[test]
fn re_export_chains_reach_the_definition_without_a_barrel_stub() {
    let (nodes, links) = build_project(&[
        ("m/a.ts", "export function deep() {}\nexport default class Thing {}\n"),
        ("m/b.ts", "export * from \"./a\";\n"),
        (
            "m/index.ts",
            "export * from \"./b\";\nexport { default as Thing } from \"./a\";\nimport { deep as d2 } from \"./a\";\nexport { d2 };\n",
        ),
        (
            "main.ts",
            "import { deep, Thing, d2, nope } from \"./m\";\nexport function go() { deep(); new Thing(); d2(); }\n",
        ),
    ]);
    assert_links(
        &links,
        &[
            "main -imports-> m_a_deep",
            "main -imports-> m_a_thing",
            "main -imports-> m_index_nope",
            "main_go -calls-> m_a_deep",
            "main_go -calls-> m_a_thing",
            "m_index -re_exports-> m_b",
            "m_index -re_exports-> m_a",
        ],
    );
    // Q9 not reproduced: resolvable names leave no `m_index_deep` stub; unresolvable ones do.
    assert!(!nodes.contains("m_index_deep"));
    assert!(nodes.contains("m_index_nope"));
}

#[test]
fn member_calls_bind_by_receiver_type_not_by_method_name() {
    let (_, links) = build_project(&[
        ("svc.ts", "export class Svc { go() {} static make() {} }\nexport class Other { log() {} }\n"),
        (
            "use.ts",
            "import { Svc } from \"./svc\";\n\
             export class User {\n  svc: Svc;\n  log() {}\n  run(s: Svc) {\n    s.go();\n    Svc.make();\n    this.svc.go();\n    console.log(1);\n    const me = this;\n    me.log();\n  }\n}\n\
             export function f() { const v = new Svc(); v.go(); }\n",
        ),
    ]);
    assert_links(
        &links,
        &[
            "use_user_run -calls-> svc_svc_go",
            "use_user_run -calls-> svc_svc_make",
            "use_user_run -calls-> use_user_log",
            "use_f -calls-> svc_svc_go",
        ],
    );
    // Q8 not reproduced: `console.log` does not bind to the same-file `log` method.
    assert!(!links.contains("use_user_log -calls-> use_user_log"));
}

//! Parity with Graphify's clustered golden for `tests/fixtures/java`.

mod common;
mod probe;

use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Q11/J1: `import com.shop.model.*;` targets Graphify's class `Model` because the package's
    // last segment `model` equals that class's stub id; dijkstral keeps the external `model`.
    "model",
    "src_main_java_com_shop_service_userservice -imports-> model",
    "src_main_java_com_shop_service_userservice -imports-> src_main_java_com_shop_model_model_model",
    // Q11/J2: `import static com.shop.service.Formatter.slug;` links every class named
    // `Formatter`; dijkstral links only the one its FQN names (the main `Formatter`).
    "src_main_java_com_shop_service_userservice -imports-> src_test_java_com_shop_service_userservicetest_formatter",
];

#[test]
fn java_fixture_matches_graphify() {
    // Community ids move with the ALLOWED differences (NMI 0.92), and the extract-time sidecars
    // with them (graphify-format.md §17.7).
    common::assert_fixture("java", ALLOWED, common::Communities::MinNmi(0.85), false);
}

/// The node `id` of a probe graph.
fn node<'g>(g: &'g Value, id: &str) -> &'g Value {
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node {id}"))
}

/// `callee@receiver_type@line@file` of a node's `metadata.unresolved_calls`.
fn unresolved(n: &Value) -> Vec<String> {
    n["metadata"]["unresolved_calls"]
        .as_array()
        .map(|l| {
            l.iter()
                .map(|c| {
                    format!(
                        "{}@{}@{}@{}",
                        c["callee"].as_str().unwrap(),
                        c["receiver_type"].as_str().unwrap(),
                        c["line"].as_str().unwrap(),
                        c["file"].as_str().unwrap()
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `_java_ret` (graphify-format.md §13.1): the simple name of a declared return type, except
/// arrays, primitives, type parameters and the builtin types; merged overloads keep it only
/// when they agree.
#[test]
fn java_return_types() {
    let g = probe::graph(&[(
        "p/O.java",
        "package p;\n\nimport org.lib.Thing;\n\npublic class O<T> {\n    public a.B qualified() { return null; }\n    public Box<B> box() { return null; }\n    public Map.Entry<String, B> entry() { return null; }\n    public Thing thing() { return null; }\n    public B[] arr() { return null; }\n    public T t() { return null; }\n    public UUID uuid() { return null; }\n    public String s() { return null; }\n    public A same(int x) { return null; }\n    public A same(String y) { return null; }\n    public A diff(int x) { return null; }\n    public void diff(String y) {}\n    public O() {}\n}\n",
    )]);
    let ret = |m: &str| {
        node(&g, &format!("p_o_o_{m}"))["_java_ret"]
            .as_str()
            .map(str::to_string)
    };
    for (m, want) in [
        ("qualified", Some("B")),
        ("box", Some("Box")),
        ("entry", Some("Entry")),
        ("thing", Some("Thing")),
        ("same", Some("A")),
        ("arr", None),
        ("t", None),
        ("uuid", None),
        ("s", None),
        ("diff", None),
        ("o", None),
    ] {
        assert_eq!(ret(m).as_deref(), want, "{m}");
    }
}

/// `metadata.unresolved_calls` (§13.1): entries carry `file`; a receiver naming a class-like
/// node of another language (a Kotlin class, object or anonymous object) is not unresolved.
#[test]
fn java_unresolved_calls_skip_other_languages_classes() {
    let g = probe::graph(&[
        (
            "j/J.java",
            "package j;\n\npublic class J {\n    void run(KtThing t, KtAnon a, Thing x) {\n        t.go();\n        a.spin();\n        x.go();\n        KtObj.z();\n        Math.max(1, 2);\n    }\n}\n",
        ),
        (
            "k/K.kt",
            "package k\n\nclass KtThing { fun go() {} }\nobject KtObj\nfun f() { val o = object : KtAnon() {} }\n",
        ),
    ]);
    assert_eq!(
        unresolved(node(&g, "j_j_j_run")),
        ["go@Thing@L7@j/J.java", "max@Math@L9@j/J.java"]
    );
}

//! Parity with Graphify's clustered golden for `tests/fixtures/kotlin`.

mod common;
mod probe;

use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §21).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn kotlin_fixture_matches_graphify() {
    common::assert_fixture("kotlin", ALLOWED, common::Communities::Exact, true);
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

/// Type aliases are class-like nodes that references bind to; type parameters are never
/// referenced (graphify-format.md §21.1).
#[test]
fn kotlin_type_aliases_and_type_parameters() {
    let src = "package a\n\nfun early(x: Later): Later = x\n\ntypealias Later = Thing\n\nclass Thing\nclass Q<T : Thing>(val t: T) { fun <U : T> m(u: U): List<U> = listOf(u) }\nfun stray(): T? = null\n";
    let l = probe::links(&[("a/A.kt", src)]);
    for want in [
        "a_a -contains-> a_a_later",
        "a_a_early -references-> a_a_later [return_type]",
        "a_a_stray -references-> t [return_type]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    assert!(
        !l.iter()
            .any(|x| x.starts_with("a_a_q") && x.contains("-references-> u")),
        "{l:#?}"
    );
    assert!(!l.contains("a_a_q -references-> t [field]"), "{l:#?}");
    let g = probe::graph(&[("a/A.kt", src)]);
    assert_eq!(node(&g, "a_a_later")["_callable_class"], true);
}

/// `metadata.unresolved_calls` (§21.4): `Type.m()` whose `Type` names no class-like node of a
/// Kotlin file (anonymous objects and aliases count, Java classes do not) and is not a denied
/// receiver; once per (callee, type), with `file`; property initializers record on their class.
#[test]
fn kotlin_unresolved_calls() {
    let g = probe::graph(&[
        (
            "a/A.kt",
            "package a\n\ntypealias Al = String\n\nclass C {\n    private val log = LoggerFactory.getLogger(\"x\")\n    fun f() {\n        Integer.valueOf(1)\n        Integer.valueOf(2)\n        Math.max(1, 2)\n        UUID.randomUUID()\n        Al.make()\n        JavaThing.go()\n        Anon.go()\n        val o = object : Anon() {}\n    }\n}\n",
        ),
        ("j/JavaThing.java", "package j;\n\npublic class JavaThing {}\n"),
    ]);
    assert_eq!(
        unresolved(node(&g, "a_a_c")),
        ["getLogger@LoggerFactory@L6@a/A.kt"]
    );
    assert_eq!(
        unresolved(node(&g, "a_a_c_f")),
        ["valueOf@Integer@L8@a/A.kt", "go@JavaThing@L13@a/A.kt"]
    );
}

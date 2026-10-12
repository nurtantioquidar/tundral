//! Parity with Graphify's clustered golden for `tests/fixtures/python`, plus focused checks of the
//! Python rules (docs/graphify-format.md §9) on small probe repositories.

mod common;

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Q2: in the import cycle `pkg/__init__.py` ↔ `pkg/service.py` the collapse keeps the
    // higher-ranked `re_exports` (§8.2) instead of Graphify's larger-source `imports_from`.
    "pkg_service -imports_from-> pkg_init",
    "pkg_init -re_exports-> pkg_service",
    // `U(name=name)` with `User as U`: the aliased constructor call is resolved (Graphify drops
    // it when the alias also annotates the function), so `calls` outranks the return-type
    // `references` on that pair, and `user.save()` resolves through the typed receiver.
    "pkg_service_userservice_register -references-> pkg_models_user",
    "pkg_service_userservice_register -calls-> pkg_models_user",
    "pkg_service_userservice_register -calls-> pkg_models_user_save",
    // Q12: a call qualified by a local module (`models.validate(user)`) is resolved.
    "pkg_service_userservice_register -calls-> pkg_models_validate",
    // An aliased import passed as an argument (`map(util_helper, …)`) is resolved like the
    // aliased call it names (§9.3 notes Graphify leaves it unresolved).
    "pkg_service_userservice_export -indirect_call-> pkg_util_helper",
    // `osp.basename(path)` in top-level `run` follows §9.4 (`import x.y as z` in a top-level
    // function → the external node); Graphify emits this form in isolation but not here.
    "pkg_service_run -calls-> os_path",
];

#[test]
fn python_fixture_matches_graphify() {
    common::assert_golden("python", 1.0, 1.0, ALLOWED);
}

/// Build a probe repository and return its links as `source -relation-> target [context]`.
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

fn has(links: &BTreeSet<String>, l: &str) -> bool {
    links.contains(l)
}

#[test]
fn reexports_are_followed_and_stubs_resolve_by_import() {
    let l = links(&[
        ("pkg/__init__.py", "from .m import K\n"),
        ("pkg/m.py", "class K:\n    pass\n"),
        (
            "use.py",
            "from pkg import K\nfrom ext import ExtBase\n\n\nclass E(K, ExtBase):\n    pass\n\n\ndef h(k: K) -> ExtBase:\n    K()\n",
        ),
    ]);
    for want in [
        "use -imports-> pkg_m_k [import]",
        "use_e -inherits-> pkg_m_k",
        "use_e -inherits-> extbase",
        "use_h -calls-> pkg_m_k [call]",
        "use_h -references-> extbase [return_type]",
        "pkg_init -re_exports-> pkg_m [export]",
    ] {
        assert!(has(&l, want), "missing {want}: {l:#?}");
    }
    // `uses` needs a direct import from the defining module (re-exports are not followed).
    assert!(!l.iter().any(|x| x.contains("-uses->")), "{l:#?}");
}

#[test]
fn stubs_shared_by_files_are_prefixed() {
    let l = links(&[
        ("a.py", "def f(x: Dup) -> None:\n    pass\n"),
        ("b.py", "def g(x: Dup) -> None:\n    pass\n"),
        ("base.py", "X = 1\n"),
        ("sub.py", "class S(Base):\n    pass\n"),
    ]);
    assert!(
        has(&l, "a_f -references-> a_py_dup [parameter_type]"),
        "{l:#?}"
    );
    assert!(
        has(&l, "b_g -references-> b_py_dup [parameter_type]"),
        "{l:#?}"
    );
    // A stub never renames a file node (Q4 is not reproduced).
    assert!(has(&l, "sub_s -inherits-> sub_py_base"), "{l:#?}");
    assert!(has(&l, "base -contains-> base_x") || !l.iter().any(|x| x.contains("base_py_base")));
}

#[test]
fn calls_follow_scopes_imports_and_heuristics() {
    let l = links(&[
        (
            "b.py",
            "def lonely_b():\n    pass\n\n\nclass Base:\n    def inherited(self):\n        pass\n",
        ),
        ("n.py", "def foo():\n    pass\n\n\ndef only_n():\n    pass\n"),
        (
            "a.py",
            "from b import Base\nfrom n import foo as al\nimport n\n\n\ndef caller(cb):\n    lonely_b()\n    al()\n    n.only_n()\n    cb()\n    print(1)\n\n\nclass K(Base):\n    def m(self):\n        self.inherited()\n        super().inherited()\n\n\ndef typed(k: K):\n    k.m()\n",
        ),
    ]);
    for want in [
        "a_caller -calls-> b_lonely_b [call]",
        "a_caller -calls-> n_foo [call]",
        "a_caller -calls-> n_only_n [call]",
        "a_k_m -calls-> b_base_inherited [call]",
        "a_typed -calls-> a_k_m [call]",
    ] {
        assert!(has(&l, want), "missing {want}: {l:#?}");
    }
    // A parameter shadows any global of the same name.
    assert!(!l
        .iter()
        .any(|x| x.starts_with("a_caller -calls-> ") && x.contains("cb")));
}

#[test]
fn indirect_calls_skip_names_already_called() {
    let l = links(&[(
        "a.py",
        "def helper(x):\n    return x\n\n\ndef helper2(x):\n    return x\n\n\nHANDLERS = {\"h\": helper}\n\n\ndef outer(xs):\n    def inner(x):\n        return x\n\n    inner(1)\n    sorted(xs, key=inner)\n    map(helper2, xs)\n    helper2(3)\n    return inner\n",
    )]);
    assert!(
        has(&l, "a -indirect_call-> a_helper [collection]"),
        "{l:#?}"
    );
    assert!(
        has(&l, "a_outer -indirect_call-> a_helper2 [argument]"),
        "{l:#?}"
    );
    // `inner` was called before it was passed: the pair keeps `contains` with a call context.
    assert!(
        has(&l, "a_outer -contains-> a_outer_inner [call]"),
        "{l:#?}"
    );
}

#[test]
fn nested_signatures_and_rationale() {
    let l = links(&[
        ("m.py", "class Foo:\n    pass\n"),
        (
            "a.py",
            "\"\"\"Module docstring that is long enough.\"\"\"\nfrom m import Foo\n\n\ndef outer():\n    # NOTE: the inner signature records no references\n    def inner(x: Foo) -> Foo:\n        pass\n\n    return inner\n",
        ),
    ]);
    assert!(
        !l.iter()
            .any(|x| x.starts_with("a_outer_inner -references->")),
        "{l:#?}"
    );
    assert!(has(&l, "a_outer -uses-> m_foo"), "{l:#?}");
    assert!(has(&l, "a_rationale_1 -rationale_for-> a"), "{l:#?}");
    assert!(has(&l, "a_rationale_6 -rationale_for-> a"), "{l:#?}");
}

#[test]
fn same_file_id_clashes() {
    // Probe p21: plain functions and methods keep apart with a name hash; classes, nested and
    // decorated functions merge into the first definition.
    let l = links(&[(
        "a.py",
        "def get_x():\n    pass\n\n\ndef _get_x():\n    def inner():\n        pass\n\n\ndef Get_Y():\n    pass\n\n\ndef get_y():\n    pass\n\n\nclass K:\n    def m(self):\n        pass\n\n    @classmethod\n    def _m(cls):\n        pass\n\n\nclass Foo:\n    pass\n\n\ndef foo():\n    pass\n",
    )]);
    for want in [
        "a -contains-> a_get_x",
        "a -contains-> a_get_x_d74068",
        "a_get_x_d74068 -contains-> a_get_x_d74068_inner",
        "a -contains-> a_get_y_966285",
        "a -contains-> a_get_y_9d4c8e",
        "a_k -method-> a_k_m",
        "a -contains-> a_foo",
    ] {
        assert!(has(&l, want), "missing {want}: {l:#?}");
    }
    assert!(!l.iter().any(|x| x.contains("a_k_m_")), "{l:#?}");
}

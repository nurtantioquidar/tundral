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
    // An aliased import passed as an argument (`map(util_helper, …)`) is resolved like the
    // aliased call it names (§9.3 notes Graphify leaves it unresolved).
    "pkg_service_userservice_export -indirect_call-> pkg_util_helper",
    // `osp.basename(path)` in top-level `run` follows §9.4 (`import x.y as z` in a top-level
    // function → the external node); Graphify emits this form in isolation but not here.
    "pkg_service_run -calls-> os_path",
    // A string annotation (`admin: "Admin"`) names its type like an unquoted one; Graphify
    // skips forward references (§9.1), the SCIP oracle counts them.
    "pkg_service_userservice_promote -references-> pkg_models_admin",
];

#[test]
fn python_fixture_matches_graphify() {
    // Community ids move with the ALLOWED differences (graphify-format.md §17.7).
    common::assert_fixture("python", ALLOWED, common::Communities::MinNmi(0.90), false);
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
    // Probe p21: module-level functions and classes and the methods of module-level classes,
    // decorated or not, keep apart with a name hash; nested definitions merge into the first.
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
        "a_k -method-> a_k_m_75f7d7",
        "a -contains-> a_foo_201a6b",
        "a -contains-> a_foo_0beec7",
    ] {
        assert!(has(&l, want), "missing {want}: {l:#?}");
    }
}

#[test]
fn private_imported_names_never_produce_uses() {
    // Probe: Graphify emits `uses` for an imported class unless its original name starts with
    // `_` (the alias does not matter); the `references` edge stays.
    let l = links(&[
        ("pkg/__init__.py", ""),
        (
            "pkg/models.py",
            "class Pub:\n    pass\n\n\nclass _Priv:\n    pass\n",
        ),
        (
            "pkg/use.py",
            "from pkg.models import Pub as _P, _Priv, _Priv as Q\n\n\ndef a(x: _P):\n    pass\n\n\ndef b(x: _Priv):\n    pass\n\n\ndef c(x: Q):\n    pass\n",
        ),
    ]);
    assert!(
        has(&l, "pkg_use_a -uses-> pkg_models_pub [parameter_type]"),
        "{l:#?}"
    );
    assert!(
        has(
            &l,
            "pkg_use_b -references-> pkg_models_priv [parameter_type]"
        ),
        "{l:#?}"
    );
    assert!(
        has(
            &l,
            "pkg_use_c -references-> pkg_models_priv [parameter_type]"
        ),
        "{l:#?}"
    );
    assert!(
        !l.iter().any(|x| x.contains("-uses-> pkg_models_priv")),
        "{l:#?}"
    );
}

#[test]
fn absolute_imports_resolve_next_to_the_importer() {
    // A script directory imports its siblings by bare name (`sys.path[0]`, Pyright's local
    // directory fallback); stdlib names never resolve by directory.
    let l = links(&[
        ("raw/models.py", "class Request:\n    pass\n"),
        ("raw/json.py", "def dumps():\n    pass\n"),
        (
            "raw/auth.py",
            "import json\nfrom models import Request\n\n\ndef flow(r: Request) -> \"Request\":\n    json.dumps(r)\n",
        ),
        (
            "raw/sub/deep.py",
            "import models\n\n\ndef f():\n    models.Request()\n",
        ),
    ]);
    for want in [
        "raw_auth -imports_from-> raw_models [import]",
        "raw_auth -imports-> raw_models_request [import]",
        "raw_auth -imports-> json [import]",
        "raw_sub_deep -imports-> raw_models [import]",
        "raw_sub_deep_f -calls-> raw_models_request [call]",
    ] {
        assert!(has(&l, want), "missing {want}: {l:#?}");
    }
    assert!(
        !l.iter()
            .any(|x| x.starts_with("raw_auth") && x.contains("raw_json")),
        "{l:#?}"
    );
}

#[test]
fn names_imported_from_outside_never_bind_by_name() {
    // `Response` comes from an external package (directly, or re-exported by a local package
    // from one): a same-named class in the repository is not its definition. A class defined
    // inside a function is local to it.
    let l = links(&[
        (
            "app/__init__.py",
            "from starlette.responses import Response\n",
        ),
        (
            "app/models.py",
            "class Response:\n    pass\n\n\nclass State:\n    pass\n",
        ),
        (
            "app/handlers.py",
            "from starlette.exceptions import HTTPException\nfrom app import Response\n\n\ndef h():\n    raise HTTPException(404)\n\n\ndef g():\n    return Response()\n",
        ),
        ("app/errors.py", "class HTTPException(Exception):\n    pass\n"),
        (
            "app/local.py",
            "def f():\n    class State:\n        pass\n\n    return State()\n",
        ),
    ]);
    for bad in [
        "-calls-> app_errors_httpexception",
        "-calls-> app_models_response",
        "-calls-> app_models_state",
    ] {
        assert!(
            !l.iter().any(|x| x.contains(bad)),
            "unexpected {bad}: {l:#?}"
        );
    }
}

#[test]
fn class_fields_and_string_annotations_reference_their_types() {
    let l = links(&[
        ("m.py", "class Fact:\n    pass\n\n\nclass Slice:\n    pass\n"),
        (
            "a.py",
            "from m import Fact, Slice\n\n\nclass Facts:\n    items: list[Fact]\n    parent: \"Facts | None\" = None\n\n\nclass _Local:\n    pass\n\n\nclass Holder:\n    item: _Local\n\n\ndef unit(u: \"Path | Slice\") -> None:\n    pass\n",
        ),
    ]);
    for want in [
        // A mention of an imported class from the class body is also a `uses`, which outranks
        // the field reference in the collapse (§8.2).
        "a_facts -uses-> m_fact",
        "a_holder -references-> a_local [field]",
        "a_unit -references-> m_slice [parameter_type]",
    ] {
        assert!(
            l.iter().any(|x| x.starts_with(want)),
            "missing {want}: {l:#?}"
        );
    }
    // A recursive field is not a link from the class to itself.
    assert!(
        !l.iter()
            .any(|x| x.starts_with("a_facts -references-> a_facts")),
        "{l:#?}"
    );
}

#[test]
fn receivers_typed_by_attributes_annotations_and_cls() {
    // Probe: `self.attr = Cls()` or `self.attr: Cls` in any method types `self.attr.m()` in
    // every method of the class; annotated locals type their variable; `cls.m()` in a
    // classmethod is the class's own method.
    let l = links(&[
        ("pkg/__init__.py", ""),
        (
            "pkg/q.py",
            "class Queue:\n    def add_stock(self, n):\n        pass\n\n\nclass Stack:\n    def only_stack(self):\n        pass\n",
        ),
        (
            "pkg/t.py",
            "from pkg.q import Queue, Stack\n\n\nclass T:\n    def test_b(self):\n        self.later.only_stack()\n\n    def other(self):\n        self.later = Stack()\n        self.q: Queue = make()\n\n    def go(self):\n        self.q.add_stock(1)\n\n    @classmethod\n    def mk(cls):\n        return cls.helper()\n\n    @classmethod\n    def helper(cls):\n        pass\n\n\ndef local_annot(rows):\n    x: Queue = rows[0]\n    x.add_stock(1)\n\n\ndef make():\n    pass\n",
        ),
    ]);
    for want in [
        "pkg_t_t_test_b -calls-> pkg_q_stack_only_stack [call]",
        "pkg_t_t_go -calls-> pkg_q_queue_add_stock [call]",
        "pkg_t_t_mk -calls-> pkg_t_t_helper [call]",
        "pkg_t_local_annot -calls-> pkg_q_queue_add_stock [call]",
    ] {
        assert!(has(&l, want), "missing {want}: {l:#?}");
    }
}

#[test]
fn function_imports_shadow_module_names_and_merged_defs_take_no_calls() {
    let l = links(&[
        ("ledger.py", "def make_entries():\n    pass\n"),
        (
            "a.py",
            "def make_entries():\n    from ledger import make_entries\n\n    make_entries()\n\n\n@deco\ndef rate():\n    pass\n\n\ndef _rate():\n    pass\n\n\ndef deco(f):\n    return f\n",
        ),
        ("b.py", "from a import _rate\n\n\ndef use():\n    _rate()\n"),
    ]);
    assert!(
        has(&l, "a_make_entries -calls-> ledger_make_entries [call]"),
        "{l:#?}"
    );
    assert!(
        !has(&l, "a_make_entries -calls-> a_make_entries [call]"),
        "{l:#?}"
    );
    // `_rate` lost its id to the decorated `rate` above it (Q5): a call to it is not a call to
    // `rate`; the import still links the files.
    assert!(!has(&l, "b_use -calls-> a_rate [call]"), "{l:#?}");
    assert!(has(&l, "b -imports_from-> a [import]"), "{l:#?}");
}

//! Parity with Graphify's clustered golden for `tests/fixtures/go`.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Q24 not reproduced: a method call on a receiver whose type a declaration fixes resolves to
    // that type's method in another file (`svc := NewUserService(); svc.Register(..)`; the
    // field `s.users` is a `*store.Store[User]`).
    "cmd_app_main_main -calls-> service_userservice_register",
    "service_service_test_testregister -calls-> service_userservice_register",
    "service_userservice_register -calls-> store_store_put",
    // Q24: `u := User{..}; u.Identity()`, promoted from the embedded `store.Model`.
    "service_userservice_register -calls-> store_model_identity",
    // Q24: `store.NewStore[User]()`, an explicitly instantiated generic function.
    "service_service_newuserservice -calls-> store_store_newstore",
];

#[test]
fn go_fixture_matches_graphify() {
    // Community ids move with the ALLOWED differences (graphify-format.md §17.7): on this
    // 45-node fixture the five Q24 call edges move Leiden's partition (NMI 0.91 with them), so
    // agreement is measured without the ALLOWED links, and the report goldens by section.
    common::assert_fixture(
        "go",
        ALLOWED,
        common::Communities::MinNmiWithoutAllowedLinks(0.95),
        false,
    );
}

/// Build a probe repository and return its `graph.json`.
fn graph(files: &[(&str, &str)]) -> Value {
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
    serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap()
}

/// Build a probe repository and return its links as `source -relation-> target`.
fn links(files: &[(&str, &str)]) -> BTreeSet<String> {
    let g = graph(files);
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

/// `recv.M()` through the receiver's declared or inferred type (graphify-format.md Q24, §12.2):
/// constructors, composite literals, `var x T`, parameters, multi-value results, chains through
/// declared results, fields, embedded (promoted) methods, package-level variables, generic
/// instantiations, type switches and assertions. Interfaces, type parameters and shadowed names
/// resolve nothing.
#[test]
fn method_calls_resolve_through_local_types() {
    let l = links(&[
        ("go.mod", "module ex.com/p\n"),
        (
            "store/store.go",
            "package store\n\n\
             type Engine struct {\n\tCfg Config\n\tBase\n}\n\n\
             type Base struct{}\n\nfunc (b *Base) Reset() {}\n\n\
             type Config struct{}\n\nfunc (c Config) Check() {}\n\n\
             type Runner interface {\n\tRun()\n}\n\n\
             type Job struct{}\n\nfunc (j *Job) Run() {}\n\n\
             func NewEngine() *Engine { return &Engine{} }\n\
             func Open() (*Engine, error) { return &Engine{}, nil }\n\
             func Make[T any]() *Engine { return &Engine{} }\n\n\
             var Default = NewEngine()\n",
        ),
        (
            "store/engine.go",
            "package store\n\n\
             func (e *Engine) Start() {}\nfunc (e *Engine) Stop() {}\n\
             func (e *Engine) Spin() {}\nfunc (e *Engine) Tick() {}\nfunc (e *Engine) Halt() {}\n\
             func (e *Engine) WithName(n string) *Engine { return e }\n",
        ),
        (
            "app/app.go",
            "package app\n\nimport \"ex.com/p/store\"\n\n\
             func Ctor() { e := store.NewEngine(); e.Start() }\n\
             func Literal() { e := &store.Engine{}; e.Stop(); e.Cfg.Check() }\n\
             func Multi() { e, err := store.Open(); _ = err; e.Spin() }\n\
             func Param(e *store.Engine) { e.Tick() }\n\
             func Chain() { store.NewEngine().WithName(\"x\").Halt() }\n\
             func Var() { var e store.Engine; e.Start() }\n\
             func Promoted(e *store.Engine) { e.Reset() }\n\
             func PkgVar() { store.Default.Stop() }\n\
             func Generic() { store.Make[int]().Spin() }\n\
             func Switch(x any) {\n\tswitch v := x.(type) {\n\tcase *store.Job:\n\t\tv.Run()\n\t}\n}\n\
             func Assert(x any) { j := x.(*store.Job); j.Run() }\n\
             func Rebound() { e := store.NewEngine(); for _, e := range []int{1} { _ = e }; e.Halt() }\n\
             func Iface(r store.Runner) { r.Run() }\n\
             func TypeParam[T store.Runner](t T) { t.Run() }\n\
             func Closure() { e := store.NewEngine(); f := func(e interface{ Tick() }) { e.Tick() }; f(e) }\n\
             func Shadowed(x any) {\n\te := store.NewEngine()\n\t_ = e\n\t\
             if e, ok := x.(interface{ Halt() }); ok {\n\t\te.Halt()\n\t}\n}\n",
        ),
    ]);
    let edge = |from: &str, to: &str| format!("app_app_{from} -calls-> store_{to}");
    for want in [
        edge("ctor", "engine_start"),
        edge("literal", "engine_stop"),
        edge("literal", "config_check"),
        edge("multi", "engine_spin"),
        edge("param", "engine_tick"),
        edge("chain", "engine_withname"),
        edge("chain", "engine_halt"),
        edge("var", "engine_start"),
        edge("promoted", "base_reset"),
        edge("pkgvar", "engine_stop"),
        edge("generic", "store_make"),
        edge("generic", "engine_spin"),
        edge("switch", "job_run"),
        edge("assert", "job_run"),
        // The range variable shadows `e` only inside the loop.
        edge("rebound", "engine_halt"),
    ] {
        assert!(l.contains(&want), "missing {want}: {l:#?}");
    }
    for unwanted in [
        // Interfaces and type parameters dispatch at run time or per instantiation.
        edge("iface", "job_run"),
        edge("typeparam", "job_run"),
        // A closure parameter and an `if` binding of unnamed interface types shadow `e`.
        edge("closure", "engine_tick"),
        edge("shadowed", "engine_halt"),
    ] {
        assert!(!l.contains(&unwanted), "unexpected {unwanted}: {l:#?}");
    }
}

/// Node id → (label, source_file) of a probe graph.
fn nodes(g: &Value) -> BTreeMap<String, (String, String)> {
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            (
                n["id"].as_str().unwrap().to_string(),
                (
                    n["label"].as_str().unwrap().to_string(),
                    n["source_file"].as_str().unwrap_or("").to_string(),
                ),
            )
        })
        .collect()
}

/// Go type ids held by several files (graphify-format.md §12.1): one node when all holders are
/// in one directory and of one kind (test or not), else every holder renamed (§4.5). Aliases are
/// types.
#[test]
fn package_type_ids_merge_within_one_directory_and_kind() {
    let g = graph(&[
        ("go.mod", "module ex.com/p\n"),
        (
            "a/types.go",
            "package a\n\ntype Svc struct{}\n\ntype K = string\n",
        ),
        ("a/ext.go", "package a\n\nfunc (s *Svc) Extra() {}\n"),
        ("c/c.go", "package c\n\ntype Q struct{}\n"),
        ("c/q_test.go", "package c\n\nfunc (q *Q) T() {}\n"),
        ("x/app/a.go", "package app\n\ntype U struct{}\n"),
        ("x/app/b.go", "package app\n\nfunc (u U) M() {}\n"),
        ("y/app/c.go", "package app\n\ntype U struct{}\n"),
        ("t/t_linux.go", "package t\n\ntype R struct{ a int }\n"),
        ("t/t_other.go", "package t\n\n\ntype R struct{ b int }\n"),
    ]);
    let n = nodes(&g);
    let at = |id: &str| n.get(id).map(|(l, f)| (l.as_str(), f.as_str()));
    assert_eq!(at("a_svc"), Some(("Svc", "a/types.go")));
    assert_eq!(at("a_k"), Some(("K", "a/types.go")));
    assert_eq!(at("c_c_go_c_q"), Some(("Q", "c/c.go")));
    assert_eq!(at("c_q_test_go_c_q"), Some(("Q", "c/q_test.go")));
    assert_eq!(at("x_app_b_go_app_u"), Some(("U", "x/app/b.go")));
    assert_eq!(at("y_app_c_go_app_u"), Some(("U", "y/app/c.go")));
    assert_eq!(at("t_r"), Some(("R", "t/t_linux.go")));
    assert!(!n.keys().any(|id| id.contains("t_other_go")), "{n:#?}");
    let l = links(&[
        ("go.mod", "module ex.com/p\n"),
        ("a/types.go", "package a\n\ntype Svc struct{}\n"),
        ("a/ext.go", "package a\n\nfunc (s *Svc) Extra() {}\n"),
    ]);
    assert!(l.contains("a_svc -method-> a_svc_extra"), "{l:#?}");
}

/// Graphify's typed receiver calls (graphify-format.md §12.2): INFERRED 0.85 to the method of the
/// unique Go type a declaration names, `unresolved_calls` for a type no Go node carries, a
/// receiver's field typed by the file's declarations, and delegation to a same-named method.
#[test]
fn typed_receiver_calls_follow_graphify() {
    let g = graph(&[
        ("go.mod", "module ex.com/p\n"),
        (
            "a/types.go",
            "package a\n\ntype Svc struct{}\n\nfunc (s *Svc) Do() {}\n\n\
             type Inner interface{ Encode() }\n",
        ),
        (
            "a/use.go",
            "package a\n\n\
             func First(z *Svc) {}\n\
             func Later() { z.Do() }\n\
             func Unknown(g Ghost) { g.Boo(); g.Boo() }\n\
             type Outer struct{ in Inner }\n\
             func (o *Outer) Encode() { o.in.Encode() }\n",
        ),
    ]);
    let calls: BTreeSet<String> = g["links"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["relation"] == "calls")
        .map(|e| {
            format!(
                "{} -> {} {} {}",
                e["source"].as_str().unwrap(),
                e["target"].as_str().unwrap(),
                e["confidence"].as_str().unwrap(),
                e["confidence_score"]
            )
        })
        .collect();
    for want in [
        "a_use_later -> a_svc_do INFERRED 0.85",
        "a_outer_encode -> a_inner_encode INFERRED 0.85",
    ] {
        assert!(calls.contains(want), "missing {want}: {calls:#?}");
    }
    let unknown = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "a_use_unknown")
        .unwrap();
    assert_eq!(
        unknown["metadata"],
        serde_json::json!({"unresolved_calls": [{
            "callee": "Boo", "receiver_type": "Ghost", "lang": "go", "line": "L5",
            "file": "a/use.go"
        }]})
    );
}

/// A label any other definition carries (a Python class, a YAML key) leaves a Go type name
/// unbound, as in Graphify.
#[test]
fn labels_of_other_languages_block_type_binding() {
    let l = links(&[
        ("go.mod", "module ex.com/p\n"),
        (
            "u/u.go",
            "package u\n\ntype T struct{}\n\ntype S struct{}\n",
        ),
        ("u/use.go", "package u\n\nfunc Use(t *T, s *S) {}\n"),
        ("w/far.py", "class T:\n    pass\n"),
    ]);
    assert!(l.contains("u_use_use -references-> t"), "{l:#?}");
    assert!(l.contains("u_use_use -references-> u_s"), "{l:#?}");
}

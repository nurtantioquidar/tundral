//! Parity with Graphify's clustered golden for `tests/fixtures/go`.

mod common;

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Q24 not reproduced: a method call on a receiver whose type a declaration fixes resolves to
    // that type's method in another file (`svc := NewUserService(); svc.Register(..)`; the
    // field `s.users` is a `*store.Store[User]`, whose `Put` hangs off the G3 receiver node).
    "cmd_app_main_main -calls-> service_userservice_register",
    "service_service_test_testregister -calls-> service_userservice_register",
    "service_userservice_register -calls-> store_store_t_put",
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

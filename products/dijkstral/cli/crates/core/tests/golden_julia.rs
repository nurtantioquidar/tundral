//! Parity with Graphify's clustered golden for `tests/fixtures/julia`, plus focused checks of the
//! Julia rules (docs/graphify-format.md §32) on small probe repositories.

mod common;

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §32).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn julia_fixture_matches_graphify() {
    common::assert_fixture("julia", ALLOWED, common::Communities::Exact, true, false);
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

#[test]
fn calls_bind_only_to_definitions_of_the_same_file() {
    // Probe j1 of §32: a call targets `file_<callee>`; it survives only when the file defines
    // that name (a struct constructor counts, self-calls stay), never across files.
    let l = probe_links(&[
        (
            "src/a.jl",
            "struct P end\nhelper(x) = x\nfunction f(x)\n    helper(x)\n    P()\n    f(x)\n    other(x)\n    x |> helper\n    function inner(y)\n        hidden(y)\n    end\nend\nhidden(y) = y\n",
        ),
        ("src/b.jl", "other(x) = x\n"),
    ]);
    for want in [
        "src_a_f -calls-> src_a_helper [call]",
        "src_a_f -calls-> src_a_p [call]",
        "src_a_f -calls-> src_a_f [call]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    assert!(
        !l.iter()
            .any(|x| x.contains("other") && x.contains("-calls->")),
        "{l:#?}"
    );
    // A function nested in a body is neither a node nor a source of calls.
    assert!(
        !l.iter()
            .any(|x| x.contains("hidden") && x.contains("-calls->")),
        "{l:#?}"
    );
}

#[test]
fn type_stubs_bind_by_label_across_files() {
    let l = probe_links(&[
        (
            "a.jl",
            "struct A1 <: Later\n    x::B1\n    y::Mod1\n    w::OnlyFn\n    d::Dup\n    p::Vector{B1}\nend\nstruct B1 end\nabstract type Later end\n",
        ),
        (
            "b.jl",
            "module Mod1\nOnlyFn(x) = x\nstruct Dup end\nend\n",
        ),
        ("c.jl", "struct Dup end\n"),
    ]);
    for want in [
        "a_a1 -inherits-> a_later",
        "a_a1 -references-> a_b1 [field]",
        "a_a1 -references-> b_mod1 [field]",
        "a_a1 -references-> b_onlyfn [field]",
        "a_a1 -references-> dup [field]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
}

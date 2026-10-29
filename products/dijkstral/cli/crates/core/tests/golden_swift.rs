//! Parity with Graphify's clustered golden for `tests/fixtures/swift`.

mod common;
mod probe;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary. None for this fixture.
const ALLOWED: &[&str] = &[];

#[test]
fn swift_fixture_matches_graphify() {
    common::assert_fixture("swift", ALLOWED, common::Communities::Exact, true);
}

/// Type parameters and `Self` are never referenced, and a variable typed by a type parameter in
/// scope is not entered in the variable table (graphify-format.md §23).
#[test]
fn swift_type_parameters_and_self() {
    let l = probe::links(&[(
        "S/A.swift",
        "func b(_ x: Thing) { x.go() }\nfunc a<X>(_ x: X) {}\nstruct W<Gadget> { var y: Gadget }\nfunc c(_ y: Gadget) {}\nfinal class K { func me() -> Self { return self } }\n",
    )]);
    assert!(
        l.contains("s_a_c -references-> gadget [parameter_type]"),
        "{l:#?}"
    );
    for gone in [
        "s_a_a -references-> x [parameter_type]",
        "s_a_w -references-> gadget [field]",
        "s_a_k_me -references-> self [return_type]",
    ] {
        assert!(!l.contains(gone), "{gone}: {l:#?}");
    }
    let g = probe::graph(&[(
        "S/A.swift",
        "func b(_ x: Thing) { x.go() }\nfunc a<X>(_ x: X) {}\n",
    )]);
    let b = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "s_a_b")
        .unwrap();
    assert_eq!(
        b["metadata"]["unresolved_calls"][0]["receiver_type"],
        "Thing"
    );
    assert_eq!(b["metadata"]["unresolved_calls"][0]["file"], "S/A.swift");
}

/// A protocol requirement dispatches to every conforming type's member: INFERRED 0.85 with one
/// conforming type, 0.75 with two, 0.65 with more, at the conforming type's line.
#[test]
fn swift_dispatch_to_every_conformer() {
    let l = probe::scored_links(&[(
        "S/A.swift",
        "protocol One { func go() }\nstruct OnlyOne: One { func go() {} }\n\nprotocol Two { func x() }\nstruct E1: Two { func x() {} }\nstruct E2: Two { func x() {} }\n\nprotocol Three { func run() }\nstruct A3: Three { func run() {} }\nstruct B3: Three { func run() {} }\nstruct C3: Three {}\n",
    )]);
    for want in [
        "s_a_one_go -dispatches_to-> s_a_onlyone_go 0.85 L2",
        "s_a_two_x -dispatches_to-> s_a_e1_x 0.75 L5",
        "s_a_two_x -dispatches_to-> s_a_e2_x 0.75 L6",
        "s_a_three_run -dispatches_to-> s_a_a3_run 0.65 L9",
        "s_a_three_run -dispatches_to-> s_a_b3_run 0.65 L10",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
}

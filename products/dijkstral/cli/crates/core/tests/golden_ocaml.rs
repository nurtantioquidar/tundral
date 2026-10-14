//! Parity with Graphify's clustered golden for `tests/fixtures/ocaml` (docs/graphify-format.md
//! §31): `.ml` and `.mli` files are code but contribute no nodes, because Graphify's OCaml
//! grammar is an optional extra the pinned install lacks. The fixture's Python tools carry
//! the graph; the report still counts the OCaml files.

mod common;

/// No deliberate differences.
const ALLOWED: &[&str] = &[];

#[test]
fn ocaml_fixture_matches_graphify() {
    common::assert_fixture("ocaml", ALLOWED, common::Communities::Exact, true, false);
}

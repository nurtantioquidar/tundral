//! Parity with Graphify's clustered golden for `tests/fixtures/erlang` (docs/graphify-format.md
//! §30): `.erl` and `.hrl` files are code but contribute no nodes, because Graphify's Erlang
//! grammar is an optional extra the pinned install lacks. The fixture's Python tools carry
//! the graph; the report still counts the Erlang files.

mod common;

/// No deliberate differences.
const ALLOWED: &[&str] = &[];

#[test]
fn erlang_fixture_matches_graphify() {
    common::assert_fixture("erlang", ALLOWED, common::Communities::Exact, true);
}

//! Parity with Graphify's clustered golden for `tests/fixtures/solidity` (§37): `.sol` files are
//! counted in the report's corpus check but contribute no node (Graphify ships without the
//! optional Solidity grammar).

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn solidity_fixture_matches_graphify() {
    common::assert_fixture("solidity", ALLOWED, common::Communities::Exact, true, false);
}

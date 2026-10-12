//! Parity with Graphify's clustered golden for `tests/fixtures/go`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn go_fixture_matches_graphify() {
    // Same graph, same communities; the report differs only in the order of two equal-score
    // surprises, which follows the cross-file pass's emission order (graphify-format.md §17.7).
    common::assert_fixture("go", ALLOWED, common::Communities::Exact, false, false);
}

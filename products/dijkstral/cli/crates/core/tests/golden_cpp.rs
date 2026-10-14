//! Parity with Graphify's clustered golden for `tests/fixtures/cpp`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn cpp_fixture_matches_graphify() {
    common::assert_fixture("cpp", ALLOWED, common::Communities::Exact, true, false);
}

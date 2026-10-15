//! Parity with Graphify's clustered golden for `tests/fixtures/zig`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §33).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn zig_fixture_matches_graphify() {
    common::assert_fixture("zig", ALLOWED, common::Communities::Exact, true);
}

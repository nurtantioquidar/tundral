//! Parity with Graphify's clustered golden for `tests/fixtures/typescript`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn typescript_fixture_matches_graphify() {
    common::assert_golden("typescript", 0.0, 0.0, ALLOWED);
}

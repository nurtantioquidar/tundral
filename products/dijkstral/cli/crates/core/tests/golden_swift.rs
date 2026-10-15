//! Parity with Graphify's clustered golden for `tests/fixtures/swift`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary. None for this fixture.
const ALLOWED: &[&str] = &[];

#[test]
fn swift_fixture_matches_graphify() {
    common::assert_fixture("swift", ALLOWED, common::Communities::Exact, true);
}

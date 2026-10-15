//! Parity with Graphify's clustered golden for `tests/fixtures/scala`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §22).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn scala_fixture_matches_graphify() {
    common::assert_fixture("scala", ALLOWED, common::Communities::Exact, true);
}

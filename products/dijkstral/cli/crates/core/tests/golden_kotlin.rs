//! Parity with Graphify's clustered golden for `tests/fixtures/kotlin`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §21).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn kotlin_fixture_matches_graphify() {
    common::assert_fixture("kotlin", ALLOWED, common::Communities::Exact, true, false);
}

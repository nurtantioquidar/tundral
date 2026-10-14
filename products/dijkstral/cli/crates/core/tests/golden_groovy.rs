//! Parity with Graphify's clustered golden for `tests/fixtures/groovy` (docs/graphify-format.md
//! §26).

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §26).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn groovy_fixture_matches_graphify() {
    common::assert_fixture("groovy", ALLOWED, common::Communities::Exact, true, false);
}

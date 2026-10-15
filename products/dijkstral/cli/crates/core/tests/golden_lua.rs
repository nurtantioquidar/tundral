//! Parity with Graphify's clustered golden for `tests/fixtures/lua` (docs/graphify-format.md §28).

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §28).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn lua_fixture_matches_graphify() {
    // Re-clustering Graphify's graph keeps the bare-number import locations (quirk LU1), so the
    // link order matches too.
    common::assert_fixture("lua", ALLOWED, common::Communities::Exact, true);
}

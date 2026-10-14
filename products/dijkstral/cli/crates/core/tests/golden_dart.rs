//! Parity with Graphify's clustered golden for `tests/fixtures/dart` (docs/graphify-format.md
//! §27).
//!
//! Three link field differences remain and are printed, not counted: two `variable_type`
//! reference locations (which duplicate Graphify keeps, quirk DA2) and the `context` a
//! `defines` link absorbs from a reference to the same definition (§27.4, a shared fold rule).

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §27).
/// None: the fixture's node and link sets are reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn dart_fixture_matches_graphify() {
    common::assert_fixture("dart", ALLOWED, common::Communities::Exact, true, false);
}

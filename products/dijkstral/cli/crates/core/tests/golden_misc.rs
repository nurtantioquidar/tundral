//! Parity with Graphify's clustered golden for `tests/fixtures/misc`: Pascal/Delphi, forms,
//! Lazarus packages, Apex, COBOL, BYOND, Metal, code Graphify counts but never extracts, and
//! shebang scripts (docs/graphify-format.md §44).

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §44.7).
/// None: node and link sets match.
const ALLOWED: &[&str] = &[];

#[test]
fn misc_fixture_matches_graphify() {
    // With no cross-file candidate the surprises fall back to a community bridge (§17.4), and
    // the `update` report counts the extensionless file it cannot classify.
    common::assert_fixture("misc", ALLOWED, common::Communities::Exact, true);
}

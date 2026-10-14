//! Parity with Graphify's clustered golden for `tests/fixtures/fortran`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §35.6).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn fortran_fixture_matches_graphify() {
    common::assert_fixture("fortran", ALLOWED, common::Communities::Exact, true, false);
}

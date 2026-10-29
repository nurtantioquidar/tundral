//! Parity with Graphify's clustered golden for `tests/fixtures/sql` (Graphify 0.9.97 with its
//! `[sql]` extra, §45).

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §45).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn sql_fixture_matches_graphify() {
    common::assert_fixture("sql", ALLOWED, common::Communities::Exact, true);
}

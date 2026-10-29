//! Parity with Graphify's clustered golden for `tests/fixtures/yaml` (Graphify 0.9.97, §46).

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §46).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn yaml_fixture_matches_graphify() {
    common::assert_fixture("yaml", ALLOWED, common::Communities::Exact, true);
}

//! Parity with Graphify's clustered golden for `tests/fixtures/verilog`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §36).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn verilog_fixture_matches_graphify() {
    common::assert_fixture("verilog", ALLOWED, common::Communities::Exact, true, false);
}

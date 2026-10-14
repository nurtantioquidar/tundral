//! Parity with Graphify's clustered golden for `tests/fixtures/objc` (§34).
//!
//! The fixture has no `A+B.h` category file: Graphify prefixes those categories' ids with the
//! absolute scan root (quirk OC1), which a golden made in one scratch directory cannot share with
//! a test run in another. `extract::objc` covers that id rule in its unit tests.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn objc_fixture_matches_graphify() {
    // Same graph, same communities; the extract-time report orders equal-score surprises by
    // build edge order, which follows the cross-file passes' emission order (§17.7), and
    // cluster-only surprises tie the same way.
    common::assert_fixture("objc", ALLOWED, common::Communities::Exact, false, true);
}

//! Parity with Graphify's clustered golden for `tests/fixtures/objc` (§34).
//!
//! The fixture has no `A+B.h` category file (Graphify 0.9.86 prefixed those categories' ids with
//! the absolute scan root, quirk OC1, fixed in 0.9.97); `extract::objc` covers that id rule in its
//! unit tests.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn objc_fixture_matches_graphify() {
    // Same graph, same communities, cluster-only exact. The extract-time report lists three
    // equal-score surprises in another order: ties follow the node order of the build, and
    // Graphify's differs where headers and implementations merge (§34.7, §17.7).
    common::assert_fixture("objc", ALLOWED, common::Communities::Exact, false);
}

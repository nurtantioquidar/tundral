//! Parity with Graphify's clustered golden for `tests/fixtures/scala`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §19).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn scala_fixture_matches_graphify() {
    // The `update` report lists `build.sbt` as unclassified, which dijkstral's report does not
    // (graphify-format.md §17.7); the analysis and the cluster-only report are exact.
    common::assert_fixture("scala", ALLOWED, common::Communities::Exact, false, false);
}

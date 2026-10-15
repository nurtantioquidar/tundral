//! Parity with Graphify's clustered golden for `tests/fixtures/csproj`: MSBuild projects,
//! solutions, Razor, XAML and the C# they bind to (docs/graphify-format.md §43).

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §43.7).
/// None: node and link sets match.
const ALLOWED: &[&str] = &[];

#[test]
fn csproj_fixture_matches_graphify() {
    // Same graph and communities; Graphify's own graph re-clusters exactly. The extract-time
    // sidecars are not byte-exact: `update` builds project files into a larger graph than
    // `extract` (118 nodes), and two equal-score XAML-binding surprises tie in the build's node
    // order, which differs (§43.7).
    common::assert_fixture("csproj", ALLOWED, common::Communities::Exact, false);
}

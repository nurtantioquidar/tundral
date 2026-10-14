//! Parity with Graphify's clustered golden for `tests/fixtures/csproj`: MSBuild projects,
//! solutions, Razor, XAML and the C# they bind to (docs/graphify-format.md §43).

mod common;

use dijkstral_core::pipeline::cluster_only;
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §43.7).
/// None: node and link sets match.
const ALLOWED: &[&str] = &[];

#[test]
fn csproj_fixture_matches_graphify() {
    let (built, agreement) = common::assert_golden("csproj", 1.0, 1.0, ALLOWED);
    assert_eq!(agreement.exact(), 1.0, "csproj: community ids differ");
    let ids = |g: &Value| -> Vec<String> {
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        ids(&built.ours),
        ids(&built.golden),
        "csproj: node array order"
    );

    // Graphify's own graph re-clustered: same communities and order. The surprises of the
    // analysis and report differ (§43.7: Graphify lists project-file concept nodes such as
    // `net9.0` as surprise endpoints; the shared analysis does not), so the sidecars are compared
    // by shape only.
    let golden: Value = serde_json::from_slice(
        &std::fs::read(common::fixtures().join("csproj.clustered.graphify.json")).unwrap(),
    )
    .unwrap();
    let out = cluster_only(&golden, "csproj").unwrap();
    let ours: Value = serde_json::from_str(&out.graph_json).unwrap();
    let again = common::community_agreement(&ours, &golden);
    assert_eq!(
        again.exact(),
        1.0,
        "csproj: cluster-only communities differ"
    );
    assert_eq!(ids(&ours), ids(&golden), "csproj: cluster-only node order");
    let without_surprises = |text: &str| -> Value {
        let mut v: Value = serde_json::from_str(text).unwrap();
        v.as_object_mut().unwrap().remove("surprises");
        v
    };
    let want = std::fs::read_to_string(
        common::fixtures().join("csproj.cluster_only.graphify_analysis.json"),
    )
    .unwrap();
    assert_eq!(
        without_surprises(&out.analysis_json),
        without_surprises(&want),
        "csproj: cluster-only analysis beyond surprises"
    );
    common::check_report_goldens("csproj", &built, false);
}

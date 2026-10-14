//! Parity with Graphify's clustered golden for `tests/fixtures/misc`: Pascal/Delphi, forms,
//! Lazarus packages, Apex, COBOL, BYOND, Metal, code Graphify counts but never extracts, and
//! shebang scripts (docs/graphify-format.md §44).

mod common;

use dijkstral_core::pipeline::cluster_only;
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §44.7).
/// None: node and link sets match.
const ALLOWED: &[&str] = &[];

#[test]
fn misc_fixture_matches_graphify() {
    let (built, agreement) = common::assert_golden("misc", 1.0, 1.0, ALLOWED);
    assert_eq!(agreement.exact(), 1.0, "misc: community ids differ");
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
        "misc: node array order"
    );

    // Graphify's own graph re-clustered: same communities and order. The surprises of the
    // analysis and report differ (§44.7: with no cross-file candidate Graphify falls back to a
    // community bridge between Pascal routines, labelled `Area()` without a leading dot; the
    // shared analysis lists none), so the sidecars are compared by shape only.
    let golden: Value = serde_json::from_slice(
        &std::fs::read(common::fixtures().join("misc.clustered.graphify.json")).unwrap(),
    )
    .unwrap();
    let out = cluster_only(&golden, "misc").unwrap();
    let ours: Value = serde_json::from_str(&out.graph_json).unwrap();
    let again = common::community_agreement(&ours, &golden);
    assert_eq!(again.exact(), 1.0, "misc: cluster-only communities differ");
    assert_eq!(ids(&ours), ids(&golden), "misc: cluster-only node order");
    let without_surprises = |text: &str| -> Value {
        let mut v: Value = serde_json::from_str(text).unwrap();
        v.as_object_mut().unwrap().remove("surprises");
        v
    };
    let want = std::fs::read_to_string(
        common::fixtures().join("misc.cluster_only.graphify_analysis.json"),
    )
    .unwrap();
    assert_eq!(
        without_surprises(&out.analysis_json),
        without_surprises(&want),
        "misc: cluster-only analysis beyond surprises"
    );
    common::check_report_goldens("misc", &built, false);
}

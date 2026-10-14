//! Parity with Graphify's clustered golden for `tests/fixtures/swift`.

mod common;

use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary. None for this fixture.
const ALLOWED: &[&str] = &[];

#[test]
fn swift_fixture_matches_graphify() {
    let (built, agreement) = common::assert_golden("swift", 1.0, 1.0, ALLOWED);
    assert!(
        agreement.exact() == 1.0,
        "swift: community ids differ on {:.3} of shared nodes",
        1.0 - agreement.exact()
    );
    let order = |g: &Value| -> Vec<String> {
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        order(&built.ours),
        order(&built.golden),
        "swift: node array order"
    );
    // Not exact: equal-score surprise ties follow insertion order, and Graphify inserts module
    // nodes at their first import and shared supertype stubs at first use (§18.9).
    common::check_report_goldens("swift", &built, false);
}

/// `dijkstral cluster-only` on Graphify's own graph matches its communities, array order and
/// god nodes, but Graphify lets the `extension String` symbol be a surprise endpoint and a
/// bridge (its noise labels only keep `String` out of the god nodes) while dijkstral's
/// analysis skips it for all three (graphify-format.md §18.9).
#[test]
#[ignore = "analysis noise labels apply to surprises and bridges too: graphify-format.md §18.9"]
fn swift_cluster_only_matches_graphify() {
    common::assert_cluster_only("swift", true);
}

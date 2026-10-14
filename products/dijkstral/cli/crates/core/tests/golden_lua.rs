//! Parity with Graphify's clustered golden for `tests/fixtures/lua` (docs/graphify-format.md §28).
//!
//! `common::assert_fixture` is spelled out here because of one shared-loader gap: Graphify writes
//! Lua `imports` locations as bare numbers (`"source_location": "3"`, quirk LU1). The build writes
//! them, but the `graph.json` loader behind `dijkstral cluster-only` reads only `L<n>` locations,
//! so re-clustering Graphify's own Lua graph sorts those links by an empty location. Everything
//! else the cluster-only check covers (communities, node order, analysis, report) is checked.

mod common;

use std::collections::BTreeSet;

use dijkstral_core::pipeline::cluster_only;
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §28).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

fn ids(g: &Value, key: &str) -> Vec<String> {
    g[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap_or("").to_string())
        .collect()
}

#[test]
fn lua_fixture_matches_graphify() {
    let (built, agreement) = common::assert_golden("lua", 1.0, 1.0, ALLOWED);
    assert_eq!(agreement.exact(), 1.0, "lua: community ids differ");
    let (ours, golden) = (ids(&built.ours, "nodes"), ids(&built.golden, "nodes"));
    let shared: BTreeSet<&String> = ours.iter().filter(|i| golden.contains(i)).collect();
    let ours: Vec<&String> = ours.iter().filter(|i| shared.contains(i)).collect();
    let golden: Vec<&String> = golden.iter().filter(|i| shared.contains(i)).collect();
    assert_eq!(ours, golden, "lua: node array order");
    common::check_report_goldens("lua", &built, true);
}

#[test]
fn lua_cluster_only_matches_graphify() {
    let path = common::fixtures().join("lua.clustered.graphify.json");
    let golden: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let out = cluster_only(&golden, "lua").unwrap();
    let ours: Value = serde_json::from_str(&out.graph_json).unwrap();
    let agreement = common::community_agreement(&ours, &golden);
    assert_eq!(agreement.exact(), 1.0, "lua: cluster-only communities");
    assert_eq!(
        ids(&ours, "nodes"),
        ids(&golden, "nodes"),
        "lua: node order"
    );
    let read = |f: &str| std::fs::read_to_string(common::fixtures().join(f)).unwrap();
    assert_eq!(
        out.analysis_json,
        read("lua.cluster_only.graphify_analysis.json")
    );
    assert_eq!(
        common::normalise_report(&out.report),
        common::normalise_report(&read("lua.cluster_only.graphify_report.md"))
    );
}

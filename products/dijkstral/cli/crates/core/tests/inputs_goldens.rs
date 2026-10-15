//! `extract --cargo` and `extract --postgres` against Graphify's goldens in
//! `tests/fixtures/inputs/` (`docs/graphify-inputs.md` §2, §3), record for record, communities
//! included. The PostgreSQL graphs are built offline from the catalog snapshots
//! (`<db>.catalog.json`) that the live test in `crates/cli/src/inputs_cmd.rs` checks against the
//! probe databases.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use dijkstral_core::inputs::postgres::Snapshot;
use dijkstral_core::inputs::Inputs;
use dijkstral_core::{build_inputs, BuildOptions};
use serde_json::Value;

/// Deliberate differences, as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Quirk P9: Graphify misses the second join of a parenthesised join chain
    // (`FROM ((orders o JOIN order_items i …) JOIN products p …)`); the view does read products.
    "postgresql_127_0_0_1_shop_public_order_totals -reads_from-> postgresql_127_0_0_1_shop_public_products",
];

fn inputs_dir() -> std::path::PathBuf {
    common::fixtures().join("inputs")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn build(root: &Path, inputs: &Inputs) -> Value {
    build_inputs(
        &BuildOptions {
            root: root.to_path_buf(),
            out: None,
            threads: 2,
            cluster: true,
        },
        inputs,
        &[],
        None,
    )
    .unwrap();
    serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap()
}

fn golden(name: &str) -> Value {
    serde_json::from_slice(&fs::read(inputs_dir().join(name)).unwrap()).unwrap()
}

fn assert_parity(name: &str, ours: &Value, golden: &Value) {
    let mut p = common::compare(ours, golden);
    let allowed: BTreeSet<&str> = ALLOWED.iter().copied().collect();
    p.links_only_ours.retain(|x| !allowed.contains(x.as_str()));
    p.links_only_golden
        .retain(|x| !allowed.contains(x.as_str()));
    let summary = p.summary();
    assert!(
        p.node_recall() == 1.0
            && p.node_precision() == 1.0
            && p.link_recall() == 1.0
            && p.link_precision() == 1.0
            && p.node_field_diffs.is_empty()
            && p.link_field_diffs.is_empty(),
        "{name}:\n{summary}"
    );
    let agreement = common::community_agreement(ours, golden);
    assert_eq!(agreement.exact(), 1.0, "{name}: community ids differ");
}

#[test]
fn cargo_workspace_matches_graphify() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("cargo");
    copy_dir(&inputs_dir().join("cargo"), &root);
    let with = build(
        &root,
        &Inputs {
            cargo: true,
            ..Inputs::default()
        },
    );
    assert_parity("cargo --cargo", &with, &golden("cargo.graphify.json"));
    // The renamed dependency (`gamma-lib = { package = "gamma" }`) is the link only --cargo adds.
    let crate_links: Vec<&Value> = with["links"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["relation"] == "crate_depends_on")
        .collect();
    assert_eq!(crate_links.len(), 1);
    assert_eq!(
        (&crate_links[0]["source"], &crate_links[0]["target"]),
        (&Value::from("pkg_alpha"), &Value::from("pkg_gamma"))
    );
    // Without the flag the same tree is Graphify's plain build (and the digest notices the flag).
    let without = build(&root, &Inputs::default());
    assert_parity("cargo", &without, &golden("cargo.plain.graphify.json"));
}

#[test]
fn postgres_schemas_match_graphify() {
    for db in ["shop", "probe2", "probe3"] {
        let snap: Snapshot = serde_json::from_slice(
            &fs::read(inputs_dir().join(format!("postgres/{db}.catalog.json"))).unwrap(),
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let ours = build(
            tmp.path(),
            &Inputs {
                postgres: Some(snap),
                ..Inputs::default()
            },
        );
        assert_parity(db, &ours, &golden(&format!("postgres/{db}.graphify.json")));
    }
}

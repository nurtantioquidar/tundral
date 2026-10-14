//! Visual Basic (`.vb`, docs/graphify-format.md §42): Graphify 0.9.86 counts `.vb` files as code
//! but, without the optional `tree-sitter-vb-dotnet` grammar (`graphifyy[vbnet]`, absent from the
//! reference install), contributes nothing for them: `vb.graphify.json` (the raw
//! `--no-cluster` build of `tests/fixtures/vb`) is empty, and the clustered build stops with
//! "graph is empty". Dijkstral reproduces the empty graph.

mod common;

use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

#[test]
fn vb_fixture_contributes_nothing_as_in_graphify() {
    let golden: Value =
        serde_json::from_slice(&fs::read(common::fixtures().join("vb.graphify.json")).unwrap())
            .unwrap();
    assert_eq!(golden["nodes"], Value::Array(vec![]));
    assert_eq!(golden["edges"], Value::Array(vec![]));

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vb");
    let src = common::fixtures().join("vb");
    for entry in walk(&src) {
        let rel = entry.strip_prefix(&src).unwrap();
        let dest = root.join(rel);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(&entry, dest).unwrap();
    }
    build(&BuildOptions {
        root: root.clone(),
        out: None,
        threads: 1,
        cluster: false,
    })
    .unwrap();
    let ours: Value =
        serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap();
    assert_eq!(ours["nodes"], Value::Array(vec![]), "{ours}");
    assert_eq!(ours["links"], Value::Array(vec![]), "{ours}");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

//! `dijkstral update`'s markdown structure on `tests/fixtures/semantic/markdown` against
//! `graphify update` (`markdown.update.graphify.json`, `markdown.graphify_report.md`;
//! `docs/graphify-semantic.md` §3).

#[path = "../../core/tests/common/mod.rs"]
mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use dijkstral_core::{build_full, BuildOptions};
use dijkstral_semantic::markdown::MarkdownPass;
use serde_json::Value;

/// Documented, deliberate differences, as printed by the parity summary. Empty: the fixture
/// exercises every rule of §3 and dijkstral reproduces all of them.
const ALLOWED: &[&str] = &[];

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

struct Update {
    graph: Value,
    report: String,
}

fn update(name: &str, pass: &MarkdownPass) -> Update {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(name);
    copy_dir(&common::fixtures().join("semantic").join(name), &root);
    build_full(
        &BuildOptions {
            root: root.clone(),
            out: None,
            threads: 2,
            cluster: true,
        },
        &[],
        Some(pass),
    )
    .unwrap();
    let out = root.join("dijkstral-out");
    Update {
        graph: serde_json::from_slice(&fs::read(out.join("graph.json")).unwrap()).unwrap(),
        report: fs::read_to_string(out.join("GRAPH_REPORT.md")).unwrap(),
    }
}

#[test]
fn markdown_update_matches_graphify() {
    let ours = update("markdown", &MarkdownPass::new());
    let golden: Value = serde_json::from_slice(
        &fs::read(common::fixtures().join("semantic/markdown.update.graphify.json")).unwrap(),
    )
    .unwrap();

    let mut p = common::compare(&ours.graph, &golden);
    let allowed: BTreeSet<&str> = ALLOWED.iter().copied().collect();
    for list in [
        &mut p.nodes_only_ours,
        &mut p.nodes_only_golden,
        &mut p.links_only_ours,
        &mut p.links_only_golden,
    ] {
        list.retain(|x| !allowed.contains(x.as_str()));
    }
    let summary = p.summary();
    eprintln!("markdown:\n{summary}");
    assert!(
        p.node_recall() == 1.0
            && p.node_precision() == 1.0
            && p.link_recall() == 1.0
            && p.link_precision() == 1.0
            && p.node_field_diffs.is_empty()
            && p.link_field_diffs.is_empty(),
        "markdown below parity:\n{summary}"
    );

    let agreement = common::community_agreement(&ours.graph, &golden);
    assert_eq!(
        agreement.exact(),
        1.0,
        "community ids differ: {agreement:?}"
    );
    let order = |g: &Value, key: &str, f: &dyn Fn(&Value) -> String| -> Vec<String> {
        g[key].as_array().unwrap().iter().map(f).collect()
    };
    let node_id = |n: &Value| n["id"].as_str().unwrap().to_string();
    let link_id = |e: &Value| format!("{} {} {}", e["source"], e["target"], e["relation"]);
    assert_eq!(
        order(&ours.graph, "nodes", &node_id),
        order(&golden, "nodes", &node_id)
    );
    assert_eq!(
        order(&ours.graph, "links", &link_id),
        order(&golden, "links", &link_id)
    );

    let want = common::normalise_report(
        &fs::read_to_string(common::fixtures().join("semantic/markdown.graphify_report.md"))
            .unwrap(),
    );
    let got = common::normalise_report(&ours.report);
    assert!(
        got == want,
        "report differs:\n{}",
        common::first_differences(&got, &want, 8)
    );
}

/// A second build over the same files is skipped; editing a markdown file rebuilds.
#[test]
fn markdown_changes_invalidate_the_build() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("md");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.py"), "def run():\n    return 1\n").unwrap();
    fs::write(root.join("README.md"), "# Intro\n\nCall `run`.\n").unwrap();
    let opts = BuildOptions {
        root: root.clone(),
        out: None,
        threads: 1,
        cluster: false,
    };
    let pass = MarkdownPass::new();
    let first = build_full(&opts, &[], Some(&pass)).unwrap();
    assert!(!first.unchanged);
    assert_eq!(first.files, 2);
    assert!(build_full(&opts, &[], Some(&pass)).unwrap().unchanged);
    fs::write(root.join("README.md"), "# Intro\n\nNothing.\n").unwrap();
    assert!(!build_full(&opts, &[], Some(&pass)).unwrap().unchanged);
    let graph: Value =
        serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap();
    let links = graph["links"].as_array().unwrap();
    assert!(links.iter().all(|e| e["relation"] != "references"));
    // Without the pass (`extract`), the same files build code only.
    assert!(!build_full(&opts, &[], None).unwrap().unchanged);
}

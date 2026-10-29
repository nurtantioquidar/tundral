//! `extract` with the semantic pass, `label` and `add` on the command line, offline: the model is
//! the `replay` backend answering with Graphify's recorded Codex replies for
//! `tests/fixtures/semantic/mixed` (`docs/graphify-semantic.md` §7–§11).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantic")
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

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .args(args)
        .env("SOURCE_DATE_EPOCH", "1760400000")
        .output()
        .unwrap()
}

fn ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "dijkstral {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn graph(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap()
}

#[test]
fn extract_merges_replies_and_reruns_without_a_model_call() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("mixed");
    copy_dir(&fixtures().join("mixed"), &root);
    let r = root.to_str().unwrap();
    // Graphify's replies name the office conversions under graphify-out/converted; dijkstral
    // frames them as dijkstral-out/converted/…, so replay them under that name.
    let replay = tmp.path().join("transcript");
    fs::create_dir(&replay).unwrap();
    for entry in fs::read_dir(fixtures().join("mixed.transcript")).unwrap() {
        let path = entry.unwrap().path();
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("graphify-out/converted", "dijkstral-out/converted")
            .replace("graphify_out_converted", "dijkstral_out_converted");
        fs::write(replay.join(path.file_name().unwrap()), text).unwrap();
    }
    let replay = replay.to_str().unwrap();
    let extract = [
        "extract",
        r,
        "--no-viz",
        "--backend",
        "replay",
        "--replay-dir",
        replay,
    ];

    let first = ok(&extract);
    assert!(first.contains("model calls: 1"), "{first}");
    let g = graph(&root);
    let ids: Vec<&str> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"readme_cache") && ids.contains(&"src_app_service_jobqueue"));
    assert!(ids.contains(&"papers_write_through"));
    assert!(ids.contains(&"dijkstral_out_converted_capacity_e7fd8f6a"));
    assert_eq!(g["graph"]["hyperedges"], g["hyperedges"]);
    assert_eq!(g["hyperedges"].as_array().unwrap().len(), 2);
    let analysis: Value = serde_json::from_slice(
        &fs::read(root.join("dijkstral-out/.graphify_analysis.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(analysis["tokens"]["input"], 16843);
    let report = fs::read_to_string(root.join("dijkstral-out/GRAPH_REPORT.md")).unwrap();
    assert!(report.contains("- Token cost: 16,843 input · 3,275 output"));
    assert!(report.contains("## Hyperedges (group relationships)"));
    assert!(report.contains("## Ambiguous Edges - Review These"));

    // Every file answered (YAML is code since 0.9.97): the rerun takes all of them from the cache,
    // then there is nothing to do at all.
    let second = ok(&extract);
    assert!(
        second.contains("7 hit / 0 miss") && second.contains("model calls: 0"),
        "{second}"
    );
    assert!(graph(&root)["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["_origin"] == "semantic"));
    assert!(ok(&extract).contains("unchanged"));

    // A code-only build of the same files is a different build, with no semantic nodes.
    let code = ok(&["extract", r, "--no-viz", "--code-only"]);
    assert!(!code.contains("unchanged"));
    assert!(graph(&root)["hyperedges"].as_array().unwrap().is_empty());
}

#[test]
fn label_names_communities_and_writes_the_labels_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("mixed");
    copy_dir(&fixtures().join("mixed"), &root);
    let r = root.to_str().unwrap();
    ok(&["extract", r, "--no-viz", "--code-only"]);
    // A replay whose replies do not name these communities: every name stays structural.
    let replay = fixtures().join("mixed.transcript");
    let out = run(&[
        "label",
        r,
        "--backend",
        "replay",
        "--replay-dir",
        replay.to_str().unwrap(),
        "--max-concurrency",
        "1",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let labels: Value = serde_json::from_slice(
        &fs::read(root.join("dijkstral-out/.graphify_labels.json")).unwrap(),
    )
    .unwrap();
    let g = graph(&root);
    for n in g["nodes"].as_array().unwrap() {
        let c = n["community"].as_u64().unwrap().to_string();
        assert_eq!(n["community_name"], labels[&c]);
    }
}

#[test]
fn add_refuses_local_urls_and_other_schemes() {
    for (url, msg) in [
        ("file:///etc/hosts", "Blocked URL scheme 'file'"),
        (
            "http://127.0.0.1:9/x",
            "Blocked private/internal IP 127.0.0.1",
        ),
        ("http://[::1]/x", "Blocked private/internal IP ::1"),
    ] {
        let out = run(&["add", url, "--dir", "/nonexistent-dijkstral-raw"]);
        assert!(!out.status.success());
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(msg), "{url}: {err}");
    }
}

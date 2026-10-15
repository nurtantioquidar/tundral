//! `diagnose multigraph` against Graphify's goldens (`tests/fixtures/memory/diagnose/`,
//! `docs/graphify-memory.md` §6–§7).
//!
//! Every counter, example and scan site must match. Dijkstral words the banner, `input_stage`,
//! `note` and `notes` itself, and reports its own reason when the post-build simulation fails; its
//! option errors come from clap (exit 2, Graphify exits 1).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Cases where dijkstral's clap parser rejects the arguments with exit 2.
const USAGE_ERRORS: &[&str] = &[
    "err.both_flags",
    "err.max_examples_text",
    "err.max_examples_negative",
    "err.unknown_option",
    "err.no_subcommand",
    "err.bad_subcommand",
];

/// Text lines compared by presence only, and prose lines skipped.
fn comparable(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| {
            !(l.starts_with("[graphify]")
                || l.starts_with("[dijkstral]")
                || l.starts_with("input_stage: ")
                || l.starts_with("note: "))
        })
        .map(|l| {
            if l.starts_with("post_build_error: ") {
                "post_build_error: <reason>".to_string()
            } else {
                l.to_string()
            }
        })
        .collect()
}

fn comparable_json(text: &str) -> Value {
    let mut v: Value = serde_json::from_str(text).unwrap();
    let o = v.as_object_mut().unwrap();
    o.remove("notes");
    let summary = o["summary"].as_object_mut().unwrap();
    let failed = summary["post_build_error"]
        .as_str()
        .is_some_and(|e| !e.is_empty());
    summary.insert("post_build_error".into(), Value::Bool(failed));
    v
}

#[test]
fn diagnose_matches_graphify() {
    let cases: Value =
        serde_json::from_slice(&fs::read(fixtures().join("memory/cases.json")).unwrap()).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::copy(
        fixtures().join("memory/diagnose/scan_probe.py.txt"),
        dir.join("scan.py"),
    )
    .unwrap();
    for fx in ["python", "rust", "typescript", "go"] {
        for kind in ["graphify", "clustered.graphify"] {
            let name = format!("{fx}.{kind}.json");
            fs::copy(fixtures().join(&name), dir.join(&name)).unwrap();
        }
    }
    for entry in fs::read_dir(fixtures().join("memory/diagnose/inputs")).unwrap() {
        let p = entry.unwrap().path();
        fs::copy(&p, dir.join(p.file_name().unwrap())).unwrap();
    }
    fs::write(dir.join("bad.json"), "not json").unwrap();

    let mut checked = 0;
    for c in cases["diagnose"].as_array().unwrap() {
        let name = c["case"].as_str().unwrap();
        let argv: Vec<&str> = c["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        let out = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
            .args(&argv)
            .current_dir(dir)
            .output()
            .unwrap();
        let code = out.status.code().unwrap();
        let want_code = if USAGE_ERRORS.contains(&name) {
            2
        } else {
            c["exit"].as_i64().unwrap() as i32
        };
        assert_eq!(code, want_code, "{name}: {out:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        if code != 0 {
            assert!(stdout.is_empty(), "{name}: no report on failure");
            assert!(!out.stderr.is_empty(), "{name}: an error message");
            checked += 1;
            continue;
        }
        let golden = fs::read_to_string(
            fixtures()
                .join("memory")
                .join(c["stdout"].as_str().unwrap()),
        )
        .unwrap();
        if name.ends_with(".json") {
            assert!(stdout.ends_with("}\n"), "{name}");
            assert_eq!(
                comparable_json(&stdout),
                comparable_json(&golden),
                "{name}:\n{stdout}"
            );
        } else {
            assert_eq!(
                comparable(&stdout),
                comparable(&golden),
                "{name}:\n{stdout}"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 45);
}

#[test]
fn without_extract_path_there_is_no_scan() {
    let tmp = tempfile::tempdir().unwrap();
    fs::copy(
        fixtures().join("python.graphify.json"),
        tmp.path().join("g.json"),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .args(["diagnose", "multigraph", "--graph", "g.json", "--json"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["producer_suppression"]["path"], "");
    assert_eq!(v["producer_suppression"]["total_sites"], 0);
    assert_eq!(v["summary"]["directed_same_endpoint_collapsed_edges"], 9);
    // The default graph is dijkstral's, else Graphify's.
    fs::create_dir(tmp.path().join("graphify-out")).unwrap();
    fs::rename(
        tmp.path().join("g.json"),
        tmp.path().join("graphify-out/graph.json"),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .args(["diagnose", "multigraph"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("input: graphify-out/graph.json\n"), "{text}");
}

//! Graphify's query-surface goldens (`tests/fixtures/query/cases.json`, `docs/graphify-query.md`
//! §14): every case runs the `dijkstral` binary on a copy of the fixture's clustered Graphify
//! graph, from the directory holding it, and must reproduce stdout, stderr and the exit code.
//! BFS `EDGE` lines (`edge_lines: "set"`) are compared as a multiset (§3.9).
//!
//! Each case runs twice: on the `graph.json` (indexed in memory on load) and on a `graph.dij`
//! encoded from it (memory-mapped), whose output must be identical apart from the header path.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Cases dijkstral is allowed to answer differently, with the reason.
const ALLOWED: &[(&str, &str)] = &[];

fn split_edges(s: &str) -> (Vec<&str>, Vec<&str>) {
    let (mut edges, mut rest) = (Vec::new(), Vec::new());
    for line in s.split('\n') {
        if line.starts_with("EDGE ") {
            edges.push(line);
        } else {
            rest.push(line);
        }
    }
    edges.sort_unstable();
    (edges, rest)
}

fn same_output(got: &str, want: &str, as_set: bool) -> bool {
    if !as_set {
        return got == want;
    }
    split_edges(got) == split_edges(want)
}

#[test]
fn query_goldens() {
    let root = fixtures();
    let golden_dir = root.join("query");
    let cases: Value =
        serde_json::from_str(&fs::read_to_string(golden_dir.join("cases.json")).unwrap()).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let mut failures = Vec::new();
    let mut ran = 0;
    for case in cases["cases"].as_array().unwrap() {
        let fixture = case["fixture"].as_str().unwrap();
        let json_name = format!("{fixture}.clustered.graphify.json");
        let dij_name = format!("{fixture}.dij");
        let json_path = scratch.path().join(&json_name);
        if !json_path.exists() {
            let bytes = fs::read(root.join(&json_name)).unwrap();
            fs::write(&json_path, &bytes).unwrap();
            let encoded = dijkstral_core::index::encode_graph_json(&bytes).unwrap();
            fs::write(scratch.path().join(&dij_name), encoded).unwrap();
        }
        let argv: Vec<String> = case["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect();
        let want_stdout =
            fs::read_to_string(golden_dir.join(case["stdout"].as_str().unwrap())).unwrap();
        let want_stderr = case["stderr"].as_str().unwrap();
        let want_exit = case["exit"].as_i64().unwrap() as i32;
        let as_set = case["edge_lines"] == "set";
        let name = format!("{} {}", fixture, case["stdout"].as_str().unwrap());

        for use_index in [false, true] {
            let args: Vec<String> = argv
                .iter()
                .map(|a| {
                    if use_index && *a == json_name {
                        dij_name.clone()
                    } else {
                        a.clone()
                    }
                })
                .collect();
            let out = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
                .args(&args)
                .current_dir(scratch.path())
                .output()
                .unwrap();
            ran += 1;
            let mut stdout = String::from_utf8(out.stdout).unwrap();
            if use_index {
                stdout = stdout.replacen(
                    &format!("Graph: {dij_name} "),
                    &format!("Graph: {json_name} "),
                    1,
                );
            }
            let stderr = String::from_utf8(out.stderr).unwrap();
            let code = out.status.code().unwrap_or(-1);
            let ok = code == want_exit
                && stderr == want_stderr
                && same_output(&stdout, &want_stdout, as_set);
            if !ok && !ALLOWED.iter().any(|(n, _)| *n == name) {
                failures.push(format!(
                    "== {name} ({}) argv={args:?}\n-- exit {code} (want {want_exit})\n-- stderr:\n{stderr}-- want stderr:\n{want_stderr}-- stdout:\n{stdout}-- want stdout:\n{want_stdout}",
                    if use_index { "graph.dij" } else { "graph.json" }
                ));
            }
        }
    }
    assert!(ran > 0);
    assert!(
        failures.is_empty(),
        "{} of {ran} runs differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// dijkstral extension (`docs/graphify-query.md` §7): `affected` takes the `path::symbol` name
/// `explain` suggests for an ambiguous one; the bare ambiguous name keeps Graphify's message.
#[test]
fn affected_accepts_path_qualified_names() {
    let graph = fixtures().join("python.graphify.json");
    let run = |name: &str| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_dijkstral"))
            .args(["affected", name, "--graph"])
            .arg(&graph)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(run("helper"), "No unique node match for helper\n");
    let qualified = run("pkg/util.py::helper");
    assert!(
        qualified.starts_with("Affected nodes for helper()\n"),
        "{qualified}"
    );
    assert!(
        qualified.contains("- slugify() [calls] pkg/util.py:L15\n"),
        "{qualified}"
    );
}

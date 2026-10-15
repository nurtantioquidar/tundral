//! Graphify's cross-repo goldens (`tests/fixtures/multirepo/cases.json`,
//! `docs/graphify-multirepo.md` §1): every case runs its steps with the `dijkstral` binary in a
//! fresh workspace (`DIJKSTRAL_HOME=<workspace>/home/.dijkstral`) and must reproduce the exit
//! codes and every recorded file byte for byte (manifests after masking the time and workspace).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Cases dijkstral is allowed to answer differently, with the reason. Each must still differ,
/// so an entry cannot outlive its quirk.
const ALLOWED: &[(&str, &str)] = &[(
    "merge_tags_suffix_taken",
    "§3.2 quirk M1: Graphify gives the third input the tag `proj-2` already held by the \
     `proj-2` directory and merges the two repositories' nodes; dijkstral takes `proj-3`",
)];

/// `added_at` and the workspace path are per run (regen.py's `normalize_manifest`).
fn normalize_manifest(text: &str, ws: &str) -> String {
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| match line.find("\"added_at\": ") {
            Some(i) if line[..i].trim().is_empty() => {
                format!("{}\"added_at\": \"<added_at>\",", &line[..i])
            }
            _ => line.to_string(),
        })
        .collect();
    let escaped = serde_json::to_string(ws).unwrap();
    lines
        .join("\n")
        .replace(&escaped[1..escaped.len() - 1], "<ws>")
}

#[test]
fn multirepo_goldens() {
    let root = fixtures();
    let dir = root.join("multirepo");
    let cases: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    let mut differing = Vec::new();
    let mut ran = 0;
    for case in cases["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let home = ws.join("home/.dijkstral");
        for (dest, source) in case["files"].as_object().unwrap() {
            let d = ws.join(dest);
            fs::create_dir_all(d.parent().unwrap()).unwrap();
            fs::copy(root.join(source.as_str().unwrap()), &d).unwrap();
        }
        let global = home.to_str().unwrap();
        let mut case_fail = Vec::new();
        for (i, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let args: Vec<String> = step["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().replace("@global", global))
                .collect();
            let cwd = ws.join(step.get("cwd").and_then(Value::as_str).unwrap_or(""));
            let out = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
                .args(&args)
                .current_dir(&cwd)
                .env("DIJKSTRAL_HOME", &home)
                .output()
                .unwrap();
            ran += 1;
            let want = step["exit"].as_i64().unwrap() as i32;
            if out.status.code() != Some(want) {
                case_fail.push(format!(
                    "step {i} {args:?}: exit {:?}, want {want}\n{}",
                    out.status.code(),
                    String::from_utf8_lossy(&out.stderr)
                ));
                break;
            }
            for (path, golden) in step["outputs"].as_object().unwrap() {
                let golden = golden.as_str().unwrap();
                let got = fs::read_to_string(ws.join(path.replace("@global", global)))
                    .unwrap_or_else(|e| format!("<{e}>"));
                let got = if golden.ends_with(".manifest.json") {
                    normalize_manifest(&got, ws.to_str().unwrap())
                } else {
                    got
                };
                let want = fs::read_to_string(dir.join("goldens").join(golden)).unwrap();
                if got != want {
                    case_fail.push(format!(
                        "step {i} {args:?}: {path} differs from {golden}\n got: {}\nwant: {}",
                        &got[..got.len().min(600)],
                        &want[..want.len().min(600)]
                    ));
                }
            }
        }
        let allowed = ALLOWED.iter().any(|(n, _)| *n == name);
        match (case_fail.is_empty(), allowed) {
            (true, true) => failures.push(format!("{name}: listed in ALLOWED but matches")),
            (false, true) => differing.push(name),
            (false, false) => failures.push(format!("{name}:\n{}", case_fail.join("\n"))),
            (true, false) => {}
        }
    }
    assert!(ran > 60, "only {ran} steps ran");
    assert_eq!(differing.len(), ALLOWED.len(), "{differing:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

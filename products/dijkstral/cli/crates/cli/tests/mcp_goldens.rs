//! Graphify's MCP transcripts (`tests/fixtures/query/mcp/`, `docs/graphify-query.md` §10.11):
//! every recorded request is replayed against `dijkstral mcp`, started like `graphify-mcp` was
//! (no arguments, cwd holding `graphify-out/graph.json`), and the response must match.
//!
//! Tool and resource descriptions are dijkstral's own and are not compared. `EDGE` lines of
//! `query_graph` are compared as a multiset (§10.6). Each fixture runs twice: on the `graph.json`
//! and on a `graph.dij` encoded from it under `dijkstral-out/` (memory-mapped), whose answers must
//! be identical apart from the header path.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Transcripts whose text dijkstral answers differently on purpose (§10.9), with the reason.
const TEXT_DIFFS: &[(&str, &str)] = &[
    (
        "rust.get_node.1",
        "the ambiguity text echoes 'Circle' as given; Graphify lowercases it",
    ),
    (
        "rust.query_graph.4",
        "a string context_filter is one context ('call'); Graphify iterates its characters",
    ),
    (
        "python.resources_read.6",
        "the missing-report hint names `dijkstral extract`",
    ),
];

/// Transcripts of failed calls, which dijkstral flags with `isError: true` (Graphify: false).
const FLAGGED_FAILURES: &[&str] = &[
    "python.error.1",
    "python.error.2",
    "python.get_node.2",
    "python.get_node.4",
    "python.get_node.5",
    "python.get_community.3",
    "python.shortest_path.4",
    "rust.get_node.1",
    "rust.get_node.2",
];

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

/// Drop every `description` key of tool and resource objects (not of schema properties).
fn strip_descriptions(list: &mut Value) {
    if let Some(items) = list.as_array_mut() {
        for item in items {
            if let Some(o) = item.as_object_mut() {
                o.remove("description");
            }
        }
    }
}

struct Transcript {
    name: String,
    request: Value,
    response: Value,
}

fn transcripts(fixture: &str) -> Vec<Transcript> {
    let dir = fixtures().join("query/mcp");
    let mut out: Vec<Transcript> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&format!("{fixture}."))
        })
        .map(|p| {
            let doc: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
            Transcript {
                name: p.file_stem().unwrap().to_str().unwrap().to_string(),
                request: doc["request"].clone(),
                response: doc["response"].clone(),
            }
        })
        .collect();
    out.sort_by_key(|t| t.request["id"].as_i64().unwrap());
    out
}

/// Replay the requests (in id order, one server process) and return the responses by id.
fn replay(cwd: &Path, requests: &[&Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .arg("mcp")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            writeln!(stdin, "{r}").unwrap();
            if r["method"] == "initialize" {
                writeln!(
                    stdin,
                    r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
                )
                .unwrap();
            }
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "dijkstral mcp exited {}", out.status);
    assert!(out.stderr.is_empty(), "stderr without --verbose");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect()
}

/// Compare one response; `Err` describes the difference.
fn compare(t: &Transcript, got: &Value) -> Result<(), String> {
    let want = &t.response;
    if got["id"] != want["id"] || got["jsonrpc"] != "2.0" {
        return Err(format!("envelope {got}"));
    }
    let kind = t.name.split('.').nth(1).unwrap();
    match kind {
        "initialize" => {
            let (mut g, mut w) = (got["result"].clone(), want["result"].clone());
            assert_eq!(g["serverInfo"]["name"], "dijkstral");
            g.as_object_mut().unwrap().remove("serverInfo");
            w.as_object_mut().unwrap().remove("serverInfo");
            (g == w).then_some(()).ok_or(format!("{g} != {w}"))
        }
        "tools_list" => {
            let mut g = got["result"]["tools"].clone();
            let mut w = want["result"]["tools"].clone();
            strip_descriptions(&mut g);
            strip_descriptions(&mut w);
            (g == w).then_some(()).ok_or(format!("{g} != {w}"))
        }
        "resources_list" => {
            let mut g = got["result"]["resources"].clone();
            let mut w = want["result"]["resources"].clone();
            strip_descriptions(&mut g);
            strip_descriptions(&mut w);
            (g == w).then_some(()).ok_or(format!("{g} != {w}"))
        }
        _ if want.get("error").is_some() => (got.get("error") == want.get("error"))
            .then_some(())
            .ok_or(format!("{got}")),
        _ if t.request["method"] == "resources/read" => {
            let g = &got["result"]["contents"][0];
            let w = &want["result"]["contents"][0];
            (g == w).then_some(()).ok_or(format!(
                "-- got:\n{}\n-- want:\n{}",
                g["text"].as_str().unwrap_or_default(),
                w["text"].as_str().unwrap_or_default()
            ))
        }
        _ => {
            let g = got["result"]["content"][0]["text"].as_str().unwrap_or("");
            let w = want["result"]["content"][0]["text"].as_str().unwrap();
            let same = if kind == "query_graph" {
                split_edges(g) == split_edges(w)
            } else {
                g == w
            };
            let flagged = FLAGGED_FAILURES.contains(&t.name.as_str());
            let is_error = got["result"]["isError"].as_bool();
            if is_error != Some(flagged) {
                return Err(format!("isError {is_error:?}, expected {flagged}"));
            }
            same.then_some(())
                .ok_or(format!("-- got:\n{g}\n-- want:\n{w}"))
        }
    }
}

#[test]
fn mcp_goldens() {
    let root = fixtures();
    let scratch = tempfile::tempdir().unwrap();
    let mut failures = Vec::new();
    let mut identical = 0;
    let mut total = 0;
    for fixture in ["python", "rust"] {
        let ts = transcripts(fixture);
        assert!(!ts.is_empty());
        let bytes = fs::read(root.join(format!("{fixture}.clustered.graphify.json"))).unwrap();
        let json_dir = scratch.path().join(fixture).join("json");
        fs::create_dir_all(json_dir.join("graphify-out")).unwrap();
        fs::write(json_dir.join("graphify-out/graph.json"), &bytes).unwrap();
        let dij_dir = scratch.path().join(fixture).join("dij");
        fs::create_dir_all(dij_dir.join("dijkstral-out")).unwrap();
        fs::write(
            dij_dir.join("dijkstral-out/graph.dij"),
            dijkstral_core::index::encode_graph_json(&bytes).unwrap(),
        )
        .unwrap();

        let requests: Vec<&Value> = ts.iter().map(|t| &t.request).collect();
        for (variant, dir) in [("graph.json", &json_dir), ("graph.dij", &dij_dir)] {
            let responses = replay(dir, &requests);
            assert_eq!(
                responses.len(),
                ts.len(),
                "{fixture} {variant}: one reply each"
            );
            for (t, got) in ts.iter().zip(&responses) {
                let mut got = got.clone();
                if variant == "graph.dij" {
                    if let Some(text) = got["result"]["content"][0]["text"].as_str() {
                        let text = text.replacen(
                            "Graph: dijkstral-out/graph.dij ",
                            "Graph: graphify-out/graph.json ",
                            1,
                        );
                        got["result"]["content"][0]["text"] = Value::String(text);
                    }
                }
                total += 1;
                let allowed = TEXT_DIFFS.iter().any(|(n, _)| *n == t.name);
                match compare(t, &got) {
                    Ok(()) => {
                        identical += 1;
                        assert!(!allowed, "{} now matches; drop it from TEXT_DIFFS", t.name);
                    }
                    Err(e) if allowed && !e.starts_with("isError") => {}
                    Err(e) => failures.push(format!("== {} ({variant})\n{e}", t.name)),
                }
            }
        }
    }
    eprintln!("mcp goldens: {identical} of {total} replies match");
    assert!(
        failures.is_empty(),
        "{} of {total} replies differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

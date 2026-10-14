//! Small probe repositories for the component and PowerShell tests: build one without
//! clustering and list its nodes and links as strings.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Build `files` (path, text) and return the raw `graph.json`.
pub fn graph(files: &[(&str, &str)]) -> Value {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("probe");
    for (path, text) in files {
        let p = root.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    build(&BuildOptions {
        root: root.clone(),
        out: None,
        threads: 1,
        cluster: false,
    })
    .unwrap();
    serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap()
}

fn edges(g: &Value) -> &Vec<Value> {
    g.get("links")
        .or_else(|| g.get("edges"))
        .and_then(Value::as_array)
        .unwrap()
}

/// `source -relation-> target [context]` for every link.
pub fn links(files: &[(&str, &str)]) -> BTreeSet<String> {
    edges(&graph(files))
        .iter()
        .map(|e| {
            let ctx = e["context"]
                .as_str()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            format!(
                "{} -{}-> {}{}",
                e["source"].as_str().unwrap(),
                e["relation"].as_str().unwrap(),
                e["target"].as_str().unwrap(),
                ctx
            )
        })
        .collect()
}

/// As [`links`], plus `confidence_score` (`None` when absent) and the line.
pub fn scored_links(files: &[(&str, &str)]) -> BTreeSet<String> {
    edges(&graph(files))
        .iter()
        .map(|e| {
            format!(
                "{} -{}-> {} {} {}",
                e["source"].as_str().unwrap(),
                e["relation"].as_str().unwrap(),
                e["target"].as_str().unwrap(),
                e.get("confidence_score")
                    .map_or("None".to_string(), Value::to_string),
                e["source_location"].as_str().unwrap_or("-"),
            )
        })
        .collect()
}

/// `id label Lnn` for every node.
pub fn nodes(files: &[(&str, &str)]) -> BTreeSet<String> {
    graph(files)["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            format!(
                "{} {} {}",
                n["id"].as_str().unwrap(),
                n["label"].as_str().unwrap(),
                n["source_location"].as_str().unwrap_or("-")
            )
        })
        .collect()
}

//! Golden comparison against Graphify's clustered output for the fixtures in `cli/tests/fixtures`.
//!
//! The fixture is copied to a temporary directory (not a git repository, as when the goldens were
//! made), built, and compared to `<name>.clustered.graphify.json` as sets: nodes by id with all
//! fields except `community`/`community_name`, links by (source, target, relation) with all fields.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
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

/// Build the fixture and return our graph and Graphify's golden graph.
pub fn build_fixture(name: &str) -> (Value, Value) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(name);
    copy_dir(&fixtures().join(name), &root);
    build(&BuildOptions {
        root: root.clone(),
        out: None,
        threads: 2,
    })
    .unwrap();
    let ours: Value =
        serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap();
    let golden: Value = serde_json::from_slice(
        &fs::read(fixtures().join(format!("{name}.clustered.graphify.json"))).unwrap(),
    )
    .unwrap();
    (ours, golden)
}

#[derive(Debug, Default)]
pub struct Parity {
    pub nodes_both: usize,
    pub nodes_only_ours: Vec<String>,
    pub nodes_only_golden: Vec<String>,
    /// Shared node ids whose fields differ: (id, field, ours, golden).
    pub node_field_diffs: Vec<(String, String, Value, Value)>,
    pub links_both: usize,
    pub links_only_ours: Vec<String>,
    pub links_only_golden: Vec<String>,
    pub link_field_diffs: Vec<(String, String, Value, Value)>,
}

impl Parity {
    pub fn node_recall(&self) -> f64 {
        ratio(
            self.nodes_both,
            self.nodes_both + self.nodes_only_golden.len(),
        )
    }
    pub fn node_precision(&self) -> f64 {
        ratio(
            self.nodes_both,
            self.nodes_both + self.nodes_only_ours.len(),
        )
    }
    pub fn link_recall(&self) -> f64 {
        ratio(
            self.links_both,
            self.links_both + self.links_only_golden.len(),
        )
    }
    pub fn link_precision(&self) -> f64 {
        ratio(
            self.links_both,
            self.links_both + self.links_only_ours.len(),
        )
    }

    pub fn summary(&self) -> String {
        let mut s = format!(
            "nodes: recall {:.3} precision {:.3} ({} shared, {} field diffs)\nlinks: recall {:.3} precision {:.3} ({} shared, {} field diffs)\n",
            self.node_recall(),
            self.node_precision(),
            self.nodes_both,
            self.node_field_diffs.len(),
            self.link_recall(),
            self.link_precision(),
            self.links_both,
            self.link_field_diffs.len(),
        );
        for (title, items) in [
            ("nodes only in golden", &self.nodes_only_golden),
            ("nodes only in ours", &self.nodes_only_ours),
            ("links only in golden", &self.links_only_golden),
            ("links only in ours", &self.links_only_ours),
        ] {
            if !items.is_empty() {
                s.push_str(&format!("{title}:\n"));
                for i in items {
                    s.push_str(&format!("  {i}\n"));
                }
            }
        }
        for (title, diffs) in [
            ("node field diffs", &self.node_field_diffs),
            ("link field diffs", &self.link_field_diffs),
        ] {
            if !diffs.is_empty() {
                s.push_str(&format!("{title}:\n"));
                for (id, field, a, b) in diffs {
                    s.push_str(&format!("  {id} .{field}: ours {a} golden {b}\n"));
                }
            }
        }
        s
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        1.0
    } else {
        a as f64 / b as f64
    }
}

fn nodes(g: &Value) -> BTreeMap<String, BTreeMap<String, Value>> {
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            let mut m: BTreeMap<String, Value> =
                n.as_object().unwrap().clone().into_iter().collect();
            m.remove("community");
            m.remove("community_name");
            (n["id"].as_str().unwrap().to_string(), m)
        })
        .collect()
}

fn links(g: &Value) -> BTreeMap<String, BTreeMap<String, Value>> {
    g["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let key = format!(
                "{} -{}-> {}",
                e["source"].as_str().unwrap(),
                e["relation"].as_str().unwrap(),
                e["target"].as_str().unwrap()
            );
            (key, e.as_object().unwrap().clone().into_iter().collect())
        })
        .collect()
}

/// (shared, only ours, only golden, field diffs on shared keys).
type SetDiff = (usize, Vec<String>, Vec<String>, Vec<FieldDiff>);
/// (key, field, ours, golden).
type FieldDiff = (String, String, Value, Value);

fn diff_sets(
    ours: &BTreeMap<String, BTreeMap<String, Value>>,
    golden: &BTreeMap<String, BTreeMap<String, Value>>,
) -> SetDiff {
    let mut both = 0;
    let mut fields = Vec::new();
    for (k, a) in ours {
        if let Some(b) = golden.get(k) {
            both += 1;
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for f in keys {
                let (x, y) = (
                    a.get(f).cloned().unwrap_or(Value::Null),
                    b.get(f).cloned().unwrap_or(Value::Null),
                );
                if a.get(f) != b.get(f) {
                    fields.push((k.clone(), f.clone(), x, y));
                }
            }
        }
    }
    let only_ours = ours
        .keys()
        .filter(|k| !golden.contains_key(*k))
        .cloned()
        .collect();
    let only_golden = golden
        .keys()
        .filter(|k| !ours.contains_key(*k))
        .cloned()
        .collect();
    (both, only_ours, only_golden, fields)
}

pub fn compare(ours: &Value, golden: &Value) -> Parity {
    let (nodes_both, nodes_only_ours, nodes_only_golden, node_field_diffs) =
        diff_sets(&nodes(ours), &nodes(golden));
    let (links_both, links_only_ours, links_only_golden, link_field_diffs) =
        diff_sets(&links(ours), &links(golden));
    Parity {
        nodes_both,
        nodes_only_ours,
        nodes_only_golden,
        node_field_diffs,
        links_both,
        links_only_ours,
        links_only_golden,
        link_field_diffs,
    }
}

/// Assert parity with a fixture's golden at the given minimums, printing the full diff on failure.
/// `allowed` lists diff lines (as printed by `Parity::summary`) that are documented, deliberate
/// differences (quirks we do not reproduce); they are removed before thresholds are checked.
pub fn assert_golden(name: &str, min_node: f64, min_link: f64, allowed: &[&str]) {
    let (ours, golden) = build_fixture(name);
    let mut p = compare(&ours, &golden);
    let allowed: BTreeSet<&str> = allowed.iter().copied().collect();
    p.nodes_only_ours.retain(|x| !allowed.contains(x.as_str()));
    p.nodes_only_golden
        .retain(|x| !allowed.contains(x.as_str()));
    p.links_only_ours.retain(|x| !allowed.contains(x.as_str()));
    p.links_only_golden
        .retain(|x| !allowed.contains(x.as_str()));
    let summary = p.summary();
    eprintln!("{name}:\n{summary}");
    assert!(
        p.node_recall() >= min_node
            && p.node_precision() >= min_node
            && p.link_recall() >= min_link
            && p.link_precision() >= min_link,
        "{name} below parity thresholds (nodes {min_node}, links {min_link}):\n{summary}"
    );
}

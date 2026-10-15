//! The merge driver against Graphify's `merge-driver` on the cases in
//! `tests/fixtures/git/merge/` (`docs/graphify-git.md` §4, goldens made by
//! `bench/git_goldens.py`), and against the build's own canonical output.
//!
//! Graphify's result is compared as data: nodes by id, links by endpoint pair, graph attributes.
//! Array order and key order are not compared (Graphify writes NetworkX order; dijkstral writes a
//! build's order, checked byte for byte by `identity_merge_reproduces_every_fixture`). Where
//! dijkstral deliberately differs (§4.6), the test applies exactly that difference to Graphify's
//! result before comparing.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use dijkstral_git::merge::merge;
use serde_json::{json, Value};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// A graph as data: graph attributes, nodes by id, links by unordered endpoints, hyperedges.
#[derive(Debug, PartialEq)]
struct Data {
    graph: Value,
    nodes: BTreeMap<String, Value>,
    links: BTreeMap<(String, String), Value>,
    hyperedges: Option<Value>,
    graph_hyperedges: Option<Value>,
}

fn data(text: &str) -> Data {
    let v: Value = serde_json::from_str(text).unwrap();
    let mut graph = v.get("graph").cloned().unwrap_or_else(|| json!({}));
    let graph_hyperedges = graph.as_object_mut().unwrap().shift_remove("hyperedges");
    let nodes = v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["id"].as_str().unwrap().to_string(), n.clone()))
        .collect();
    let links = v
        .get("links")
        .or_else(|| v.get("edges"))
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            let (s, t) = (l["source"].as_str().unwrap(), l["target"].as_str().unwrap());
            let key = if s <= t { (s, t) } else { (t, s) };
            ((key.0.to_string(), key.1.to_string()), l.clone())
        })
        .collect();
    Data {
        graph,
        nodes,
        links,
        hyperedges: v.get("hyperedges").cloned(),
        graph_hyperedges,
    }
}

fn case_file(case: &str, name: &str) -> Vec<u8> {
    fs::read(fixtures().join("git/merge").join(case).join(name)).unwrap()
}

fn run(case: &str) -> anyhow::Result<String> {
    merge(
        &case_file(case, "base.json"),
        &case_file(case, "current.json"),
        &case_file(case, "other.json"),
    )
    .map(|m| m.json)
}

fn graphify(case: &str) -> Data {
    data(&String::from_utf8(case_file(case, "graphify.json")).unwrap())
}

fn link<'a>(d: &'a mut Data, a: &str, b: &str) -> &'a mut Value {
    let key = if a <= b { (a, b) } else { (b, a) };
    d.links
        .get_mut(&(key.0.to_string(), key.1.to_string()))
        .unwrap()
}

/// Graphify leaves `graph.hyperedges` as the last writer's list and drops the top-level copy;
/// dijkstral writes the merged list in both places, like a build.
fn with_hyperedges(mut d: Data, list: Value) -> Data {
    d.hyperedges = Some(list.clone());
    d.graph_hyperedges = (list != json!([])).then_some(list);
    d
}

#[test]
fn cases_index_matches_the_case_directories() {
    let index: Value =
        serde_json::from_slice(&fs::read(fixtures().join("git/cases.json")).unwrap()).unwrap();
    let cases = index["merge"].as_array().unwrap();
    assert_eq!(cases.len(), 11);
    for c in cases {
        let name = c["case"].as_str().unwrap();
        let has_result = fixtures()
            .join("git/merge")
            .join(name)
            .join("graphify.json")
            .is_file();
        assert_eq!(has_result, c["exit"] == 0, "{name}");
        let ours = run(name);
        match name {
            // Graphify cannot merge without a common ancestor (§4.5); dijkstral takes the union.
            "empty-base" => assert!(ours.is_ok()),
            _ => assert_eq!(ours.is_ok(), c["exit"] == 0, "{name}: {ours:?}"),
        }
        if c["exit"] != 0 {
            assert_eq!(c["current_untouched"], true, "{name}");
        }
    }
}

#[test]
fn union_cases_match_graphify() {
    for case in ["identical", "disjoint"] {
        let mut want = graphify(case);
        want.hyperedges = Some(json!([]));
        assert_eq!(data(&run(case).unwrap()), want, "{case}");
    }
}

#[test]
fn raw_graphs_stay_raw() {
    // Graphify loads a header-less raw graph as a multigraph and writes it back clustered-shaped
    // with `"multigraph":true` and a `key` on every link (§4.4); dijkstral keeps the raw shape.
    let text = run("raw-shape").unwrap();
    assert!(text.starts_with("{\"nodes\":["), "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "nodes",
            "edges",
            "hyperedges",
            "input_tokens",
            "output_tokens"
        ]
    );
    let mut want = graphify("raw-shape");
    for l in want.links.values_mut() {
        assert_eq!(
            l.as_object_mut().unwrap().shift_remove("key"),
            Some(json!(0))
        );
    }
    want.hyperedges = Some(json!([]));
    assert_eq!(data(&text), want);
}

#[test]
fn one_sided_changes_survive_where_graphify_reverts_them() {
    // §4.6 (1): Graphify lets the other side's untouched copy overwrite the current side's edit.
    let mut want = graphify("ours-only");
    want.hyperedges = Some(json!([]));
    want.nodes.get_mut("tools_versions_current").unwrap()["label"] = json!("current (ours)");
    link(&mut want, "tools_release_main", "tools_release_release")["weight"] = json!(3.0);
    assert_eq!(data(&run("ours-only").unwrap()), want);

    let mut want = graphify("attr-conflict");
    want.hyperedges = Some(json!([]));
    // Current relabelled main, other re-clustered it: both edits are kept.
    want.nodes.get_mut("tools_release_main").unwrap()["label"] = json!("main (ours)");
    // A real conflict (both relabelled bump) goes to the other side, as in Graphify.
    assert_eq!(want.nodes["tools_versions_bump"]["label"], "bump (theirs)");
    assert_eq!(data(&run("attr-conflict").unwrap()), want);

    let mut want = graphify("reversed-edge");
    want.hyperedges = Some(json!([]));
    // Other flipped the link and changed its relation, current reweighted it: one link, the
    // other side's direction and relation, the current side's weight.
    let l = link(&mut want, "tools_versions_current", "tools_versions_parse");
    assert_eq!(
        (&l["source"], &l["relation"]),
        (&json!("tools_versions_parse"), &json!("uses"))
    );
    l["weight"] = json!(2.0);
    assert_eq!(data(&run("reversed-edge").unwrap()), want);
}

#[test]
fn deletions_stand_where_graphify_resurrects_them() {
    // §4.6 (2): the current side deleted tools_versions_parse; the other side did not touch it.
    let mut want = graphify("deletion");
    want.hyperedges = Some(json!([]));
    let gone = "tools_versions_parse";
    assert!(want.nodes.remove(gone).is_some());
    want.links.retain(|(a, b), _| a != gone && b != gone);
    let got = data(&run("deletion").unwrap());
    assert_eq!(got, want);
    assert_eq!(got.nodes["tools_release_main"]["label"], "main (theirs)");
}

#[test]
fn hyperedges_merge_by_id() {
    let want = graphify("hyperedges");
    // Graphify keeps only the other side's `graph.hyperedges` and drops the top-level list.
    assert_eq!(want.hyperedges, None);
    let theirs = want.graph_hyperedges.clone().unwrap();
    let ids: Vec<&str> = theirs
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["h_code", "h_one", "h_two", "h_new"]);
    // dijkstral: h_one was deleted by the current side and left alone by the other.
    let merged: Vec<Value> = theirs
        .as_array()
        .unwrap()
        .iter()
        .filter(|h| h["id"] != "h_one")
        .cloned()
        .collect();
    let got = data(&run("hyperedges").unwrap());
    assert_eq!(got, with_hyperedges(want, Value::Array(merged)));
}

#[test]
fn empty_base_is_a_union() {
    let got = data(&run("empty-base").unwrap());
    let cur = String::from_utf8(case_file("empty-base", "current.json")).unwrap();
    let cur: Value = serde_json::from_str(&cur).unwrap();
    // The raw current side (6 nodes + `sys`) and the clustered other side share their ids.
    assert_eq!(got.nodes.len(), cur["nodes"].as_array().unwrap().len());
    assert!(got.nodes.contains_key("sys"));
    assert_eq!(got.nodes["scripts_deploy"]["community"], json!(1));
}

#[test]
fn identity_merge_reproduces_every_fixture() {
    // A merge of a graph with itself is the graph, for every clustered Graphify graph in the
    // fixtures: same arrays in the same order, same keys in the same order (graphify-format.md
    // §2.2–§2.3), and the same bytes where the fixture kept Graphify's compact formatting (the
    // per-language goldens were re-indented when stored).
    let mut files: Vec<PathBuf> = fs::read_dir(fixtures())
        .unwrap()
        .chain(fs::read_dir(fixtures().join("semantic")).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".graphify.json"))
        .collect();
    files.sort();
    let (mut checked, mut bytewise) = (0, 0);
    for f in files {
        let bytes = fs::read(&f).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        if v.as_object().unwrap().keys().next().map(String::as_str) != Some("directed") {
            continue; // raw (--no-cluster) shapes
        }
        let m = merge(&bytes, &bytes, &bytes).unwrap();
        let back: Value = serde_json::from_str(&m.json).unwrap();
        assert!(
            serde_json::to_string(&back).unwrap() == serde_json::to_string(&v).unwrap(),
            "{} is not reproduced",
            f.display()
        );
        if bytes.starts_with(b"{\"directed\"") {
            assert!(
                m.json.as_bytes() == bytes.as_slice(),
                "{} differs in formatting",
                f.display()
            );
            bytewise += 1;
        }
        checked += 1;
    }
    assert!(
        checked >= 40 && bytewise >= 5,
        "{checked} graphs, {bytewise} compact"
    );
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

#[test]
fn identity_merge_reproduces_a_dijkstral_build() {
    for lang in ["python", "typescript", "rust"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(lang);
        copy_dir(&fixtures().join(lang), &root);
        let opts = dijkstral_core::BuildOptions {
            root: root.clone(),
            out: None,
            threads: 2,
            cluster: true,
        };
        dijkstral_core::build(&opts).unwrap();
        let built = fs::read(root.join("dijkstral-out/graph.json")).unwrap();
        let m = merge(&built, &built, &built).unwrap();
        assert!(m.json.as_bytes() == built.as_slice(), "{lang}");
    }
}

#[test]
fn merge_is_deterministic_and_order_independent() {
    let base = case_file("disjoint", "base.json");
    let cur = case_file("disjoint", "current.json");
    let oth = case_file("disjoint", "other.json");
    let a = merge(&base, &cur, &oth).unwrap().json;
    // No conflicting values in this case, so the sides can swap.
    let b = merge(&base, &oth, &cur).unwrap().json;
    assert_eq!(a, b);
    // Shuffled input arrays give the same bytes.
    let mut v: Value = serde_json::from_slice(&cur).unwrap();
    v["nodes"].as_array_mut().unwrap().reverse();
    v["links"].as_array_mut().unwrap().reverse();
    let c = merge(&base, &serde_json::to_vec(&v).unwrap(), &oth)
        .unwrap()
        .json;
    assert_eq!(a, c);
}

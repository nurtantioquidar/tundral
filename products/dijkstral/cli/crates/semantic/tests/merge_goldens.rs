//! Merge against Graphify's goldens (`docs/graphify-semantic.md` §7, §8): the recorded Codex
//! reply (`mixed.transcript/req-0001.json`) and the crafted `merge_probe{1,2}.reply.json`, fed
//! through `merge` on dijkstral's code graph of `tests/fixtures/semantic/mixed`, must give
//! Graphify's clustered `graph.json` for the same reply: every node and link with every field,
//! the same communities and array order, and the hyperedges.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use dijkstral_core::graph::Graph;
use dijkstral_core::pipeline::{build_semantic, cluster_only, Semantic, SemanticReport};
use dijkstral_core::BuildOptions;
use dijkstral_semantic::merge::{merge, Dispatched, MergeInput};
use serde_json::Value;

/// Graphify prints the two file kinds of a `crosses file types (a ↔ b)` surprise in Python set
/// order, which changes from run to run; dijkstral sorts them. Sort both sides before comparing.
fn sort_kind_pairs(text: &str) -> String {
    let kinds = ["code", "doc", "image", "paper"];
    let mut out = text.to_string();
    for a in kinds {
        for b in kinds {
            if a > b {
                for arrow in ["\u{2194}", "\\u2194"] {
                    out = out.replace(&format!("({a} {arrow} {b})"), &format!("({b} {arrow} {a})"));
                }
            }
        }
    }
    out
}

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

/// The eight files Graphify dispatched for `mixed/` (office files as their conversions).
const DISPATCHED: &[&str] = &[
    "README.md",
    "config/queue.yaml",
    "docs/glossary.txt",
    "docs/guide.md",
    "graphify-out/converted/capacity_e7fd8f6a.md",
    "graphify-out/converted/runbook_b3207a85.md",
    "images/architecture.png",
    "papers/write_through.pdf",
];

/// A semantic stage that merges fixed replies.
struct Replay {
    replies: Vec<MergeInput>,
    tokens: (u64, u64),
}

impl Semantic for Replay {
    fn fingerprint(&self, _root: &Path) -> Result<String> {
        Ok("replay".into())
    }
    fn apply(&self, root: &Path, _out: &Path, graph: &mut Graph) -> Result<SemanticReport> {
        let dispatched: Vec<Dispatched> = DISPATCHED
            .iter()
            .map(|rel| {
                let text = rel
                    .ends_with(".md")
                    .then(|| fs::read_to_string(root.join(rel)).ok())
                    .flatten()
                    .or_else(|| {
                        (rel.ends_with(".txt") || rel.ends_with(".yaml"))
                            .then(|| fs::read_to_string(root.join(rel)).unwrap())
                    });
                Dispatched::new(*rel, text)
            })
            .collect();
        let stats = merge(graph, &dispatched, &self.replies);
        Ok(SemanticReport {
            input_tokens: self.tokens.0,
            output_tokens: self.tokens.1,
            nodes_added: stats.nodes_added,
            edges_added: stats.edges_added,
            notes: stats.notes(),
            ..Default::default()
        })
    }
}

struct Ours {
    graph: Value,
    analysis: String,
}

fn build_with_reply(reply: MergeInput, tokens: (u64, u64)) -> Ours {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("mixed");
    copy_dir(&fixtures().join("mixed"), &root);
    let stage = Replay {
        replies: vec![reply],
        tokens,
    };
    build_semantic(
        &BuildOptions {
            root: root.clone(),
            out: None,
            threads: 2,
            cluster: true,
        },
        &[],
        Some(&stage),
    )
    .unwrap();
    let out = root.join("dijkstral-out");
    Ours {
        graph: serde_json::from_slice(&fs::read(out.join("graph.json")).unwrap()).unwrap(),
        analysis: fs::read_to_string(out.join(".graphify_analysis.json")).unwrap(),
    }
}

fn golden(name: &str) -> Value {
    serde_json::from_slice(&fs::read(fixtures().join(name)).unwrap()).unwrap()
}

fn transcript_reply(file: &str) -> (MergeInput, (u64, u64)) {
    let t = golden(file);
    let text = t["response"]["text"].as_str().unwrap();
    let usage = &t["response"]["usage"];
    (
        MergeInput::parse(text, false).unwrap(),
        (
            usage["input_tokens"].as_u64().unwrap(),
            usage["output_tokens"].as_u64().unwrap(),
        ),
    )
}

type Items = BTreeMap<String, BTreeMap<String, Value>>;

fn nodes(g: &Value) -> Items {
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            let m: BTreeMap<String, Value> = n.as_object().unwrap().clone().into_iter().collect();
            (n["id"].as_str().unwrap().to_string(), m)
        })
        .collect()
}

fn links(g: &Value) -> Items {
    g["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let key = format!(
                "{} -{}-> {}",
                e["source"].as_str().unwrap(),
                e["relation"].as_str().unwrap_or("<none>"),
                e["target"].as_str().unwrap()
            );
            (key, e.as_object().unwrap().clone().into_iter().collect())
        })
        .collect()
}

/// Differences as printable lines: `only ours/golden <key>` and `<key> .<field>: ours … golden …`.
fn diff(ours: &Items, golden: &Items) -> Vec<String> {
    let mut out = Vec::new();
    for (k, a) in ours {
        match golden.get(k) {
            None => out.push(format!("only ours {k}")),
            Some(b) => {
                let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
                for f in keys {
                    if a.get(f) != b.get(f) {
                        out.push(format!(
                            "{k} .{f}: ours {} golden {}",
                            a.get(f).map_or("<absent>".into(), Value::to_string),
                            b.get(f).map_or("<absent>".into(), Value::to_string)
                        ));
                    }
                }
            }
        }
    }
    for k in golden.keys() {
        if !ours.contains_key(k) {
            out.push(format!("only golden {k}"));
        }
    }
    out
}

/// Full parity: nodes and links field for field (communities included), array order, top-level
/// keys, `graph` and `hyperedges`, after removing the `allowed` difference lines.
fn assert_matches(name: &str, ours: &Value, golden: &Value, allowed: &[&str]) {
    let mut diffs = diff(&nodes(ours), &nodes(golden));
    diffs.extend(diff(&links(ours), &links(golden)));
    diffs.retain(|d| !allowed.contains(&d.as_str()));
    assert!(
        diffs.is_empty(),
        "{name}: differs from Graphify's golden:\n  {}",
        diffs.join("\n  ")
    );
    let ids = |g: &Value, k: &str, f: &dyn Fn(&Value) -> String| -> Vec<String> {
        g[k].as_array().unwrap().iter().map(f).collect()
    };
    let node_id = |n: &Value| n["id"].as_str().unwrap().to_string();
    let link_id = |e: &Value| format!("{} {} {}", e["source"], e["target"], e["relation"]);
    assert_eq!(
        ids(ours, "nodes", &node_id),
        ids(golden, "nodes", &node_id),
        "{name}: node order"
    );
    assert_eq!(
        ids(ours, "links", &link_id),
        ids(golden, "links", &link_id),
        "{name}: link order"
    );
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(ours), keys(golden), "{name}: top-level keys");
    assert_eq!(ours["graph"], golden["graph"], "{name}: graph attributes");
    assert_eq!(
        ours["hyperedges"], golden["hyperedges"],
        "{name}: hyperedges"
    );
}

#[test]
fn codex_reply_matches_graphify_extract() {
    let (reply, tokens) = transcript_reply("mixed.transcript/req-0001.json");
    let ours = build_with_reply(reply, tokens);
    assert_matches(
        "mixed.extract",
        &ours.graph,
        &golden("mixed.extract.graphify.json"),
        &[],
    );
    // The analysis sidecar: same communities, gods, surprises and this run's tokens.
    let want = sort_kind_pairs(
        &fs::read_to_string(fixtures().join("mixed.extract.graphify_analysis.json")).unwrap(),
    );
    assert!(
        ours.analysis == want,
        "analysis differs:\nours   {}\ngolden {}",
        ours.analysis,
        want
    );
}

#[test]
fn deep_reply_matches_graphify_deep_extract() {
    let (reply, tokens) = transcript_reply("mixed.transcript/req-0003.json");
    let ours = build_with_reply(reply, tokens);
    assert_matches(
        "mixed.deep",
        &ours.graph,
        &golden("mixed.deep.graphify.json"),
        &[],
    );
}

/// A crafted reply served by the static shim to `graphify extract --backend openai` on `mixed/`.
/// `allowed` lists documented differences, each a line as printed by `diff`.
fn assert_probe(name: &str, allowed: &[&str]) {
    let reply = MergeInput::parse(
        &fs::read_to_string(fixtures().join(format!("{name}.reply.json"))).unwrap(),
        false,
    )
    .unwrap();
    let ours = build_with_reply(reply, (0, 0));
    assert_matches(
        name,
        &ours.graph,
        &golden(&format!("{name}.extract.graphify.json")),
        allowed,
    );
}

/// Validation and merging: dropped items, id rewriting, attributes, hyperedges (§7).
#[test]
fn merge_probe1_matches_graphify() {
    assert_probe("merge_probe1", &[]);
}

/// Id rewriting collisions, `file_type` defaults, the evidence check (§7.2).
#[test]
fn merge_probe2_matches_graphify() {
    assert_probe("merge_probe2", &[]);
}

/// Semantic edges on code links: calls, contains, imports, rationale and reversed pairs (§7.3).
#[test]
fn fold_probe1_matches_graphify() {
    assert_probe("fold_probe1", &[]);
}

/// 330 pairs of relations, both orders and both orientations: the semantic fold's tiers and
/// name order (§7.3).
#[test]
fn fold_probe2_matches_graphify() {
    assert_probe("fold_probe2", &[]);
}

/// Seven AMBIGUOUS links and four hyperedges (order, missing confidence, missing id).
#[test]
fn report_probe_matches_graphify() {
    assert_probe("report_probe", &[]);
}

/// `dijkstral cluster-only` on Graphify's graph `input` reproduces `graphify cluster-only
/// --no-label`: graph, analysis and report.
fn assert_cluster_only(input: &str, prefix: &str) {
    let doc = golden(input);
    let out = cluster_only(&doc, "mixed").unwrap();
    let want_graph = golden(&format!("{prefix}.graphify.json"));
    let ours: Value = serde_json::from_str(&out.graph_json).unwrap();
    assert_matches(prefix, &ours, &want_graph, &[]);
    let want = sort_kind_pairs(
        &fs::read_to_string(fixtures().join(format!("{prefix}.graphify_analysis.json"))).unwrap(),
    );
    assert!(
        out.analysis_json == want,
        "{prefix}: analysis differs:\nours   {}\ngolden {}",
        out.analysis_json,
        want
    );
    let norm = |t: &str| {
        let mut lines: Vec<String> = t.split('\n').map(str::to_string).collect();
        lines[0] = "# Graph Report".into();
        lines
            .join("\n")
            .replace("`graphify query", "`dijkstral query")
    };
    let want_report =
        fs::read_to_string(fixtures().join(format!("{prefix}.graphify_report.md"))).unwrap();
    assert_eq!(norm(&out.report), norm(&want_report), "{prefix}: report");
}

/// Hyperedges, an AMBIGUOUS link and its low-confidence hint, `crosses file types` surprises.
#[test]
fn cluster_only_on_a_semantic_graph_matches_graphify() {
    assert_cluster_only("mixed.extract.graphify.json", "mixed.extract.cluster_only");
}

/// The `Ambiguous Edges` section, the high-ambiguity gap, hints taking six of seven question
/// slots, and hyperedge lines without a score or confidence.
#[test]
fn cluster_only_report_probe_matches_graphify() {
    assert_cluster_only(
        "report_probe.extract.graphify.json",
        "report_probe.cluster_only",
    );
}

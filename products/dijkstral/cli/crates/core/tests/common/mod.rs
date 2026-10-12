//! Golden comparison against Graphify's output for the fixtures in `cli/tests/fixtures`.
//!
//! The fixture is copied to a temporary directory (not a git repository, as when the goldens were
//! made), built, and compared to `<name>.clustered.graphify.json` as sets: nodes by id with all
//! fields except `community`/`community_name`, links by (source, target, relation) with all fields.
//! Community agreement is measured separately (`community_agreement`), because one extraction
//! difference moves community ids everywhere while the node and link sets stay comparable.
//!
//! Report goldens (`docs/graphify-format.md` §17.6):
//!
//! - `<name>.graphify_analysis.json`: `graphify extract --code-only`'s sidecar, byte for byte.
//! - `<name>.graphify_report.md`: `graphify update`'s report (the no-model report over the same
//!   graph; for `mixed`, made on a copy without `README.md`, which `update` would index).
//! - `<name>.cluster_only.graphify_{analysis.json,report.md}`: `graphify cluster-only --no-label`
//!   run on `<name>.clustered.graphify.json`; `dijkstral cluster-only` must reproduce them from the
//!   same input, which tests clustering, analysis and the report independently of extraction.
//!
//! Reports are compared after `normalise_report`: the title line carries the directory name and
//! the date, and the hints name `graphify` where dijkstral names itself.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use dijkstral_core::pipeline::cluster_only;
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

/// One fixture built by dijkstral, next to Graphify's goldens.
pub struct Built {
    pub ours: Value,
    pub golden: Value,
    pub analysis: String,
    pub report: String,
}

fn golden_text(file: &str) -> String {
    fs::read_to_string(fixtures().join(file)).unwrap()
}

fn golden_graph(name: &str) -> Value {
    serde_json::from_slice(
        &fs::read(fixtures().join(format!("{name}.clustered.graphify.json"))).unwrap(),
    )
    .unwrap()
}

/// Build the fixture and return our outputs and Graphify's golden graph.
pub fn build_fixture(name: &str) -> Built {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(name);
    copy_dir(&fixtures().join(name), &root);
    build(&BuildOptions {
        root: root.clone(),
        out: None,
        threads: 2,
        cluster: true,
    })
    .unwrap();
    let out = root.join("dijkstral-out");
    let ours: Value = serde_json::from_slice(&fs::read(out.join("graph.json")).unwrap()).unwrap();
    Built {
        ours,
        golden: golden_graph(name),
        analysis: fs::read_to_string(out.join(".graphify_analysis.json")).unwrap(),
        report: fs::read_to_string(out.join("GRAPH_REPORT.md")).unwrap(),
    }
}

/// Mask what legitimately differs between Graphify's report and ours: the title line (directory
/// name and date) and the command named in hints.
pub fn normalise_report(text: &str) -> String {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if let Some(first) = lines.first_mut() {
        if first.starts_with("# Graph Report - ") {
            *first = "# Graph Report - <name>  (<date>)".into();
        }
    }
    lines
        .join("\n")
        .replace("`graphify update", "`dijkstral update")
        .replace("`graphify query", "`dijkstral query")
}

/// The `## ` section headings of a report, in order.
pub fn report_sections(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with("## "))
        .map(|l| l.split(" (").next().unwrap().to_string())
        .collect()
}

/// Line diff summary (first differing lines) for assertion messages.
pub fn first_differences(a: &str, b: &str, max: usize) -> String {
    let (al, bl): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let mut out = String::new();
    let mut shown = 0;
    for i in 0..al.len().max(bl.len()) {
        let (x, y) = (al.get(i).copied(), bl.get(i).copied());
        if x != y {
            out.push_str(&format!(
                "  line {}:\n    ours   {:?}\n    golden {:?}\n",
                i + 1,
                x.unwrap_or(""),
                y.unwrap_or("")
            ));
            shown += 1;
            if shown == max {
                break;
            }
        }
    }
    out
}

/// How far our communities agree with Graphify's on the nodes both graphs have: exact id
/// matches, and the partition similarity (NMI, ARI) independent of numbering.
#[derive(Debug, Clone, Copy)]
pub struct CommunityAgreement {
    pub shared: usize,
    pub equal: usize,
    pub nmi: f64,
    pub ari: f64,
}

impl CommunityAgreement {
    pub fn exact(&self) -> f64 {
        ratio(self.equal, self.shared)
    }
}

pub fn community_agreement(ours: &Value, golden: &Value) -> CommunityAgreement {
    let map = |g: &Value| -> HashMap<String, i64> {
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| Some((n["id"].as_str()?.to_string(), n["community"].as_i64()?)))
            .collect()
    };
    let (a, b) = (map(ours), map(golden));
    let pairs: Vec<(i64, i64)> = a
        .iter()
        .filter_map(|(id, &ca)| b.get(id).map(|&cb| (ca, cb)))
        .collect();
    let equal = pairs.iter().filter(|(x, y)| x == y).count();
    let (nmi, ari) = partition_similarity(&pairs);
    CommunityAgreement {
        shared: pairs.len(),
        equal,
        nmi,
        ari,
    }
}

/// (NMI with arithmetic-mean normalisation, adjusted Rand index) of two labelings.
fn partition_similarity(pairs: &[(i64, i64)]) -> (f64, f64) {
    let n = pairs.len() as f64;
    if pairs.is_empty() {
        return (1.0, 1.0);
    }
    let mut joint: HashMap<(i64, i64), f64> = HashMap::new();
    let mut ca: HashMap<i64, f64> = HashMap::new();
    let mut cb: HashMap<i64, f64> = HashMap::new();
    for &(x, y) in pairs {
        *joint.entry((x, y)).or_default() += 1.0;
        *ca.entry(x).or_default() += 1.0;
        *cb.entry(y).or_default() += 1.0;
    }
    let h = |m: &HashMap<i64, f64>| -> f64 {
        m.values().map(|&c| -(c / n) * (c / n).ln()).sum::<f64>()
    };
    let mi: f64 = joint
        .iter()
        .map(|(&(x, y), &c)| (c / n) * ((c * n) / (ca[&x] * cb[&y])).ln())
        .sum();
    let (ha, hb) = (h(&ca), h(&cb));
    let nmi = if ha + hb == 0.0 {
        1.0
    } else {
        2.0 * mi / (ha + hb)
    };
    let comb = |c: f64| c * (c - 1.0) / 2.0;
    let index: f64 = joint.values().map(|&c| comb(c)).sum();
    let (sa, sb): (f64, f64) = (
        ca.values().map(|&c| comb(c)).sum(),
        cb.values().map(|&c| comb(c)).sum(),
    );
    let expected = sa * sb / comb(n);
    let max = (sa + sb) / 2.0;
    let ari = if max == expected {
        1.0
    } else {
        (index - expected) / (max - expected)
    };
    (nmi, ari)
}

/// `dijkstral cluster-only` on Graphify's clustered golden must reproduce Graphify's
/// `cluster-only --no-label` exactly: community of every node, node and link array order, the
/// analysis sidecar and the report. `unordered_surprises` names fixtures whose surprise ties
/// Graphify orders differently (`docs/graphify-format.md` §17.4); there the surprises are compared
/// as sets.
pub fn assert_cluster_only(name: &str, unordered_surprises: bool) {
    let golden = golden_graph(name);
    let out = cluster_only(&golden, name).unwrap();
    let ours: Value = serde_json::from_str(&out.graph_json).unwrap();

    let agreement = community_agreement(&ours, &golden);
    let nodes = golden["nodes"].as_array().unwrap().len();
    assert!(
        agreement.shared == nodes && agreement.equal == nodes,
        "{name}: communities differ from Graphify's on its own graph: {agreement:?}"
    );
    let ids = |g: &Value, k: &str, f: &dyn Fn(&Value) -> String| -> Vec<String> {
        g[k].as_array().unwrap().iter().map(f).collect()
    };
    let node_id = |n: &Value| n["id"].as_str().unwrap().to_string();
    let link_id = |e: &Value| format!("{} {} {}", e["source"], e["target"], e["relation"]);
    assert_eq!(
        ids(&ours, "nodes", &node_id),
        ids(&golden, "nodes", &node_id),
        "{name}: node order"
    );
    assert_eq!(
        ids(&ours, "links", &link_id),
        ids(&golden, "links", &link_id),
        "{name}: link order"
    );

    let mut analysis = out.analysis_json.clone();
    let mut want = golden_text(&format!("{name}.cluster_only.graphify_analysis.json"));
    let mut report = normalise_report(&out.report);
    let mut want_report = normalise_report(&golden_text(&format!(
        "{name}.cluster_only.graphify_report.md"
    )));
    if unordered_surprises {
        analysis = sort_surprises_json(&analysis);
        want = sort_surprises_json(&want);
        report = sort_surprise_lines(&report);
        want_report = sort_surprise_lines(&want_report);
    }
    assert!(
        analysis == want,
        "{name}: cluster-only analysis differs:\n{}",
        first_differences(&analysis, &want, 6)
    );
    assert!(
        report == want_report,
        "{name}: cluster-only report differs:\n{}",
        first_differences(&report, &want_report, 6)
    );
}

fn sort_surprises_json(text: &str) -> String {
    let mut v: Value = serde_json::from_str(text).unwrap();
    if let Some(list) = v["surprises"].as_array_mut() {
        list.sort_by_key(|s| s.to_string());
    }
    v.to_string()
}

fn sort_surprise_lines(text: &str) -> String {
    let start = text.find("## Surprising Connections").unwrap();
    let end = start + text[start..].find("\n\n## ").unwrap();
    let section = &text[start..end];
    let mut head = section.lines().take(1).collect::<Vec<_>>().join("\n");
    let lines: Vec<&str> = section.lines().skip(1).collect();
    let mut items: Vec<String> = lines.chunks(2).map(|c| c.join("\n")).collect();
    items.sort();
    for i in items {
        head.push('\n');
        head.push_str(&i);
    }
    format!("{}{}{}", &text[..start], head, &text[end..])
}

/// Our extract-time sidecars against Graphify's (`graphify extract` analysis, `graphify update`
/// report). `exact` fixtures must match after normalisation; the others share the shape (keys
/// and section order) while their contents follow the graph differences listed in `ALLOWED`.
pub fn check_report_goldens(name: &str, built: &Built, exact: bool) {
    let want = golden_text(&format!("{name}.graphify_analysis.json"));
    let want_report = normalise_report(&golden_text(&format!("{name}.graphify_report.md")));
    let report = normalise_report(&built.report);
    let (a, b): (Value, Value) = (
        serde_json::from_str(&built.analysis).unwrap(),
        serde_json::from_str(&want).unwrap(),
    );
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&a), keys(&b), "{name}: analysis keys");
    assert_eq!(
        report_sections(&report),
        report_sections(&want_report),
        "{name}: report sections"
    );
    let analysis_same = built.analysis == want;
    let report_same = report == want_report;
    eprintln!(
        "{name}: analysis {} / report {} Graphify's",
        if analysis_same {
            "equals"
        } else {
            "differs from"
        },
        if report_same {
            "equals"
        } else {
            "differs from"
        }
    );
    if exact {
        assert!(
            analysis_same,
            "{name}: analysis differs:\n{}",
            first_differences(&built.analysis, &want, 6)
        );
        assert!(
            report_same,
            "{name}: report differs:\n{}",
            first_differences(&report, &want_report, 6)
        );
    }
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
///
/// Community agreement is printed and returned with the build (`Built`) for the caller to check:
/// it is exact only where extraction matches Graphify's (see each golden's `COMMUNITY` note).
pub fn assert_golden(
    name: &str,
    min_node: f64,
    min_link: f64,
    allowed: &[&str],
) -> (Built, CommunityAgreement) {
    let built = build_fixture(name);
    let (ours, golden) = (&built.ours, &built.golden);
    let mut p = compare(ours, golden);
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
    let agreement = community_agreement(ours, golden);
    eprintln!(
        "{name}: community equal on {}/{} shared nodes ({:.3}), NMI {:.3}, ARI {:.3}",
        agreement.equal,
        agreement.shared,
        agreement.exact(),
        agreement.nmi,
        agreement.ari
    );
    (built, agreement)
}

/// How a fixture's communities must agree with Graphify's on a full build.
pub enum Communities {
    /// Identical graphs: identical community ids (and so Graphify's array order).
    Exact,
    /// The graph differs by `ALLOWED` entries, which reshuffle Leiden's size-ranked ids even when
    /// the partition barely moves; gate on normalised mutual information instead.
    MinNmi(f64),
}

/// The full golden check for one fixture: node/link parity, community agreement (see
/// `Communities`), Graphify's own graph re-clustered exactly, and the
/// report goldens (byte-exact when `exact_report`).
pub fn assert_fixture(
    name: &str,
    allowed: &[&str],
    communities: Communities,
    exact_report: bool,
    unordered_surprises: bool,
) {
    let (built, agreement) = assert_golden(name, 1.0, 1.0, allowed);
    match communities {
        Communities::Exact => assert!(
            agreement.exact() == 1.0,
            "{name}: community ids differ on {:.3} of shared nodes",
            1.0 - agreement.exact()
        ),
        Communities::MinNmi(min) => assert!(
            agreement.nmi >= min,
            "{name}: community NMI {:.3} below {min}",
            agreement.nmi
        ),
    }
    if agreement.exact() == 1.0 {
        // Same communities and node set: the arrays must come out in Graphify's order too.
        let order = |g: &Value| -> Vec<String> {
            g["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["id"].as_str().unwrap().to_string())
                .collect()
        };
        let (ours, golden) = (order(&built.ours), order(&built.golden));
        let in_ours: BTreeSet<&String> = ours.iter().collect();
        let in_golden: BTreeSet<&String> = golden.iter().collect();
        let ours: Vec<&String> = ours.iter().filter(|i| in_golden.contains(i)).collect();
        let golden: Vec<&String> = golden.iter().filter(|i| in_ours.contains(i)).collect();
        assert_eq!(ours, golden, "{name}: node array order");
    }
    assert_cluster_only(name, unordered_surprises);
    check_report_goldens(name, &built, exact_report);
}

//! The call-flow page against Graphify's own (`docs/graphify-exports.md` §13).
//!
//! `tests/fixtures/exports/<lang>/callflow/page.json` holds the data of
//! `graphify export callflow-html` for each clustered fixture: headings, prose, Mermaid text
//! without the `%%{init}%%`/`classDef` styling lines, and table cells (Graphify's page markup is
//! not stored). Normalisation: the product name (`graphify` → `dijkstral`) and the generation
//! time. Components whose rules are established must match exactly on every fixture; the
//! diagrams (node selection and order, §13.5 Open F4) and a few heuristics are compared and
//! their match rate reported, with a floor so regressions fail.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use dijkstral_export::callflow::{self, CallflowOptions, Lang, LangChoice};
use dijkstral_export::nx::NxGraph;
use serde_json::{json, Value};

const LANGS: [&str; 15] = [
    "python",
    "typescript",
    "javascript",
    "rust",
    "go",
    "java",
    "mixed",
    "c",
    "cpp",
    "csharp",
    "kotlin",
    "php",
    "ruby",
    "scala",
    "swift",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn graph(lang: &str) -> NxGraph {
    let path = fixtures().join(format!("graphs-0.9.86/{lang}.clustered.graphify.json"));
    NxGraph::from_json(serde_json::from_slice(&fs::read(path).unwrap()).unwrap()).unwrap()
}

fn options(project: &str) -> CallflowOptions {
    CallflowOptions {
        project: project.into(),
        lang: LangChoice::Fixed(Lang::En),
        generated: "<time>".into(),
        ..CallflowOptions::default()
    }
}

fn normalise(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.replace("graphify", "dijkstral")),
        Value::Array(a) => Value::Array(a.iter().map(normalise).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), normalise(v))).collect())
        }
        v => v.clone(),
    }
}

/// Named comparable pieces of a page's data.
fn components(d: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    out.insert("title".into(), d["title"].clone());
    out.insert("subtitle".into(), d["subtitle"].clone());
    out.insert("nav".into(), d["nav"].clone());
    let secs = d["sections"].as_array().unwrap();
    for (k, s) in secs.iter().enumerate() {
        let id = s["id"].as_str().unwrap();
        let p = format!("{k:02}:{id}");
        out.insert(format!("{p}/heading"), s["heading"].clone());
        out.insert(format!("{p}/h"), json!([s["h3"], s["h4"]]));
        match id {
            "overview" => {
                let lines = s["diagrams"][0].as_array().unwrap();
                let (edges, nodes): (Vec<&Value>, Vec<&Value>) = lines
                    .iter()
                    .partition(|l| l.as_str().unwrap().contains("-->"));
                out.insert(format!("{p}/nodes"), json!(nodes));
                out.insert(format!("{p}/edges"), json!(edges));
                out.insert(format!("{p}/layers"), s["tables"][0].clone());
                out.insert(format!("{p}/chain"), s["chain"].clone());
            }
            "stats" => {
                out.insert(format!("{p}/tables"), s["tables"].clone());
                out.insert(format!("{p}/footer"), s["paragraphs"].clone());
            }
            _ => {
                out.insert(format!("{p}/intro"), s["paragraphs"][0].clone());
                out.insert(format!("{p}/note"), s["paragraphs"][1].clone());
                out.insert(format!("{p}/diagram"), s["diagrams"][0].clone());
                out.insert(format!("{p}/files"), s["tables"][1].clone());
                for (r, row) in s["tables"][0].as_array().unwrap().iter().enumerate() {
                    for (c, col) in ["n", "node", "type", "callers", "callees", "desc"]
                        .iter()
                        .enumerate()
                    {
                        out.insert(format!("{p}/calls/{r:02}/{col}"), row[c].clone());
                    }
                }
            }
        }
    }
    out
}

/// Component kind for the rate table: the last path segment, or `calls/<column>`.
fn kind(name: &str) -> String {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() >= 4 && parts[1] == "calls" {
        format!("calls/{}", parts[3])
    } else {
        parts.last().unwrap().to_string()
    }
}

/// Components that must match exactly on every fixture.
const EXACT: &[&str] = &[
    "title",
    "subtitle",
    "nav",
    "heading",
    "h",
    "nodes",
    "layers",
    "tables",
    "footer",
    "intro",
    "files",
    "calls/n",
    "calls/node",
    "calls/desc",
];

/// Minimum match rate (percent) for the heuristic components.
const FLOORS: &[(&str, f64)] = &[
    ("edges", 25.0),
    ("chain", 25.0),
    ("note", 98.0),
    ("diagram", 69.0),
    ("calls/type", 98.0),
    ("calls/callers", 99.5),
    ("calls/callees", 99.0),
];

#[test]
fn page_data_matches_graphify() {
    let mut tally: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut failures = Vec::new();
    for lang in LANGS {
        let want = normalise(
            &serde_json::from_slice(
                &fs::read(fixtures().join(format!("exports/{lang}/callflow/page.json"))).unwrap(),
            )
            .unwrap(),
        );
        let ours = callflow::build(&graph(lang), &options(lang)).data();
        let (w, o) = (components(&want), components(&ours));
        let keys: std::collections::BTreeSet<&String> = w.keys().chain(o.keys()).collect();
        for k in keys {
            let kd = kind(k);
            let same = w.get(k) == o.get(k);
            let e = tally.entry(kd.clone()).or_default();
            e.1 += 1;
            if same {
                e.0 += 1;
            } else if EXACT.contains(&kd.as_str()) {
                failures.push(format!(
                    "{lang} {k}\n  graphify: {}\n  ours:     {}",
                    w.get(k).map_or("-".into(), |v| v.to_string()),
                    o.get(k).map_or("-".into(), |v| v.to_string())
                ));
            } else if std::env::var_os("CALLFLOW_DIFF").is_some() {
                eprintln!(
                    "~ {lang} {k}\n  graphify: {}\n  ours:     {}",
                    w.get(k).map_or("-".into(), |v| v.to_string()),
                    o.get(k).map_or("-".into(), |v| v.to_string())
                );
            }
        }
    }
    eprintln!("component match rates over {} fixtures:", LANGS.len());
    for (k, (ok, n)) in &tally {
        eprintln!(
            "  {k:16} {ok:4}/{n:<4} {:5.1}%",
            100.0 * *ok as f64 / *n as f64
        );
    }
    for (k, floor) in FLOORS {
        if let Some((ok, n)) = tally.get(*k) {
            let rate = 100.0 * *ok as f64 / *n as f64;
            assert!(rate >= *floor, "{k}: {rate:.1}% below the {floor}% floor");
        }
    }
    assert!(
        failures.is_empty(),
        "{} exact mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// `(golden directory, multirepo golden)`: global graphs, whose links carry `_src`/`_tgt` with
/// `source`/`target` often reversed. Graphify draws them from `_src` to `_tgt`; the global graph
/// also has nodes without a community, grouped as `unknown`.
const GLOBAL: [(&str, &str); 2] = [
    ("global_direction", "global_direction"),
    ("global_py_go", "global_py_go.3"),
];

/// Components of the global pages allowed to differ, each an open heuristic of §13.
const GLOBAL_ALLOWED: [(&str, &str); 6] = [
    // F4: order of unlinked nodes (Graphify puts `é` before `F`).
    ("global_direction", "02:fpy-apy/diagram"),
    // F7: the overview label of a pair with several relations (`references` against `imports`).
    ("global_py_go", "00:overview/edges"),
    // F11: Go module labels shown as `com google uuid`.
    ("global_py_go", "01:extract-pipeline/calls/16/callees"),
    ("global_py_go", "01:extract-pipeline/calls/27/callees"),
    // F4: which 24 links a 116-node section shows, and the order of a section's nodes.
    ("global_py_go", "01:extract-pipeline/diagram"),
    ("global_py_go", "02:tests-fixtures/diagram"),
];

/// Every component of the global graphs' pages matches Graphify's: callers, callees, relation
/// summaries, internal link counts and diagram links all follow `_src` → `_tgt`.
#[test]
fn global_graphs_follow_the_true_direction() {
    let mut failures = Vec::new();
    for (golden, input) in GLOBAL {
        let want = normalise(
            &serde_json::from_slice(
                &fs::read(fixtures().join(format!("exports/{golden}/callflow/page.json"))).unwrap(),
            )
            .unwrap(),
        );
        let g = NxGraph::read(&fixtures().join(format!("multirepo/goldens/{input}.graph.json")))
            .unwrap();
        let ours = callflow::build(&g, &options(input)).data();
        let (w, o) = (components(&want), components(&ours));
        let keys: std::collections::BTreeSet<&String> = w.keys().chain(o.keys()).collect();
        for k in keys {
            let allowed = GLOBAL_ALLOWED.contains(&(golden, k.as_str()));
            if (w.get(k) == o.get(k)) == allowed {
                failures.push(format!(
                    "{golden} {k} (allowed: {allowed})\n  graphify: {}\n  ours:     {}",
                    w.get(k).map_or("-".into(), |v| v.to_string()),
                    o.get(k).map_or("-".into(), |v| v.to_string())
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Communities are grouped by Python's `str` of the value, a missing or `null` one as
/// `unknown` (Graphify's page for this graph: layers `unknown` 9, `0` 6, `x` 3, `7.5` 2,
/// `True` 1; 5 communities), and a single cross-section link is labelled without ` x1`.
#[test]
fn communities_are_grouped_by_their_text() {
    let comms = [
        (json!(0), 6),
        (Value::Null, 5),
        (json!(null), 4),
        (json!("x"), 3),
        (json!(7.5), 2),
        (json!(true), 1),
    ];
    let mut nodes = Vec::new();
    for (k, (c, n)) in comms.iter().enumerate() {
        for _ in 0..*n {
            let i = nodes.len();
            let mut node = json!({"id": format!("n{i}"), "label": format!("Fn{i}()"),
                                  "file_type": "code", "source_file": format!("m{}.py", i % 3)});
            if k != 1 {
                node["community"] = c.clone();
            }
            nodes.push(node);
        }
    }
    let links: Vec<Value> = (1..nodes.len())
        .map(|i| {
            json!({"source": format!("n{}", i - 1), "target": format!("n{i}"),
                        "relation": "calls", "confidence": "EXTRACTED"})
        })
        .collect();
    let g = NxGraph::from_json(json!({"directed": false, "multigraph": false, "graph": {},
                                      "nodes": nodes, "links": links}))
    .unwrap();
    let d = callflow::build(&g, &options("comm")).data();
    assert_eq!(
        d["subtitle"],
        "Generated from dijkstral knowledge graph: 21 nodes, 20 edges, 5 communities. Commit: unknown"
    );
    let layers: Vec<(String, String)> = d["sections"][0]["tables"][0]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .map(|r| (r[1].as_str().unwrap().into(), r[2].as_str().unwrap().into()))
        .collect();
    let want = [
        ("9", "unknown"),
        ("6", "0"),
        ("3", "x"),
        ("2", "7.5"),
        ("1", "True"),
    ];
    assert_eq!(layers, want.map(|(n, c)| (n.to_string(), c.to_string())));
    let edges: Vec<&str> = d["sections"][0]["diagrams"][0]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| l.as_str().filter(|l| l.contains("-->")))
        .collect();
    assert_eq!(edges.len(), 4);
    assert!(edges.iter().all(|l| l.contains("-->|calls| ")), "{edges:?}");
}

const HOSTILE: [&str; 9] = [
    "</script><script>alert(1)</script>",
    "<img src=x onerror=alert(2)>",
    "he said \"hi\" [x] (y) {z} #1; %%c --> click",
    "`tick` | pipe \\ back",
    "click callback alert",
    "%%{init: {\"securityLevel\": \"loose\"}}%%",
    "line\nbreak\u{2028}sep",
    "& &amp; &lt;b&gt;",
    "]]> -.-> ==> ---",
];

fn hostile_graph() -> NxGraph {
    let nodes: Vec<Value> = HOSTILE
        .iter()
        .enumerate()
        .map(|(i, l)| {
            json!({"id": format!("n{i}\"><x"), "label": l, "file_type": "code",
                   "source_file": format!("src/{l}.py"), "community": i % 2})
        })
        .collect();
    let links: Vec<Value> = (1..HOSTILE.len())
        .map(|i| {
            json!({"source": format!("n{}\"><x", i - 1), "target": format!("n{i}\"><x"),
                   "relation": HOSTILE[i], "confidence": "EXTRACTED"})
        })
        .chain([
            json!({"source": "n0\"><x", "target": "n2\"><x", "relation": "calls"}),
            json!({"source": "n0\"><x", "target": "n4\"><x", "relation": ""}),
        ])
        .collect();
    NxGraph::from_json(json!({"directed": false, "multigraph": false, "graph": {},
                              "nodes": nodes, "links": links}))
    .unwrap()
}

#[test]
fn hostile_strings_stay_inert() {
    let g = hostile_graph();
    for lang in [Lang::En, Lang::ZhCn] {
        let opts = CallflowOptions {
            lang: LangChoice::Fixed(lang),
            ..options("<b>proj</b>")
        };
        let page = callflow::build(&g, &opts);
        let html = page.render();
        // Only the page's own elements: no label, id, relation or file name became markup.
        assert!(
            !html.contains("<script><"),
            "a label opened a script element"
        );
        assert!(!html.contains("<img"), "a label became an element");
        assert!(!html.contains("<b>"), "the project name became markup");
        assert_eq!(
            html.matches("<script").count(),
            2,
            "only Mermaid and the page script"
        );
        // Mermaid sources: no angle brackets or quotes from data, no directives, no click lines.
        let mut lines: Vec<&String> = page.overview_diagram.iter().collect();
        for s in &page.sections {
            lines.extend(s.diagram.iter());
        }
        for line in lines {
            let data = line
                .replace("<br/><small>", "")
                .replace("</small>", "")
                .replace(" -->|", "")
                .replace(" -.-> ", "");
            assert!(
                !data.contains('<') && !data.contains('>'),
                "angle bracket in {line}"
            );
            assert!(!line.contains("%%{"), "a directive in {line}");
            assert!(
                !line.trim_start().starts_with("click"),
                "a click line: {line}"
            );
            assert!(line.matches('"').count() <= 2, "a quote broke out: {line}");
            // Edge labels parse: never empty, and quoted unless plain words (a bracket or
            // parenthesis in an unquoted label is a Mermaid syntax error).
            if let Some((_, rest)) = line.split_once("-->|") {
                let label = rest.split('|').next().unwrap();
                assert!(!label.is_empty(), "empty edge label: {line}");
                let quoted = label.len() >= 2 && label.starts_with('"') && label.ends_with('"');
                assert!(
                    quoted
                        || label
                            .chars()
                            .all(|c| c.is_alphanumeric() || " _-.,".contains(c)),
                    "unquoted punctuation in edge label: {line}"
                );
            }
            assert!(
                !line.contains('\n') && !line.contains('\u{2028}'),
                "a line break in {line}"
            );
        }
    }
}

#[test]
fn output_is_deterministic() {
    for lang in ["python", "rust", "mixed"] {
        let a = callflow::build(&graph(lang), &options(lang)).render();
        let b = callflow::build(&graph(lang), &options(lang)).render();
        assert_eq!(a, b, "{lang}");
    }
}

#[test]
fn chinese_is_picked_for_cjk_labels() {
    let g = NxGraph::from_json(json!({"nodes": [
        {"id": "a", "label": "处理请求()", "source_file": "src/a.py", "community": 0},
        {"id": "b", "label": "helper()", "source_file": "src/a.py", "community": 0}],
        "links": [{"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED"}]}))
    .unwrap();
    let opts = CallflowOptions {
        lang: LangChoice::Auto,
        ..options("p")
    };
    let d = callflow::build(&g, &opts).data();
    assert_eq!(d["nav"][0][1], "架构总览");
    let note = d["sections"][1]["paragraphs"][1].as_str().unwrap();
    assert!(note.contains("关系概况：调用 x1"), "{note}");
}

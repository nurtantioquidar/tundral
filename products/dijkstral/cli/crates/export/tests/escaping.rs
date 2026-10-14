//! Labels, paths and relations are untrusted: none of them may end the data element or reach
//! the page as markup, in either view.

use dijkstral_export::html::{self, HtmlOptions, Mode};
use dijkstral_export::{wiki, Loaded, Names};
use serde_json::{json, Value};

const HOSTILE: [&str; 8] = [
    "</script><script>alert(1)</script>",
    "</SCRIPT ><img src=x onerror=alert(2)>",
    "<!-- <script>",
    "]]> & &amp; \"q\" 'q' `q`",
    "line\u{2028}sep\u{2029}end",
    "@@DATA@@ @@SCRIPTS@@ @@TITLE@@",
    "tab\tnew\nline\u{0}nul",
    "${alert(3)} {{x}} \\u003c",
];

fn graph() -> Value {
    let nodes: Vec<Value> = HOSTILE
        .iter()
        .enumerate()
        .map(|(i, label)| {
            json!({
                "id": format!("n{i}</script>"),
                "label": label,
                "file_type": "code",
                "source_file": format!("src/{label}.py"),
                "source_location": "L7",
                "community": i % 3,
            })
        })
        .collect();
    let links: Vec<Value> = (1..HOSTILE.len())
        .map(|i| {
            json!({
                "source": format!("n{}</script>", i - 1),
                "target": format!("n{i}</script>"),
                "relation": HOSTILE[i],
                "confidence": "INFERRED",
            })
        })
        .collect();
    json!({"directed": false, "multigraph": false, "graph": {}, "nodes": nodes, "links": links,
           "hyperedges": [{"label": "</script>"}]})
}

fn data_element(page: &str) -> &str {
    let open = "<script id=\"graph-data\" type=\"application/json\">";
    let start = page.find(open).unwrap() + open.len();
    let end = start + page[start..].find("</script>").unwrap();
    &page[start..end]
}

#[test]
fn hostile_strings_stay_data() {
    let loaded = Loaded::from_json(&graph()).unwrap();
    let mut names = std::collections::HashMap::new();
    names.insert(0, "</script><b>community</b>".to_string());
    for (limit, mode) in [(1000, Mode::Vis), (2, Mode::WebGl)] {
        let view = loaded.view(Names::Named(names.clone()));
        let opts = HtmlOptions {
            node_limit: limit,
            title: "</title><script>x()</script>".into(),
        };
        let page = html::render(&view, &opts);
        assert_eq!(html::mode(&view, &opts), mode);

        // The data element ends where the template ends it, and holds no markup characters.
        let data = data_element(&page);
        assert!(!data.contains(['<', '>', '&', '\u{2028}', '\u{2029}']));
        let parsed: Value = serde_json::from_str(data).unwrap();

        // Every string survives exactly (labels lose control characters, as in Graphify).
        let labels: Vec<String> = match mode {
            Mode::Vis => parsed["RAW_NODES"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["label"].as_str().unwrap().to_string())
                .collect(),
            Mode::WebGl => parsed["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n[1].as_str().unwrap().to_string())
                .collect(),
        };
        let want: Vec<String> = HOSTILE
            .iter()
            .map(|l| l.chars().filter(|&c| c >= ' ').collect())
            .collect();
        assert_eq!(labels, want);
        assert_eq!(parsed["hyperedges"][0]["label"], "</script>");
        assert_eq!(parsed["LEGEND"][0]["label"], "</script><b>community</b>");

        // Nothing else of the input appears outside the data element: the page minus that
        // element is the template with its three markers filled.
        let outside = page.replace(data, "");
        for needle in [
            "alert(",
            "onerror",
            "<b>",
            "@@DATA@@",
            "@@TITLE@@",
            "@@SCRIPTS@@",
        ] {
            assert!(
                !outside.contains(needle),
                "{needle} leaked outside the data element"
            );
        }
        assert!(outside.contains(
            "<title>&lt;/title&gt;&lt;script&gt;x()&lt;/script&gt; · dijkstral graph</title>"
        ));
    }
}

#[test]
fn hostile_wiki_names_stay_inside_the_directory() {
    let loaded = Loaded::from_json(&graph()).unwrap();
    let view = loaded.view(Names::Placeholder);
    for (name, _) in wiki::render(&view).unwrap() {
        assert!(
            !name.contains(['/', '\\', '\0']) && name != ".." && name.ends_with(".md"),
            "{name:?}"
        );
    }
}

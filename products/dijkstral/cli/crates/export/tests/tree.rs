//! `GRAPH_TREE.html` against Graphify's `graphify tree` (`docs/graphify-exports.md`, tree
//! section). Goldens in `tests/fixtures/exports/<lang>/tree/` hold only the hierarchy Graphify's
//! page embeds (`const initialJsonData = …`), for the default options (`tree.json`) and
//! `--max-children 3` (`tree_max3.json`); dijkstral's page must embed the same text byte for byte.
//! Hostile labels, paths and names must stay data, and the page must not depend on run or thread.

use std::fs;
use std::path::{Path, PathBuf};

use dijkstral_export::nx::NxGraph;
use dijkstral_export::tree::{self, TreeOptions};
use serde_json::{json, Value};

const LANGS: [&str; 7] = [
    "python",
    "typescript",
    "javascript",
    "rust",
    "go",
    "java",
    "mixed",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn graph(lang: &str) -> NxGraph {
    NxGraph::read(&fixtures().join(format!("{lang}.clustered.graphify.json"))).unwrap()
}

fn page(g: &NxGraph, opts: &TreeOptions) -> String {
    tree::render(g, opts).unwrap().0
}

#[test]
fn hierarchy_matches_graphify_byte_for_byte() {
    for lang in LANGS {
        let g = graph(lang);
        for (file, max_children) in [("tree.json", 200), ("tree_max3.json", 3)] {
            let opts = TreeOptions {
                max_children,
                ..TreeOptions::default()
            };
            let want = fs::read_to_string(
                fixtures()
                    .join("exports")
                    .join(lang)
                    .join("tree")
                    .join(file),
            )
            .unwrap();
            let page = page(&g, &opts);
            let ours = tree::embedded_tree(&page).expect("tree data element");
            assert_eq!(ours, want, "{lang}: {file}");
        }
    }
}

#[test]
fn reads_graphify_pages_too() {
    let page = "<script>\n    const initialJsonData = {\"name\":\"/\"};\n</script>";
    assert_eq!(tree::embedded_tree(page), Some("{\"name\":\"/\"}"));
}

fn data(page: &str) -> Value {
    serde_json::from_str(tree::embedded_tree(page).unwrap()).unwrap()
}

/// The JSON of the inspector element.
fn inspector(page: &str) -> Value {
    let open = "<script id=\"tree-inspector\" type=\"application/json\">";
    let start = page.find(open).unwrap() + open.len();
    let end = start + page[start..].find("</script>").unwrap();
    serde_json::from_str(&page[start..end]).unwrap()
}

#[test]
fn leaves_line_up_with_inspector_records() {
    let g = graph("python");
    let page = page(&g, &TreeOptions::default());
    let (tree, info) = (data(&page), inspector(&page));
    let mut leaves = Vec::new();
    let mut entries = 0;
    fn walk<'a>(n: &'a Value, leaves: &mut Vec<&'a str>, entries: &mut usize) {
        *entries += 1;
        let kids = n["children"].as_array().unwrap();
        if kids.is_empty() {
            leaves.push(n["name"].as_str().unwrap());
        }
        for c in kids {
            walk(c, leaves, entries);
        }
    }
    walk(&tree, &mut leaves, &mut entries);
    let records = info["leaves"].as_array().unwrap();
    assert_eq!(records.len(), leaves.len());
    assert_eq!(info["kinds"].as_str().unwrap().len(), entries);
    let targets = info["targets"].as_array().unwrap();
    for (name, rec) in leaves.iter().zip(records) {
        let id = rec[0].as_str().unwrap();
        let node = g.index[id];
        assert_eq!(g.label(node), *name);
        for out in rec[7].as_array().unwrap() {
            assert!(out[1].as_u64().unwrap() < targets.len() as u64);
        }
        assert!(rec[7].as_array().unwrap().len() <= 12);
    }
    // `User` (degree 17) lists its outbound links, best-connected targets first.
    let user = records
        .iter()
        .find(|r| r[0] == "pkg_models_user")
        .expect("User record");
    let degrees: Vec<usize> = user[7]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let t = &targets[o[1].as_u64().unwrap() as usize];
            let label = t[0].as_str().unwrap();
            let i = (0..g.nodes.len() as u32)
                .find(|&i| g.label(i) == label && g.node_str(i, "source_file") == t[1].as_str())
                .unwrap();
            g.degree(i)
        })
        .collect();
    assert!(!degrees.is_empty());
    assert!(degrees.windows(2).all(|w| w[0] >= w[1]), "{degrees:?}");
}

const HOSTILE: [&str; 7] = [
    "</script><script>alert(1)</script>",
    "</SCRIPT ><img src=x onerror=alert(2)>",
    "<!-- <script>",
    "line\u{2028}sep\u{2029}end",
    "@@TREE@@ @@INSPECTOR@@ @@TITLE@@ @@SCRIPTS@@",
    "tab\tnew\nline\u{0}nul \"q\" 'q' `q` & &amp;",
    "${alert(3)} {{x}} \\u003c",
];

fn hostile_graph() -> NxGraph {
    let mut nodes = Vec::new();
    for (i, label) in HOSTILE.iter().enumerate() {
        nodes.push(json!({
            "id": format!("n{i}</script>"),
            "label": label,
            "file_type": "code",
            "source_file": format!("{label}/f{i}.py"),
            "source_location": label,
        }));
    }
    let links: Vec<Value> = (1..HOSTILE.len())
        .map(|i| {
            json!({
                "source": format!("n{}</script>", i - 1),
                "target": format!("n{i}</script>"),
                "relation": HOSTILE[i],
            })
        })
        .collect();
    NxGraph::from_json(json!({"directed": false, "multigraph": false, "graph": {},
        "nodes": nodes, "links": links}))
    .unwrap()
}

#[test]
fn hostile_strings_stay_data() {
    let g = hostile_graph();
    for label in [None, Some("</script><b>root</b> & \"q\"".to_string())] {
        let opts = TreeOptions {
            label,
            ..TreeOptions::default()
        };
        let page = page(&g, &opts);
        // Only the page's own elements: two data elements, D3 and the page script.
        assert_eq!(page.matches("</script>").count(), 4, "script elements");
        assert_eq!(page.to_lowercase().matches("<script").count(), 4);
        assert!(!page.contains("<img"));
        assert!(!page.contains("<!--"));
        assert!(!page.contains("<b>"));
        assert!(!page.contains('\u{2028}') && !page.contains('\u{2029}'));
        // Placeholders in labels are not expanded.
        assert!(page.contains("@@TREE@@ @@INSPECTOR@@"));
        // The data still parses and keeps every label as a symbol name.
        let tree = data(&page);
        let mut names = Vec::new();
        fn collect(n: &Value, out: &mut Vec<String>) {
            out.push(n["name"].as_str().unwrap().to_string());
            for c in n["children"].as_array().unwrap() {
                collect(c, out);
            }
        }
        collect(&tree, &mut names);
        for h in HOSTILE {
            assert!(names.iter().any(|n| n == h), "{h:?} kept");
        }
        let info = inspector(&page);
        assert_eq!(info["leaves"].as_array().unwrap().len(), HOSTILE.len());
        // The title reaches <title> HTML-escaped.
        let title = &page[page.find("<title>").unwrap()..page.find("</title>").unwrap()];
        assert!(!title[7..].contains('<'));
    }
}

#[test]
fn root_errors_like_graphify() {
    let g = graph("python");
    let opts = TreeOptions {
        root: Some("nowhere".into()),
        ..TreeOptions::default()
    };
    let err = tree::render(&g, &opts).unwrap_err().to_string();
    assert_eq!(
        err,
        "--root nowhere matched 0 of 6 source files (source_file paths are stored \
         repo-relative, e.g. 'main.py')"
    );
    let opts = TreeOptions {
        root: Some("pkg".into()),
        ..TreeOptions::default()
    };
    let tree = data(&page(&g, &opts));
    assert_eq!(tree["name"], "pkg");
    // Files outside the root are listed by name at the top.
    let names: Vec<&str> = tree["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "__init__.py",
            "main.py",
            "models.py",
            "service.py",
            "test_service.py",
            "util.py"
        ]
    );
}

#[test]
fn empty_graph() {
    let g = NxGraph::from_json(json!({"nodes": [{"id": "x"}], "links": []})).unwrap();
    let opts = TreeOptions {
        label: Some("ignored".into()),
        ..TreeOptions::default()
    };
    let p = page(&g, &opts);
    assert_eq!(
        tree::embedded_tree(&p).unwrap(),
        "{\"name\":\"(empty graph)\",\"total_count\":0,\"children\":[]}"
    );
}

#[test]
fn page_is_deterministic() {
    let g = graph("mixed");
    let a = page(&g, &TreeOptions::default());
    let b = std::thread::scope(|s| {
        let pages: Vec<String> = (0..4)
            .map(|_| s.spawn(|| page(&graph("mixed"), &TreeOptions::default())))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        pages
    });
    for p in b {
        assert_eq!(p, a);
    }
}

//! The Obsidian vault against Graphify's (`docs/graphify-exports.md`, Obsidian section):
//! `graphify export obsidian --graph <lang>.clustered.graphify.json` for each fixture, stored
//! under `tests/fixtures/exports/<lang>/obsidian/`, must match file for file, byte for byte
//! (notes, `graph.canvas`, `.obsidian/graph.json` and the manifest). Hostile labels must stay
//! inside the vault, inside their wikilinks and out of the frontmatter's structure.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use dijkstral_export::nx::NxGraph;
use dijkstral_export::obsidian::{self, CANVAS_FILE, MANIFEST_FILE};
use dijkstral_export::Names;
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
    let path = fixtures().join(format!("graphs-0.9.86/{lang}.clustered.graphify.json"));
    NxGraph::from_json(serde_json::from_slice(&fs::read(path).unwrap()).unwrap()).unwrap()
}

/// Every file under `dir`, by relative path.
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn vault_matches_graphify_byte_for_byte() {
    for lang in LANGS {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("vault");
        let done = obsidian::write(&graph(lang), &Names::Placeholder, &dir).unwrap();
        assert!(done.skipped.is_empty() && done.pruned == 0, "{lang}");
        let ours = tree(&dir);
        let want = tree(&fixtures().join("exports").join(lang).join("obsidian"));
        assert_eq!(
            ours.keys().collect::<Vec<_>>(),
            want.keys().collect::<Vec<_>>(),
            "{lang}: file names"
        );
        for (name, bytes) in &want {
            assert!(
                &ours[name] == bytes,
                "{lang}: {name} differs:\n--- ours\n{}\n--- graphify\n{}",
                String::from_utf8_lossy(&ours[name]),
                String::from_utf8_lossy(bytes)
            );
        }
    }
}

/// Global graphs (links with `_src`/`_tgt`, `source`/`target` often reversed): the vault is
/// Graphify's byte for byte; `graph.canvas` draws each link from `_src` to `_tgt` (200 links of
/// `global_py_go.3`, 99 of them stored reversed).
#[test]
fn global_vaults_match_graphify_byte_for_byte() {
    for (golden, input) in [
        ("global_direction", "global_direction"),
        ("global_py_go", "global_py_go.3"),
    ] {
        let path = fixtures().join(format!("multirepo/goldens/{input}.graph.json"));
        let g = NxGraph::read(&path).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("vault");
        obsidian::write(&g, &Names::Placeholder, &dir).unwrap();
        let ours = tree(&dir);
        let want = tree(&fixtures().join("exports").join(golden).join("obsidian"));
        assert_eq!(
            ours.keys().collect::<Vec<_>>(),
            want.keys().collect::<Vec<_>>(),
            "{golden}: file names"
        );
        for (name, bytes) in &want {
            assert!(&ours[name] == bytes, "{golden}: {name} differs");
        }
    }
}

#[test]
fn rendering_is_deterministic() {
    for lang in LANGS {
        let a = obsidian::render(&graph(lang), &Names::Placeholder);
        let b = obsidian::render(&graph(lang), &Names::Placeholder);
        assert_eq!(a.files, b.files, "{lang}");
        assert_eq!(a.canvas, b.canvas, "{lang}");
    }
}

/// Graphify's `.obsidian/graph.json` and community note names for the python fixture with
/// `--labels {"3": "Three", "99": "Ghost", "1": "One", "12": "Twelve"}`: one colour group per
/// labels-file id in id order (present in the graph or not), coloured by id.
#[test]
fn labels_give_colour_groups() {
    let names = Names::Named(HashMap::from([
        (3, "Three".to_string()),
        (99, "Ghost".to_string()),
        (1, "One".to_string()),
        (12, "Twelve".to_string()),
    ]));
    let vault = obsidian::render(&graph("python"), &names);
    let settings = &vault
        .files
        .iter()
        .find(|(f, _)| f == ".obsidian/graph.json")
        .unwrap()
        .1;
    let doc: Value = serde_json::from_str(settings).unwrap();
    let groups: Vec<(String, i64)> = doc["colorGroups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            (
                g["query"].as_str().unwrap().to_string(),
                g["color"]["rgb"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        groups,
        [
            ("tag:#community/One".to_string(), 0xF28E2B),
            ("tag:#community/Three".to_string(), 0x76B7B2),
            ("tag:#community/Twelve".to_string(), 0xE15759),
            ("tag:#community/Ghost".to_string(), 0xBAB0AC),
        ]
    );
    let notes: Vec<&str> = vault
        .files
        .iter()
        .map(|(f, _)| f.as_str())
        .filter(|f| f.starts_with("_COMMUNITY_"))
        .collect();
    assert!(notes.contains(&"_COMMUNITY_One.md") && notes.contains(&"_COMMUNITY_Three.md"));
    assert!(notes.contains(&"_COMMUNITY_Community 0.md"));
}

const HOSTILE: [&str; 12] = [
    "../../outside",
    "/etc/passwd",
    "..\\..\\win",
    "[[evil]] | #tag ^b",
    "x\n---\nkey: value\n# heading",
    "a: b\n- [[injected]]",
    "\"quoted\" 'single' \\ back",
    "`tick` ``double``",
    ".hidden",
    "",
    "\u{0}\u{1}\u{7f}\u{2028}\u{2029}",
    "_COMMUNITY_dup",
];

fn hostile_graph() -> Value {
    let nodes: Vec<Value> = HOSTILE
        .iter()
        .enumerate()
        .map(|(i, label)| {
            json!({
                "id": format!("n{i}\"]}}"),
                "label": label,
                "file_type": label,
                "source_file": label,
                "source_location": label,
                "community": i % 3,
            })
        })
        .collect();
    let links: Vec<Value> = (1..HOSTILE.len())
        .map(|i| {
            json!({
                "source": format!("n{}\"]}}", i - 1),
                "target": format!("n{i}\"]}}"),
                "relation": HOSTILE[i],
                "confidence": HOSTILE[i - 1],
                "weight": i,
            })
        })
        .collect();
    json!({"directed": false, "multigraph": false, "graph": {}, "nodes": nodes, "links": links})
}

fn hostile_names() -> Names {
    Names::Named(HashMap::from([
        (0, "dup".to_string()),
        (1, "../../x\n# y".to_string()),
        (2, "a: [b] #c".to_string()),
    ]))
}

/// The frontmatter of a note: its lines between the two `---`.
fn frontmatter(text: &str) -> Vec<&str> {
    let mut lines = text.split('\n');
    assert_eq!(lines.next(), Some("---"));
    lines.take_while(|l| *l != "---").collect()
}

#[test]
fn hostile_labels_stay_data() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    let g = NxGraph::from_json(hostile_graph()).unwrap();
    obsidian::write(&g, &hostile_names(), &dir).unwrap();

    // Nothing outside the vault, every note directly inside it.
    let outside: Vec<_> = fs::read_dir(tmp.path()).unwrap().collect();
    assert_eq!(outside.len(), 1, "only the vault directory");
    let files = tree(&dir);
    for name in files.keys() {
        assert!(
            name == ".obsidian/graph.json" || !name.contains('/'),
            "{name:?} is nested"
        );
        assert!(
            !name.contains(['\\', '\n', '\0', '[', ']', '|', '#', '^']),
            "{name:?}"
        );
    }
    assert_eq!(files.keys().filter(|f| f.ends_with(".md")).count(), 12 + 3);

    let canvas: Value = serde_json::from_slice(&files[CANVAS_FILE]).expect("canvas is JSON");
    assert_eq!(canvas["nodes"].as_array().unwrap().len(), 12 + 3);
    let _: Value = serde_json::from_slice(&files[MANIFEST_FILE]).expect("manifest is JSON");
    let _: Value =
        serde_json::from_slice(&files[".obsidian/graph.json"]).expect("settings are JSON");

    for (name, bytes) in &files {
        if !name.ends_with(".md") {
            continue;
        }
        let text = std::str::from_utf8(bytes).unwrap();
        for line in frontmatter(text) {
            // `key: "<escaped>"`, `key: plain`, `tags:` or `  - <tag>`: one YAML item per line.
            let ok = if let Some(tag) = line.strip_prefix("  - ") {
                !tag.is_empty()
                    && tag
                        .chars()
                        .all(|c| c.is_alphanumeric() || "_/-".contains(c))
            } else if let Some((key, value)) = line.split_once(": ") {
                key.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                    && (value.chars().all(|c| c.is_ascii_alphanumeric())
                        || (value.starts_with('"')
                            && value.ends_with('"')
                            && value.len() >= 2
                            && !value[1..value.len() - 1]
                                .replace("\\\\", "")
                                .replace("\\\"", "")
                                .contains('"')))
            } else {
                line == "tags:"
            };
            assert!(ok, "{name}: frontmatter line {line:?}");
            assert!(!line.chars().any(|c| (c as u32) < 0x20), "{name}: {line:?}");
        }
        // The exporter's wikilinks (list items and community links): the target is a note
        // name, so it cannot hold `]`, `[`, `|` or a newline, and it names a note. (Wikilink
        // syntax inside a label or file type is shown as Graphify shows it.)
        for line in text.split('\n') {
            let link = line
                .strip_prefix("- [[")
                .or_else(|| line.split_once(" to [[").map(|(_, r)| r));
            let Some(rest) = link else { continue };
            let target = &rest[..rest.find("]]").expect("closed wikilink")];
            assert!(
                !target.contains(['[', ']', '|', '\n', '#', '^']),
                "{name}: {target:?}"
            );
            assert!(
                files.contains_key(&format!("{target}.md")),
                "{name}: [[{target}]] names no note"
            );
        }
        // Headings and list items stay on their line: no line starts a new section or
        // frontmatter except the exporter's own.
        for line in text.split('\n') {
            assert!(
                !line.starts_with("key:") && !line.starts_with("- [[injected"),
                "{name}: {line:?}"
            );
        }
    }
}

#[test]
fn reruns_keep_user_files_and_prune_stale_notes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("vault");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("User.md"), "mine").unwrap();
    let g = graph("python");
    let first = obsidian::write(&g, &Names::Placeholder, &dir).unwrap();
    assert_eq!(first.skipped, ["User.md"]);
    assert_eq!(fs::read_to_string(dir.join("User.md")).unwrap(), "mine");
    assert_eq!(first.notes, 59 + 9 - 1);

    // A doctored manifest naming files outside the vault, and a node gone from the graph.
    let outside = tmp.path().join("outside.md");
    fs::write(&outside, "keep").unwrap();
    let mut manifest: Value =
        serde_json::from_str(&fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    for evil in ["../outside.md", "/tmp/x.md", "sub/../../outside.md"] {
        manifest["files"].as_array_mut().unwrap().push(json!(evil));
    }
    fs::write(dir.join(MANIFEST_FILE), manifest.to_string()).unwrap();
    let mut doc: Value = serde_json::from_slice(
        &fs::read(fixtures().join("graphs-0.9.86/python.clustered.graphify.json")).unwrap(),
    )
    .unwrap();
    doc["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|n| n["id"] != "pkg_models_admin");
    doc["links"]
        .as_array_mut()
        .unwrap()
        .retain(|l| l["source"] != "pkg_models_admin" && l["target"] != "pkg_models_admin");
    let second =
        obsidian::write(&NxGraph::from_json(doc).unwrap(), &Names::Placeholder, &dir).unwrap();
    assert_eq!(second.pruned, 1);
    assert!(!dir.join("Admin.md").exists());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
    assert_eq!(fs::read_to_string(dir.join("User.md")).unwrap(), "mine");
}

/// A raw extraction fragment (`edges`, no flags) loads as a multigraph: a note lists each
/// neighbour once, with its first parallel link, as Graphify's does.
#[test]
fn multigraph_lists_each_neighbour_once() {
    let g = NxGraph::from_json(json!({
        "nodes": [{"id": "a", "label": "A", "community": 0}, {"id": "b", "label": "B", "community": 1}],
        "edges": [
            {"source": "a", "target": "b", "relation": "r1", "confidence": "INFERRED"},
            {"source": "b", "target": "a", "relation": "r2", "confidence": "AMBIGUOUS"},
            {"source": "a", "target": "b", "relation": "r3", "confidence": "AMBIGUOUS"},
        ],
    }))
    .unwrap();
    let vault = obsidian::render(&g, &Names::Placeholder);
    let note = |name: &str| &vault.files.iter().find(|(f, _)| f == name).unwrap().1;
    assert!(note("B.md").contains("## Connections\n- [[A]] - `r1` [INFERRED]\n\n"));
    assert!(note("B.md").contains("  - graphify/AMBIGUOUS\n"));
    assert!(note("_COMMUNITY_Community 0.md").contains("- 3 edges to [[_COMMUNITY_Community 1]]"));
    assert!(
        note("_COMMUNITY_Community 0.md").ends_with("- [[A]] - degree 3, connects to 1 community")
    );
}

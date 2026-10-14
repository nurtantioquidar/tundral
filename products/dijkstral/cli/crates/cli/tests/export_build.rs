//! `extract` writes `graph.html` by default (`--no-viz` skips it, `--wiki` adds the wiki), an
//! unchanged build is skipped only while those outputs exist, and `dijkstral export` reproduces
//! the build's page from `graph.json` (`docs/graphify-exports.md` §6).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
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

fn dijkstral(args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .args(args)
        .env("SOURCE_DATE_EPOCH", "1760400000")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "dijkstral {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn extract_writes_graph_html_and_skips_only_when_it_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("python");
    copy_dir(&fixture("python"), &root);
    let r = root.to_str().unwrap();
    let out = root.join("dijkstral-out");

    dijkstral(&["extract", r]);
    let html = out.join("graph.html");
    let page = fs::read_to_string(&html).unwrap();
    assert!(page.contains("vis-network@9.1.9") && page.contains("\"RAW_NODES\""));
    assert!(!out.join("wiki").exists());

    // Unchanged: skipped while graph.html exists, rebuilt once it is gone.
    assert!(stdout(&dijkstral(&["extract", r])).contains("unchanged"));
    fs::remove_file(&html).unwrap();
    assert!(!stdout(&dijkstral(&["extract", r])).contains("unchanged"));
    assert_eq!(fs::read_to_string(&html).unwrap(), page);

    // --wiki is an output too: asking for it forces a build that writes it.
    assert!(!stdout(&dijkstral(&["update", r, "--wiki"])).contains("unchanged"));
    assert!(out.join("wiki/index.md").is_file());
    assert!(stdout(&dijkstral(&["update", r, "--wiki"])).contains("unchanged"));

    // The export command reproduces the build's page and wiki from graph.json alone.
    let graph = out.join("graph.json");
    let wiki_index = fs::read_to_string(out.join("wiki/index.md")).unwrap();
    fs::remove_dir_all(out.join("wiki")).unwrap();
    dijkstral(&["export", "html", "--graph", graph.to_str().unwrap()]);
    dijkstral(&["export", "wiki", "--graph", graph.to_str().unwrap()]);
    assert_eq!(fs::read_to_string(&html).unwrap(), page);
    assert_eq!(
        fs::read_to_string(out.join("wiki/index.md")).unwrap(),
        wiki_index
    );
}

#[test]
fn no_viz_skips_graph_html() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("go");
    copy_dir(&fixture("go"), &root);
    dijkstral(&["extract", root.to_str().unwrap(), "--no-viz"]);
    let out = root.join("dijkstral-out");
    assert!(out.join("graph.json").is_file());
    assert!(!out.join("graph.html").exists());
}

#[test]
fn export_works_on_graphify_graphs() {
    let tmp = tempfile::tempdir().unwrap();
    let graph = tmp.path().join("graph.json");
    fs::copy(fixture("java.clustered.graphify.json"), &graph).unwrap();
    let g = graph.to_str().unwrap();
    dijkstral(&["export", "graphml", "--graph", g]);
    assert_eq!(
        fs::read_to_string(tmp.path().join("graph.graphml")).unwrap(),
        fs::read_to_string(fixture("exports/java/graph.graphml")).unwrap()
    );
    let msg = stdout(&dijkstral(&[
        "export",
        "html",
        "--graph",
        g,
        "--node-limit",
        "10",
    ]));
    assert!(msg.contains("WebGL"), "{msg}");
    let page = fs::read_to_string(tmp.path().join("graph.html")).unwrap();
    assert!(page.contains("sigma@3.0.2") && page.contains("\"mode\":\"webgl\""));
    dijkstral(&["export", "wiki", "--graph", g, "--no-label"]);
    assert!(fs::read_to_string(tmp.path().join("wiki/index.md"))
        .unwrap()
        .contains("- [Community 0](Community_0.md)"));
}

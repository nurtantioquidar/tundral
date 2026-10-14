//! The export-only formats from the command line (`docs/graphify-exports.md` §9–§14): each
//! writes the same bytes whatever `--threads` built the graph and however often it runs, writes
//! next to the graph by default, and `--push` fails cleanly without a server or credentials.

use std::collections::BTreeMap;
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

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .args(args)
        .env("SOURCE_DATE_EPOCH", "1760400000")
        .env_remove("NEO4J_PASSWORD")
        .env_remove("FALKORDB_PASSWORD")
        .output()
        .unwrap()
}

fn ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "dijkstral {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every file under `dir`, by relative path.
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    out
}

/// Build `lang` with `threads` (into `<tmp>/t<threads>/<lang>`, so the project name is the
/// same); returns its `dijkstral-out/` and the files the build wrote there.
fn build(tmp: &Path, lang: &str, threads: &str) -> (PathBuf, Vec<String>) {
    let root = tmp.join(format!("t{threads}")).join(lang);
    copy_dir(&fixture(lang), &root);
    ok(&[
        "extract",
        root.to_str().unwrap(),
        "--threads",
        threads,
        "--no-viz",
    ]);
    let out = root.join("dijkstral-out");
    let built = files(&out).into_keys().collect();
    (out, built)
}

/// Run every export-only format on `out/graph.json`; returns the files they wrote.
fn export_all((out, built): &(PathBuf, Vec<String>)) -> BTreeMap<String, Vec<u8>> {
    let graph = out.join("graph.json");
    let g = graph.to_str().unwrap();
    ok(&["export", "cypher", "--graph", g]);
    ok(&["export", "neo4j", "--graph", g]);
    ok(&["export", "falkordb", "--graph", g]);
    ok(&["export", "obsidian", "--graph", g]);
    ok(&["export", "svg", "--graph", g]);
    ok(&["export", "callflow-html", "--graph", g]);
    ok(&["tree", "--graph", g]);
    let mut written = files(out);
    written.retain(|k, _| !built.contains(k));
    written
}

#[test]
fn exports_are_deterministic_across_threads_and_runs() {
    let tmp = tempfile::tempdir().unwrap();
    for lang in ["python", "typescript"] {
        let out1 = build(tmp.path(), lang, "1");
        let one = export_all(&out1);
        let many = export_all(&build(tmp.path(), lang, "8"));
        let names: Vec<&String> = one.keys().collect();
        for want in [
            "cypher.txt",
            "graph.svg",
            "GRAPH_TREE.html",
            "obsidian/graph.canvas",
        ] {
            assert!(
                one.contains_key(want),
                "{lang}: {want} missing from {names:?}"
            );
        }
        assert!(
            one.keys().any(|k| k.ends_with("-callflow.html")),
            "{lang}: no call-flow page in {names:?}"
        );
        assert_eq!(
            one.keys().collect::<Vec<_>>(),
            many.keys().collect::<Vec<_>>()
        );
        for (name, bytes) in &one {
            assert!(
                &many[name] == bytes,
                "{lang}: {name} differs across --threads"
            );
        }
        // Running again over the previous outputs writes the same bytes.
        let again = export_all(&out1);
        assert_eq!(again.len(), one.len());
        for (name, bytes) in &one {
            assert!(&again[name] == bytes, "{lang}: {name} differs between runs");
        }
    }
}

#[test]
fn push_fails_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let graph = tmp.path().join("graph.json");
    fs::copy(fixture("python.clustered.graphify.json"), &graph).unwrap();
    let g = graph.to_str().unwrap();

    let out = run(&[
        "export",
        "neo4j",
        "--graph",
        g,
        "--push",
        "bolt://127.0.0.1:9",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--password (or NEO4J_PASSWORD)"));

    let out = run(&[
        "export",
        "neo4j",
        "--graph",
        g,
        "--push",
        "neo4j+s://127.0.0.1:9",
        "--password",
        "x",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not available in this build"));

    // Nothing listens on port 9 of the loopback: a connection error, not a hang or a panic.
    let out = run(&["export", "falkordb", "--graph", g, "--push", "127.0.0.1:9"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("connecting to 127.0.0.1:9"));
    assert!(!tmp.path().join("cypher.txt").exists());
}

#[test]
fn cypher_file_messages_match_graphify() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("sub");
    fs::create_dir_all(&dir).unwrap();
    fs::copy(
        fixture("go.clustered.graphify.json"),
        dir.join("graph.json"),
    )
    .unwrap();
    let g = dir.join("graph.json");
    let g = g.to_str().unwrap();
    let shown = format!("{}/cypher.txt", dir.display());
    assert_eq!(
        ok(&["export", "neo4j", "--graph", g]).trim_end(),
        format!("cypher.txt written - import with: cypher-shell < {shown}")
    );
    assert!(
        ok(&["export", "falkordb", "--graph", g]).starts_with(&format!(
            "cypher.txt written ({shown}) - statements are OpenCypher."
        ))
    );
    let want = fs::read_to_string(fixture("exports/go/cypher.txt")).unwrap();
    let ours = fs::read_to_string(dir.join("cypher.txt")).unwrap();
    assert_eq!(
        ours.split_once('\n').unwrap().1,
        want.split_once('\n').unwrap().1
    );
}

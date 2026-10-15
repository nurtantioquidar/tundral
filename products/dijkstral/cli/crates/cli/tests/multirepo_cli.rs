//! Cross-repo commands end to end, offline (`docs/graphify-multirepo.md`): graphs built by
//! dijkstral merged and queried, `extract --global` and the `global` commands under a temporary
//! `DIJKSTRAL_HOME`, and `clone` against a local repository through `DIJKSTRAL_CLONE_BASE`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn fixtures() -> PathBuf {
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

/// Git that ignores the user's and the system's configuration (no signing, no URL rewrites).
fn isolate_git(cmd: &mut Command, home: &Path) {
    cmd.env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
}

fn dijkstral(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dijkstral"));
    cmd.args(args).current_dir(cwd).env("DIJKSTRAL_HOME", home);
    isolate_git(&mut cmd, home);
    cmd.output().unwrap()
}

fn ok(home: &Path, cwd: &Path, args: &[&str]) -> String {
    let out = dijkstral(home, cwd, args);
    assert!(
        out.status.success(),
        "dijkstral {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn read_json(p: &Path) -> Value {
    serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
}

/// Every link names two nodes, ids are unique, and the given tags are the repositories.
fn check_graph(g: &Value, tags: &[&str]) {
    let ids: Vec<&str> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    let set: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(set.len(), ids.len(), "duplicate ids");
    for l in g["links"].as_array().unwrap() {
        assert!(set.contains(l["source"].as_str().unwrap()), "{l}");
        assert!(set.contains(l["target"].as_str().unwrap()), "{l}");
    }
    let repos: HashSet<&str> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["repo"].as_str())
        .collect();
    assert_eq!(repos, tags.iter().copied().collect());
}

#[test]
fn merged_graphs_of_dijkstral_builds_answer_queries() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let home = ws.join("home");
    copy_dir(&fixtures().join("python"), &ws.join("pyrepo"));
    copy_dir(&fixtures().join("go"), &ws.join("gorepo"));
    for repo in ["pyrepo", "gorepo"] {
        ok(&home, ws, &["extract", repo, "--code-only"]);
    }
    let out = ok(
        &home,
        ws,
        &[
            "merge-graphs",
            "pyrepo/dijkstral-out/graph.json",
            "gorepo/dijkstral-out/graph.json",
        ],
    );
    assert!(out.contains("2 graphs merged"), "{out}");
    let merged = ws.join("dijkstral-out/merged-graph.json");
    let g = read_json(&merged);
    check_graph(&g, &["pyrepo", "gorepo"]);
    // Graphify's encoding: compact, ASCII, no trailing newline.
    let text = fs::read_to_string(&merged).unwrap();
    assert!(text.starts_with(r#"{"directed":false,"multigraph":false,"graph":"#));
    assert!(!text.ends_with('\n'));

    let m = merged.to_str().unwrap();
    // The same name in two repositories is ambiguous; the namespaced id is exact.
    let ambiguous = dijkstral(&home, ws, &["explain", "UserService", "--graph", m]);
    assert_eq!(ambiguous.status.code(), Some(3));
    let id = "pyrepo::pkg_service_userservice";
    let explain = ok(&home, ws, &["explain", id, "--graph", m]);
    assert!(explain.contains(&format!("ID:        {id}")), "{explain}");
    let query = ok(&home, ws, &["query", "register user", "--graph", m]);
    assert!(query.contains("NODE "), "{query}");
    let path = ok(&home, ws, &["path", "main.py", id, "--graph", m]);
    assert!(path.contains("Shortest path"), "{path}");

    // Re-merging with the first merge as --previous keeps every community.
    let again = ws.join("again.json");
    ok(
        &home,
        ws,
        &[
            "merge-graphs",
            "pyrepo/dijkstral-out/graph.json",
            "gorepo/dijkstral-out/graph.json",
            "--out",
            again.to_str().unwrap(),
            "--previous",
            m,
        ],
    );
    assert_eq!(fs::read(&again).unwrap(), fs::read(&merged).unwrap());

    // Fewer than two graphs is a usage error; a missing one fails cleanly.
    let one = dijkstral(
        &home,
        ws,
        &["merge-graphs", "pyrepo/dijkstral-out/graph.json"],
    );
    assert_eq!(one.status.code(), Some(2));
    let missing = dijkstral(
        &home,
        ws,
        &[
            "merge-graphs",
            "pyrepo/dijkstral-out/graph.json",
            "nope.json",
        ],
    );
    assert_eq!(missing.status.code(), Some(1));
}

#[test]
fn extract_global_and_the_global_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let home = ws.join("home");
    copy_dir(&fixtures().join("python"), &ws.join("pyrepo"));
    copy_dir(&fixtures().join("go"), &ws.join("gorepo"));
    let global = home.join("global-graph.json");

    let path = ok(&home, ws, &["global", "path"]);
    assert_eq!(path.trim_end(), global.to_str().unwrap());
    assert!(ok(&home, ws, &["global", "list"]).contains("no projects"));
    assert!(!global.exists());

    ok(&home, ws, &["extract", "pyrepo", "--code-only", "--global"]);
    ok(
        &home,
        ws,
        &[
            "extract",
            "gorepo",
            "--code-only",
            "--global",
            "--as",
            "golang",
        ],
    );
    let list = ok(&home, ws, &["global", "list"]);
    assert!(
        list.contains("  pyrepo  ") && list.contains("  golang  "),
        "{list}"
    );
    let g = read_json(&global);
    check_graph(&g, &["pyrepo", "golang"]);
    // The global graph is what `global add` of each build gives.
    let solo = ws.join("solo");
    for (repo, tag) in [("pyrepo", "pyrepo"), ("gorepo", "golang")] {
        let graph = ws.join(repo).join("dijkstral-out/graph.json");
        ok(
            &solo,
            ws,
            &["global", "add", graph.to_str().unwrap(), "--as", tag],
        );
    }
    assert_eq!(
        fs::read(solo.join("global-graph.json")).unwrap(),
        fs::read(&global).unwrap()
    );

    // An unchanged rebuild leaves the global graph alone; update --global works too.
    let before = fs::read(&global).unwrap();
    let again = ok(&home, ws, &["update", "pyrepo", "--code-only", "--global"]);
    assert!(again.contains("has not changed"), "{again}");
    assert_eq!(fs::read(&global).unwrap(), before);

    // The global graph answers queries.
    let gp = global.to_str().unwrap();
    let explain = ok(
        &home,
        ws,
        &["explain", "golang::service_userservice", "--graph", gp],
    );
    assert!(explain.contains("Connections"), "{explain}");

    ok(&home, ws, &["global", "remove", "pyrepo"]);
    let g = read_json(&global);
    check_graph(&g, &["golang"]);
    let gone = dijkstral(&home, ws, &["global", "remove", "pyrepo"]);
    assert_eq!(gone.status.code(), Some(1));
    let missing = dijkstral(&home, ws, &["global", "add", "nope/graph.json"]);
    assert_eq!(missing.status.code(), Some(1));
    // --as only makes sense with --global.
    let stray = dijkstral(
        &home,
        ws,
        &["extract", "pyrepo", "--code-only", "--as", "x"],
    );
    assert_eq!(stray.status.code(), Some(2));
}

fn git(home: &Path, dir: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir);
    isolate_git(&mut cmd, home);
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

#[test]
fn clone_into_home_and_out_then_pull() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().canonicalize().unwrap();
    let home = ws.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("gitconfig"), "").unwrap();
    // A local stand-in for github.com/octo/hello with a second branch.
    let remote = ws.join("remote/octo/hello.git");
    fs::create_dir_all(&remote).unwrap();
    git(&home, &remote, &["init", "-q"]);
    fs::write(remote.join("README"), "hello\n").unwrap();
    git(&home, &remote, &["add", "README"]);
    git(&home, &remote, &["commit", "-q", "-m", "init"]);
    git(&home, &remote, &["checkout", "-q", "-b", "feature"]);
    fs::write(remote.join("FEATURE"), "f\n").unwrap();
    git(&home, &remote, &["add", "FEATURE"]);
    git(&home, &remote, &["commit", "-q", "-m", "feature"]);
    git(&home, &remote, &["checkout", "-q", "main"]);
    let base = format!("file://{}", ws.join("remote").display());

    let clone = |args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_dijkstral"));
        cmd.arg("clone")
            .args(args)
            .current_dir(&ws)
            .env("DIJKSTRAL_HOME", &home)
            .env("DIJKSTRAL_CLONE_BASE", &base);
        isolate_git(&mut cmd, &home);
        cmd.output().unwrap()
    };
    let out = clone(&["https://github.com/octo/hello"]);
    assert!(out.status.success(), "{out:?}");
    let dest = home.join("repos/octo/hello");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{}\n", dest.display())
    );
    assert!(dest.join("README").is_file());
    assert!(!dest.join("FEATURE").exists());
    assert!(dest.join(".git/shallow").is_file(), "clones are shallow");

    // Cloned already: pulled, same path, other URL forms included.
    for url in ["git@github.com:octo/hello.git", "github.com/octo/hello/"] {
        let out = clone(&[url]);
        assert!(out.status.success(), "{out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{}\n", dest.display())
        );
    }

    let out = clone(&[
        "https://github.com/octo/hello",
        "--branch",
        "feature",
        "--out",
        "br",
    ]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "br\n");
    assert!(ws.join("br/FEATURE").is_file());

    for bad in [
        "octo/hello",
        "https://gitlab.com/octo/hello",
        "https://github.com/octo/..",
    ] {
        let out = clone(&[bad]);
        assert_eq!(out.status.code(), Some(1), "{bad}");
        assert!(out.stdout.is_empty());
    }
    let out = clone(&["https://github.com/octo/missing"]);
    assert_eq!(out.status.code(), Some(1));
    fs::create_dir_all(ws.join("taken")).unwrap();
    fs::write(ws.join("taken/file"), "x").unwrap();
    let out = clone(&["https://github.com/octo/hello", "--out", "taken"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn global_graph_links_follow_src_and_tgt() {
    // Graphify's global graph writes `Alpha -calls-> Beta` as source=Beta, target=Alpha with the
    // direction in `_src`/`_tgt` (§5.2); queries follow `_src`/`_tgt`, as Graphify's do.
    let tmp = tempfile::tempdir().unwrap();
    let g = fixtures().join("multirepo/goldens/global_direction.graph.json");
    let g = g.to_str().unwrap();
    let home = tmp.path().join("home");
    let fwd = ok(&home, tmp.path(), &["path", "Alpha", "Beta", "--graph", g]);
    assert_eq!(
        fwd,
        "Shortest path (1 hops):\n  Alpha --calls [EXTRACTED]--> Beta\n"
    );
    let back = ok(&home, tmp.path(), &["path", "Beta", "Alpha", "--graph", g]);
    assert!(back.starts_with("No directed path found"), "{back}");
}

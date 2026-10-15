//! `dijkstral hook install|uninstall|status`, the hooks themselves and `merge-driver` in scratch
//! git repositories (`docs/graphify-git.md` §2–§4). Offline; the user's and the system's git
//! config are kept out (`GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

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

/// A command with the user's git configuration kept out.
fn isolated(cmd: &mut Command, dir: &Path) {
    cmd.current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("SOURCE_DATE_EPOCH", "1760400000")
        .env_remove("DIJKSTRAL_HOOK_SKIP")
        .env_remove("DIJKSTRAL_HOOK_SYNC");
}

fn git_with(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("git");
    isolated(&mut cmd, dir);
    cmd.args([
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "init.defaultBranch=main",
    ])
    .args(args)
    .envs(env.iter().copied());
    cmd.output().unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_with(dir, args, &[]);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn dij(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dijkstral"));
    isolated(&mut cmd, dir);
    cmd.args(args).output().unwrap()
}

fn dij_ok(dir: &Path, args: &[&str]) -> String {
    let out = dij(dir, args);
    assert!(
        out.status.success(),
        "dijkstral {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo(tmp: &Path, fixture: Option<&str>) -> PathBuf {
    let root = tmp.join("repo");
    match fixture {
        Some(f) => copy_dir(&fixtures().join(f), &root),
        None => fs::create_dir_all(&root).unwrap(),
    }
    git(&root, &["init", "-q"]);
    root
}

fn node_ids(root: &Path) -> Vec<String> {
    let g: Value =
        serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap();
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect()
}

#[cfg(unix)]
fn mode(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[test]
fn install_status_uninstall_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let root = repo(tmp.path(), None);
    let hooks = root.join(".git/hooks");
    let foreign = "#!/bin/sh\necho user hook >> \"$(git rev-parse --git-dir)/user-hook-ran\"\n";
    fs::write(hooks.join("post-commit"), foreign).unwrap();
    fs::write(root.join(".gitattributes"), "*.png binary\n").unwrap();

    let status = dij_ok(&root, &["hook", "status"]);
    assert_eq!(
        status,
        "post-commit: not installed (a hook exists, without dijkstral's block)\n\
         post-checkout: not installed\nmerge driver: not configured\n"
    );

    let out = dij_ok(&root, &["hook", "install"]);
    assert!(
        out.contains("post-commit: added to the existing hook"),
        "{out}"
    );
    assert!(out.contains("post-checkout: installed ("), "{out}");
    assert!(out.contains("merge driver: configured"), "{out}");
    assert!(
        out.contains("note: there is no dijkstral-out/graph.json yet"),
        "{out}"
    );
    let commit_hook = fs::read_to_string(hooks.join("post-commit")).unwrap();
    assert!(commit_hook.starts_with(foreign));
    assert!(commit_hook.contains("# dijkstral-hook-start\n"));
    assert!(commit_hook.contains(" hook run post-commit \"$@\""));
    let checkout_hook = fs::read_to_string(hooks.join("post-checkout")).unwrap();
    assert!(checkout_hook.starts_with("#!/bin/sh\n# dijkstral-hook-start\n"));
    #[cfg(unix)]
    assert_eq!(mode(&hooks.join("post-checkout")), 0o755);
    let driver = git(&root, &["config", "--local", "merge.dijkstral.driver"]);
    assert!(
        driver.trim_end().ends_with(" merge-driver %O %A %B"),
        "{driver}"
    );
    assert_eq!(
        fs::read_to_string(root.join(".gitattributes")).unwrap(),
        "*.png binary\ndijkstral-out/graph.json merge=dijkstral\n"
    );

    // Idempotent.
    let before = (
        commit_hook.clone(),
        checkout_hook.clone(),
        fs::read(root.join(".git/config")).unwrap(),
    );
    let again = dij_ok(&root, &["hook", "install"]);
    assert!(again.contains("post-commit: already installed"), "{again}");
    assert!(
        again.contains("merge driver: already configured"),
        "{again}"
    );
    assert_eq!(
        before,
        (
            fs::read_to_string(hooks.join("post-commit")).unwrap(),
            fs::read_to_string(hooks.join("post-checkout")).unwrap(),
            fs::read(root.join(".git/config")).unwrap(),
        )
    );
    assert_eq!(
        dij_ok(&root, &["hook", "status"]),
        "post-commit: installed\npost-checkout: installed\nmerge driver: configured\n"
    );

    // An edited block is outdated until the next install.
    fs::write(
        hooks.join("post-checkout"),
        checkout_hook.replace("|| :", "|| true"),
    )
    .unwrap();
    let status = dij_ok(&root, &["hook", "status"]);
    assert!(
        status.contains("post-checkout: installed, outdated"),
        "{status}"
    );
    assert!(dij_ok(&root, &["hook", "install"]).contains("post-checkout: refreshed"));

    let out = dij_ok(&root, &["hook", "uninstall"]);
    assert!(
        out.contains("post-commit: removed dijkstral's block"),
        "{out}"
    );
    assert!(out.contains("post-checkout: removed "), "{out}");
    assert!(out.contains("merge driver: removed git config"), "{out}");
    assert_eq!(
        fs::read_to_string(hooks.join("post-commit")).unwrap(),
        foreign
    );
    assert!(!hooks.join("post-checkout").exists());
    assert_eq!(
        fs::read_to_string(root.join(".gitattributes")).unwrap(),
        "*.png binary\n"
    );
    assert!(
        !git_with(&root, &["config", "--local", "merge.dijkstral.driver"], &[])
            .status
            .success()
    );
    let again = dij_ok(&root, &["hook", "uninstall"]);
    assert_eq!(
        again,
        "post-commit: nothing to remove\npost-checkout: nothing to remove\n\
         merge driver: nothing to remove\n"
    );
}

#[test]
fn hooks_follow_core_hooks_path_and_worktrees() {
    let tmp = tempfile::tempdir().unwrap();
    let root = repo(tmp.path(), None);
    fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    git(&root, &["config", "core.hooksPath", ".githooks"]);
    dij_ok(&root, &["hook", "install"]);
    assert!(root.join(".githooks/post-commit").is_file());
    assert!(!root.join(".git/hooks/post-commit").exists());

    // A linked worktree shares the hooks: status from there sees them.
    git(&root, &["config", "--unset", "core.hooksPath"]);
    dij_ok(&root, &["hook", "install"]);
    let wt = tmp.path().join("wt");
    git(&root, &["worktree", "add", "-q", wt.to_str().unwrap()]);
    let status = dij_ok(&wt, &["hook", "status"]);
    assert!(
        status.starts_with("post-commit: installed\npost-checkout: installed\n"),
        "{status}"
    );
}

#[test]
fn outside_a_work_tree_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    for action in ["install", "uninstall", "status"] {
        let out = dij(tmp.path(), &["hook", action]);
        assert_eq!(out.status.code(), Some(1), "{action}");
        assert!(out.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "error: not inside a git work tree\n"
        );
    }
}

#[test]
fn refuses_to_extend_a_hook_in_another_language() {
    let tmp = tempfile::tempdir().unwrap();
    let root = repo(tmp.path(), None);
    let python = "#!/usr/bin/env python3\nprint('hi')\n";
    fs::write(root.join(".git/hooks/post-commit"), python).unwrap();
    let out = dij(&root, &["hook", "install"]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("post-commit: skipped:"), "{stdout}");
    assert!(stdout.contains("post-checkout: installed"), "{stdout}");
    assert_eq!(
        fs::read_to_string(root.join(".git/hooks/post-commit")).unwrap(),
        python
    );
}

#[test]
fn hook_run_fails_open() {
    let tmp = tempfile::tempdir().unwrap();
    for args in [
        &["hook", "run", "post-commit"][..],
        &["hook", "run", "post-checkout", "a", "b", "1"],
        &["hook", "run", "no-such-hook", "--weird"],
    ] {
        let out = dij(tmp.path(), args);
        assert!(out.status.success(), "{args:?}");
        assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{args:?}");
    }
    assert!(
        !tmp.path().join("dijkstral-out").exists(),
        "no graph, no build"
    );
}

#[test]
fn commits_and_branch_switches_rebuild_the_graph() {
    let tmp = tempfile::tempdir().unwrap();
    let root = repo(tmp.path(), Some("python"));
    fs::write(root.join(".gitignore"), "dijkstral-out/\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);
    dij_ok(&root, &["update", "."]);
    dij_ok(&root, &["hook", "install"]);
    let sync = [("DIJKSTRAL_HOOK_SYNC", "1")];

    // post-commit, in the foreground for the test.
    fs::write(
        root.join("added.py"),
        "def added_by_commit():\n    return 1\n",
    )
    .unwrap();
    git(&root, &["add", "-A"]);
    let out = git_with(&root, &["commit", "-qm", "add"], &sync);
    assert!(out.status.success());
    assert!(node_ids(&root)
        .iter()
        .any(|id| id == "added_added_by_commit"));
    let log = fs::read_to_string(root.join("dijkstral-out/hook.log")).unwrap();
    assert!(log.contains("wrote "), "{log}");

    // post-checkout: a branch switch rebuilds; a file checkout does not.
    git(&root, &["checkout", "-qb", "side"]);
    fs::write(root.join("side.py"), "def only_on_side():\n    return 2\n").unwrap();
    git(&root, &["add", "-A"]);
    git_with(&root, &["commit", "-qm", "side"], &sync);
    assert!(node_ids(&root).iter().any(|id| id == "side_only_on_side"));
    git_with(&root, &["checkout", "-q", "main"], &sync);
    assert!(!node_ids(&root).iter().any(|id| id == "side_only_on_side"));

    // Skipped on request.
    git_with(
        &root,
        &["checkout", "-q", "side"],
        &[("DIJKSTRAL_HOOK_SYNC", "1"), ("DIJKSTRAL_HOOK_SKIP", "1")],
    );
    assert!(!node_ids(&root).iter().any(|id| id == "side_only_on_side"));

    // In the background: the commit returns, the graph catches up.
    fs::write(
        root.join("later.py"),
        "def built_in_background():\n    return 3\n",
    )
    .unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "later"]);
    let deadline = Instant::now() + Duration::from_secs(60);
    while !node_ids(&root)
        .iter()
        .any(|id| id == "later_built_in_background")
    {
        assert!(
            Instant::now() < deadline,
            "the background rebuild never landed"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_hook_never_breaks_a_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = repo(tmp.path(), Some("python"));
    fs::write(root.join(".gitignore"), "dijkstral-out/\n").unwrap();
    dij_ok(&root, &["update", "."]);
    dij_ok(&root, &["hook", "install"]);
    // The installed binary disappears and none is on PATH: the block does nothing.
    let hook = root.join(".git/hooks/post-commit");
    let text = fs::read_to_string(&hook).unwrap();
    let exe = env!("CARGO_BIN_EXE_dijkstral");
    fs::write(&hook, text.replace(exe, "/nonexistent/dijkstral")).unwrap();
    git(&root, &["add", "-A"]);
    // Only git's own directory on PATH, so no other dijkstral is found either.
    let path = std::env::var_os("PATH").unwrap();
    let git_dir = std::env::split_paths(&path)
        .find(|d| d.join("git").is_file() && !d.join("dijkstral").exists())
        .expect("git on PATH");
    let path = format!("{}:/bin", git_dir.display());
    let out = git_with(&root, &["commit", "-qm", "x"], &[("PATH", &path)]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Two branches that each add a file and commit their rebuilt graph.json.
fn diverged(tmp: &Path, install: bool) -> PathBuf {
    let root = repo(tmp, Some("python"));
    // Only graph.json is tracked from the output directory.
    fs::write(
        root.join(".gitignore"),
        "dijkstral-out/*\n!dijkstral-out/graph.json\n",
    )
    .unwrap();
    dij_ok(&root, &["update", ".", "--no-viz"]);
    if install {
        dij_ok(&root, &["hook", "install"]);
    }
    let skip = [("DIJKSTRAL_HOOK_SKIP", "1")];
    let commit = |msg: &str| {
        git(&root, &["add", "-A"]);
        assert!(git_with(&root, &["commit", "-qm", msg], &skip)
            .status
            .success());
    };
    commit("init");
    git_with(&root, &["checkout", "-qb", "feature"], &skip);
    fs::write(
        root.join("feature.py"),
        "def from_feature():\n    return 1\n",
    )
    .unwrap();
    dij_ok(&root, &["update", ".", "--no-viz"]);
    commit("feature");
    git_with(&root, &["checkout", "-q", "main"], &skip);
    fs::write(root.join("mainline.py"), "def from_main():\n    return 2\n").unwrap();
    dij_ok(&root, &["update", ".", "--no-viz"]);
    commit("main");
    root
}

#[test]
fn git_merge_resolves_graph_json_through_the_driver() {
    let tmp = tempfile::tempdir().unwrap();
    let root = diverged(tmp.path(), true);
    let out = git_with(
        &root,
        &["merge", "--no-edit", "feature"],
        &[("DIJKSTRAL_HOOK_SKIP", "1")],
    );
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let ids = node_ids(&root);
    assert!(ids.iter().any(|id| id == "feature_from_feature"));
    assert!(ids.iter().any(|id| id == "mainline_from_main"));
    assert!(git(&root, &["status", "--porcelain"]).is_empty());
    // The merged graph is canonical: merging it with itself changes nothing.
    let merged = fs::read(root.join("dijkstral-out/graph.json")).unwrap();
    let again = dijkstral_git_identity(&root, &merged);
    assert!(again == merged);

    // Without the driver the same merge conflicts on the one-line JSON.
    let tmp = tempfile::tempdir().unwrap();
    let root = diverged(tmp.path(), false);
    let out = git_with(&root, &["merge", "--no-edit", "feature"], &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("dijkstral-out/graph.json"));
}

/// `merge-driver` of a graph with itself, through the CLI.
fn dijkstral_git_identity(dir: &Path, graph: &[u8]) -> Vec<u8> {
    let work = dir.join(".git/identity-check");
    fs::create_dir_all(&work).unwrap();
    for n in ["O", "A", "B"] {
        fs::write(work.join(n), graph).unwrap();
    }
    dij_ok(&work, &["merge-driver", "O", "A", "B"]);
    fs::read(work.join("A")).unwrap()
}

#[test]
fn merge_driver_leaves_a_conflict_on_bad_input() {
    let tmp = tempfile::tempdir().unwrap();
    let good = fs::read(fixtures().join("git/merge/identical/current.json")).unwrap();
    fs::write(tmp.path().join("O"), &good).unwrap();
    fs::write(tmp.path().join("A"), &good).unwrap();
    fs::write(tmp.path().join("B"), "{not json").unwrap();
    let out = dij(tmp.path(), &["merge-driver", "O", "A", "B"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.starts_with("dijkstral merge-driver: cannot merge graph.json (other version"),
        "{err}"
    );
    assert_eq!(fs::read(tmp.path().join("A")).unwrap(), good);
    // A missing base is a union, not an error.
    fs::write(tmp.path().join("B"), &good).unwrap();
    let out = dij(tmp.path(), &["merge-driver", "missing", "A", "B"]);
    assert!(out.status.success());
}

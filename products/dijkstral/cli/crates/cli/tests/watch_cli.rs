//! `dijkstral watch` on a scratch copy of a fixture (`docs/graphify-git.md` §5): an initial
//! update, a rebuild after an edit, silence for its own writes and for skipped paths, and a clean
//! stop on SIGINT. Every wait is bounded.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

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

/// Kills the watcher if the test fails before stopping it.
struct Watcher(Child);

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Lines of stdout until one contains `needle`, or panic after `secs`.
fn wait_for(lines: &Receiver<String>, needle: &str, secs: u64, seen: &mut Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) => {
                let hit = line.contains(needle);
                seen.push(line);
                if hit {
                    return;
                }
            }
            Err(_) => panic!("no line with {needle:?} within {secs}s; output so far:\n{seen:#?}"),
        }
    }
}

/// No line containing `needle` arrives within `secs`.
fn quiet_for(lines: &Receiver<String>, needle: &str, secs: u64, seen: &mut Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while let Ok(line) = lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        assert!(
            !line.contains(needle),
            "unexpected {line:?}; output:\n{seen:#?}"
        );
        seen.push(line);
    }
}

#[test]
#[cfg(unix)]
fn watch_rebuilds_on_edits_and_stops_on_sigint() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("python");
    copy_dir(&fixtures().join("python"), &root);

    let child = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .args(["watch", root.to_str().unwrap(), "--debounce", "0.3"])
        .env("SOURCE_DATE_EPOCH", "1760400000")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut w = Watcher(child);
    let stdout = w.0.stdout.take().unwrap();
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap_or_default()).is_err() {
                break;
            }
        }
    });
    let mut seen = Vec::new();

    // The initial update builds the graph.
    wait_for(&lines, "[dijkstral watch] watching ", 30, &mut seen);
    wait_for(&lines, "wrote ", 60, &mut seen);
    assert!(root.join("dijkstral-out/graph.json").is_file());
    // Its own writes (graph.json, the cache, graph.html) do not start another build.
    quiet_for(&lines, "changed", 2, &mut seen);

    // A code edit rebuilds once, after the quiet period.
    let main = root.join("main.py");
    let mut text = fs::read_to_string(&main).unwrap();
    text.push_str("\n\ndef added_while_watching():\n    return 42\n");
    fs::write(&main, text).unwrap();
    wait_for(
        &lines,
        "[dijkstral watch] 1 changed: main.py",
        30,
        &mut seen,
    );
    wait_for(&lines, "wrote ", 60, &mut seen);
    let graph = fs::read_to_string(root.join("dijkstral-out/graph.json")).unwrap();
    assert!(graph.contains("\"main_added_while_watching\""));

    // Skipped directories and the output directory are not watched for changes.
    fs::create_dir_all(root.join("__pycache__")).unwrap();
    fs::write(root.join("__pycache__/x.py"), "def ignored():\n    pass\n").unwrap();
    fs::write(root.join("dijkstral-out/scratch.txt"), "x").unwrap();
    quiet_for(&lines, "changed", 2, &mut seen);

    // SIGINT stops it cleanly.
    let pid = w.0.id().to_string();
    assert!(Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .unwrap()
        .success());
    wait_for(&lines, "[dijkstral watch] stopped", 20, &mut seen);
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = w.0.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < deadline, "watch did not exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status}");
}

#[test]
fn watch_rejects_a_missing_directory() {
    let out = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .args(["watch", "/nonexistent/dijkstral-watch-test"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error: reading /nonexistent"));
}

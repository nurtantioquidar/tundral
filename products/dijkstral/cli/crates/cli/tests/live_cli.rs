//! The `dijkstral-live` split (`src/live.rs`): `dijkstral` itself links nothing beyond the system
//! libraries, so its start (paid on every assistant hook call) stays minimal; `watch` and
//! `extract --postgres` reach the helper next to it, and fail cleanly when it is missing or from
//! another build. Offline: the PostgreSQL case connects to a closed port.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// macOS loads every linked framework at each start (the file watcher's CoreServices and the
/// PostgreSQL client's SystemConfiguration and Foundation cost about 4 MB and 0.6 ms); they
/// belong in `dijkstral-live`.
#[cfg(target_os = "macos")]
#[test]
fn dijkstral_links_only_system_libraries() {
    const ALLOWED: [&str; 2] = ["/usr/lib/libSystem.B.dylib", "/usr/lib/libiconv.2.dylib"];
    let Ok(out) = Command::new("otool")
        .arg("-L")
        .arg(env!("CARGO_BIN_EXE_dijkstral"))
        .output()
    else {
        eprintln!("otool not available; skipped");
        return;
    };
    assert!(out.status.success(), "{}", stderr(&out));
    let text = String::from_utf8_lossy(&out.stdout);
    let libs: Vec<&str> = text
        .lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert!(!libs.is_empty(), "{text}");
    for lib in libs {
        assert!(
            ALLOWED.contains(&lib),
            "dijkstral links {lib}; move its user to dijkstral-live"
        );
    }
}

#[test]
fn postgres_errors_come_back_without_the_password() {
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .arg("extract")
        .arg(fixtures().join("python"))
        .args(["--code-only", "--no-viz", "--out"])
        .arg(tmp.path())
        .args([
            "--postgres",
            "postgresql://probe:hunter2secret@127.0.0.1:1/shop",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("could not connect to PostgreSQL at 127.0.0.1/shop"),
        "{err}"
    );
    assert!(!err.contains("hunter2secret"), "{err}");
}

#[test]
fn a_missing_helper_fails_only_its_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let lone = tmp.path().join("dijkstral");
    fs::copy(env!("CARGO_BIN_EXE_dijkstral"), &lone).unwrap();
    let out = Command::new(&lone)
        .arg("watch")
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("dijkstral-live is missing"), "{err}");
    let out = Command::new(&lone).arg("--version").output().unwrap();
    assert!(out.status.success());
}

#[test]
fn the_helper_refuses_another_build() {
    let helper = Path::new(env!("CARGO_BIN_EXE_dijkstral")).with_file_name("dijkstral-live");
    let out = Command::new(&helper)
        .args(["0.0.0-other", "postgres"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("cannot serve dijkstral 0.0.0-other"),
        "{}",
        stderr(&out)
    );
    let out = Command::new(&helper).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

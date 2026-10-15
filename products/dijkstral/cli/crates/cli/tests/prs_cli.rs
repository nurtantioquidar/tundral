//! `prs` and the MCP PR tools against Graphify's goldens (`tests/fixtures/memory/prs/`,
//! `docs/graphify-memory.md` §5, §7), offline: a fake `gh` first on `PATH` serves the scenario
//! files and logs its arguments. Nothing reaches GitHub.
//!
//! The dashboard and detail view are compared as data (rows, states, ages, titles, worktree marks,
//! summary counts, detail fields); the header lines and messages are dijkstral's own wording. The
//! MCP entry lines must match Graphify's exactly.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

const FAKE_GH: &str = r#"#!/bin/sh
# Fake gh for tests: serves files from $FAKE_GH_DIR and logs its arguments.
d="$FAKE_GH_DIR"
printf '%s\n' "$*" >> "$d/log"
case "$1 $2" in
  "repo view") f="$d/repo_view.json" ;;
  "pr list") f="$d/list.json" ;;
  "pr diff") f="$d/diff_$3.txt" ;;
  "pr view") f="$d/view_$3.json" ;;
  *) exit 1 ;;
esac
[ -f "$f" ] || exit 1
cat "$f"
"#;

struct Env {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    worktree: String,
    bin: PathBuf,
    gh: PathBuf,
}

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(st.success(), "git {args:?}");
}

/// A repository on `main` with a local branch and a worktree on `wt-branch`, Graphify's python
/// graph under graphify-out/, and the fake gh.
fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(
        &repo,
        &[
            "-c",
            "user.email=a@b",
            "-c",
            "user.name=n",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    git(&repo, &["branch", "local-only"]);
    let wt = tmp.path().join("wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            wt.to_str().unwrap(),
        ],
    );
    fs::create_dir(repo.join("graphify-out")).unwrap();
    fs::copy(
        fixtures().join("python.clustered.graphify.json"),
        repo.join("graphify-out/graph.json"),
    )
    .unwrap();
    let bin = tmp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("gh"), FAKE_GH).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let gh = tmp.path().join("gh");
    let worktree = fs::canonicalize(&wt).unwrap().display().to_string();
    let repo = fs::canonicalize(&repo).unwrap();
    Env {
        _tmp: tmp,
        repo,
        worktree,
        bin,
        gh,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Days since the epoch → (y, m, d), for RFC 3339 stamps.
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn rfc3339(t: i64) -> String {
    let (y, m, d) = civil(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// Write the fake gh's files for a scenario (and a variant of it).
fn serve(env: &Env, scenario: &Value, variant: &str) {
    if env.gh.exists() {
        fs::remove_dir_all(&env.gh).unwrap();
    }
    fs::create_dir(&env.gh).unwrap();
    let now = now();
    let mut prs = Vec::new();
    for p in scenario["prs"].as_array().unwrap() {
        let mut p = p.clone();
        let u = &p["updatedAt"];
        if u.is_object() {
            let ago = u["days_ago"].as_i64().unwrap_or(0) * 86_400
                + u["hours_ago"].as_i64().unwrap_or(0) * 3600
                + u["minutes_ago"].as_i64().unwrap_or(0) * 60;
            p["updatedAt"] = Value::String(rfc3339(now - ago));
        }
        let mut view = p.clone();
        view.as_object_mut().unwrap().remove("number");
        fs::write(
            env.gh.join(format!("view_{}.json", p["number"])),
            view.to_string(),
        )
        .unwrap();
        prs.push(p);
    }
    if variant == "empty" {
        prs.clear();
    }
    match variant {
        "listfail" => {}
        "badjson" => fs::write(env.gh.join("list.json"), "not json").unwrap(),
        _ => fs::write(env.gh.join("list.json"), Value::Array(prs).to_string()).unwrap(),
    }
    if let (Some(b), false) = (scenario["default_branch"].as_str(), variant == "nobranch") {
        fs::write(
            env.gh.join("repo_view.json"),
            json!({"defaultBranchRef": {"name": b}}).to_string(),
        )
        .unwrap();
    }
    for (n, files) in scenario["files"].as_object().into_iter().flatten() {
        let lines: Vec<&str> = files
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        fs::write(
            env.gh.join(format!("diff_{n}.txt")),
            lines.join("\n") + "\n",
        )
        .unwrap();
    }
    fs::write(env.gh.join("log"), "").unwrap();
}

fn path_var(env: &Env) -> String {
    let rest = std::env::var("PATH").unwrap_or_default();
    format!("{}:{rest}", env.bin.display())
}

fn command(env: &Env) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_dijkstral"));
    c.current_dir(&env.repo)
        .env("PATH", path_var(env))
        .env("FAKE_GH_DIR", &env.gh)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    c
}

fn calls(env: &Env) -> Vec<String> {
    fs::read_to_string(env.gh.join("log"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn scenario(name: &str) -> Value {
    serde_json::from_slice(
        &fs::read(fixtures().join(format!("memory/prs/{name}.scenario.json"))).unwrap(),
    )
    .unwrap()
}

/// A dashboard as data.
#[derive(Debug, PartialEq)]
struct Dashboard {
    base: String,
    count: String,
    /// number, worktree mark, CI glyph, status, age, impact, title.
    rows: Vec<[String; 7]>,
    summary: BTreeMap<String, u64>,
}

fn summary_label(l: &str) -> &str {
    match l {
        "pending CI" => "CI pending",
        "wrong base" => "on another base",
        other => other,
    }
}

fn parse_dashboard(text: &str) -> Dashboard {
    let lines: Vec<&str> = text.lines().collect();
    let header = lines.iter().find(|l| l.contains("·  base: ")).unwrap();
    let parts: Vec<&str> = header.split('·').map(str::trim).collect();
    let base = parts[1].trim_start_matches("base: ").to_string();
    let count = parts[2].split(' ').next().unwrap().to_string();
    let mut rows: Vec<[String; 7]> = Vec::new();
    let mut summary = BTreeMap::new();
    for l in &lines {
        let t = l.trim_start();
        if t.starts_with('#') && t.len() > 1 && t.as_bytes()[1].is_ascii_digit() {
            let mut tok = t.split_whitespace().peekable();
            let num = tok.next().unwrap().to_string();
            let mark = if tok.peek() == Some(&"⬡") {
                tok.next();
                "⬡"
            } else {
                ""
            };
            let glyph = tok.next().unwrap().to_string();
            let status = tok.next().unwrap().to_string();
            let age = tok.next().unwrap().to_string();
            let impact = tok.next().unwrap().to_string();
            let title = tok.collect::<Vec<_>>().join(" ");
            rows.push([num, mark.into(), glyph, status, age, impact, title]);
        } else if t.contains("base: ") {
            continue;
        } else if t
            .split(' ')
            .next()
            .is_some_and(|w| w.parse::<u64>().is_ok())
        {
            for part in t.split(" · ") {
                let (n, label) = part.split_once(' ').unwrap();
                summary.insert(summary_label(label).to_string(), n.parse().unwrap());
            }
        } else if !t.is_empty()
            && !t.starts_with('─')
            && !t.starts_with('#')
            && !t.contains("base: ")
            && !t.starts_with("No open")
            && !t.contains("STATUS")
        {
            // A title continued on the next line (Graphify prints titles raw).
            let last = rows.last_mut().expect("continuation after a row");
            last[6].push(' ');
            last[6].push_str(t);
        }
    }
    Dashboard {
        base,
        count,
        rows,
        summary,
    }
}

/// A detail view as data: field name → value, plus the files listed.
fn parse_detail(text: &str, ours: bool) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let head = lines[1];
    let (num, status) = head.trim_start_matches("PR ").split_once("  ·  ").unwrap();
    m.insert("number".into(), num.to_string());
    m.insert("status".into(), status.to_string());
    m.insert("title".into(), lines[2].to_string());
    let mut files = Vec::new();
    for l in &lines[3..] {
        let (k, v) = if ours {
            match l.split_once(char::is_whitespace) {
                Some((k, v)) => (k, v.trim_start()),
                None => (*l, ""),
            }
        } else {
            match l.split_once(':') {
                Some((k, v)) => (k, v.trim_start()),
                None => ("", *l),
            }
        };
        match k {
            "branch" => {
                let v = v.replace("  ->  ", " -> ");
                m.insert("branch".into(), v);
            }
            "author" | "updated" | "CI" | "review" | "worktree" => {
                m.insert(k.into(), v.to_string());
            }
            "Graph impact" | "impact" => {
                // Graphify: "<n> nodes / <c> communities" (+ next line); dijkstral one line.
                let nodes = v.split(' ').next().unwrap().to_string();
                m.insert("impact_nodes".into(), nodes);
                if ours {
                    let list = &v[v.find('[').unwrap()..];
                    m.insert("communities".into(), list.to_string());
                }
            }
            "communities" => {
                m.insert("communities".into(), v.to_string());
            }
            "files changed" => {
                m.insert("files".into(), v.to_string());
            }
            "files" => {
                m.insert("files".into(), v.trim_end_matches(" changed").to_string());
            }
            _ if !l.is_empty() => files.push(l.to_string()),
            _ => {}
        }
    }
    m.insert("listed".into(), files.join("|"));
    m
}

#[test]
fn prs_matches_graphify() {
    let env = setup();
    let cases: Value =
        serde_json::from_slice(&fs::read(fixtures().join("memory/cases.json")).unwrap()).unwrap();
    let mut checked = 0;
    for c in cases["prs"].as_array().unwrap() {
        let name = c["case"].as_str().unwrap();
        serve(
            &env,
            &scenario(c["scenario"].as_str().unwrap()),
            c["variant"].as_str().unwrap(),
        );
        let args: Vec<&str> = c["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        let out: Output = command(&env).args(&args).output().unwrap();
        let code = out.status.code().unwrap();
        assert_eq!(code as i64, c["exit"].as_i64().unwrap(), "{name}: {out:?}");
        let stdout = String::from_utf8(out.stdout)
            .unwrap()
            .replace(&env.worktree, "<WORKTREE>")
            .replace(&env.repo.display().to_string(), "<REPO>");
        let golden = fs::read_to_string(
            fixtures()
                .join("memory")
                .join(c["stdout"].as_str().unwrap()),
        )
        .unwrap();
        let got_calls = calls(&env);
        let want_calls: Vec<String> = c["gh_calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                a.as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        if code != 0 {
            assert!(stdout.is_empty() && golden.is_empty(), "{name}");
            assert!(!out.stderr.is_empty(), "{name}");
        } else if args.len() == 2 && args[1].parse::<i64>().is_ok() {
            assert_eq!(
                parse_detail(&stdout, true),
                parse_detail(&golden, false),
                "{name}:\n{stdout}"
            );
        } else {
            assert_eq!(
                parse_dashboard(&stdout),
                parse_dashboard(&golden),
                "{name}:\n{stdout}"
            );
        }
        // The same read-only calls; the detail view diffs only the PR it shows.
        let own_diff = format!("pr diff {} --name-only", args.get(1).unwrap_or(&""));
        for call in &got_calls {
            assert!(
                want_calls.contains(call) || *call == own_diff,
                "{name}: unexpected gh {call}"
            );
            assert!(
                call.starts_with("repo view")
                    || call.starts_with("pr list")
                    || call.starts_with("pr diff"),
                "{name}: {call}"
            );
        }
        if !(args.len() == 2 && args[1].parse::<i64>().is_ok()) {
            assert_eq!(got_calls, want_calls, "{name}");
        }
        checked += 1;
    }
    assert_eq!(checked, 19);
}

fn mcp_call(env: &Env, calls: &[(String, Value)]) -> Vec<(String, bool)> {
    let mut child = command(env)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |v: Value| -> Value {
        writeln!(stdin, "{v}").unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    send(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
    );
    let mut out = Vec::new();
    for (i, (tool, args)) in calls.iter().enumerate() {
        let r = send(
            json!({"jsonrpc": "2.0", "id": 10 + i, "method": "tools/call", "params": {"name": tool, "arguments": args}}),
        );
        out.push((
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string(),
            r["result"]["isError"].as_bool().unwrap(),
        ));
    }
    drop(stdin);
    child.wait().unwrap();
    out
}

#[test]
fn mcp_pr_tools_match_graphify() {
    let env = setup();
    let cases: Value =
        serde_json::from_slice(&fs::read(fixtures().join("memory/cases.json")).unwrap()).unwrap();
    let mcp = cases["prs_mcp"].as_array().unwrap();
    let mut checked = 0;
    for sc_name in ["sc3", "sc2"] {
        let mut sc = scenario(sc_name);
        if sc_name == "sc2" {
            sc["files"] = json!({"201": ["pkg/service.py"], "203": ["nothing.py"], "999": ["pkg/util.py"], "216": ["pkg/models.py"]});
        }
        serve(&env, &sc, "");
        let rows: Vec<&Value> = mcp.iter().filter(|c| c["scenario"] == sc_name).collect();
        let calls: Vec<(String, Value)> = rows
            .iter()
            .map(|c| {
                (
                    c["tool"].as_str().unwrap().to_string(),
                    c["arguments"].clone(),
                )
            })
            .collect();
        let got = mcp_call(&env, &calls);
        for (c, (text, is_error)) in rows.iter().zip(got) {
            let tool = c["tool"].as_str().unwrap();
            let text = text
                .replace(&env.worktree, "<WORKTREE>")
                .replace(&env.repo.display().to_string(), "<REPO>");
            // Graphify prints titles raw; dijkstral keeps one entry per two lines.
            let golden =
                fs::read_to_string(fixtures().join("memory").join(c["text"].as_str().unwrap()))
                    .unwrap()
                    .replace("multi\nline title", "multi line title");
            let label = format!("{sc_name} {tool} {}", c["arguments"]);
            let want_error = c["isError"].as_bool().unwrap();
            match tool {
                "list_prs" => {
                    assert!(!is_error, "{label}");
                    let skip = |t: &str| t.split_once('\n').map_or("", |x| x.1).to_string();
                    assert_eq!(skip(&text), skip(&golden), "{label}:\n{text}");
                }
                "triage_prs" if golden.contains('\n') => {
                    assert!(!is_error, "{label}");
                    let skip = |t: &str| t.splitn(3, '\n').nth(2).unwrap_or("").to_string();
                    assert_eq!(skip(&text), skip(&golden), "{label}:\n{text}");
                }
                "triage_prs" => assert!(!is_error && !text.contains('\n'), "{label}"),
                _ if want_error => {
                    assert!(is_error && text.contains("#77"), "{label}: {text}");
                }
                _ if golden.ends_with("(may require gh auth).") => {
                    assert!(
                        !is_error && !text.contains('\n') && text.contains("#3"),
                        "{label}"
                    );
                }
                _ => {
                    assert!(!is_error, "{label}");
                    assert_eq!(text, golden, "{label}");
                }
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 12);
}

#[test]
fn prs_without_gh_fails_cleanly() {
    let env = setup();
    let out = Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .arg("prs")
        .current_dir(&env.repo)
        .env("PATH", env.gh.display().to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("gh auth login"));
}

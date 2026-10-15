//! The track-D command surface offline (`docs/graphify-inputs.md`): `extract --cargo`,
//! `--no-gitignore`, `--max-workers`, `--output`, `--google-workspace` (a fake `gws`),
//! `provider list|show|add|remove` (a scratch `HOME`) and custom providers as `--backend` for
//! `extract`, `label --missing-only` and `cluster-only --backend` (a local fake
//! OpenAI-compatible server). Nothing here reaches a model or the network.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/inputs")
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

/// `dijkstral` with `HOME` at `home`, no model keys and no Codex, plus `env`.
fn run_in(home: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dijkstral"));
    cmd.args(args)
        .env("HOME", home)
        .env("SOURCE_DATE_EPOCH", "1760400000")
        .env("DIJKSTRAL_CODEX", home.join("no-codex"));
    for k in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "MOONSHOT_API_KEY",
        "DEEPSEEK_API_KEY",
        "GRAPHIFY_FORCE",
        "DIJKSTRAL_GWS",
    ] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn text(out: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn ok(home: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let out = run_in(home, args, env);
    let (stdout, stderr) = text(&out);
    assert!(out.status.success(), "dijkstral {args:?}: {stderr}");
    stdout
}

fn graph(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap()
}

fn sources(root: &Path) -> Vec<String> {
    let mut s: Vec<String> = graph(root)["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["source_file"].as_str().unwrap().to_string())
        .collect();
    s.sort();
    s.dedup();
    s
}

#[test]
fn cargo_flag_adds_the_crate_graph() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("ws");
    copy_dir(&fixtures().join("cargo"), &root);
    let r = root.to_str().unwrap();
    let out = ok(
        tmp.path(),
        &["extract", r, "--code-only", "--no-viz", "--cargo"],
        &[],
    );
    assert!(out.contains("cargo: 4 crates, 4 dependencies"), "{out}");
    let links = graph(&root)["links"].clone();
    let crate_links: Vec<&Value> = links
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["relation"] == "crate_depends_on")
        .collect();
    assert_eq!(crate_links.len(), 1, "{crate_links:?}");
    // Dropping the flag rebuilds without them.
    ok(tmp.path(), &["extract", r, "--code-only", "--no-viz"], &[]);
    assert!(!graph(&root).to_string().contains("crate_depends_on"));
}

#[test]
fn walk_and_output_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("c");
    for (p, t) in [
        ("gen/g.py", "def g():\n    return 1\n"),
        ("keep/k.py", "def k():\n    return 1\n"),
        ("top.py", "def top():\n    return 1\n"),
        (".gitignore", "gen/\n"),
        (".graphifyignore", "keep/\n"),
    ] {
        let path = root.join(p);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, t).unwrap();
    }
    let r = root.to_str().unwrap();
    ok(tmp.path(), &["extract", r, "--code-only", "--no-viz"], &[]);
    assert_eq!(sources(&root), ["top.py"]);
    ok(
        tmp.path(),
        &[
            "extract",
            r,
            "--code-only",
            "--no-viz",
            "--no-gitignore",
            "--max-workers",
            "2",
        ],
        &[],
    );
    assert_eq!(sources(&root), ["gen/g.py", "top.py"]);

    let out = run_in(
        tmp.path(),
        &["extract", r, "--code-only", "--max-workers", "0"],
        &[],
    );
    assert_eq!(out.status.code(), Some(2));
    let elsewhere = tmp.path().join("elsewhere");
    ok(
        tmp.path(),
        &[
            "extract",
            r,
            "--code-only",
            "--no-viz",
            "--output",
            elsewhere.to_str().unwrap(),
        ],
        &[],
    );
    assert!(elsewhere.join("dijkstral-out/graph.json").is_file());
}

#[test]
fn provider_registry() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    assert!(ok(h, &["provider"], &[]).contains("usage"));
    assert_eq!(
        ok(h, &["provider", "list"], &[]).trim(),
        "No custom providers."
    );
    let add = |args: &[&str]| {
        let mut all = vec!["provider", "add"];
        all.extend_from_slice(args);
        run_in(h, &all, &[])
    };
    let out = add(&[
        "gateway",
        "--base-url",
        "https://llm.example.com/v1",
        "--default-model",
        "big-model",
        "--env-key",
        "GATEWAY_API_KEY",
        "--pricing-input",
        "2.5",
        "--pricing-output",
        "10",
    ]);
    assert!(out.status.success(), "{:?}", text(&out));
    // Graphify's file layout, byte for byte (tests/fixtures/inputs/providers/providers.json
    // holds the same entry after a local one).
    let ours = fs::read_to_string(h.join(".dijkstral/providers.json")).unwrap();
    let graphify: Value =
        serde_json::from_slice(&fs::read(fixtures().join("providers/providers.json")).unwrap())
            .unwrap();
    let mut only = serde_json::Map::new();
    only.insert("gateway".into(), graphify["gateway"].clone());
    assert_eq!(
        ours,
        serde_json::to_string_pretty(&Value::Object(only)).unwrap() + "\n"
    );
    // `show` prints Graphify's shape.
    assert_eq!(
        ok(h, &["provider", "show", "gateway"], &[]),
        fs::read_to_string(fixtures().join("providers/show.stdout")).unwrap()
    );

    // A pasted key is refused and never echoed; built-in names and duplicates are refused.
    let out = add(&[
        "leaky",
        "--base-url",
        "https://h/v1",
        "--default-model",
        "m",
        "--env-key",
        "sk-live-0123456789abcdef",
    ]);
    let (_, err) = text(&out);
    assert_eq!(out.status.code(), Some(1));
    assert!(!err.contains("0123456789abcdef"), "{err}");
    for name in ["openai", "gateway", "bad name"] {
        let out = add(&[
            name,
            "--base-url",
            "https://h/v1",
            "--default-model",
            "m",
            "--env-key",
            "K",
        ]);
        assert_eq!(out.status.code(), Some(1), "{name}");
    }
    // Plain http to another machine is accepted with a warning; loopback is not warned about.
    let out = add(&[
        "lan",
        "--base-url",
        "http://192.168.1.2/v1",
        "--default-model",
        "m",
        "--env-key",
        "K",
    ]);
    assert!(out.status.success() && text(&out).1.contains("192.168.1.2"));
    let out = add(&[
        "local",
        "--base-url",
        "http://127.0.0.1:8080/v1",
        "--default-model",
        "m",
        "--env-key",
        "K",
    ]);
    assert!(
        out.status.success() && text(&out).1.is_empty(),
        "{:?}",
        text(&out)
    );

    // Graphify's registry is read, not written.
    fs::create_dir_all(h.join(".graphify")).unwrap();
    fs::copy(
        fixtures().join("providers/providers.json"),
        h.join(".graphify/providers.json"),
    )
    .unwrap();
    let list = ok(h, &["provider", "list"], &[]);
    assert_eq!(
        list,
        "  gateway  (https://llm.example.com/v1)\n  lan  (http://192.168.1.2/v1)\n  local  (http://127.0.0.1:8080/v1)\n  local-llm  (http://127.0.0.1:8080/v1)  [graphify]\n"
    );
    let out = run_in(h, &["provider", "remove", "local-llm"], &[]);
    assert_eq!(out.status.code(), Some(1));
    ok(h, &["provider", "remove", "lan"], &[]);
    assert_eq!(
        run_in(h, &["provider", "remove", "lan"], &[]).status.code(),
        Some(1)
    );
    assert_eq!(
        run_in(h, &["provider", "show", "lan"], &[]).status.code(),
        Some(1)
    );
    assert_eq!(
        fs::read(h.join(".graphify/providers.json")).unwrap(),
        fs::read(fixtures().join("providers/providers.json")).unwrap()
    );
}

/// A local OpenAI-compatible server: records each request's path, Authorization header and
/// body; answers naming requests with `Named <id>` for every listed community and anything else
/// with an empty extraction.
struct FakeServer {
    url: String,
    requests: Arc<Mutex<Vec<(String, String, Value)>>>,
}

impl FakeServer {
    fn start() -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests: Arc<Mutex<Vec<(String, String, Value)>>> = Arc::default();
        let log = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let (mut len, mut auth) = (0usize, String::new());
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    let (k, v) = h.split_once(':').unwrap_or((h, ""));
                    match k.to_ascii_lowercase().as_str() {
                        "content-length" => len = v.trim().parse().unwrap_or(0),
                        "authorization" => auth = v.trim().to_string(),
                        _ => {}
                    }
                }
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let prompt = body["messages"]
                    .as_array()
                    .map(|m| {
                        m.iter()
                            .filter_map(|m| m["content"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                let content = if prompt.to_lowercase().contains("communit") {
                    let names: serde_json::Map<String, Value> = prompt
                        .lines()
                        .filter_map(|l| l.split_once(": "))
                        .filter(|(id, _)| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
                        .map(|(id, _)| (id.to_string(), Value::from(format!("Named {id}"))))
                        .collect();
                    Value::Object(names).to_string()
                } else {
                    r#"{"nodes": [], "edges": [], "hyperedges": []}"#.to_string()
                };
                log.lock().unwrap().push((path, auth, body));
                let reply = serde_json::json!({
                    "choices": [{"message": {"role": "assistant", "content": content},
                                 "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 5},
                })
                .to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
            }
        });
        FakeServer { url, requests }
    }

    fn take(&self) -> Vec<(String, String, Value)> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

fn labels(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join("dijkstral-out/.graphify_labels.json")).unwrap())
        .unwrap()
}

#[test]
fn custom_provider_as_backend() {
    let server = FakeServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path();
    let key = [("FAKEP_KEY", "fake-key-123")];
    ok(
        h,
        &[
            "provider",
            "add",
            "fakep",
            "--base-url",
            &server.url,
            "--default-model",
            "fake-model",
            "--env-key",
            "FAKEP_KEY",
        ],
        &[],
    );
    let root = h.join("ws");
    copy_dir(&fixtures().join("cargo"), &root);
    let r = root.to_str().unwrap();
    ok(h, &["extract", r, "--code-only", "--no-viz"], &[]);

    // Without its key the provider is refused, naming only the variable.
    let out = run_in(h, &["label", r, "--backend", "fakep"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out).1.contains("FAKEP_KEY"));
    let out = run_in(h, &["label", r, "--backend", "nope"], &key);
    assert!(text(&out).1.contains("fakep"), "{:?}", text(&out));

    ok(
        h,
        &["label", r, "--backend", "fakep", "--max-concurrency", "1"],
        &key,
    );
    let reqs = server.take();
    assert_eq!(reqs.len(), 1);
    let (path, auth, body) = &reqs[0];
    assert_eq!(path, "/v1/chat/completions");
    assert_eq!(auth, "Bearer fake-key-123");
    assert_eq!(body["model"], "fake-model");
    assert_eq!(body["temperature"], 0);
    let communities = labels(&root).as_object().unwrap().len();
    assert!(communities >= 3);
    assert_eq!(labels(&root)["1"], "Named 1");

    // --missing-only: keeps real names, asks for placeholders and missing ones only.
    let mut saved = labels(&root);
    saved["0"] = "Kept Zero".into();
    saved["1"] = "Community 1".into();
    saved.as_object_mut().unwrap().remove("2");
    fs::write(
        root.join("dijkstral-out/.graphify_labels.json"),
        saved.to_string(),
    )
    .unwrap();
    ok(
        h,
        &[
            "label",
            r,
            "--backend",
            "fakep",
            "--missing-only",
            "--model",
            "other",
        ],
        &key,
    );
    let reqs = server.take();
    assert_eq!(reqs.len(), 1);
    let prompt = reqs[0].2["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    let asked: Vec<&str> = prompt
        .lines()
        .filter_map(|l| l.split_once(": ").map(|(id, _)| id))
        .filter(|id| id.chars().all(|c| c.is_ascii_digit()))
        .collect();
    assert_eq!(asked, ["1", "2"], "{prompt}");
    assert_eq!(reqs[0].2["model"], "other");
    assert_eq!(labels(&root)["0"], "Kept Zero");
    assert_eq!(labels(&root)["1"], "Named 1");

    // cluster-only --backend names only what has no saved name: here nothing, so no request.
    ok(h, &["cluster-only", r, "--backend", "fakep"], &key);
    assert!(server.take().is_empty());
    assert_eq!(labels(&root)["0"], "Kept Zero");
    // Without --backend (or with --no-label) no model is asked.
    ok(
        h,
        &[
            "cluster-only",
            r,
            "--no-label",
            "--backend",
            "fakep",
            "--no-viz",
        ],
        &key,
    );
    assert!(server.take().is_empty());

    // The semantic pass picks the provider automatically when only its key is set.
    fs::write(
        root.join("NOTES.md"),
        "# Notes\n\nThe Widget talks to the Gadget.\n",
    )
    .unwrap();
    let out = run_in(h, &["extract", r, "--no-viz"], &key);
    let reqs = server.take();
    assert!(!reqs.is_empty(), "{:?}", text(&out));
    assert_eq!(reqs[0].1, "Bearer fake-key-123");
    let (stdout, stderr) = text(&out);
    assert!(!stdout.contains("fake-key-123") && !stderr.contains("fake-key-123"));
}

#[cfg(unix)]
#[test]
fn google_workspace_shortcuts_are_exported_through_gws() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path();
    let root = h.join("c");
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("a.py"), "def f():\n    return 1\n").unwrap();
    fs::write(
        root.join("plan.gdoc"),
        r#"{"url": "https://docs.google.com/document/d/DOCID123/edit", "doc_id": "DOCID123", "email": "someone@example.com"}"#,
    )
    .unwrap();
    fs::write(
        root.join("sub/deck.gslides"),
        r#"{"url": "https://docs.google.com/presentation/d/SLIDES7/edit"}"#,
    )
    .unwrap();
    fs::write(root.join("sub/broken.gdoc"), "not json").unwrap();
    let log = h.join("gws.log");
    let gws = h.join("gws");
    fs::write(
        &gws,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nout=\"\"\nwhile [ $# -gt 0 ]; do [ \"$1\" = \"-o\" ] && out=\"$2\"; shift; done\n\
             case \"$out\" in *.md) printf '# Plan\\n\\nThe Widget uses the Gadget.\\n' > \"$out\";; *.txt) printf 'Roadmap\\n' > \"$out\";; esac\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&gws, fs::Permissions::from_mode(0o755)).unwrap();
    let r = root.to_str().unwrap();
    let env = [("DIJKSTRAL_GWS", gws.to_str().unwrap())];

    // --code-only sends nothing to a model, so nothing is exported.
    ok(
        h,
        &[
            "extract",
            r,
            "--code-only",
            "--no-viz",
            "--google-workspace",
        ],
        &env,
    );
    assert!(!log.exists());

    let out = run_in(
        h,
        &[
            "extract",
            r,
            "--no-viz",
            "--google-workspace",
            "--backend",
            "handoff",
        ],
        &env,
    );
    let (stdout, stderr) = text(&out);
    let calls = fs::read_to_string(&log).unwrap();
    // In path order, one call per shortcut, run in the converted directory (rule G3).
    let expected = format!(
        "drive files export --params {{\"fileId\": \"DOCID123\", \"mimeType\": \"text/markdown\"}} -o .{}.export.md\n\
         drive files export --params {{\"fileId\": \"SLIDES7\", \"mimeType\": \"text/plain\"}} -o .{}.export.txt\n",
        sha256_hex("plan.gdoc"),
        sha256_hex("sub/deck.gslides")
    );
    assert_eq!(calls, expected, "{stdout}{stderr}");
    let converted = root.join("dijkstral-out/converted");
    let plan = fs::read_to_string(converted.join("plan_4ae9f699.md")).unwrap();
    assert!(
        plan.starts_with("---\nsource_file: \"plan.gdoc\"\nsource_type: \"google_workspace\"\n")
    );
    assert!(plan.contains("google_account_hash: \"72497f475e4f\"\n"));
    assert!(plan.ends_with(
        "<!-- converted from Google Workspace shortcut: plan.gdoc -->\n\n# Plan\n\nThe Widget uses the Gadget.\n"
    ));
    let deck = dijkstral_semantic::ingest::converted_name("sub/deck.gslides");
    assert!(fs::read_to_string(converted.join(deck))
        .unwrap()
        .ends_with("deck.gslides -->\n\nRoadmap\n"));
    assert!(
        stdout.contains("sub/broken.gdoc") || stderr.contains("sub/broken.gdoc"),
        "{stdout}{stderr}"
    );
    let pending: String = fs::read_dir(root.join("dijkstral-out/semantic/pending"))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert!(
        pending.contains("dijkstral-out/converted/plan_4ae9f699.md"),
        "{pending}"
    );
}

fn sha256_hex(s: &str) -> String {
    dijkstral_semantic::ingest::sha256_hex(s.as_bytes())
}

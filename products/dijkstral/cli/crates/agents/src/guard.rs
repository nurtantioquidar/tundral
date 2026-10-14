//! `dijkstral hook-guard search|read [--strict]`: the Claude Code `PreToolUse` guard
//! (`docs/graphify-query.md` §11).
//!
//! Reads the hook payload, and when the project has a graph, adds a short note to the agent's
//! context steering a search or a source read toward `dijkstral query`. It fails open: anything it
//! cannot parse or decide is allowed silently, the exit code is always 0 and nothing is written to
//! stderr. Allow is empty stdout. It never opens the graph; the decision costs a JSON parse and at
//! most a handful of `stat` calls.

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value};

use crate::bash::runs_search;
use crate::text::{READ_NUDGE, SEARCH_NUDGE, STALE_NOTE, STRICT_DENY};
use crate::{DIGEST_FILE, GRAPH_FILE, NEEDS_UPDATE_FILE, OUT_DIR};

/// Which tools a guard instance handles: `search` is `Grep` and `Bash`, `read` is `Read` and
/// `Glob`. Each is wired to its own matcher by the installer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Search,
    Read,
}

/// The guard's verdict on one tool call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Let the call through with no output.
    Allow,
    /// Let the call through and add this note to the agent's context.
    Context(&'static str),
    /// Block the call; the reason is shown to the agent.
    Deny(&'static str),
}

impl Outcome {
    /// The stdout line for this outcome: nothing for `Allow`, else one compact JSON object and a
    /// newline, in Claude Code's `hookSpecificOutput` shape for `PreToolUse`.
    pub fn to_stdout(self) -> Option<String> {
        let mut out = Map::new();
        out.insert("hookEventName".into(), "PreToolUse".into());
        match self {
            Outcome::Allow => return None,
            Outcome::Context(text) => {
                out.insert("additionalContext".into(), text.into());
            }
            Outcome::Deny(text) => {
                out.insert("permissionDecision".into(), "deny".into());
                out.insert("permissionDecisionReason".into(), text.into());
            }
        }
        let mut top = Map::new();
        top.insert("hookSpecificOutput".into(), Value::Object(out));
        let mut line = Value::Object(top).to_string();
        line.push('\n');
        Some(line)
    }
}

/// Parses the arguments after `hook-guard`: the mode, then an optional `--strict` anywhere after
/// it. Anything else after the mode is ignored; a missing or unknown mode (including a leading
/// `--strict`) yields `None`, which the caller treats as "allow".
pub fn parse_args<I, S>(args: I) -> Option<(Mode, bool)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut args = args.into_iter();
    let mode = match args.next()?.as_ref().to_str()? {
        "search" => Mode::Search,
        "read" => Mode::Read,
        _ => return None,
    };
    let strict = args.any(|a| a.as_ref() == "--strict");
    Some((mode, strict))
}

/// Decides one tool call. `cwd` is the hook process's working directory, which Claude Code sets to
/// the session's project; the payload's own `cwd` field is not consulted, matching Graphify.
pub fn decide(mode: Mode, strict: bool, payload: &[u8], cwd: &Path) -> Outcome {
    let Ok(Value::Object(call)) = serde_json::from_slice::<Value>(payload) else {
        return Outcome::Allow;
    };
    let tool = call.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let input = call.get("tool_input").and_then(Value::as_object);
    let field = |name: &str| {
        input
            .and_then(|i| i.get(name))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let out_dir = cwd.join(OUT_DIR);
    match (mode, tool) {
        (Mode::Search, "Grep") => {
            if field("pattern").is_some() && graph_mtime(&out_dir).is_some() {
                return Outcome::Context(SEARCH_NUDGE);
            }
        }
        (Mode::Search, "Bash") => {
            if field("command").is_some_and(runs_search) && graph_mtime(&out_dir).is_some() {
                return Outcome::Context(SEARCH_NUDGE);
            }
        }
        (Mode::Read, "Read") => {
            if let Some(path) = field("file_path") {
                return decide_read(path, strict, &call, cwd, &out_dir);
            }
        }
        (Mode::Read, "Glob") => {
            if let Some(pattern) = field("pattern") {
                let in_project = field("path").is_none_or(|p| within(cwd, &absolute(cwd, p)));
                if in_project && glob_targets_code(pattern, cwd) && graph_mtime(&out_dir).is_some()
                {
                    return Outcome::Context(READ_NUDGE);
                }
            }
        }
        _ => {}
    }
    Outcome::Allow
}

fn decide_read(
    path: &str,
    strict: bool,
    call: &Map<String, Value>,
    cwd: &Path,
    out_dir: &Path,
) -> Outcome {
    if !is_code_extension(Path::new(path).extension().and_then(|e| e.to_str())) {
        return Outcome::Allow;
    }
    let target = absolute(cwd, path);
    let Some(rel) = relative_to(cwd, &target) else {
        return Outcome::Allow;
    };
    if rel.components().next() == Some(Component::Normal(OUT_DIR.as_ref())) {
        return Outcome::Allow;
    }
    let Some(graph_time) = graph_mtime(out_dir) else {
        return Outcome::Allow;
    };
    if is_stale(&target, graph_time, out_dir) {
        return Outcome::Context(STALE_NOTE);
    }
    if strict {
        if let Some(sid) = session_key(call.get("session_id")) {
            if first_read_this_session(out_dir, &sid) {
                return Outcome::Deny(STRICT_DENY);
            }
        }
    }
    Outcome::Context(READ_NUDGE)
}

/// Extensions whose reads (and globs) get a nudge: Graphify's observed set (§11.4, including the
/// prose formats its `update` indexes structurally) plus the code extensions it leaves silent even
/// though both tools index them (`mjs`, `mts`, `cts`, `pyi`, `bash`, `zsh`).
const CODE_EXTENSIONS: &[&str] = &[
    "py", "pyi", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "rs", "go", "java", "kt",
    "scala", "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php", "swift", "lua", "vue", "svelte",
    "astro", "sh", "bash", "zsh", "ps1", "md", "mdx", "rst", "txt",
];

fn is_code_extension(ext: Option<&str>) -> bool {
    ext.is_some_and(|e| {
        CODE_EXTENSIONS
            .iter()
            .any(|known| known.eq_ignore_ascii_case(e))
    })
}

/// A glob aims at source files when the extension of its last component is a code extension, or
/// a brace list (`*.{ts,tsx}`) containing one. An absolute pattern must also point into the
/// project.
fn glob_targets_code(pattern: &str, cwd: &Path) -> bool {
    if pattern.starts_with('/') {
        let literal: PathBuf = Path::new(pattern)
            .components()
            .take_while(|c| {
                !c.as_os_str()
                    .to_string_lossy()
                    .contains(['*', '?', '[', '{'])
            })
            .collect();
        if !within(cwd, &literal) {
            return false;
        }
    }
    let last = pattern.rsplit('/').next().unwrap_or(pattern);
    let Some((_, ext)) = last.rsplit_once('.') else {
        return false;
    };
    match ext.strip_prefix('{').and_then(|e| e.strip_suffix('}')) {
        Some(list) => list.split(',').any(|e| is_code_extension(Some(e))),
        None => is_code_extension(Some(ext)),
    }
}

/// Modification time of `dijkstral-out/graph.json` when it is a regular file.
fn graph_mtime(out_dir: &Path) -> Option<SystemTime> {
    let meta = fs::metadata(out_dir.join(GRAPH_FILE)).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH))
}

/// The graph is stale for `target` when a `needs_update` marker exists, or when the file was
/// modified after the graph was last written or confirmed current by an unchanged rebuild (which
/// touches the build digest). A file that does not exist is never stale.
fn is_stale(target: &Path, graph_time: SystemTime, out_dir: &Path) -> bool {
    if fs::symlink_metadata(out_dir.join(NEEDS_UPDATE_FILE)).is_ok() {
        return true;
    }
    let Some(file_time) = fs::metadata(target).ok().and_then(|m| m.modified().ok()) else {
        return false;
    };
    if file_time <= graph_time {
        return false;
    }
    let confirmed = fs::metadata(out_dir.join(DIGEST_FILE))
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    file_time > confirmed
}

/// The session id as a file-name-safe key: strings and numbers as written, every character
/// outside `[A-Za-z0-9_-]` replaced by `_`, at most 64 characters. Missing, empty or other types
/// give `None` (strict mode then never denies).
fn session_key(id: Option<&Value>) -> Option<String> {
    let raw = match id? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let key: String = raw
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    (!key.is_empty()).then_some(key)
}

/// Records the strict-mode denial for this session; true only the first time. Fails open: when
/// the marker cannot be written the read is not denied, so a read-only checkout is never blocked
/// on every call.
fn first_read_this_session(out_dir: &Path, sid: &str) -> bool {
    let dir = out_dir.join("cache").join("hook_sessions");
    let marker = dir.join(format!("{sid}.denied"));
    let create = || {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
    };
    match create() {
        Ok(_) => true,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => false,
        Err(_) => fs::create_dir_all(&dir).is_ok() && create().is_ok(),
    }
}

/// `path` made absolute against `cwd` (with `~/` expanded) and lexically normalised.
fn absolute(cwd: &Path, path: &str) -> PathBuf {
    let joined = match path.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => Path::new(&home).join(rest),
            None => cwd.join(path),
        },
        None => cwd.join(path),
    };
    normalize(&joined)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn within(cwd: &Path, target: &Path) -> bool {
    relative_to(cwd, target).is_some()
}

/// `target` relative to `cwd` when it lies inside it. Tries the lexical form first; only a path
/// that looks outside is resolved through symlinks (e.g. `/tmp` versus `/private/tmp`).
fn relative_to(cwd: &Path, target: &Path) -> Option<PathBuf> {
    if let Ok(rel) = target.strip_prefix(cwd) {
        return Some(rel.to_path_buf());
    }
    let real_cwd = fs::canonicalize(cwd).ok()?;
    // Resolve the deepest existing ancestor, then re-append the rest (the file may not exist).
    let mut existing = target;
    let mut rest = Vec::new();
    let real = loop {
        if let Ok(real) = fs::canonicalize(existing) {
            break real;
        }
        rest.push(existing.file_name()?);
        existing = existing.parent()?;
    };
    let full = rest.iter().rev().fold(real, |p, name| p.join(name));
    full.strip_prefix(&real_cwd).ok().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    struct Project {
        dir: tempfile::TempDir,
    }

    impl Project {
        fn new(with_graph: bool) -> Project {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("pkg")).unwrap();
            fs::write(dir.path().join("pkg/service.py"), "x = 1\n").unwrap();
            if with_graph {
                fs::create_dir_all(dir.path().join(OUT_DIR)).unwrap();
                fs::write(dir.path().join(OUT_DIR).join(GRAPH_FILE), "{}").unwrap();
                // Make the graph clearly newer than the source written above.
                set_mtime(&dir.path().join("pkg/service.py"), -60);
            }
            Project { dir }
        }

        fn root(&self) -> PathBuf {
            fs::canonicalize(self.dir.path()).unwrap()
        }

        fn run(&self, mode: Mode, strict: bool, payload: &Value) -> Outcome {
            decide(mode, strict, payload.to_string().as_bytes(), &self.root())
        }
    }

    fn set_mtime(path: &Path, offset_secs: i64) {
        let now = SystemTime::now();
        let t = if offset_secs < 0 {
            now - Duration::from_secs(offset_secs.unsigned_abs())
        } else {
            now + Duration::from_secs(offset_secs as u64)
        };
        let f = OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(t).unwrap();
    }

    fn call(tool: &str, input: Value) -> Value {
        json!({"session_id": "s1", "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input})
    }

    const SEARCH: Outcome = Outcome::Context(SEARCH_NUDGE);
    const READ: Outcome = Outcome::Context(READ_NUDGE);
    const STALE: Outcome = Outcome::Context(STALE_NOTE);
    const ALLOW: Outcome = Outcome::Allow;

    #[test]
    fn output_shapes_are_compact_hook_json() {
        assert_eq!(Outcome::Allow.to_stdout(), None);
        let ctx = Outcome::Context("a — b").to_stdout().unwrap();
        assert_eq!(
            ctx,
            "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"additionalContext\":\"a — b\"}}\n"
        );
        let deny = Outcome::Deny("no").to_stdout().unwrap();
        assert_eq!(
            deny,
            "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"no\"}}\n"
        );
    }

    #[test]
    fn args() {
        assert_eq!(parse_args(["search"]), Some((Mode::Search, false)));
        assert_eq!(parse_args(["read", "--strict"]), Some((Mode::Read, true)));
        assert_eq!(parse_args(["search", "extra"]), Some((Mode::Search, false)));
        assert_eq!(parse_args(["--strict", "search"]), None);
        assert_eq!(parse_args(["bogus"]), None);
        assert_eq!(parse_args(Vec::<String>::new()), None);
    }

    #[test]
    fn malformed_input_fails_open() {
        let p = Project::new(true);
        for raw in [
            &b""[..],
            b"not json",
            b"[]",
            b"null",
            b"{}",
            b"\xff\xfe{",
            b"{\"tool_name\":\"Grep\"}",
            b"{\"tool_name\":\"Grep\",\"tool_input\":\"rg x\"}",
            b"{\"tool_name\":\"Grep\",\"tool_input\":{\"pattern\":5}}",
            b"{\"tool_name\":\"Read\",\"tool_input\":{\"file_path\":null}}",
        ] {
            for mode in [Mode::Search, Mode::Read] {
                assert_eq!(decide(mode, true, raw, &p.root()), ALLOW, "{raw:?}");
            }
        }
    }

    #[test]
    fn search_mode() {
        let p = Project::new(true);
        let m = Mode::Search;
        assert_eq!(
            p.run(m, false, &call("Grep", json!({"pattern": "foo"}))),
            SEARCH
        );
        assert_eq!(
            p.run(
                m,
                false,
                &call("Grep", json!({"pattern": "foo", "path": "/etc"}))
            ),
            SEARCH
        );
        assert_eq!(
            p.run(m, false, &call("Grep", json!({"pattern": ""}))),
            ALLOW
        );
        assert_eq!(p.run(m, false, &call("Grep", json!({}))), ALLOW);
        assert_eq!(
            p.run(m, false, &call("Bash", json!({"command": "rg foo src"}))),
            SEARCH
        );
        assert_eq!(
            p.run(m, false, &call("Bash", json!({"command": "git status"}))),
            ALLOW
        );
        // Other tools are the read guard's business; strict never denies a search.
        assert_eq!(
            p.run(
                m,
                true,
                &call("Read", json!({"file_path": "pkg/service.py"}))
            ),
            ALLOW
        );
        assert_eq!(
            p.run(m, true, &call("Grep", json!({"pattern": "foo"}))),
            SEARCH
        );
        // The payload's hook_event_name and cwd are not consulted.
        let mut odd = call("Grep", json!({"pattern": "x"}));
        odd["hook_event_name"] = json!("PostToolUse");
        odd["cwd"] = json!("/nowhere");
        assert_eq!(p.run(m, false, &odd), SEARCH);
    }

    #[test]
    fn silent_without_a_graph() {
        let p = Project::new(false);
        assert_eq!(
            p.run(
                Mode::Search,
                false,
                &call("Grep", json!({"pattern": "foo"}))
            ),
            ALLOW
        );
        assert_eq!(
            p.run(
                Mode::Search,
                false,
                &call("Bash", json!({"command": "rg x"}))
            ),
            ALLOW
        );
        let read = call("Read", json!({"file_path": "pkg/service.py"}));
        assert_eq!(p.run(Mode::Read, true, &read), ALLOW);
        assert_eq!(
            p.run(
                Mode::Read,
                false,
                &call("Glob", json!({"pattern": "**/*.py"}))
            ),
            ALLOW
        );
        // A directory where the graph should be is not a graph.
        fs::create_dir_all(p.root().join(OUT_DIR).join(GRAPH_FILE)).unwrap();
        assert_eq!(p.run(Mode::Read, true, &read), ALLOW);
        // No upward search: a subdirectory of a project with a graph is silent.
        let q = Project::new(true);
        let sub = q.root().join("pkg");
        let payload = call("Grep", json!({"pattern": "x"})).to_string();
        assert_eq!(decide(Mode::Search, false, payload.as_bytes(), &sub), ALLOW);
    }

    #[test]
    fn read_mode_paths_and_extensions() {
        let p = Project::new(true);
        let root = p.root();
        let read = |path: &str| p.run(Mode::Read, false, &call("Read", json!({"file_path": path})));
        for path in [
            "pkg/service.py",
            "./pkg/service.py",
            "pkg/missing.py",
            "x.PY",
            "a.d.ts",
            "a.test.py",
            "a.md.txt",
            "CMakeLists.txt",
            "lib/a.mjs",
            "types.pyi",
            "node_modules/x.js",
            "README.md",
        ] {
            assert_eq!(read(path), READ, "{path}");
        }
        assert_eq!(read(&root.join("pkg/service.py").to_string_lossy()), READ);
        for path in [
            "a.py.bak",
            "a.tar.gz",
            "package.json",
            "Makefile",
            ".env",
            ".bashrc",
            "/etc/hosts",
            "/etc/x.py",
            "../elsewhere/main.py",
            "dijkstral-out/GRAPH_REPORT.md",
        ] {
            assert_eq!(read(path), ALLOW, "{path}");
        }
        let escape = format!("{}/../nograph/main.py", root.display());
        assert_eq!(read(&escape), ALLOW);
    }

    #[test]
    fn glob_mode() {
        let p = Project::new(true);
        let glob = |input: Value| p.run(Mode::Read, true, &call("Glob", input));
        for pattern in [
            "**/*.py",
            "*.py",
            "**/*.md",
            "**/*.PY",
            "**/service.py",
            "pkg/service.py",
            "**/*.{ts,tsx}",
        ] {
            assert_eq!(glob(json!({"pattern": pattern})), READ, "{pattern}");
        }
        for pattern in [
            "**/*",
            "*",
            "src/**",
            "pkg/*",
            "**/*.json",
            "**/test_*",
            "**/*.py*",
            "**/*.py,**/*.json",
            "",
            "/etc/**/*.py",
        ] {
            assert_eq!(glob(json!({"pattern": pattern})), ALLOW, "{pattern}");
        }
        assert_eq!(glob(json!({"pattern": "**/*.py", "path": "/etc"})), ALLOW);
        assert_eq!(glob(json!({"pattern": "**/*.py", "path": "pkg"})), READ);
        let abs = format!("{}/pkg/**/*.py", p.root().display());
        assert_eq!(glob(json!({"pattern": abs})), READ);
    }

    #[test]
    fn staleness() {
        let p = Project::new(true);
        let root = p.root();
        let read = |path: &str, strict: bool| {
            p.run(
                Mode::Read,
                strict,
                &call("Read", json!({"file_path": path})),
            )
        };
        fs::write(root.join("pkg/new.py"), "y = 2\n").unwrap();
        set_mtime(&root.join("pkg/new.py"), 60);
        assert_eq!(read("pkg/new.py", false), STALE);
        // Staleness is per file, and strict mode never denies a stale read.
        assert_eq!(read("pkg/service.py", false), READ);
        assert_eq!(read("pkg/new.py", true), STALE);
        // An unchanged rebuild touches the digest, which confirms the graph is current.
        let digest = root.join(OUT_DIR).join(DIGEST_FILE);
        fs::write(&digest, "d").unwrap();
        set_mtime(&digest, 120);
        assert_eq!(read("pkg/new.py", false), READ);
        // needs_update makes every read stale, existing file or not.
        fs::write(root.join(OUT_DIR).join(NEEDS_UPDATE_FILE), "").unwrap();
        assert_eq!(read("pkg/service.py", false), STALE);
        // Glob and search ignore staleness.
        assert_eq!(
            p.run(
                Mode::Read,
                false,
                &call("Glob", json!({"pattern": "**/*.py"}))
            ),
            READ
        );
        assert_eq!(
            p.run(Mode::Search, false, &call("Grep", json!({"pattern": "x"}))),
            SEARCH
        );
    }

    #[test]
    fn strict_denies_once_per_session() {
        let p = Project::new(true);
        let root = p.root();
        let read = |sid: Value| {
            let mut c = call("Read", json!({"file_path": "pkg/service.py"}));
            c["session_id"] = sid;
            p.run(Mode::Read, true, &c)
        };
        assert_eq!(read(json!("abc")), Outcome::Deny(STRICT_DENY));
        assert_eq!(read(json!("abc")), READ);
        assert_eq!(read(json!("other")), Outcome::Deny(STRICT_DENY));
        assert_eq!(read(json!(5)), Outcome::Deny(STRICT_DENY));
        assert_eq!(read(json!("../escape")), Outcome::Deny(STRICT_DENY));
        assert_eq!(read(json!("")), READ);
        assert_eq!(read(Value::Null), READ);
        let sessions = root.join(OUT_DIR).join("cache/hook_sessions");
        let mut names: Vec<String> = fs::read_dir(&sessions)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["5.denied", "___escape.denied", "abc.denied", "other.denied"]
        );
        // Without --strict the same call only nudges.
        let c = call("Read", json!({"file_path": "pkg/service.py"}));
        assert_eq!(p.run(Mode::Read, false, &c), READ);
    }

    #[test]
    fn session_keys() {
        assert_eq!(session_key(Some(&json!("a/b"))).as_deref(), Some("a_b"));
        assert_eq!(
            session_key(Some(&json!(".hidden"))).as_deref(),
            Some("_hidden")
        );
        assert_eq!(
            session_key(Some(&json!("é-ab_C9"))).as_deref(),
            Some("_-ab_C9")
        );
        let long = "x".repeat(100);
        assert_eq!(session_key(Some(&json!(long))).unwrap().len(), 64);
        assert_eq!(session_key(Some(&json!(true))), None);
        assert_eq!(session_key(None), None);
    }

    /// In-process cost of one decision, without process start-up:
    /// `cargo test --release -p dijkstral-agents decision_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn decision_cost() {
        let p = Project::new(true);
        let root = p.root();
        let service = root.join("pkg/service.py").display().to_string();
        let payloads = [
            (
                Mode::Search,
                call(
                    "Bash",
                    json!({"command": "cd pkg && rg -n 'def ' . | head"}),
                ),
            ),
            (
                Mode::Search,
                call("Grep", json!({"pattern": "class .*Service"})),
            ),
            (Mode::Read, call("Read", json!({"file_path": service}))),
            (Mode::Read, call("Glob", json!({"pattern": "**/*.py"}))),
        ];
        for (mode, payload) in payloads {
            let bytes = payload.to_string().into_bytes();
            let n = 20_000;
            let start = std::time::Instant::now();
            for _ in 0..n {
                std::hint::black_box(decide(mode, false, &bytes, &root).to_stdout());
            }
            let per = start.elapsed().as_secs_f64() * 1e6 / n as f64;
            println!("{:?} {}: {per:.1} µs/decision", mode, payload["tool_name"]);
        }
    }
}
